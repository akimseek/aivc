//! 本地 Gateway：路由、上游转发、故障转移、熔断、规则、用量与日志。
//!
//! # 结构
//!
//! - [`routes`]：路由注册表、对外 URL 构造与鉴权（含常量时间令牌比较）。
//! - [`upstream`]：上游请求头构造与 URL 拼接。
//!
//! # 领域与实现的分工
//!
//! 故障转移规则、用量汇总、请求记录等**领域模型**已经在 [`aivc_core::gateway`] 与
//! [`aivc_core::gateway_usage`] 里；本 crate 只负责把它们接到真实的 HTTP 代理上。
//! 因此这里不重复定义规则语义，只做「读取 → 判定 → 转发」的编排。
//!
//! # 安全要点
//!
//! - 令牌比较一律常量时间，避免逐字节爆破；
//! - 本地上游凭据（`local_token`）与外部访问凭据都不透传给供应商；
//! - 上游地址只允许 `http` / `https`，且不得携带查询串或片段。

#![forbid(unsafe_code)]

pub mod failover;
pub mod proxy;
pub mod record;
pub mod response;
pub mod routes;
pub mod server;
pub mod source;
pub mod upstream;

pub use failover::{
    CircuitConfig, HealthTracker, choose_next_vendor, choose_vendor, is_configured_candidate,
};
pub use proxy::{
    FailureAction, FailureContext, NoNextAction, decide_after_failure, is_retryable_method,
    is_self_upstream, plan_max_attempts,
};
pub use record::{RequestTracker, sanitize_path};
pub use response::{
    extract_gateway_response_error, is_builtin_gateway_failover_error,
    matches_gateway_failover_error,
};
pub use routes::{
    AuthRequest, RouteMode, RouteRegistry, VendorRoute, build_route_url, has_external_access_token,
    has_route_token, route_prefix,
};
pub use server::{
    GatewayHandle, GatewaySettings, GatewaySource, GatewayState, HealthHook, LoadHealth,
    SaveHealth, SwitchHook, SwitchNotice, router, serve,
};
pub use source::{RecordHook, RepositorySource};
pub use upstream::{build_upstream_headers, join_upstream_url};
