//! 跨模块共享常量。
//!
//! 这些值直接决定产品行为（分页尺寸、请求上限、故障识别规则），因此必须与现网
//! `shared/constants.ts` 逐字一致。**只抽取稳定的跨模块契约**，不把所有界面文案
//! 机械集中化——那是现网 DEV-GUIDE 明确记录的取舍。

/// 分页默认每页条数。
pub const PAGINATION_DEFAULT_PAGE_SIZE: u32 = 10;
/// 分页可选的每页条数。
pub const PAGINATION_PAGE_SIZE_OPTIONS: [u32; 3] = [10, 20, 50];
/// 分页每页条数上限。
pub const PAGINATION_MAX_PAGE_SIZE: u32 = 50;

/// 网关文本日志配置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayLogConfig {
    /// 内存中请求记录的环形缓冲容量。
    pub request_ring_capacity: usize,
    /// 内存中事件记录的环形缓冲容量。
    pub event_ring_capacity: usize,
    /// 落盘刷新间隔（毫秒）。
    pub flush_interval_ms: u64,
    /// 单个日志文件上限（字节）。
    pub max_file_bytes: u64,
}

/// 网关日志配置常量。
pub const GATEWAY_LOG_CONFIG: GatewayLogConfig = GatewayLogConfig {
    request_ring_capacity: 200,
    event_ring_capacity: 100,
    flush_interval_ms: 200,
    max_file_bytes: 2 * 1024 * 1024,
};

/// 网关请求限制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayRequestLimits {
    /// 请求体上限（字节）。
    pub max_body_bytes: u64,
    /// 可缓冲以便重试的请求体上限（字节）。
    ///
    /// 只有声明长度不超过此值的请求才会被缓冲，也**只有被缓冲的请求才能安全重试**。
    pub retry_buffer_bytes: u64,
    /// 响应体用量捕获上限（字节）。
    pub response_capture_bytes: usize,
    /// 上游超时（毫秒）。
    pub upstream_timeout_ms: u64,
}

/// 网关请求限制常量。
pub const GATEWAY_REQUEST_LIMITS: GatewayRequestLimits = GatewayRequestLimits {
    max_body_bytes: 32 * 1024 * 1024,
    retry_buffer_bytes: 2 * 1024 * 1024,
    response_capture_bytes: 512 * 1024,
    upstream_timeout_ms: 10 * 60 * 1000,
};

/// 探测上游响应前多少个字节，用于识别「HTTP 200 但携带错误」。
///
/// 达到 SSE 事件边界（`\n\n`）或此上限即放行，避免正常流的首 token 被无意义阻塞。
pub const RESPONSE_PROBE_MAX_BYTES: usize = 64 * 1024;

/// 内置故障识别文本模式（不区分大小写的包含匹配）。
pub const GATEWAY_FAILOVER_DEFAULT_PATTERNS: [&str; 7] = [
    "selected model is at capacity",
    "exceeded retry limit",
    "429 too many requests",
    "too many requests",
    "currently overloaded",
    "rate limit",
    "temporarily unavailable",
];

/// 内置故障识别 HTTP 状态码。
pub const GATEWAY_FAILOVER_DEFAULT_STATUS_CODES: [u16; 10] =
    [401, 403, 404, 408, 425, 429, 500, 502, 503, 504];

/// 故障规则限制。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayFailoverRuleLimits {
    /// 单条规则的文本模式长度上限（字符）。
    pub max_pattern_length: usize,
    /// 最多保存多少条自定义规则。
    pub max_rule_count: usize,
    /// 自定义响应正文上限（字节）。
    pub max_custom_response_body_bytes: usize,
}

/// 故障规则限制常量。
pub const GATEWAY_FAILOVER_RULE_LIMITS: GatewayFailoverRuleLimits = GatewayFailoverRuleLimits {
    max_pattern_length: 200,
    max_rule_count: 100,
    max_custom_response_body_bytes: 32 * 1024,
};

/// 健康状态的失败统计窗口（毫秒）。
pub const GATEWAY_FAILURE_WINDOW_MS: i64 = 5 * 60 * 1000;

/// 健康状态成功路径的持久化节流间隔（毫秒）。
///
/// 状态变化立即落盘，只有「持续成功」的重复写入才被节流。
pub const GATEWAY_HEALTH_SUCCESS_PERSIST_INTERVAL_MS: i64 = 5_000;

/// 供应商查询配置的 JSON 体积上限（字节）。
pub const QUERY_CONFIG_MAX_JSON_BYTES: usize = 32 * 1024;

/// 模型列表查询超时（毫秒）。
pub const MODEL_LIST_TIMEOUT_MS: u64 = 15_000;

/// 模型列表响应体积上限（字节）。
pub const MODEL_LIST_MAX_BYTES: usize = 2 * 1024 * 1024;

/// 模型列表最多返回多少条。
pub const MODEL_LIST_MAX_COUNT: usize = 500;

/// 供应商名称长度上限（字符）。
pub const VENDOR_NAME_MAX_LENGTH: usize = 80;

/// 供应商配置标签长度上限（字符）。
pub const VENDOR_CONFIG_LABEL_MAX_LENGTH: usize = 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failover_defaults_match_the_existing_rule_set() {
        // 这些模式与状态码决定「什么算上游故障」，改动会直接改变故障转移行为。
        assert_eq!(GATEWAY_FAILOVER_DEFAULT_PATTERNS.len(), 7);
        assert_eq!(GATEWAY_FAILOVER_DEFAULT_STATUS_CODES.len(), 10);
        assert!(GATEWAY_FAILOVER_DEFAULT_PATTERNS.contains(&"429 too many requests"));
        assert!(GATEWAY_FAILOVER_DEFAULT_STATUS_CODES.contains(&429));
        assert!(GATEWAY_FAILOVER_DEFAULT_STATUS_CODES.contains(&504));
    }

    #[test]
    fn probe_limit_is_sixty_four_kilobytes() {
        assert_eq!(RESPONSE_PROBE_MAX_BYTES, 65_536);
    }

    #[test]
    fn request_limits_match_the_existing_values() {
        assert_eq!(GATEWAY_REQUEST_LIMITS.max_body_bytes, 32 * 1024 * 1024);
        assert_eq!(GATEWAY_REQUEST_LIMITS.retry_buffer_bytes, 2 * 1024 * 1024);
        assert_eq!(GATEWAY_REQUEST_LIMITS.response_capture_bytes, 512 * 1024);
        assert_eq!(GATEWAY_REQUEST_LIMITS.upstream_timeout_ms, 600_000);
        // 可重试缓冲必须远小于请求体上限，否则「缓冲以支持重试」会吃掉全部内存预算。
        // 用 const 块让编译期即可发现回归。
        const {
            assert!(
                GATEWAY_REQUEST_LIMITS.retry_buffer_bytes < GATEWAY_REQUEST_LIMITS.max_body_bytes
            );
        }
    }

    #[test]
    fn pagination_options_are_consistent() {
        assert!(PAGINATION_PAGE_SIZE_OPTIONS.contains(&PAGINATION_DEFAULT_PAGE_SIZE));
        assert_eq!(
            PAGINATION_PAGE_SIZE_OPTIONS.iter().copied().max(),
            Some(PAGINATION_MAX_PAGE_SIZE)
        );
    }

    #[test]
    fn rule_limits_match_the_existing_values() {
        assert_eq!(GATEWAY_FAILOVER_RULE_LIMITS.max_pattern_length, 200);
        assert_eq!(GATEWAY_FAILOVER_RULE_LIMITS.max_rule_count, 100);
        assert_eq!(
            GATEWAY_FAILOVER_RULE_LIMITS.max_custom_response_body_bytes,
            32 * 1024
        );
    }
}
