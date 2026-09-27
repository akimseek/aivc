//! 供应商模型列表查询：端点候选推导、认证方式、响应解析。
//!
//! # 为什么要推导「候选端点」
//!
//! 用户填的 `apiBaseUrl` 千奇百怪：有的到根、有的到 `/v1`、有的到 `/v1beta`、
//! 有的直接到 `/chat/completions`。而模型列表几乎总在 `<根>/v1/models` 或
//! `<根>/models`。因此这里按平台与路径形状推出**一串候选**，逐个试。
//!
//! # 为什么要推导「认证方式」
//!
//! 同样是 `OpenAI` 兼容接口，有的认 `Authorization: Bearer`，有的认 `x-api-key`，
//! 有的认 `x-goog-api-key`（Gemini），还有的只认查询参数。用户很难知道自己那个
//! 中转站认哪种，因此按平台给出**优先级序列**逐个试。
//!
//! # 认证失败才换方式，其他错误直接换地址
//!
//! 401/403 说明「这个端点存在但认证方式不对」，值得换一种再试；
//! 404/500 说明端点本身有问题，换认证方式是白费力气——直接换下一个地址。
//! 不区分会让一次查询发出十几次请求。
//!
//! # 不引 URL 解析库
//!
//! 需要的只是「拆出 origin 与 path，替换 path，拼回去」。引入完整 URL 解析库会带来
//! 依赖面，而这里的输入已被上游校验过协议（只允许 http/https）。

use aivc_core::target::ProviderId;
use serde_json::Value;

/// 模型查询超时（15 秒）。
pub const MODEL_LIST_TIMEOUT_MS: u64 = 15_000;
/// 响应体积上限（2 MB）。
pub const MODEL_LIST_MAX_BYTES: usize = 2 * 1024 * 1024;
/// 返回条数上限。
pub const MODEL_LIST_MAX_COUNT: usize = 500;
/// 模型 ID 长度上限。
pub const MODEL_ID_MAX_LENGTH: usize = 200;

/// 已知的兼容路径后缀。
///
/// 用户常把地址填成「Anthropic 兼容入口」（`/api/claudecode` 之类），
/// 此时模型列表在**去掉该后缀的根**下面。
pub const MODEL_COMPAT_SUFFIXES: [&str; 9] = [
    "/api/claudecode",
    "/api/anthropic",
    "/apps/anthropic",
    "/api/coding",
    "/claudecode",
    "/anthropic",
    "/step_plan",
    "/coding",
    "/claude",
];

/// 一种认证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMode {
    /// `Authorization: Bearer <key>`。
    Bearer,
    /// `x-api-key: <key>`。
    XApiKey,
    /// `x-goog-api-key: <key>`。
    XGoogApiKey,
    /// `api-key: <key>`。
    ApiKey,
    /// 放进查询参数。
    Query,
    /// 不附加认证（现网的 `QueryAuthMode.None`）。
    None,
}

impl AuthMode {
    /// 请求头名（[`AuthMode::Query`] 没有头名）。
    #[must_use]
    pub const fn header_name(self) -> Option<&'static str> {
        match self {
            Self::Bearer => Some("authorization"),
            Self::XApiKey => Some("x-api-key"),
            Self::XGoogApiKey => Some("x-goog-api-key"),
            Self::ApiKey => Some("api-key"),
            Self::Query | Self::None => None,
        }
    }

    /// 是否是「认证类」失败应当换方式的依据。
    #[must_use]
    pub const fn is_query(self) -> bool {
        matches!(self, Self::Query)
    }

    /// 是否附加认证。
    #[must_use]
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }
}

/// 某个平台的认证方式优先级序列。
#[must_use]
pub fn model_auth_modes(provider: ProviderId) -> &'static [AuthMode] {
    match provider {
        ProviderId::Gemini => &[
            AuthMode::XGoogApiKey,
            AuthMode::Query,
            AuthMode::Bearer,
            AuthMode::ApiKey,
        ],
        ProviderId::Claude => &[AuthMode::XApiKey, AuthMode::Bearer, AuthMode::ApiKey],
        // Codex 与 Qoder 走 OpenAI 兼容约定。
        ProviderId::Codex | ProviderId::Qoder => {
            &[AuthMode::Bearer, AuthMode::ApiKey, AuthMode::XApiKey]
        }
    }
}

/// 状态码是否值得换一种认证方式再试。
///
/// 只有认证失败才换；路径/服务端错误换认证方式是白费力气。
#[must_use]
pub const fn should_retry_with_another_auth(status: u16) -> bool {
    status == 401 || status == 403
}

/// 把 URL 拆成 origin 与 path。
fn split_origin_path(url: &str) -> Option<(&str, &str)> {
    let scheme_end = url.find("://")?;
    let after = scheme_end + 3;
    let rest = &url[after..];
    let path_start = rest.find('/');
    match path_start {
        Some(index) => Some((&url[..after + index], &rest[index..])),
        // 没有路径时 path 视为空串。
        None => Some((url, "")),
    }
}

/// 去掉末尾的斜杠。
fn trim_trailing_slashes(path: &str) -> &str {
    path.trim_end_matches('/')
}

/// 把多个斜杠压成一个，并保证以 `/` 开头。
fn normalize_path(path: &str) -> String {
    let collapsed = {
        let mut result = String::with_capacity(path.len());
        let mut previous_slash = false;
        for ch in path.chars() {
            if ch == '/' {
                if !previous_slash {
                    result.push(ch);
                }
                previous_slash = true;
            } else {
                result.push(ch);
                previous_slash = false;
            }
        }
        result
    };
    if collapsed.starts_with('/') {
        collapsed
    } else {
        format!("/{collapsed}")
    }
}

/// 拼接 origin 与 path。
fn join_url(origin: &str, path: &str) -> String {
    format!("{origin}{}", normalize_path(path))
}

/// 大小写不敏感的后缀判断。
fn ends_with_ignore_case(value: &str, suffix: &str) -> bool {
    value.len() >= suffix.len() && value[value.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

/// 推导模型列表的候选端点。
///
/// 顺序即优先级：越靠前越可能是正确端点。
#[must_use]
pub fn build_model_endpoint_candidates(base_url: &str, provider: ProviderId) -> Vec<String> {
    let Some((origin, raw_path)) = split_origin_path(base_url.trim()) else {
        return Vec::new();
    };
    let base_path = trim_trailing_slashes(raw_path).to_owned();
    let mut candidates: Vec<String> = Vec::new();
    let mut add = |path: &str| {
        let value = join_url(origin, path);
        if !candidates.contains(&value) {
            candidates.push(value);
        }
    };

    if ends_with_ignore_case(&base_path, "/models") {
        // 用户已经填到 `/models`：直接用。
        add(&base_path);
    } else if provider == ProviderId::Gemini {
        if ends_with_ignore_case(&base_path, "/v1beta") {
            add(&format!("{base_path}/models"));
        } else if ends_with_ignore_case(&base_path, "/v1") {
            // 从 `/v1` 换成 `/v1beta`：Gemini 的模型列表在 v1beta 下。
            let root = &base_path[..base_path.len() - 3];
            add(&format!("{root}/v1beta/models"));
        } else {
            add(&format!("{base_path}/v1beta/models"));
        }
        add(&format!("{base_path}/v1/models"));
        add(&format!("{base_path}/models"));
    } else if is_version_path(&base_path) {
        add(&format!("{base_path}/models"));
        // 已经在 `/v1` 上就不必再试 `/v1/models`（重复）。
        if !ends_with_ignore_case(&base_path, "/v1") {
            add(&format!("{base_path}/v1/models"));
        }
    } else {
        // 兜底：绝大多数服务的模型列表在 `<根>/v1/models` 或 `<根>/models`。
        add(&format!("{base_path}/v1/models"));
        add(&format!("{base_path}/models"));
    }

    // 兼容后缀与对话端点的处理**无条件执行**（不是 else 分支）：
    // 一个地址可能同时是 `/v1/chat/completions`，既要在版本分支里找，也要回到根找。
    for suffix in MODEL_COMPAT_SUFFIXES {
        if !base_path.ends_with(suffix) {
            continue;
        }
        let root = trim_trailing_slashes(&base_path[..base_path.len() - suffix.len()]);
        if !root.is_empty() {
            add(&format!("{root}/v1/models"));
            add(&format!("{root}/models"));
        }
        break;
    }
    for suffix in ["/responses", "/chat/completions"] {
        if ends_with_ignore_case(&base_path, suffix) {
            let root = &base_path[..base_path.len() - suffix.len()];
            add(&format!("{root}/v1/models"));
            add(&format!("{root}/models"));
            break;
        }
    }
    candidates
}
/// 是否是 `/v1`、`/v2` 这类版本路径。
fn is_version_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/v") else {
        return false;
    };
    // 版本号可以是 `1` 或 `1.5`：点号只允许出现一次且两侧都是数字。
    let (major, minor) = match rest.split_once('.') {
        Some((major, minor)) => (major, Some(minor)),
        None => (rest, None),
    };
    !major.is_empty()
        && major.chars().all(|ch| ch.is_ascii_digit())
        && minor
            .is_none_or(|value| !value.is_empty() && value.chars().all(|ch| ch.is_ascii_digit()))
}

/// 从响应体里提取模型记录。
///
/// 各家把数组放在不同键下（`data` / `models` / `items` / `results` / `model_list`），
/// 有的直接返回数组。逐个找而不是只认一种。
#[must_use]
pub fn extract_model_rows(body: &Value) -> Vec<&Value> {
    if let Some(array) = body.as_array() {
        return array.iter().filter(|item| item.is_object()).collect();
    }
    let Some(object) = body.as_object() else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    for key in ["data", "models", "items", "results", "model_list"] {
        if let Some(array) = object.get(key).and_then(Value::as_array) {
            rows.extend(array.iter().filter(|item| item.is_object()));
        }
    }
    rows
}

/// 一条模型记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendorModel {
    /// 模型编号。
    pub id: String,
    /// 对象类型。
    pub object: Option<String>,
    /// 归属方。
    pub owned_by: Option<String>,
    /// 创建时间。
    pub created: Option<i64>,
    /// 描述。
    pub description: Option<String>,
}

/// 把一条原始记录转成模型。
///
/// 返回 [`None`] 表示这条记录不可用——**跳过而不是报错**：响应里常混着
/// 非模型条目，因为一条坏记录让整次查询失败是不划算的。
#[must_use]
pub fn to_vendor_model(item: &Value, provider: ProviderId) -> Option<VendorModel> {
    let object = item.as_object()?;
    let raw_id = ["id", "model", "name", "slug"].iter().find_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    })?;
    // Gemini 返回的是 `models/gemini-pro`，前缀要去掉才是真正的模型名。
    let id = if provider == ProviderId::Gemini {
        raw_id
            .strip_prefix("models/")
            .or_else(|| raw_id.strip_prefix("MODELS/"))
            .unwrap_or(raw_id)
            .to_owned()
    } else {
        raw_id.to_owned()
    };
    if id.is_empty()
        || id.chars().count() > MODEL_ID_MAX_LENGTH
        || id.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return None;
    }
    Some(VendorModel {
        id,
        object: object
            .get("object")
            .and_then(Value::as_str)
            .map(str::to_owned),
        owned_by: object
            .get("owned_by")
            .or_else(|| object.get("ownedBy"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        created: object.get("created").and_then(Value::as_i64),
        description: object
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}

/// 归一、去重、排序并截断模型列表。
///
/// 去重按**小写编号**：不同端点可能用不同大小写返回同一个模型。
/// 排序后截断：不排序直接截断会让「返回哪些」取决于服务端顺序。
#[must_use]
pub fn normalize_models(models: Vec<VendorModel>) -> Vec<VendorModel> {
    let mut seen = std::collections::BTreeMap::new();
    for model in models {
        // 先出现的胜出（保持候选端点的优先级语义）。
        seen.entry(model.id.to_ascii_lowercase()).or_insert(model);
    }
    let mut values: Vec<VendorModel> = seen.into_values().collect();
    // 近似 JS 的 `localeCompare`：先按小写比较（`alpha` 排在 `Beta` 前），
    // 相同时用原串做稳定兜底。直接按字节比较会把大写都排到前面。
    values.sort_by(|left, right| {
        left.id
            .to_ascii_lowercase()
            .cmp(&right.id.to_ascii_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    values.truncate(MODEL_LIST_MAX_COUNT);
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_limits_match_the_desktop_reference() {
        assert_eq!(MODEL_LIST_TIMEOUT_MS, 15_000);
        assert_eq!(MODEL_LIST_MAX_BYTES, 2 * 1024 * 1024);
        assert_eq!(MODEL_LIST_MAX_COUNT, 500);
        assert_eq!(MODEL_ID_MAX_LENGTH, 200);
    }

    #[test]
    fn the_auth_mode_order_is_per_provider() {
        // 同样是 OpenAI 兼容接口，各家认的认证方式不同，用户很难自己判断。
        assert_eq!(
            model_auth_modes(ProviderId::Gemini)[0],
            AuthMode::XGoogApiKey
        );
        assert_eq!(model_auth_modes(ProviderId::Claude)[0], AuthMode::XApiKey);
        assert_eq!(model_auth_modes(ProviderId::Codex)[0], AuthMode::Bearer);
        assert_eq!(model_auth_modes(ProviderId::Qoder)[0], AuthMode::Bearer);
        // 每种方式的头名要么有、要么是查询参数。
        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            for mode in model_auth_modes(provider) {
                assert!(mode.header_name().is_some() || mode.is_query());
            }
        }
    }

    #[test]
    fn only_auth_failures_justify_another_auth_mode() {
        // 404/500 换认证方式是白费力气，会让一次查询发出十几次请求。
        assert!(should_retry_with_another_auth(401));
        assert!(should_retry_with_another_auth(403));
        assert!(!should_retry_with_another_auth(404));
        assert!(!should_retry_with_another_auth(500));
        assert!(!should_retry_with_another_auth(200));
    }

    #[test]
    fn a_bare_base_url_yields_the_standard_candidates() {
        let candidates =
            build_model_endpoint_candidates("https://api.example.com", ProviderId::Codex);
        assert_eq!(
            candidates,
            vec![
                "https://api.example.com/v1/models",
                "https://api.example.com/models"
            ]
        );
    }

    #[test]
    fn a_trailing_slash_does_not_change_the_result() {
        assert_eq!(
            build_model_endpoint_candidates("https://api.example.com/", ProviderId::Codex),
            build_model_endpoint_candidates("https://api.example.com", ProviderId::Codex)
        );
    }

    #[test]
    fn an_address_already_ending_in_models_is_used_directly() {
        let candidates =
            build_model_endpoint_candidates("https://api.example.com/v1/models", ProviderId::Codex);
        assert_eq!(candidates, vec!["https://api.example.com/v1/models"]);
    }

    #[test]
    fn a_version_path_gets_models_appended() {
        let candidates =
            build_model_endpoint_candidates("https://api.example.com/v1", ProviderId::Codex);
        assert_eq!(candidates[0], "https://api.example.com/v1/models");
    }

    #[test]
    fn gemini_prefers_the_v1beta_endpoint() {
        // Gemini 的模型列表在 v1beta 下。
        let candidates = build_model_endpoint_candidates(
            "https://generativelanguage.googleapis.com",
            ProviderId::Gemini,
        );
        assert_eq!(
            candidates[0],
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
        assert!(
            candidates.contains(&"https://generativelanguage.googleapis.com/v1/models".to_owned())
        );
    }

    #[test]
    fn gemini_rewrites_a_v1_address_to_v1beta() {
        let candidates = build_model_endpoint_candidates(
            "https://generativelanguage.googleapis.com/v1",
            ProviderId::Gemini,
        );
        assert_eq!(
            candidates[0],
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn gemini_with_v1beta_already_present_just_appends_models() {
        let candidates = build_model_endpoint_candidates(
            "https://generativelanguage.googleapis.com/v1beta",
            ProviderId::Gemini,
        );
        assert_eq!(
            candidates[0],
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn a_compat_suffix_rewinds_to_the_root() {
        // 用户常把地址填成 Anthropic 兼容入口，此时模型列表在去掉后缀的根下面。
        // 注意根必须非空：后缀就是整个路径时没有「上面的根」可回退。
        let candidates = build_model_endpoint_candidates(
            "https://api.example.com/proxy/api/claudecode",
            ProviderId::Claude,
        );
        assert!(
            candidates.contains(&"https://api.example.com/proxy/v1/models".to_owned()),
            "{candidates:?}"
        );
        assert!(
            candidates.contains(&"https://api.example.com/proxy/models".to_owned()),
            "{candidates:?}"
        );
    }

    #[test]
    fn a_compat_suffix_that_is_the_whole_path_adds_nothing() {
        // 去掉后缀后根为空——没有「上面的根」可回退，因此只保留原路径下的候选。
        let candidates = build_model_endpoint_candidates(
            "https://api.example.com/api/claudecode",
            ProviderId::Claude,
        );
        assert!(
            candidates.contains(&"https://api.example.com/api/claudecode/v1/models".to_owned()),
            "{candidates:?}"
        );
        assert!(
            !candidates.contains(&"https://api.example.com/v1/models".to_owned()),
            "根为空时不该回退到站点根：{candidates:?}"
        );
    }

    #[test]
    fn a_chat_endpoint_rewinds_to_the_root() {
        let candidates = build_model_endpoint_candidates(
            "https://api.example.com/v1/chat/completions",
            ProviderId::Codex,
        );
        assert!(
            candidates.contains(&"https://api.example.com/v1/models".to_owned()),
            "{candidates:?}"
        );
        let candidates = build_model_endpoint_candidates(
            "https://api.example.com/v1/responses",
            ProviderId::Codex,
        );
        assert!(
            candidates.contains(&"https://api.example.com/v1/models".to_owned()),
            "{candidates:?}"
        );
    }

    #[test]
    fn candidates_are_deduplicated() {
        let candidates =
            build_model_endpoint_candidates("https://api.example.com/v1", ProviderId::Codex);
        let unique: std::collections::BTreeSet<&String> = candidates.iter().collect();
        assert_eq!(unique.len(), candidates.len(), "有重复候选：{candidates:?}");
    }

    #[test]
    fn an_unparseable_url_yields_no_candidates() {
        assert!(build_model_endpoint_candidates("not a url", ProviderId::Codex).is_empty());
        assert!(build_model_endpoint_candidates("", ProviderId::Codex).is_empty());
    }

    #[test]
    fn rows_are_found_under_every_known_key() {
        for key in ["data", "models", "items", "results", "model_list"] {
            let body = json!({ key: [{ "id": "m" }] });
            assert_eq!(extract_model_rows(&body).len(), 1, "键 {key} 应当被识别");
        }
    }

    #[test]
    fn a_bare_array_is_accepted() {
        let body = json!([{ "id": "a" }, { "id": "b" }]);
        assert_eq!(extract_model_rows(&body).len(), 2);
    }

    #[test]
    fn non_object_entries_are_filtered_out() {
        let body = json!({ "data": [{ "id": "a" }, "字符串", 42, null] });
        assert_eq!(extract_model_rows(&body).len(), 1);
    }

    #[test]
    fn an_unrecognized_body_yields_no_rows() {
        assert!(extract_model_rows(&json!({})).is_empty());
        assert!(extract_model_rows(&json!("字符串")).is_empty());
        assert!(extract_model_rows(&json!({ "data": "不是数组" })).is_empty());
    }

    #[test]
    fn the_id_can_come_from_several_fields() {
        for key in ["id", "model", "name", "slug"] {
            let item = json!({ key: "gpt-5" });
            assert_eq!(
                to_vendor_model(&item, ProviderId::Codex)
                    .expect("应当识别")
                    .id,
                "gpt-5",
                "字段 {key} 应当被识别"
            );
        }
    }

    #[test]
    fn a_gemini_model_prefix_is_stripped() {
        // Gemini 返回 `models/gemini-pro`，前缀不是模型名的一部分。
        let item = json!({ "name": "models/gemini-pro" });
        assert_eq!(
            to_vendor_model(&item, ProviderId::Gemini)
                .expect("应当识别")
                .id,
            "gemini-pro"
        );
        // 其他平台不剥前缀。
        assert_eq!(
            to_vendor_model(&item, ProviderId::Codex)
                .expect("应当识别")
                .id,
            "models/gemini-pro"
        );
    }

    #[test]
    fn an_entry_without_an_id_is_skipped() {
        assert!(to_vendor_model(&json!({}), ProviderId::Codex).is_none());
        assert!(to_vendor_model(&json!({ "id": "   " }), ProviderId::Codex).is_none());
        assert!(to_vendor_model(&json!({ "id": 42 }), ProviderId::Codex).is_none());
    }

    #[test]
    fn an_id_with_whitespace_or_control_characters_is_rejected() {
        // 这类编号拼进命令行或 URL 都会出问题。
        for value in ["a b", "a\tb", "a\nb", "a\u{0}b"] {
            assert!(
                to_vendor_model(&json!({ "id": value }), ProviderId::Codex).is_none(),
                "{value:?} 应当被拒绝"
            );
        }
    }

    #[test]
    fn an_over_long_id_is_rejected() {
        let long = "x".repeat(MODEL_ID_MAX_LENGTH + 1);
        assert!(to_vendor_model(&json!({ "id": long }), ProviderId::Codex).is_none());
        let ok = "x".repeat(MODEL_ID_MAX_LENGTH);
        assert!(to_vendor_model(&json!({ "id": ok }), ProviderId::Codex).is_some());
    }

    #[test]
    fn the_optional_fields_are_carried_through() {
        let item = json!({
            "id": "gpt-5",
            "object": "model",
            "owned_by": "openai",
            "created": 1_758_794_400,
            "description": "描述"
        });
        let model = to_vendor_model(&item, ProviderId::Codex).expect("识别");
        assert_eq!(model.object.as_deref(), Some("model"));
        assert_eq!(model.owned_by.as_deref(), Some("openai"));
        assert_eq!(model.created, Some(1_758_794_400));
        assert_eq!(model.description.as_deref(), Some("描述"));
    }

    #[test]
    fn owned_by_accepts_both_spellings() {
        let snake = to_vendor_model(&json!({ "id": "a", "owned_by": "x" }), ProviderId::Codex);
        let camel = to_vendor_model(&json!({ "id": "a", "ownedBy": "x" }), ProviderId::Codex);
        assert_eq!(snake.expect("识别").owned_by.as_deref(), Some("x"));
        assert_eq!(camel.expect("识别").owned_by.as_deref(), Some("x"));
    }

    #[test]
    fn normalization_deduplicates_case_insensitively_and_sorts() {
        // 不同端点可能用不同大小写返回同一个模型。
        let models = vec![
            VendorModel {
                id: "Beta".to_owned(),
                object: None,
                owned_by: None,
                created: None,
                description: None,
            },
            VendorModel {
                id: "alpha".to_owned(),
                object: None,
                owned_by: None,
                created: None,
                description: None,
            },
            VendorModel {
                id: "BETA".to_owned(),
                object: None,
                owned_by: None,
                created: None,
                description: None,
            },
        ];
        let normalized = normalize_models(models);
        assert_eq!(normalized.len(), 2, "大小写不同应当算同一个模型");
        // 先出现的胜出，保持候选端点的优先级语义。
        assert_eq!(normalized[0].id, "alpha");
        assert_eq!(normalized[1].id, "Beta");
    }

    #[test]
    fn normalization_truncates_to_the_limit() {
        // 不排序直接截断会让「返回哪些」取决于服务端顺序。
        let models: Vec<VendorModel> = (0..MODEL_LIST_MAX_COUNT + 50)
            .map(|index| VendorModel {
                id: format!("m{index:04}"),
                object: None,
                owned_by: None,
                created: None,
                description: None,
            })
            .collect();
        let normalized = normalize_models(models);
        assert_eq!(normalized.len(), MODEL_LIST_MAX_COUNT);
        // 排序后截断，保留的是最小的那批编号。
        assert_eq!(normalized[0].id, "m0000");
    }
}
