//! 路由注册表、对外 URL 构造与鉴权。
//!
//! # 两类路由
//!
//! | 模式 | 含义 |
//! | --- | --- |
//! | [`RouteMode::Dynamic`] | 供应商可在失败时自动切换到候选池里的下一个 |
//! | [`RouteMode::Locked`] | 供应商被锁定，只有手动切换才会改变 |
//!
//! # 两类凭据
//!
//! - **本地路由令牌**：终端启动时注入的 `localToken`，明文比较。它只在本机回环地址上
//!   使用，明文比较不会泄露额外信息。
//! - **外部访问令牌**：面向局域网/远程的访问凭据。数据库里只存 **SHA-256**，因此
//!   比较的是「候选值的哈希」与「已存哈希」。
//!
//! # 为什么必须用常量时间比较
//!
//! 令牌比较若用 `==`，比较耗时会随「前缀匹配长度」变化，攻击者可以逐字节爆破。
//! 这里统一走 [`subtle`] 的常量时间实现。
//!
//! 长度不等时直接返回 false 会泄露**长度**——这是无法避免的（长度本身就是公开信息），
//! 现网同样如此。

use std::collections::HashMap;
use std::fmt;

use aivc_core::target::ProviderId;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// 路由前缀。
pub const ROUTE_PREFIX: &str = "/gateway";

/// 路由模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RouteMode {
    /// 动态：失败时可自动切换到候选供应商。
    Dynamic,
    /// 锁定：只有手动切换才会改变供应商。
    Locked,
}

impl RouteMode {
    /// 是否允许自动故障转移。
    #[must_use]
    pub const fn allows_failover(self) -> bool {
        matches!(self, Self::Dynamic)
    }
}

/// 一条路由。
///
/// `local_token` 是机密，因此 [`fmt::Debug`] 只输出长度而不输出内容——日志里出现
/// 明文令牌等同于泄露。
#[derive(Clone, PartialEq, Eq)]
pub struct VendorRoute {
    /// 路由编号。
    pub route_id: String,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 当前供应商编号。
    pub vendor_id: String,
    /// 路由模式。
    pub mode: RouteMode,
    /// 本地访问令牌。
    pub local_token: String,
    /// 供应商上游地址。
    pub base_url: String,
    /// 是否是外部路由（面向局域网/远程，不参与终端绑定）。
    pub external: bool,
}

impl fmt::Debug for VendorRoute {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VendorRoute")
            .field("route_id", &self.route_id)
            .field("provider_id", &self.provider_id)
            .field("vendor_id", &self.vendor_id)
            .field("mode", &self.mode)
            // 只输出长度：日志里出现明文令牌等同于泄露。
            .field("local_token", &format!("<{} 字节>", self.local_token.len()))
            .field("base_url", &self.base_url)
            .field("external", &self.external)
            .finish()
    }
}

impl VendorRoute {
    /// 该路由在网关上的前缀：`/gateway/<平台>/<路由编号>`。
    #[must_use]
    pub fn prefix(&self) -> String {
        route_prefix(self.provider_id, &self.route_id)
    }
}

/// 构造某路由在指定 host 上的完整 URL。
///
/// `host` 形如 `http://127.0.0.1:port` 或 WSL 探测到的宿主地址。
#[must_use]
pub fn build_route_url(host: &str, provider_id: ProviderId, route_id: &str) -> String {
    format!(
        "{}{}",
        host.trim_end_matches('/'),
        route_prefix(provider_id, route_id)
    )
}

/// 路由前缀。
#[must_use]
pub fn route_prefix(provider_id: ProviderId, route_id: &str) -> String {
    format!("{ROUTE_PREFIX}/{}/{route_id}", provider_id.as_str())
}

/// 从请求路径提取平台标识。
///
/// 路径形状为 `/gateway/<平台>/<路由编号>/…`，因此取第 2 段（跳过 `gateway`）。
#[must_use]
pub fn extract_provider(path: &str) -> Option<ProviderId> {
    segment(path, 1).and_then(parse_provider)
}

/// 从请求路径提取路由编号。
#[must_use]
pub fn extract_route_id(path: &str) -> String {
    segment(path, 2).unwrap_or_default().to_owned()
}

/// 取路径的第 N 段（跳过空段）。
fn segment(path: &str, index: usize) -> Option<&str> {
    let path = path.split('?').next().unwrap_or(path);
    path.split('/').filter(|part| !part.is_empty()).nth(index)
}

/// 解析平台标识。
#[must_use]
pub fn parse_provider(value: &str) -> Option<ProviderId> {
    match value {
        "codex" => Some(ProviderId::Codex),
        "claude" => Some(ProviderId::Claude),
        "gemini" => Some(ProviderId::Gemini),
        "qoder" => Some(ProviderId::Qoder),
        _ => None,
    }
}

/// 鉴权所需的请求信息。
///
/// 与具体 HTTP 框架解耦：这样鉴权逻辑可以在没有真实服务器的情况下穷举测试。
#[derive(Debug, Clone, Default)]
pub struct AuthRequest<'a> {
    /// `Authorization` 头。
    pub authorization: Option<&'a str>,
    /// `x-api-key` 头。
    pub x_api_key: Option<&'a str>,
    /// `x-goog-api-key` 头。
    pub x_goog_api_key: Option<&'a str>,
    /// 查询串里的 `key` 参数（可重复）。
    pub query_keys: Vec<&'a str>,
}

impl<'a> AuthRequest<'a> {
    /// 从查询串解析 `key` 参数。
    #[must_use]
    pub fn from_query(query: Option<&'a str>) -> Vec<&'a str> {
        let Some(query) = query else {
            return Vec::new();
        };
        query
            .split('&')
            .filter_map(|pair| {
                let (name, value) = pair.split_once('=')?;
                (name == "key").then_some(value)
            })
            .collect()
    }

    /// 收集全部候选令牌（去掉 `Bearer ` 前缀）。
    ///
    /// 三个请求头 + 查询参数都可能携带凭据，因为不同 CLI 的约定不同。
    #[must_use]
    pub fn candidates(&self) -> Vec<&str> {
        let mut values: Vec<&str> = Vec::new();
        if let Some(bearer) = bearer_token(self.authorization) {
            values.push(bearer);
        }
        if let Some(value) = self.x_api_key {
            values.push(value);
        }
        if let Some(value) = self.x_goog_api_key {
            values.push(value);
        }
        values.extend(self.query_keys.iter().copied());
        values
    }
}

/// 从 `Authorization` 头提取 `Bearer` 令牌（大小写不敏感）。
#[must_use]
pub fn bearer_token(authorization: Option<&str>) -> Option<&str> {
    let value = authorization?.trim();
    // 手动切分而不是用正则：`Bearer` 前缀是固定长度，且这里在热路径上。
    let (scheme, rest) = value.split_once(char::is_whitespace)?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = rest.trim();
    (!token.is_empty()).then_some(token)
}

/// 校验本地路由令牌。
///
/// 明文常量时间比较。本地令牌只在本机回环地址上使用，明文比较不会泄露额外信息。
#[must_use]
pub fn has_route_token(request: &AuthRequest<'_>, token: &str) -> bool {
    request
        .candidates()
        .into_iter()
        .any(|value| constant_time_eq(value.as_bytes(), token.as_bytes()))
}

/// 校验外部访问令牌。
///
/// 数据库只存 SHA-256，因此比较的是候选值的哈希与已存哈希。候选值长度限制在
/// 1–1024 字节：过短的候选没有意义，过长的候选只会浪费哈希计算（拒绝服务面）。
#[must_use]
pub fn has_external_access_token(request: &AuthRequest<'_>, expected_hash_hex: &str) -> bool {
    let Ok(expected) = hex::decode(expected_hash_hex) else {
        return false;
    };
    // SHA-256 的摘要必须是 32 字节；长度不对说明存的是坏数据，直接拒绝。
    if expected.len() != 32 {
        return false;
    }
    request.candidates().into_iter().any(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > 1024 {
            return false;
        }
        let actual = Sha256::digest(trimmed.as_bytes());
        constant_time_eq(&actual, &expected)
    })
}

/// 常量时间字节比较。
///
/// 长度不等时立即返回 `false`：长度本身是公开信息（现网同样如此），
/// 而内容比较必须无分支。
#[must_use]
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.ct_eq(right).into()
}

/// 去掉查询串里的 `key` 参数，返回 `pathname + search`。
///
/// 上游不需要看到本地的访问凭据；把它透传出去等于把令牌泄露给供应商。
#[must_use]
pub fn sanitize_external_request_path(path: &str, query: Option<&str>) -> String {
    match strip_key(query) {
        Some(rest) => format!("{path}?{rest}"),
        None => path.to_owned(),
    }
}

/// 去掉 `key` 参数后的查询串（含前导 `?`，无参数时为空串）。
#[must_use]
pub fn external_upstream_search(query: Option<&str>) -> String {
    strip_key(query).map_or_else(String::new, |rest| format!("?{rest}"))
}

/// 去掉 `key` 参数；无剩余参数时返回 [`None`]。
fn strip_key(query: Option<&str>) -> Option<String> {
    let query = query?;
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split_once('=').is_none_or(|(name, _)| name != "key"))
        .collect();
    (!kept.is_empty()).then(|| kept.join("&"))
}

/// 内存中的路由注册表。
///
/// 用普通 [`HashMap`] 而不是并发容器：调用方在自己的锁或 actor 里持有它。
/// 路由数量是「每终端一条」，量级很小，不值得为它引入并发结构。
#[derive(Debug, Default)]
pub struct RouteRegistry {
    routes: HashMap<String, VendorRoute>,
    /// 每个平台的外部路由（最多一条）。
    external: HashMap<ProviderId, String>,
}

impl RouteRegistry {
    /// 空注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 插入或替换一条路由。
    pub fn insert(&mut self, route: VendorRoute) {
        if route.external {
            self.external
                .insert(route.provider_id, route.route_id.clone());
        }
        self.routes.insert(route.route_id.clone(), route);
    }

    /// 按编号取路由。
    #[must_use]
    pub fn get(&self, route_id: &str) -> Option<&VendorRoute> {
        self.routes.get(route_id)
    }

    /// 按编号取可变路由。
    pub fn get_mut(&mut self, route_id: &str) -> Option<&mut VendorRoute> {
        self.routes.get_mut(route_id)
    }

    /// 移除路由。
    pub fn remove(&mut self, route_id: &str) -> Option<VendorRoute> {
        let route = self.routes.remove(route_id)?;
        if route.external && self.external.get(&route.provider_id) == Some(&route.route_id) {
            self.external.remove(&route.provider_id);
        }
        Some(route)
    }

    /// 取某平台的外部路由。
    #[must_use]
    pub fn external_route(&self, provider_id: ProviderId) -> Option<&VendorRoute> {
        self.external
            .get(&provider_id)
            .and_then(|route_id| self.routes.get(route_id))
    }

    /// 路由数量。
    #[must_use]
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// 按请求路径查找路由并校验凭据。
    ///
    /// 返回 [`None`] 表示「路径不匹配任何路由」或「凭据不匹配」。**不区分这两种情况**：
    /// 对外区分它们会告诉攻击者「这个路由编号存在，只是令牌不对」。
    #[must_use]
    pub fn authenticate(&self, path: &str, request: &AuthRequest<'_>) -> Option<&VendorRoute> {
        let route = self.get(&extract_route_id(path))?;
        // 平台必须与路径一致：否则一条 Codex 路由会被用来代理 Claude 请求。
        if extract_provider(path) != Some(route.provider_id) {
            return None;
        }
        has_route_token(request, &route.local_token).then_some(route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn route(route_id: &str, token: &str) -> VendorRoute {
        VendorRoute {
            route_id: route_id.to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: "v1".to_owned(),
            mode: RouteMode::Dynamic,
            local_token: token.to_owned(),
            base_url: "https://api.example.com".to_owned(),
            external: false,
        }
    }

    #[test]
    fn route_urls_use_the_documented_shape() {
        assert_eq!(
            build_route_url("http://127.0.0.1:8787", ProviderId::Codex, "r1"),
            "http://127.0.0.1:8787/gateway/codex/r1"
        );
        // host 末尾的斜杠不应产生双斜杠。
        assert_eq!(
            build_route_url("http://127.0.0.1:8787/", ProviderId::Claude, "r2"),
            "http://127.0.0.1:8787/gateway/claude/r2"
        );
    }

    #[test]
    fn path_segments_are_extracted_by_position() {
        assert_eq!(
            extract_provider("/gateway/codex/r1/v1/messages"),
            Some(ProviderId::Codex)
        );
        assert_eq!(extract_route_id("/gateway/codex/r1/v1/messages"), "r1");
        // 查询串不影响解析。
        assert_eq!(extract_route_id("/gateway/gemini/r9?key=x"), "r9");
        // 段数不足。
        assert_eq!(extract_provider("/gateway"), None);
        assert!(extract_route_id("/gateway/codex").is_empty());
    }

    #[test]
    fn unknown_providers_are_rejected() {
        assert_eq!(parse_provider("openai"), None);
        assert_eq!(extract_provider("/gateway/openai/r1"), None);
    }

    #[test]
    fn bearer_tokens_are_parsed_case_insensitively() {
        assert_eq!(bearer_token(Some("Bearer abc")), Some("abc"));
        assert_eq!(bearer_token(Some("bearer abc")), Some("abc"));
        assert_eq!(bearer_token(Some("BEARER  abc  ")), Some("abc"));
        assert_eq!(bearer_token(Some("Basic abc")), None);
        assert_eq!(bearer_token(Some("Bearer ")), None);
        assert_eq!(bearer_token(None), None);
    }

    #[test]
    fn candidates_come_from_every_documented_source() {
        let request = AuthRequest {
            authorization: Some("Bearer from-header"),
            x_api_key: Some("from-api-key"),
            x_goog_api_key: Some("from-goog"),
            query_keys: vec!["from-query"],
        };
        assert_eq!(
            request.candidates(),
            vec!["from-header", "from-api-key", "from-goog", "from-query"]
        );
    }

    #[test]
    fn query_keys_are_parsed_from_the_raw_query() {
        assert_eq!(
            AuthRequest::from_query(Some("key=abc&other=1&key=def")),
            vec!["abc", "def"]
        );
        assert!(AuthRequest::from_query(Some("other=1")).is_empty());
        assert!(AuthRequest::from_query(None).is_empty());
    }

    #[test]
    fn the_local_token_is_accepted_from_any_candidate_source() {
        let token = "local-secret";
        for request in [
            AuthRequest {
                authorization: Some("Bearer local-secret"),
                ..Default::default()
            },
            AuthRequest {
                x_api_key: Some("local-secret"),
                ..Default::default()
            },
            AuthRequest {
                x_goog_api_key: Some("local-secret"),
                ..Default::default()
            },
        ] {
            assert!(has_route_token(&request, token));
        }
        assert!(!has_route_token(&AuthRequest::default(), token));
        assert!(!has_route_token(
            &AuthRequest {
                authorization: Some("Bearer wrong"),
                ..Default::default()
            },
            token
        ));
    }

    #[test]
    fn the_local_token_rejects_prefixes_and_longer_values() {
        // 长度不等必须直接失败，不能因为前缀相同就通过。
        let token = "abcdef";
        for wrong in ["abcde", "abcdefg", "", "abcdeF"] {
            assert!(
                !has_route_token(
                    &AuthRequest {
                        x_api_key: Some(wrong),
                        ..Default::default()
                    },
                    token
                ),
                "{wrong} 不应通过"
            );
        }
    }

    #[test]
    fn external_tokens_compare_against_the_stored_hash() {
        let secret = "external-secret";
        let hash = hex::encode(Sha256::digest(secret.as_bytes()));
        let request = AuthRequest {
            authorization: Some("Bearer external-secret"),
            ..Default::default()
        };
        assert!(has_external_access_token(&request, &hash));
        assert!(!has_external_access_token(
            &AuthRequest {
                authorization: Some("Bearer other"),
                ..Default::default()
            },
            &hash
        ));
    }

    #[test]
    fn external_tokens_accept_query_keys_within_the_length_window() {
        let secret = "k".repeat(64);
        let hash = hex::encode(Sha256::digest(secret.as_bytes()));
        let request = AuthRequest {
            query_keys: vec![secret.as_str()],
            ..Default::default()
        };
        assert!(has_external_access_token(&request, &hash));

        // 超过 1024 字节的候选被拒绝（避免把哈希计算变成拒绝服务面）。
        let long = "k".repeat(1025);
        let long_hash = hex::encode(Sha256::digest(long.as_bytes()));
        let request = AuthRequest {
            query_keys: vec![long.as_str()],
            ..Default::default()
        };
        assert!(!has_external_access_token(&request, &long_hash));
    }

    #[test]
    fn external_tokens_reject_a_malformed_stored_hash() {
        let request = AuthRequest {
            x_api_key: Some("x"),
            ..Default::default()
        };
        // 不是十六进制。
        assert!(!has_external_access_token(&request, "not-hex"));
        // 长度不对（不是 32 字节摘要）。
        assert!(!has_external_access_token(&request, "abcd"));
        assert!(!has_external_access_token(&request, ""));
    }

    #[test]
    fn the_access_key_is_stripped_before_reaching_the_upstream() {
        assert_eq!(
            sanitize_external_request_path("/v1/messages", Some("key=secret&stream=true")),
            "/v1/messages?stream=true"
        );
        // 只有 key 时不留悬挂的 `?`。
        assert_eq!(
            sanitize_external_request_path("/v1/messages", Some("key=secret")),
            "/v1/messages"
        );
        assert_eq!(
            sanitize_external_request_path("/v1/messages", None),
            "/v1/messages"
        );
        assert_eq!(external_upstream_search(Some("key=s&a=1")), "?a=1");
        assert!(external_upstream_search(Some("key=s")).is_empty());
    }

    #[test]
    fn the_registry_authenticates_matching_paths_only() {
        let mut registry = RouteRegistry::new();
        registry.insert(route("r1", "tok"));
        let request = AuthRequest {
            authorization: Some("Bearer tok"),
            ..Default::default()
        };
        assert!(
            registry
                .authenticate("/gateway/codex/r1/v1/messages", &request)
                .is_some()
        );
        // 平台不匹配：一条 Codex 路由不能被用来代理 Claude 请求。
        assert!(
            registry
                .authenticate("/gateway/claude/r1", &request)
                .is_none()
        );
        // 令牌不匹配。
        let wrong = AuthRequest {
            authorization: Some("Bearer nope"),
            ..Default::default()
        };
        assert!(registry.authenticate("/gateway/codex/r1", &wrong).is_none());
        // 路由不存在。
        assert!(
            registry
                .authenticate("/gateway/codex/missing", &request)
                .is_none()
        );
    }

    #[test]
    fn external_routes_are_indexed_per_provider() {
        let mut registry = RouteRegistry::new();
        let mut external = route("ext-codex", "tok");
        external.external = true;
        registry.insert(external);
        let mut other = route("ext-claude", "tok");
        other.external = true;
        other.provider_id = ProviderId::Claude;
        registry.insert(other);

        assert_eq!(
            registry
                .external_route(ProviderId::Codex)
                .map(|r| r.route_id.as_str()),
            Some("ext-codex")
        );
        assert_eq!(
            registry
                .external_route(ProviderId::Claude)
                .map(|r| r.route_id.as_str()),
            Some("ext-claude")
        );
        assert!(registry.external_route(ProviderId::Gemini).is_none());

        // 移除后索引也要清掉。
        registry.remove("ext-codex");
        assert!(registry.external_route(ProviderId::Codex).is_none());
    }

    #[test]
    fn debug_output_never_contains_the_token() {
        let token = "super-secret-token";
        let route = route("r1", token);
        let rendered = format!("{route:?}");
        assert!(!rendered.contains(token));
        // 只输出长度，便于排障时确认令牌已被加载。
        assert!(rendered.contains(&format!("{} 字节", token.len())));
    }

    #[test]
    fn route_modes_control_failover() {
        assert!(RouteMode::Dynamic.allows_failover());
        assert!(!RouteMode::Locked.allows_failover());
    }

    proptest! {
        /// 常量时间比较必须与普通相等判断在语义上完全一致——除了耗时特征。
        #[test]
        fn constant_time_eq_matches_ordinary_equality(left: Vec<u8>, right: Vec<u8>) {
            prop_assert_eq!(constant_time_eq(&left, &right), left == right);
        }

        /// 去掉 `key` 之后，其余参数必须原样保留且顺序不变。
        #[test]
        fn stripping_the_key_keeps_every_other_parameter(
            pairs in proptest::collection::vec("[a-z]{1,6}=[a-z0-9]{0,8}", 0..6),
        ) {
            let query = pairs.join("&");
            let stripped = external_upstream_search(Some(&query));
            for pair in &pairs {
                let name = pair.split_once('=').map(|(name, _)| name).unwrap_or_default();
                if name == "key" {
                    prop_assert!(!stripped.contains(pair.as_str()));
                } else {
                    prop_assert!(stripped.contains(pair.as_str()), "{pair} 丢失于 {stripped}");
                }
            }
        }

        /// 任意路径段组合都不应让提取函数 panic。
        #[test]
        fn segment_extraction_never_panics(path in "[/a-zA-Z0-9?=&.]{0,64}") {
            let _ = extract_provider(&path);
            let _ = extract_route_id(&path);
        }
    }
}
