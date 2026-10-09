//! 代理转发的尝试规划与失败处置。
//!
//! # 为什么把这段逻辑单独抽出来
//!
//! 故障转移的判定条件多且互相嵌套（尝试次数、同一供应商连续失败数、是否还有候选、
//! 失败来自连接还是响应），而它又决定了「客户端最终看到什么」。把它写成纯函数之后，
//! 每个分支都能单独构造并断言，不必真的起一个上游服务器去制造每种失败。
//!
//! # 三条容易搞错的规则
//!
//! 1. **只有已缓冲的请求体才能重试**。流式请求体读过一次就没了，重试会发出空 body。
//!    因此 `max_attempts` 在流式场景下恒为 1。
//! 2. **失败阈值是「同一供应商连续失败多少次后切换」**，不是「总共失败多少次」。
//!    因此 `max_attempts = 阈值 × 候选数`，让每个候选都有完整的尝试额度。
//! 3. **没有其他候选时，连接失败与响应失败的处置不同**：
//!    - 连接失败 → 抛出错误（没有响应可回放）；
//!    - 响应失败 → **回放最后一次上游响应**，让客户端看到真实错误而不是 502。
//!
//! # 自我代理防护
//!
//! 若供应商地址指向网关自己，转发会变成无限循环。因此转发前必须比对网关自身地址。

use aivc_core::constants::GATEWAY_REQUEST_LIMITS;

/// 网关对外返回的错误文案（与现网逐字一致）。
pub mod messages {
    /// 路由不存在。
    pub const ROUTE_NOT_FOUND: &str = "gateway route not found";
    /// 未授权。
    pub const UNAUTHORIZED: &str = "gateway unauthorized";
    /// 外部 API 已关闭。
    pub const EXTERNAL_API_DISABLED: &str = "gateway external API is disabled";
    /// 没有可用供应商。
    pub const VENDOR_UNAVAILABLE: &str = "gateway vendor unavailable";
    /// 供应商地址指向本地网关。
    pub const LOCAL_GATEWAY_UPSTREAM: &str = "供应商地址不能指向本地 Gateway。";
    /// 完全没有可用供应商。
    pub const NO_AVAILABLE_VENDOR: &str = "没有可用的供应商。";
    /// 上游超时。
    pub const UPSTREAM_TIMEOUT: &str = "上游响应超时。";
    /// 上游请求失败。
    pub const UPSTREAM_REQUEST_FAILED: &str = "gateway upstream request failed";
    /// 兜底失败文案。
    pub const GENERIC_FAILURE: &str = "Gateway 请求失败。";
}

/// 该 HTTP 方法是否允许重试。
///
/// 查询接口（GET/HEAD）和生成接口均允许在收到失败响应前重试。
#[must_use]
pub fn is_retryable_method(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "POST" | "PUT" | "PATCH")
}

/// 规划本次请求的最大尝试次数。
///
/// 见模块文档第 1、2 条：请求体未缓冲时恒为 1；否则为「阈值 × 候选数」，下限 1。
#[must_use]
pub fn plan_max_attempts(
    body_buffered: bool,
    method: &str,
    failure_threshold: usize,
    candidate_count: usize,
) -> usize {
    if !body_buffered || !is_retryable_method(method) {
        return 1;
    }
    failure_threshold
        .saturating_mul(candidate_count.max(1))
        .max(1)
}

/// 没有其他候选时的处置方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoNextAction {
    /// 抛出错误（连接失败：没有响应可回放）。
    PropagateError,
    /// 回放最后一次上游响应（响应失败：让客户端看到真实错误而不是 502）。
    ReplayLastResponse,
}

/// 一次失败之后的处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureAction {
    /// 重试同一供应商。
    RetrySameVendor,
    /// 切换到下一个候选供应商。
    FailoverToNext,
    /// 没有其他候选，按 [`NoNextAction`] 处置。
    NoNext(NoNextAction),
    /// 不再重试：错误不可重试，或已达尝试上限。
    Stop,
}

/// 判定所需的上下文。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureContext {
    /// 当前是第几次尝试（0 起）。
    pub attempt: usize,
    /// 最大尝试次数。
    pub max_attempts: usize,
    /// 当前供应商已连续失败次数（含本次）。
    pub failures_on_vendor: usize,
    /// 切换阈值。
    pub failure_threshold: usize,
    /// 是否还有未尝试且未熔断的其他候选。
    pub has_next_vendor: bool,
    /// 该失败是否属于可重试类型。
    pub retryable: bool,
}

impl FailureContext {
    /// 是否还有剩余尝试额度。
    #[must_use]
    pub const fn has_attempts_left(&self) -> bool {
        self.attempt + 1 < self.max_attempts
    }
}

/// 判定一次失败之后的处置。
#[must_use]
pub fn decide_after_failure(context: &FailureContext, on_no_next: NoNextAction) -> FailureAction {
    if !context.retryable || !context.has_attempts_left() {
        return FailureAction::Stop;
    }
    if context.failures_on_vendor < context.failure_threshold {
        // 同一供应商还没到切换阈值：继续用它。
        return FailureAction::RetrySameVendor;
    }
    if context.has_next_vendor {
        FailureAction::FailoverToNext
    } else {
        FailureAction::NoNext(on_no_next)
    }
}

/// 供应商地址是否指向网关自身。
///
/// 命中时**必须拒绝转发**：否则请求会在网关与「上游」之间无限循环，直到耗尽资源。
/// 比较去掉末尾斜杠，避免 `http://127.0.0.1:8787` 与 `http://127.0.0.1:8787/` 漏判。
#[must_use]
pub fn is_self_upstream(upstream_url: &str, gateway_address: &str) -> bool {
    if gateway_address.is_empty() {
        return false;
    }
    let (Ok(upstream), Ok(gateway)) = (
        reqwest::Url::parse(upstream_url),
        reqwest::Url::parse(gateway_address),
    ) else {
        return false;
    };
    let is_local = |url: &reqwest::Url| {
        url.host_str().is_some_and(|host| {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback() || ip.is_unspecified())
        })
    };
    upstream.port_or_known_default() == gateway.port_or_known_default()
        && (upstream.host_str() == gateway.host_str()
            || (is_local(&upstream) && is_local(&gateway)))
}

/// 请求体读取上限。
#[must_use]
pub const fn max_body_bytes() -> u64 {
    GATEWAY_REQUEST_LIMITS.max_body_bytes
}

/// 可缓冲重试的请求体上限。
#[must_use]
pub const fn retry_buffer_bytes() -> u64 {
    GATEWAY_REQUEST_LIMITS.retry_buffer_bytes
}

/// 上游超时（毫秒）。
#[must_use]
pub const fn upstream_timeout_ms() -> u64 {
    GATEWAY_REQUEST_LIMITS.upstream_timeout_ms
}

/// 声明长度是否值得缓冲。
///
/// 超过上限的请求体不缓冲：它无法参与重试，但仍要**透传**——直接拒绝会让大请求
/// （长上下文、大附件）完全不可用。
#[must_use]
pub fn should_buffer_body(declared_length: u64) -> bool {
    declared_length <= retry_buffer_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(
        attempt: usize,
        max_attempts: usize,
        failures_on_vendor: usize,
        has_next_vendor: bool,
    ) -> FailureContext {
        FailureContext {
            attempt,
            max_attempts,
            failures_on_vendor,
            failure_threshold: 2,
            has_next_vendor,
            retryable: true,
        }
    }

    #[test]
    fn query_and_generation_methods_are_retryable() {
        assert!(is_retryable_method("POST"));
        assert!(is_retryable_method("PUT"));
        assert!(is_retryable_method("PATCH"));
        assert!(is_retryable_method("GET"));
        assert!(is_retryable_method("HEAD"));
        assert!(!is_retryable_method("DELETE"));
    }

    #[test]
    fn a_streamed_body_is_never_retried() {
        // 流式请求体读过一次就没了；重试会发出空 body。
        assert_eq!(plan_max_attempts(false, "POST", 3, 4), 1);
        assert_eq!(plan_max_attempts(true, "GET", 3, 4), 12);
        assert_eq!(plan_max_attempts(true, "POST", 3, 4), 12);
    }

    #[test]
    fn attempt_budget_covers_every_candidate() {
        // 阈值是「每个供应商的连续失败额度」，因此总尝试数是阈值 × 候选数。
        assert_eq!(plan_max_attempts(true, "POST", 2, 1), 2);
        assert_eq!(plan_max_attempts(true, "POST", 2, 3), 6);
        // 候选数为 0 时也要给出至少一次尝试，否则请求直接失败。
        assert_eq!(plan_max_attempts(true, "POST", 2, 0), 2);
        // 阈值为 0 时下限为 1。
        assert_eq!(plan_max_attempts(true, "POST", 0, 3), 1);
    }

    #[test]
    fn below_the_threshold_the_same_vendor_is_retried() {
        let action = decide_after_failure(&context(0, 6, 1, true), NoNextAction::PropagateError);
        assert_eq!(action, FailureAction::RetrySameVendor);
    }

    #[test]
    fn at_the_threshold_a_failover_happens() {
        let action = decide_after_failure(&context(1, 6, 2, true), NoNextAction::PropagateError);
        assert_eq!(action, FailureAction::FailoverToNext);
    }

    #[test]
    fn a_connection_error_without_alternatives_propagates() {
        // 连接失败没有响应可回放，只能抛出。
        let action = decide_after_failure(&context(1, 6, 2, false), NoNextAction::PropagateError);
        assert_eq!(action, FailureAction::NoNext(NoNextAction::PropagateError));
    }

    #[test]
    fn a_response_error_without_alternatives_replays_the_last_response() {
        // 响应失败时真实的上游响应还在，回放它比抛 502 更有用。
        let action =
            decide_after_failure(&context(1, 6, 2, false), NoNextAction::ReplayLastResponse);
        assert_eq!(
            action,
            FailureAction::NoNext(NoNextAction::ReplayLastResponse)
        );
    }

    #[test]
    fn the_last_attempt_never_retries() {
        // 已达尝试上限：即便额度看起来还有剩余也不再重试。
        let action = decide_after_failure(&context(5, 6, 0, true), NoNextAction::PropagateError);
        assert_eq!(action, FailureAction::Stop);
        // 单次尝试的场景。
        let action = decide_after_failure(&context(0, 1, 0, true), NoNextAction::PropagateError);
        assert_eq!(action, FailureAction::Stop);
    }

    #[test]
    fn a_non_retryable_failure_stops_immediately() {
        let mut ctx = context(0, 6, 1, true);
        ctx.retryable = false;
        assert_eq!(
            decide_after_failure(&ctx, NoNextAction::PropagateError),
            FailureAction::Stop
        );
    }

    #[test]
    fn the_self_proxy_guard_matches_with_and_without_a_trailing_slash() {
        let gateway = "http://127.0.0.1:8787";
        assert!(is_self_upstream(
            "http://127.0.0.1:8787/v1/messages",
            gateway
        ));
        assert!(is_self_upstream("http://127.0.0.1:8787/", gateway));
        assert!(is_self_upstream("http://127.0.0.1:8787", gateway));
        assert!(is_self_upstream("http://localhost:8787/v1", gateway));
        assert!(is_self_upstream("http://[::1]:8787/v1", gateway));
        // 不同端口不是自己。
        assert!(!is_self_upstream("http://127.0.0.1:8788/v1", gateway));
        // 外部地址不是自己。
        assert!(!is_self_upstream("https://api.example.com/v1", gateway));
        // 网关地址为空（尚未启动）时不做判断。
        assert!(!is_self_upstream("http://127.0.0.1:8787/v1", ""));
    }

    #[test]
    fn the_self_proxy_guard_tolerates_a_trailing_slash_on_the_gateway_address() {
        assert!(is_self_upstream(
            "http://127.0.0.1:8787/v1",
            "http://127.0.0.1:8787/"
        ));
    }

    #[test]
    fn only_reasonably_sized_declared_bodies_are_buffered() {
        assert!(should_buffer_body(0), "空请求体可以安全重试");
        assert!(should_buffer_body(1));
        assert!(should_buffer_body(retry_buffer_bytes()));
        // 超限的请求体不缓冲，但仍会透传——直接拒绝会让大请求完全不可用。
        assert!(!should_buffer_body(retry_buffer_bytes() + 1));
    }

    #[test]
    fn limits_come_from_the_shared_constants() {
        assert_eq!(max_body_bytes(), GATEWAY_REQUEST_LIMITS.max_body_bytes);
        assert_eq!(
            retry_buffer_bytes(),
            GATEWAY_REQUEST_LIMITS.retry_buffer_bytes
        );
        assert_eq!(
            upstream_timeout_ms(),
            GATEWAY_REQUEST_LIMITS.upstream_timeout_ms
        );
        assert!(retry_buffer_bytes() <= max_body_bytes());
    }
}
