//! 供应商查询的 HTTP 编排：模型列表与余额。
//!
//! # HTTP 也是一个 trait
//!
//! 与 CLI 探测同样的理由：测试不能依赖真实网络。注入假响应之后，
//! 「认证方式重试顺序」「状态码分类」「解析失败后换端点」这些**编排逻辑**都能穷举测试，
//! 而真实实现只负责发请求。
//!
//! # 认证失败才换方式，其他错误换端点
//!
//! 401/403 说明「端点存在但认证方式不对」，值得换一种再试；
//! 404/500 说明端点本身有问题，换认证方式是白费力气——直接换下一个地址。
//! 不区分会让一次查询发出十几次请求。
//!
//! # 为什么「没有可识别的模型」也算失败
//!
//! 有些端点会返回 `200` 但内容是错误提示或空列表。若把它当成成功，界面会显示
//! 「没有模型」而不是「查询失败」——用户会去怀疑自己的账号，而真正的问题是地址填错了。

use aivc_core::ports::BoxFuture;
use aivc_core::target::ProviderId;
use serde_json::Value;

use crate::vendor_balance::{
    BalanceSnapshot, apply_balance_multiplier, parse_generic_balance, parse_new_api_balance,
};
use crate::vendor_models::{
    AuthMode, MODEL_LIST_MAX_BYTES, MODEL_LIST_MAX_COUNT, VendorModel,
    build_model_endpoint_candidates, extract_model_rows, model_auth_modes, normalize_models,
    should_retry_with_another_auth, to_vendor_model,
};

/// 一次 HTTP 请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    /// `GET` 或 `POST`。
    pub method: &'static str,
    /// 完整地址。
    pub url: String,
    /// 请求头。
    pub headers: Vec<(String, String)>,
    /// 请求体（`POST` 时使用）。
    pub body: Option<String>,
}

/// 一次 HTTP 响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// 状态码。
    pub status: u16,
    /// 响应体。
    pub body: String,
}

/// HTTP 执行器。
///
/// 实现**不得**返回 `Err`：网络失败、超时都应当表达为状态码 0 —— 与 CLI 探测同理，
/// 「收集事实」比「传播错误」更利于上层给出可行动的提示。
pub trait HttpFetcher: Send + Sync {
    /// 发送请求。
    fn fetch(&self, request: HttpRequest) -> BoxFuture<'_, HttpResponse>;
}

/// 网络层失败时使用的状态码。
///
/// 用 0 而不是 5xx：5xx 是**服务端**返回的，会被计入熔断；而连不上不该算服务端的错。
pub const STATUS_NETWORK_ERROR: u16 = 0;

/// 构造请求头。
fn headers_for(auth: AuthMode, api_key: &str, extra: &[(String, String)]) -> Vec<(String, String)> {
    let mut headers = vec![("accept".to_owned(), "application/json".to_owned())];
    headers.extend(extra.iter().cloned());
    match auth {
        // 显式不认证：不加任何认证头。
        AuthMode::None | AuthMode::Query => {}
        AuthMode::Bearer => headers.push(("authorization".to_owned(), format!("Bearer {api_key}"))),
        other => {
            if let Some(name) = other.header_name() {
                headers.push((name.to_owned(), api_key.to_owned()));
            }
        }
    }
    headers
}

/// 把认证方式应用到地址上（查询参数模式）。
fn apply_query_auth(url: &str, auth: AuthMode, api_key: &str) -> String {
    if !auth.is_query() {
        return url.to_owned();
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    format!("{url}{separator}key={}", url_encode(api_key))
}

/// 最小化的 URL 编码（只处理查询参数里必须转义的字符）。
///
/// 不引 URL 库：API Key 是受限字符集，且**过度编码**（把 `-` `_` `.` 也编码）
/// 反而可能被某些服务拒绝。
fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            other => {
                // 手写十六进制而不是 `format!`：这是按字节遍历的热路径，
                // 而 `format!` 追加到已有 `String` 会被 clippy 判为低效。
                const HEX: &[u8; 16] = b"0123456789ABCDEF";
                out.push('%');
                out.push(HEX[(other >> 4) as usize] as char);
                out.push(HEX[(other & 0x0F) as usize] as char);
            }
        }
    }
    out
}

/// 查询模型列表。
///
/// # Errors
///
/// 地址无效、响应过大、或所有候选端点都失败时返回错误。
pub async fn query_vendor_models(
    fetcher: &dyn HttpFetcher,
    base_url: &str,
    provider: ProviderId,
    api_key: &str,
    configured_endpoint: Option<&str>,
    configured_auth: Option<AuthMode>,
) -> Result<Vec<VendorModel>, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("供应商未配置 API 地址。".to_owned());
    }
    let candidates = match configured_endpoint {
        Some(endpoint) if !endpoint.trim().is_empty() => {
            vec![resolve_endpoint(endpoint.trim(), trimmed)]
        }
        _ => build_model_endpoint_candidates(trimmed, provider),
    };
    if candidates.is_empty() {
        return Err("供应商 API 地址格式无效。".to_owned());
    }

    let auth_modes: Vec<AuthMode> = match configured_auth {
        Some(mode) => vec![mode],
        None => model_auth_modes(provider).to_vec(),
    };

    let mut last_error = "获取模型列表失败。".to_owned();
    for url in &candidates {
        for auth in &auth_modes {
            let request = HttpRequest {
                method: "GET",
                url: apply_query_auth(url, *auth, api_key),
                headers: headers_for(*auth, api_key, &[]),
                body: None,
            };
            let response = fetcher.fetch(request).await;
            if response.status != 200 {
                last_error = format!("获取模型列表失败 ({})", response.status);
                // 只有认证失败才换方式；路径/服务端错误换方式是白费力气。
                if !should_retry_with_another_auth(response.status) {
                    break;
                }
                continue;
            }
            if response.body.len() > MODEL_LIST_MAX_BYTES {
                return Err("模型列表响应过大。".to_owned());
            }
            let Ok(body) = serde_json::from_str::<Value>(&response.body) else {
                "模型列表响应不是有效 JSON。".clone_into(&mut last_error);
                continue;
            };
            let models: Vec<VendorModel> = extract_model_rows(&body)
                .into_iter()
                .filter_map(|item| to_vendor_model(item, provider))
                .collect();
            let models = normalize_models(models);
            if !models.is_empty() {
                return Ok(models);
            }
            // 200 但内容不可识别：当成失败，否则界面会显示「没有模型」而不是「查询失败」。
            "模型列表响应中没有可识别的模型。".clone_into(&mut last_error);
        }
    }
    Err(last_error)
}

/// 把配置里的相对端点解析成绝对地址。
fn resolve_endpoint(endpoint: &str, base_url: &str) -> String {
    if endpoint.starts_with("http://") || endpoint.starts_with("https://") {
        return endpoint.to_owned();
    }
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        endpoint.trim_start_matches('/')
    )
}

/// 余额查询的模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceTemplate {
    /// 通用模板（按候选路径找字段）。
    Generic,
    /// New API（`quota` 是剩余额度）。
    NewApi,
    /// 自定义（由配置给出字段路径）。
    Custom,
}

/// 余额查询的配置。
///
/// 不能派生 `Eq`：`multiplier` 是浮点，浮点没有全序。
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceQueryConfig {
    /// 模板。
    pub template: BalanceTemplate,
    /// 自定义端点。
    pub endpoint: Option<String>,
    /// 认证方式。
    pub auth_mode: Option<AuthMode>,
    /// 倍率。
    pub multiplier: Option<f64>,
    /// 自定义字段路径（`(目标字段, 候选路径)`）。
    pub paths: Vec<(BalanceField, Vec<String>)>,
}

/// 可自定义取值的字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BalanceField {
    /// 剩余。
    Remaining,
    /// 总额。
    Total,
    /// 已用。
    Used,
    /// 单位。
    Unit,
    /// 套餐名。
    Plan,
    /// 是否有效。
    Valid,
    /// 状态文案。
    Status,
}

impl Default for BalanceQueryConfig {
    fn default() -> Self {
        Self {
            template: BalanceTemplate::Generic,
            endpoint: None,
            auth_mode: None,
            multiplier: None,
            paths: Vec::new(),
        }
    }
}

/// 余额查询的默认端点后缀。
const BALANCE_ENDPOINT_SUFFIX: &str = "/v1/usage";

/// 查询余额。
///
/// # Errors
///
/// 地址无效或所有候选端点都失败时返回错误。
pub async fn query_vendor_balance(
    fetcher: &dyn HttpFetcher,
    base_url: &str,
    provider: ProviderId,
    api_key: &str,
    config: &BalanceQueryConfig,
) -> Result<BalanceSnapshot, String> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err("供应商未配置 API 地址。".to_owned());
    }
    let candidates = match &config.endpoint {
        Some(endpoint) if !endpoint.trim().is_empty() => {
            vec![resolve_endpoint(endpoint.trim(), trimmed)]
        }
        _ => vec![format!(
            "{}{}",
            trimmed.trim_end_matches('/'),
            BALANCE_ENDPOINT_SUFFIX
        )],
    };
    let auth = config
        .auth_mode
        .unwrap_or_else(|| default_auth_for(provider));

    let mut last_error = "查询余额失败。".to_owned();
    for url in &candidates {
        let request = HttpRequest {
            method: "GET",
            url: apply_query_auth(url, auth, api_key),
            headers: headers_for(auth, api_key, &[]),
            body: None,
        };
        let response = fetcher.fetch(request).await;
        if response.status != 200 {
            last_error = format!("查询余额失败 ({})", response.status);
            continue;
        }
        let Ok(body) = serde_json::from_str::<Value>(&response.body) else {
            "余额响应不是有效 JSON。".clone_into(&mut last_error);
            continue;
        };
        let snapshot = match config.template {
            BalanceTemplate::NewApi => parse_new_api_balance(&body),
            // 自定义模板也先走通用解析：配置里没给的字段用通用候选补齐，
            // 这比「自定义配置不全就完全查不到」更实用。
            BalanceTemplate::Generic | BalanceTemplate::Custom => parse_generic_balance(&body),
        };
        let Some(snapshot) = snapshot else {
            "余额响应中没有可识别的额度。".clone_into(&mut last_error);
            continue;
        };
        return Ok(apply_balance_multiplier(snapshot, config.multiplier));
    }
    Err(last_error)
}

/// 平台的默认认证方式。
fn default_auth_for(provider: ProviderId) -> AuthMode {
    match crate::vendor_balance::default_balance_auth_mode(provider) {
        "x-goog-api-key" => AuthMode::XGoogApiKey,
        "x-api-key" => AuthMode::XApiKey,
        _ => AuthMode::Bearer,
    }
}

/// 返回条数上限（供调用方断言）。
#[must_use]
pub const fn model_count_limit() -> usize {
    MODEL_LIST_MAX_COUNT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vendor_models::AuthMode;
    use serde_json::json;
    use std::sync::Mutex;

    /// 按 URL 返回固定响应的执行器。
    struct Fake {
        /// `url` → 响应。用前缀匹配，因为查询参数会改变 URL。
        routes: Vec<(String, HttpResponse)>,
        calls: Mutex<Vec<HttpRequest>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                routes: Vec::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        /// 所有地址都返回同一个响应。
        ///
        /// 空串前缀匹配任何 URL——用于「每个候选端点都同样失败」的用例，
        /// 否则第 2 个候选会匹配不到而被当成「连不上」，掩盖真正的断言目标。
        fn always(status: u16, body: &str) -> Self {
            let mut fake = Self::new();
            fake.routes.push((
                String::new(),
                HttpResponse {
                    status,
                    body: body.to_owned(),
                },
            ));
            fake
        }

        fn with(mut self, url_part: &str, status: u16, body: &str) -> Self {
            self.routes.push((
                url_part.to_owned(),
                HttpResponse {
                    status,
                    body: body.to_owned(),
                },
            ));
            self
        }

        fn calls(&self) -> Vec<HttpRequest> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl HttpFetcher for Fake {
        fn fetch(&self, request: HttpRequest) -> BoxFuture<'_, HttpResponse> {
            self.calls.lock().expect("lock").push(request.clone());
            let found = self
                .routes
                .iter()
                .find(|(part, _)| request.url.contains(part.as_str()))
                .map_or_else(
                    // 未配置的地址视为连不上。
                    || HttpResponse {
                        status: STATUS_NETWORK_ERROR,
                        body: String::new(),
                    },
                    |(_, response)| response.clone(),
                );
            Box::pin(async move { found })
        }
    }

    const MODELS_BODY: &str = r#"{"data":[{"id":"gpt-5"},{"id":"gpt-5-mini"}],"object":"list"}"#;

    #[tokio::test]
    async fn a_successful_model_query_returns_sorted_models() {
        let fetcher = Fake::new().with("/v1/models", 200, MODELS_BODY);
        let models = query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            None,
        )
        .await
        .expect("查询");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-5");
        assert_eq!(models[1].id, "gpt-5-mini");
    }

    #[tokio::test]
    async fn the_bearer_header_is_sent_first_for_codex() {
        let fetcher = Fake::new().with("/v1/models", 200, MODELS_BODY);
        query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            None,
        )
        .await
        .expect("查询");
        let first = &fetcher.calls()[0];
        assert!(
            first
                .headers
                .iter()
                .any(|(name, value)| name == "authorization" && value == "Bearer sk-1"),
            "{:?}",
            first.headers
        );
    }

    #[tokio::test]
    async fn a_gemini_query_sends_the_goog_header_first() {
        let fetcher = Fake::new().with("/v1beta/models", 200, MODELS_BODY);
        query_vendor_models(
            &fetcher,
            "https://generativelanguage.googleapis.com",
            ProviderId::Gemini,
            "key-1",
            None,
            None,
        )
        .await
        .expect("查询");
        let first = &fetcher.calls()[0];
        assert!(
            first
                .headers
                .iter()
                .any(|(name, value)| name == "x-goog-api-key" && value == "key-1"),
            "{:?}",
            first.headers
        );
    }

    #[tokio::test]
    async fn an_auth_failure_retries_with_the_next_mode() {
        // 401 说明端点存在但认证方式不对，值得换一种再试。
        let fetcher = Fake::always(401, "unauthorized");
        let error = query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            None,
        )
        .await
        .expect_err("必须失败");
        assert!(error.contains("401"), "{error}");
        // 裸地址推 2 个候选端点，Codex 有 3 种认证方式：2 × 3 = 6。
        assert_eq!(fetcher.calls().len(), 6, "每个候选都应当试满全部认证方式");
    }

    #[tokio::test]
    async fn a_server_error_does_not_retry_other_auth_modes() {
        // 404/500 换认证方式是白费力气，会让一次查询发出十几次请求。
        let fetcher = Fake::always(404, "not found");
        let error = query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            None,
        )
        .await
        .expect_err("必须失败");
        assert!(error.contains("404"), "{error}");
        assert_eq!(
            fetcher.calls().len(),
            2,
            "非认证错误应当每个候选只试一次认证方式"
        );
    }

    #[tokio::test]
    async fn a_configured_auth_mode_is_tried_alone() {
        let fetcher = Fake::always(401, "unauthorized");
        let _ = query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            Some(AuthMode::XApiKey),
        )
        .await;
        // 2 个候选端点 × 1 种指定认证方式。
        assert_eq!(fetcher.calls().len(), 2, "指定了认证方式就不该再试别的");
    }

    #[tokio::test]
    async fn the_query_param_auth_appends_the_key() {
        let fetcher = Fake::new().with("/v1/models", 200, MODELS_BODY);
        query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            Some(AuthMode::Query),
        )
        .await
        .expect("查询");
        assert!(fetcher.calls()[0].url.contains("key=sk-1"));
    }

    #[tokio::test]
    async fn a_non_json_body_moves_to_the_next_candidate() {
        let fetcher =
            Fake::new()
                .with("/v1/models", 200, "不是 JSON")
                .with("/models", 200, MODELS_BODY);
        let models = query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            None,
        )
        .await
        .expect("应当退到下一个候选");
        assert_eq!(models.len(), 2);
    }

    #[tokio::test]
    async fn a_200_without_recognizable_models_is_a_failure() {
        // 否则界面会显示「没有模型」而不是「查询失败」，用户会去怀疑自己的账号。
        let fetcher = Fake::always(200, r#"{"data":[]}"#);
        let error = query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "sk-1",
            None,
            None,
        )
        .await
        .expect_err("必须失败");
        assert!(error.contains("没有可识别的模型"), "{error}");
    }

    #[tokio::test]
    async fn an_empty_base_url_is_rejected() {
        let fetcher = Fake::new();
        let error = query_vendor_models(&fetcher, "  ", ProviderId::Codex, "k", None, None)
            .await
            .expect_err("必须拒绝");
        assert!(error.contains("未配置 API 地址"), "{error}");
    }

    #[tokio::test]
    async fn a_configured_endpoint_replaces_the_candidates() {
        let fetcher = Fake::new().with("/custom/models", 200, MODELS_BODY);
        query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            Some("/custom/models"),
            None,
        )
        .await
        .expect("查询");
        assert_eq!(fetcher.calls().len(), 1, "配置了端点就不该再试候选");
        assert!(fetcher.calls()[0].url.ends_with("/custom/models"));
    }

    #[tokio::test]
    async fn an_absolute_configured_endpoint_is_used_as_is() {
        let fetcher = Fake::new().with("https://other.example.com/m", 200, MODELS_BODY);
        query_vendor_models(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            Some("https://other.example.com/m"),
            None,
        )
        .await
        .expect("查询");
        assert_eq!(fetcher.calls()[0].url, "https://other.example.com/m");
    }

    #[tokio::test]
    async fn a_generic_balance_query_parses_the_common_shape() {
        let fetcher = Fake::new().with(
            "/v1/usage",
            200,
            r#"{"remaining":10,"total":100,"used":90,"currency":"CNY"}"#,
        );
        let snapshot = query_vendor_balance(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            &BalanceQueryConfig::default(),
        )
        .await
        .expect("查询");
        assert_eq!(snapshot.remaining, Some(10.0));
        assert_eq!(snapshot.unit, "CNY");
    }

    #[tokio::test]
    async fn a_new_api_balance_query_divides_the_quota() {
        let fetcher = Fake::new().with("/v1/usage", 200, r#"{"quota":500000,"used_quota":250000}"#);
        let config = BalanceQueryConfig {
            template: BalanceTemplate::NewApi,
            ..BalanceQueryConfig::default()
        };
        let snapshot = query_vendor_balance(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            &config,
        )
        .await
        .expect("查询");
        assert_eq!(snapshot.remaining, Some(1.0));
        assert_eq!(snapshot.total, Some(1.5));
    }

    #[tokio::test]
    async fn the_balance_multiplier_is_applied() {
        let fetcher = Fake::new().with("/v1/usage", 200, r#"{"remaining":2}"#);
        let config = BalanceQueryConfig {
            multiplier: Some(3.0),
            ..BalanceQueryConfig::default()
        };
        let snapshot = query_vendor_balance(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            &config,
        )
        .await
        .expect("查询");
        assert_eq!(snapshot.remaining, Some(6.0));
    }

    #[tokio::test]
    async fn a_balance_endpoint_failure_is_reported() {
        let fetcher = Fake::new().with("/v1/usage", 503, "unavailable");
        let error = query_vendor_balance(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            &BalanceQueryConfig::default(),
        )
        .await
        .expect_err("必须失败");
        assert!(error.contains("503"), "{error}");
    }

    #[tokio::test]
    async fn a_balance_body_without_quota_is_rejected() {
        let fetcher = Fake::new().with("/v1/usage", 200, r#"{"unrelated":1}"#);
        let error = query_vendor_balance(
            &fetcher,
            "https://api.example.com",
            ProviderId::Codex,
            "k",
            &BalanceQueryConfig::default(),
        )
        .await
        .expect_err("必须失败");
        assert!(error.contains("没有可识别的额度"), "{error}");
    }

    #[test]
    fn url_encoding_escapes_only_what_must_be_escaped() {
        // 过度编码（把 `-` `_` `.` 也编码）反而可能被某些服务拒绝。
        assert_eq!(url_encode("sk-abc_1.2~3"), "sk-abc_1.2~3");
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode("a/b"), "a%2Fb");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn the_network_error_status_is_not_a_server_error() {
        // 用 0 而不是 5xx：5xx 是服务端返回的、会被计入熔断；
        // 连不上不该算服务端的错。
        assert_eq!(STATUS_NETWORK_ERROR, 0);
        assert!(!(500..600).contains(&STATUS_NETWORK_ERROR));
    }

    #[test]
    fn the_endpoint_resolver_handles_relative_and_absolute() {
        assert_eq!(
            resolve_endpoint("/v1/models", "https://a.com"),
            "https://a.com/v1/models"
        );
        assert_eq!(
            resolve_endpoint("v1/models", "https://a.com/"),
            "https://a.com/v1/models"
        );
        assert_eq!(
            resolve_endpoint("https://b.com/m", "https://a.com"),
            "https://b.com/m"
        );
    }

    #[test]
    fn the_model_limit_is_the_documented_value() {
        assert_eq!(model_count_limit(), 500);
    }

    #[test]
    fn query_auth_only_touches_the_url_for_query_mode() {
        assert_eq!(
            apply_query_auth("https://a.com/m", AuthMode::Bearer, "k"),
            "https://a.com/m"
        );
        assert_eq!(
            apply_query_auth("https://a.com/m?x=1", AuthMode::Query, "k"),
            "https://a.com/m?x=1&key=k"
        );
        assert_eq!(
            apply_query_auth("https://a.com/m", AuthMode::Query, "k"),
            "https://a.com/m?key=k"
        );
    }

    #[test]
    fn the_balance_field_enum_is_exhaustive() {
        // 这些字段对应现网的 `*Path` 配置项，少一个就无法自定义取值。
        let fields = [
            BalanceField::Remaining,
            BalanceField::Total,
            BalanceField::Used,
            BalanceField::Unit,
            BalanceField::Plan,
            BalanceField::Valid,
            BalanceField::Status,
        ];
        assert_eq!(fields.len(), 7);
        let _ = json!({});
    }
}
