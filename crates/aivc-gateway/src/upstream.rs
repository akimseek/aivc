//! 上游请求头构造与 URL 拼接。
//!
//! # 为什么必须剔除一批请求头
//!
//! 转发时逐个头照抄会在两处出错：
//!
//! 1. **认证头**：客户端的凭据是**本地路由令牌**，不是供应商密钥。原样透传会让供应商
//!    收到一个无效令牌，同时把本地令牌泄露出去。
//! 2. **逐跳头（hop-by-hop）**：`host` / `content-length` / `transfer-encoding` /
//!    `connection` 描述的是**这一段连接**，不是端到端语义。照抄 `transfer-encoding`
//!    会让上游看到两个分块声明并直接报 `invalid transfer-encoding`。
//!
//! 剔除之后按平台约定补上供应商自己的认证头：
//!
//! | 平台 | 认证头 |
//! | --- | --- |
//! | Claude | `x-api-key` |
//! | Gemini | `x-goog-api-key` |
//! | Codex / Qoder | `authorization: Bearer <key>` |

use reqwest::Url;

use aivc_core::error::{Error, VendorError};
use aivc_core::target::ProviderId;
use aivc_core::vendor::ApiVendor;

/// 转发时必须剔除的请求头。
///
/// 前六个是逐跳头或与本地连接绑定的头；后三个是客户端凭据。
pub const DROPPED_HEADERS: [&str; 9] = [
    "host",
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
    "proxy-authorization",
];

/// 判断某个请求头是否应当剔除（大小写不敏感）。
#[must_use]
pub fn should_drop_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    DROPPED_HEADERS.contains(&lower.as_str())
}

/// 由**网关自己接管**、不透传给上游的请求头。
///
/// # 为什么 `accept-encoding` 必须由网关说了算
///
/// 网关要从响应体里读两样东西：**错误文本**（失败转移规则据此匹配）与**用量**
/// （报表与计费）。而它的 HTTP 客户端被配置成**不解压**——
/// `reqwest` 只开了 `rustls-tls` / `stream` / `json`。
///
/// 于是「上游压缩」与「网关读得懂正文」互斥。请求一旦声明支持压缩，上游就可能回 gzip，
/// 而网关拿到的是二进制：
///
/// 1. 错误文本匹配不到 → **按正文匹配的失败转移规则静默失效**；
/// 2. 用量解析不出来 → **用量与计费静默失准**；
/// 3. 更隐蔽的一条：响应探测靠 `\n\n` 判断「SSE 第一个事件已完整」，
///    在压缩体上永远找不到，于是**一直缓冲到上限**——流式响应的首 token 被拖慢。
///
/// # 这是换掉传输层之后新出现的问题
///
/// 老平台把 `accept-encoding` 照样透传出去，却没有这些问题：它用的 undici
/// **默认自动解压**，因此它读到的正文始终是明文。
/// 移植到 `reqwest` 之后那个前提没了，必须在请求侧显式处理。
///
/// 代价是「上游 → 网关」这一段不再压缩。要恢复它就得让客户端能解压，而那需要
/// `brotli` / `zstd` / `async-compression` 等依赖——当前离线环境里没有。
pub const GATEWAY_CONTROLLED_HEADERS: [&str; 1] = ["accept-encoding"];

/// 该请求头是否由网关接管（应当剔除以保证网关自己拿到明文）。
#[must_use]
pub fn is_gateway_controlled_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    GATEWAY_CONTROLLED_HEADERS.contains(&lower.as_str())
}

/// 一次转发要用的请求头。
///
/// # 为什么透传的头与凭据**必须分开**
///
/// 现网对这两者用的是**两种不同的操作**：
///
/// ```js
/// for (…) headers.append(name, value);           // 透传：客户端可能同名重复
/// headers.set("authorization", `Bearer ${key}`);  // 凭据：必须**替换**
/// ```
///
/// 移植时两者被「统一」成了 `HeaderMap::insert`，于是丢掉了一件事：
/// **客户端重复的同名头会被合并成一条、只剩最后一个**。代理不该这么做——
/// 同名重复是合法且常见的（`anthropic-beta` 就可能分多条发）。
///
/// 凭据那一侧的结果碰巧仍然正确（客户端的凭据已被剔除，没有同名头可撞），
/// 但它依赖的是「刚好没有同名头」而不是「显式替换」——那是会在别处坏掉的那种对。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamHeaders {
    /// 透传的客户端头（已剔除逐跳头与客户端凭据，名称小写化）。
    pub forwarded: Vec<(String, String)>,
    /// 供应商凭据头：`(名称, 值)`。调用方要用**替换**语义写入。
    pub credential: (String, String),
}

/// 构造转发到上游的请求头。
///
/// `headers` 是客户端的原始头；`vendor` 提供供应商凭据与平台。
#[must_use]
pub fn build_upstream_headers<'a, I>(headers: I, vendor: &ApiVendor) -> UpstreamHeaders
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let forwarded: Vec<(String, String)> = headers
        .into_iter()
        // 逐跳头与客户端凭据要剔除；`accept-encoding` 由网关自己接管，见其文档。
        .filter(|(name, _)| !should_drop_header(name) && !is_gateway_controlled_header(name))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();

    let api_key = vendor.api_key.trim();
    let credential = match vendor.provider_id {
        ProviderId::Claude => ("x-api-key".to_owned(), api_key.to_owned()),
        ProviderId::Gemini => ("x-goog-api-key".to_owned(), api_key.to_owned()),
        // Codex 与 Qoder 都走标准 Bearer。
        ProviderId::Codex | ProviderId::Qoder => {
            ("authorization".to_owned(), format!("Bearer {api_key}"))
        }
    };
    UpstreamHeaders {
        forwarded,
        credential,
    }
}

/// 拼接上游 URL。
///
/// # 两条校验
///
/// - 协议必须是 `http` 或 `https`：其他协议（`file:`、`ftp:`）会让转发变成任意协议
///   请求，是一个典型的 SSRF 面。
/// - 基地址不得携带查询串或片段：它们会被这里覆盖，静默丢弃用户配置的参数。
///
/// # `/v1` 去重
///
/// 供应商地址常写成 `https://api.example.com/v1`，而 CLI 发出的路径也已含 `/v1/…`。
/// 直接拼接会得到 `/v1/v1/…`。因此当基地址以 `/v1` 结尾且后缀以 `/v1/` 开头时，
/// 把后缀里的 `/v1` 去掉。
pub fn join_upstream_url(base_url: &str, suffix: &str, search: &str) -> Result<String, Error> {
    let mut base = Url::parse(base_url).map_err(|_| VendorError::BaseUrlInvalid)?;
    if base.scheme() != "http" && base.scheme() != "https" {
        return Err(VendorError::BaseUrlSchemeInvalid.into());
    }
    if base.query().is_some() || base.fragment().is_some() {
        return Err(VendorError::BaseUrlHasQueryOrFragment.into());
    }

    let base_path = base.path().trim_end_matches('/').to_owned();
    let normalized = if suffix.starts_with("/v1/") && base_path.ends_with("/v1") {
        &suffix["/v1".len()..]
    } else {
        suffix
    };
    let joined = if normalized.starts_with('/') {
        format!("{base_path}{normalized}")
    } else {
        format!("{base_path}/{normalized}")
    };
    base.set_path(&joined);
    base.set_query((!search.is_empty()).then_some(search.trim_start_matches('?')));
    Ok(base.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::VendorId;
    use aivc_core::time::Timestamp;

    fn vendor(provider_id: ProviderId, api_key: &str) -> ApiVendor {
        ApiVendor {
            id: VendorId::from("v1"),
            provider_id,
            name: "V1".to_owned(),
            api_key: api_key.to_owned(),
            api_base_url: "https://api.example.com".to_owned(),
            sort: 1,
            pricing: None,
            model_query: None,
            balance_query: None,
            configs: Vec::new(),
            enabled: true,
            created_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
            updated_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
            last_enabled_at: None,
            balance: None,
            balance_status: None,
            balance_error: None,
            balance_queried_at: None,
            gateway_health: None,
        }
    }

    #[test]
    fn client_credentials_are_never_forwarded() {
        let headers = [
            ("authorization", "Bearer local-route-token"),
            ("x-api-key", "local-key"),
            ("x-goog-api-key", "local-goog"),
            ("content-type", "application/json"),
        ];
        let built = build_upstream_headers(headers, &vendor(ProviderId::Codex, "vendor-key"));

        let names: Vec<&str> = built
            .forwarded
            .iter()
            .map(|(name, _)| name.as_str())
            .chain(std::iter::once(built.credential.0.as_str()))
            .collect();
        assert!(!names.contains(&"x-api-key"));
        assert!(!names.contains(&"x-goog-api-key"));
        // 内容类型必须保留。
        assert!(names.contains(&"content-type"));
        // 本地令牌不得出现在任何一处。
        assert!(
            built
                .forwarded
                .iter()
                .map(|(_, value)| value.as_str())
                .chain(std::iter::once(built.credential.1.as_str()))
                .all(|value| !value.contains("local-route-token")),
            "本地令牌被透传了"
        );
        // 凭据是供应商的，且只有一条。
        assert_eq!(built.credential.0, "authorization");
        assert_eq!(built.credential.1, "Bearer vendor-key");
    }

    #[test]
    fn hop_by_hop_headers_are_dropped() {
        for name in DROPPED_HEADERS {
            assert!(should_drop_header(name), "{name} 应被剔除");
            assert!(should_drop_header(&name.to_uppercase()), "大小写不敏感");
        }
        assert!(!should_drop_header("content-type"));
        assert!(!should_drop_header("accept"));
    }

    #[test]
    fn each_provider_gets_its_own_auth_header() {
        let cases = [
            (ProviderId::Claude, "x-api-key"),
            (ProviderId::Gemini, "x-goog-api-key"),
            (ProviderId::Codex, "authorization"),
            (ProviderId::Qoder, "authorization"),
        ];
        for (provider, expected) in cases {
            let built = build_upstream_headers([("accept", "*/*")], &vendor(provider, "k"));
            assert_eq!(built.credential.0, expected, "{provider:?}");
        }
    }

    /// **回归测试：重复的同名头必须全部保留。**
    ///
    /// 透传的头要用 `append`（追加），凭据要用 `insert`（替换）——现网就是两种操作，
    /// 而移植时一度都写成 `insert`，于是客户端重复的同名头被合并成一条、只剩最后一个。
    /// 同名重复是合法且常见的，代理把它们合并不是等价变换。
    ///
    /// 这里断言的是**构造结果**（`forwarded` 是成对的列表，重复项原样保留）；
    /// 写入 `HeaderMap` 时的 `append` 由 `headers_from` 保证。
    #[test]
    fn repeated_client_headers_are_kept_in_order() {
        let built = build_upstream_headers(
            [
                ("anthropic-beta", "first-flag"),
                ("accept", "*/*"),
                ("anthropic-beta", "second-flag"),
            ],
            &vendor(ProviderId::Claude, "k"),
        );
        let betas: Vec<&str> = built
            .forwarded
            .iter()
            .filter(|(name, _)| name == "anthropic-beta")
            .map(|(_, value)| value.as_str())
            .collect();
        assert_eq!(
            betas,
            vec!["first-flag", "second-flag"],
            "重复的同名头必须原样保留，不能只剩最后一个"
        );
    }

    /// **回归测试：`accept-encoding` 不能透传给上游。**
    ///
    /// 网关要读响应体里的错误文本与用量，而它的客户端不解压——
    /// 一旦上游压缩，这两件事都会静默失效，响应探测还会一直缓冲到上限、
    /// 拖慢流式响应的首 token。
    ///
    /// 老平台不需要这一步，因为 undici 默认自动解压。换掉传输层之后必须显式处理。
    #[test]
    fn accept_encoding_is_taken_over_by_the_gateway() {
        assert!(is_gateway_controlled_header("accept-encoding"));
        assert!(is_gateway_controlled_header("Accept-Encoding"));
        // 它不是逐跳头，因此**不在**那个列表里——两者的理由不同，不该混为一谈。
        assert!(!should_drop_header("accept-encoding"));

        let built = build_upstream_headers(
            [
                ("accept-encoding", "gzip, deflate, br"),
                ("accept", "text/event-stream"),
            ],
            &vendor(ProviderId::Codex, "k"),
        );
        let names: Vec<&str> = built
            .forwarded
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        assert!(
            !names.contains(&"accept-encoding"),
            "上游一旦压缩，错误文本与用量都读不出来：{names:?}"
        );
        // 其余的头照常透传。
        assert!(names.contains(&"accept"));
    }

    #[test]
    fn header_names_are_lowercased() {
        let built = build_upstream_headers(
            [("Content-Type", "text/plain")],
            &vendor(ProviderId::Codex, "k"),
        );
        assert!(
            built
                .forwarded
                .iter()
                .any(|(name, _)| name == "content-type")
        );
    }

    #[test]
    fn upstream_urls_are_joined_without_double_slashes() {
        assert_eq!(
            join_upstream_url("https://api.example.com", "/v1/messages", "").expect("join"),
            "https://api.example.com/v1/messages"
        );
        assert_eq!(
            join_upstream_url("https://api.example.com/", "/v1/messages", "").expect("join"),
            "https://api.example.com/v1/messages"
        );
    }

    #[test]
    fn a_trailing_v1_in_the_base_is_not_duplicated() {
        // 供应商地址常写成 `…/v1`，而 CLI 路径也已含 `/v1/`。
        assert_eq!(
            join_upstream_url("https://api.example.com/v1", "/v1/messages", "").expect("join"),
            "https://api.example.com/v1/messages"
        );
        // 路径不以 `/v1/` 开头时不做事。
        assert_eq!(
            join_upstream_url("https://api.example.com/v1", "/models", "").expect("join"),
            "https://api.example.com/v1/models"
        );
    }

    #[test]
    fn a_base_path_prefix_is_preserved() {
        assert_eq!(
            join_upstream_url("https://api.example.com/openai", "/v1/chat", "").expect("join"),
            "https://api.example.com/openai/v1/chat"
        );
    }

    #[test]
    fn the_search_string_is_applied() {
        assert_eq!(
            join_upstream_url("https://api.example.com", "/v1/models", "?a=1").expect("join"),
            "https://api.example.com/v1/models?a=1"
        );
        // 允许调用方带或不带前导 `?`。
        assert_eq!(
            join_upstream_url("https://api.example.com", "/v1/models", "a=1").expect("join"),
            "https://api.example.com/v1/models?a=1"
        );
    }

    #[test]
    fn non_http_schemes_are_rejected() {
        // 其他协议会让转发变成任意协议请求，是典型的 SSRF 面。
        for base in [
            "file:///etc/passwd",
            "ftp://example.com",
            "gopher://example.com",
        ] {
            let error = join_upstream_url(base, "/x", "").expect_err("必须拒绝");
            assert!(
                error.to_string().contains("HTTP") || error.to_string().contains("HTTPS"),
                "{base} → {error}"
            );
        }
    }

    #[test]
    fn a_base_with_query_or_fragment_is_rejected() {
        // 它们会被这里覆盖，静默丢弃用户配置的参数。
        for base in [
            "https://api.example.com?x=1",
            "https://api.example.com#frag",
        ] {
            let error = join_upstream_url(base, "/x", "").expect_err("必须拒绝");
            assert!(
                error.to_string().contains("查询参数") || error.to_string().contains("片段"),
                "{base} → {error}"
            );
        }
    }

    #[test]
    fn a_malformed_base_is_rejected() {
        assert!(join_upstream_url("not a url", "/x", "").is_err());
    }
}
