//! 网关领域类型：供应商健康、故障规则、用量与请求日志。
//!
//! 字段名与现网 `shared/types.ts` 一一对应（JSON 为 camelCase），因为其中大部分会经
//! IPC 直接送到渲染层。
//!
//! # 两处刻意的严格化
//!
//! - **规则文本长度按字符而非 UTF-16 码元计数**（现网 `pattern.length`）。JS 的
//!   `length` 对 emoji 等星平面字符算 2，Rust 的 `chars().count()` 算 1。取字符数更
//!   符合「200 个字符」的直觉，且不会因为代理对被切开而产生非法字符串。
//! - **用量合并取逐项最大值**（与现网一致），因为同一请求可能多次上报用量，
//!   累加会重复计数。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::constants::{
    GATEWAY_FAILOVER_RULE_LIMITS, GATEWAY_REQUEST_LIMITS, QUERY_CONFIG_MAX_JSON_BYTES,
};
use crate::error::{GatewayError, Result};
use crate::id::{RequestId, VendorId};
use crate::target::ProviderId;
use crate::time::Timestamp;

// ---------------------------------------------------------------------------
// 供应商健康
// ---------------------------------------------------------------------------

/// 供应商健康状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GatewayVendorHealthStatus {
    /// 正常。
    Healthy,
    /// 有失败但未达熔断阈值。
    Degraded,
    /// 熔断中。
    Open,
    /// 熔断期已过，等待下一次请求验证。
    HalfOpen,
}

impl GatewayVendorHealthStatus {
    /// 序列化用的短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Open => "open",
            Self::HalfOpen => "half-open",
        }
    }

    /// 由短名解析；未知值回退为 [`Self::Healthy`]。
    #[must_use]
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "degraded" => Self::Degraded,
            "open" => Self::Open,
            "half-open" => Self::HalfOpen,
            _ => Self::Healthy,
        }
    }
}

/// 供应商健康快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayVendorHealth {
    /// 供应商编号。
    pub vendor_id: VendorId,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 当前状态。
    pub status: GatewayVendorHealthStatus,
    /// 累计失败次数。
    pub failure_count: u64,
    /// 累计成功次数。
    pub success_count: u64,
    /// 失败率。
    pub failure_rate: f64,
    /// 最近失败时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_at: Option<Timestamp>,
    /// 最近成功时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<Timestamp>,
    /// 熔断截止时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circuit_until: Option<Timestamp>,
    /// 最近失败原因（已截断）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure_reason: Option<String>,
}

// ---------------------------------------------------------------------------
// 故障规则
// ---------------------------------------------------------------------------

/// 故障规则作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailoverRuleScope {
    /// 全局。
    Global,
    /// 按平台。
    Provider,
    /// 按供应商。
    Vendor,
}

/// 一条故障识别规则。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailoverRule {
    /// 规则编号。
    pub id: String,
    /// 作用域。
    pub scope: FailoverRuleScope,
    /// 平台（`global` 之外必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ProviderId>,
    /// 供应商（`vendor` 作用域必填）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<VendorId>,
    /// 错误文本模式（不区分大小写的包含匹配，不是正则）。
    pub pattern: String,
    /// 命中即视为故障的 HTTP 状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_codes: Option<Vec<u16>>,
    /// 命中时返回给客户端的自定义状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_response_status: Option<u16>,
    /// 命中时返回给客户端的自定义正文（合法 JSON）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_response_body: Option<String>,
    /// 是否启用。
    pub enabled: bool,
    /// 创建时间。
    pub created_at: Timestamp,
    /// 更新时间。
    pub updated_at: Timestamp,
}

/// 保存故障规则时的输入。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailoverRuleInput {
    /// 规则编号；为空表示新建。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 作用域。
    pub scope: Option<FailoverRuleScope>,
    /// 平台。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ProviderId>,
    /// 供应商。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor_id: Option<VendorId>,
    /// 错误文本模式。
    #[serde(default)]
    pub pattern: String,
    /// 状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_codes: Option<Vec<u16>>,
    /// 自定义响应状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_response_status: Option<u16>,
    /// 自定义响应正文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_response_body: Option<String>,
    /// 是否启用；缺省视为启用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

/// 归一化后的故障规则输入。
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedFailoverRule {
    /// 规则编号；`None` 表示新建。
    pub id: Option<String>,
    /// 作用域。
    pub scope: FailoverRuleScope,
    /// 平台。
    pub provider_id: Option<ProviderId>,
    /// 供应商。
    pub vendor_id: Option<VendorId>,
    /// 已归一化的模式。
    pub pattern: String,
    /// 已去重排序的状态码。
    pub status_codes: Vec<u16>,
    /// 自定义响应状态码。
    pub custom_response_status: Option<u16>,
    /// 自定义响应正文。
    pub custom_response_body: Option<String>,
    /// 是否启用。
    pub enabled: bool,
}

impl FailoverRuleInput {
    /// 归一化并校验。
    ///
    /// 校验顺序与现网一致：先作用域，再匹配条件，最后自定义响应。顺序会影响用户看到
    /// 哪一条错误，因此不能随意调整。
    pub fn normalize(&self) -> Result<NormalizedFailoverRule> {
        let Some(scope) = self.scope else {
            return Err(GatewayError::FailoverRuleScopeInvalid.into());
        };

        // 模式归一：换行/制表符折叠为空格（现网会把多行粘贴的规则压成一行）。
        let pattern: String = self
            .pattern
            .trim()
            .chars()
            .map(|ch| {
                if ch == '\n' || ch == '\r' || ch == '\t' {
                    ' '
                } else {
                    ch
                }
            })
            .collect();
        let pattern = pattern.trim().to_owned();

        let mut status_codes = self.status_codes.clone().unwrap_or_default();
        status_codes.sort_unstable();
        status_codes.dedup();

        if pattern.is_empty() && status_codes.is_empty() {
            return Err(GatewayError::FailoverRuleNeedsCondition.into());
        }
        if pattern.chars().count() > GATEWAY_FAILOVER_RULE_LIMITS.max_pattern_length {
            return Err(GatewayError::FailoverRulePatternTooLong {
                limit: GATEWAY_FAILOVER_RULE_LIMITS.max_pattern_length,
            }
            .into());
        }
        if status_codes.iter().any(|code| !(400..=599).contains(code)) {
            return Err(GatewayError::FailoverRuleStatusCodeInvalid.into());
        }

        let has_status = self.custom_response_status.is_some();
        let body = self
            .custom_response_body
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if has_status != body.is_some() {
            return Err(GatewayError::FailoverRuleCustomResponseIncomplete.into());
        }
        if let Some(status) = self.custom_response_status
            && !(100..=599).contains(&status)
        {
            return Err(GatewayError::FailoverRuleCustomStatusInvalid.into());
        }
        if let Some(body) = body {
            if body.len() > GATEWAY_FAILOVER_RULE_LIMITS.max_custom_response_body_bytes {
                return Err(GatewayError::FailoverRuleCustomBodyTooLarge {
                    limit_kb: GATEWAY_FAILOVER_RULE_LIMITS.max_custom_response_body_bytes / 1024,
                }
                .into());
            }
            if serde_json::from_str::<Value>(body).is_err() {
                return Err(GatewayError::FailoverRuleCustomBodyInvalidJson.into());
            }
        }

        match scope {
            FailoverRuleScope::Global => {
                if self.provider_id.is_some() || self.vendor_id.is_some() {
                    return Err(GatewayError::FailoverRuleScopeConflict.into());
                }
            }
            FailoverRuleScope::Provider => {
                if self.provider_id.is_none() {
                    return Err(GatewayError::FailoverRuleProviderRequired.into());
                }
                if self.vendor_id.is_some() {
                    return Err(GatewayError::FailoverRuleProviderScopeConflict.into());
                }
            }
            FailoverRuleScope::Vendor => {
                if self.provider_id.is_none() {
                    return Err(GatewayError::FailoverRuleProviderRequired.into());
                }
                if self.vendor_id.as_ref().is_none_or(VendorId::is_empty) {
                    return Err(GatewayError::FailoverRuleVendorRequired.into());
                }
            }
        }

        Ok(NormalizedFailoverRule {
            id: self
                .id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            scope,
            provider_id: self.provider_id,
            vendor_id: self.vendor_id.clone(),
            pattern,
            status_codes,
            custom_response_status: self.custom_response_status,
            custom_response_body: body.map(str::to_owned),
            // 现网是 `input.enabled !== false`：缺省视为启用。
            enabled: self.enabled != Some(false),
        })
    }
}

impl FailoverRule {
    /// 该规则是否适用于给定平台与供应商。
    #[must_use]
    pub fn applies_to(&self, provider_id: ProviderId, vendor_id: Option<&VendorId>) -> bool {
        match self.scope {
            FailoverRuleScope::Global => true,
            FailoverRuleScope::Provider => self.provider_id == Some(provider_id),
            FailoverRuleScope::Vendor => {
                self.provider_id == Some(provider_id)
                    && vendor_id.is_some_and(|id| self.vendor_id.as_ref() == Some(id))
            }
        }
    }

    /// 该规则是否命中给定状态码与错误文本。
    ///
    /// 匹配方式是不区分大小写的**安全包含匹配**（不开放正则，避免回溯开销与错误配置
    /// 导致的请求路径异常）。
    #[must_use]
    pub fn matches(&self, status_code: Option<u16>, message: Option<&str>) -> bool {
        match_rule(
            &self.pattern,
            self.status_codes.as_deref(),
            status_code,
            message,
        )
        .is_some()
    }
}

/// 规则命中的**是哪一条**。
///
/// 「测试规则」这个功能需要它：只回一个布尔值，用户看到「没命中」仍然不知道
/// 是自己状态码写错了还是错误文本写错了。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RuleHit {
    /// 被状态码命中。
    Status,
    /// 被错误文本命中（不区分大小写的包含匹配）。
    Pattern,
}

/// 判一条规则是否命中，以及**被哪一项命中**。
///
/// # 为什么抽成自由函数
///
/// `FailoverRule::matches`（转发路径用）与「测试规则」命令必须用**同一份**判断，
/// 否则会出现「测试说会命中、实际没切换」这种最难查的偏差。
///
/// # 顺序与语义
///
/// 状态码先判：它是**精确**匹配，而文本是包含匹配——两者都命中时报状态码更有用。
/// 任一命中即为命中（旧平台也是「或」的关系）。
#[must_use]
pub fn match_rule(
    pattern: &str,
    status_codes: Option<&[u16]>,
    status_code: Option<u16>,
    message: Option<&str>,
) -> Option<RuleHit> {
    if let (Some(code), Some(codes)) = (status_code, status_codes)
        && codes.contains(&code)
    {
        return Some(RuleHit::Status);
    }
    if pattern.is_empty() {
        return None;
    }
    let message = message?;
    message
        .to_lowercase()
        .contains(&pattern.to_lowercase())
        .then_some(RuleHit::Pattern)
}

// ---------------------------------------------------------------------------
// 用量
// ---------------------------------------------------------------------------

/// 单次请求的用量。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayUsage {
    /// 模型名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 输入 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// 输出 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// 命中缓存的输入 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    /// 推理 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_tokens: Option<u64>,
    /// 总 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    /// 费用（美元）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

impl GatewayUsage {
    /// 逐项取较大值合并。
    ///
    /// 同一请求可能多次上报用量（例如 SSE 中途与结束各一次），**累加会重复计数**，
    /// 因此现网与这里都取最大值。
    pub fn merge(&mut self, next: &Self) {
        macro_rules! take_max {
            ($($field:ident),* $(,)?) => {
                $( if let Some(value) = next.$field {
                    self.$field = Some(self.$field.map_or(value, |current| current.max(value)));
                } )*
            };
        }
        take_max!(
            input_tokens,
            output_tokens,
            cached_input_tokens,
            reasoning_tokens,
            total_tokens
        );
        if let Some(value) = next.cost_usd {
            self.cost_usd = Some(self.cost_usd.map_or(value, |current| current.max(value)));
        }
        if next.model.is_some() {
            self.model.clone_from(&next.model);
        }
    }

    /// 是否携带任何用量数据。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.reasoning_tokens.is_none()
            && self.total_tokens.is_none()
            && self.cost_usd.is_none()
    }
}

/// 请求结果分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GatewayOutcome {
    /// 成功。
    Ok,
    /// 客户端主动断开。
    ClientAborted,
    /// 上游超时。
    Timeout,
    /// 其他失败。
    Error,
}

impl GatewayOutcome {
    /// 序列化短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::ClientAborted => "client-aborted",
            Self::Timeout => "timeout",
            Self::Error => "error",
        }
    }

    /// 由短名解析；未知值回退为 [`Self::Error`]（保守：未知一律按失败统计）。
    #[must_use]
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "ok" => Self::Ok,
            "client-aborted" => Self::ClientAborted,
            "timeout" => Self::Timeout,
            _ => Self::Error,
        }
    }

    /// 是否计为成功。
    #[must_use]
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Ok)
    }
}

/// 一条网关请求记录（写入 `gateway_request_logs`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayRequestRecord {
    /// 请求编号。
    pub request_id: RequestId,
    /// 路由编号。
    pub route_id: String,
    /// 平台。
    pub provider_id: ProviderId,
    /// 最终使用的供应商。
    pub vendor_id: VendorId,
    /// HTTP 方法。
    pub method: String,
    /// 请求路径（已截断）。
    pub path: String,
    /// 模型名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 上游 HTTP 状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    /// 结果分类。
    pub outcome: GatewayOutcome,
    /// 耗时（毫秒）。
    pub duration_ms: u64,
    /// 上行字节数。
    pub bytes_in: u64,
    /// 下行字节数。
    pub bytes_out: u64,
    /// 重试次数。
    pub retry_count: u32,
    /// 是否发生过故障切换。
    pub switched: bool,
    /// 错误代码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// 错误摘要（已截断、已清洗换行）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// 用量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<GatewayUsage>,
    /// 记录时间。
    pub created_at: Timestamp,
}

/// 路径字段的截断长度。
pub const REQUEST_PATH_MAX_LENGTH: usize = 500;
/// 错误摘要的截断长度。
pub const REQUEST_ERROR_MAX_LENGTH: usize = 500;

/// 按现网规则清洗错误摘要：折叠换行与制表符，再截断。
#[must_use]
pub fn sanitize_error_message(message: &str) -> String {
    let collapsed: String = message
        .chars()
        .map(|ch| {
            if ch == '\n' || ch == '\r' || ch == '\t' {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let trimmed = collapsed.trim();
    crate::text::clamp_raw(trimmed, REQUEST_ERROR_MAX_LENGTH)
}

/// 校验查询配置的 JSON 体积。
///
/// 现网用 `Buffer.byteLength(JSON.stringify(config))`，这里用序列化后的字节数。
pub fn assert_query_config_size(value: &Value) -> Result<()> {
    let bytes = serde_json::to_string(value).map_or(0, |text| text.len());
    if bytes > QUERY_CONFIG_MAX_JSON_BYTES {
        return Err(GatewayError::GenericFailure.into());
    }
    Ok(())
}

/// 请求体是否在可缓冲重试的范围内。
#[must_use]
pub const fn is_retry_bufferable(declared_length: u64) -> bool {
    declared_length > 0 && declared_length <= GATEWAY_REQUEST_LIMITS.retry_buffer_bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(input: &FailoverRuleInput) -> Result<NormalizedFailoverRule> {
        input.normalize()
    }

    fn global_rule(pattern: &str) -> FailoverRuleInput {
        FailoverRuleInput {
            scope: Some(FailoverRuleScope::Global),
            pattern: pattern.to_owned(),
            ..FailoverRuleInput::default()
        }
    }

    #[test]
    fn health_status_round_trips_through_its_short_name() {
        for status in [
            GatewayVendorHealthStatus::Healthy,
            GatewayVendorHealthStatus::Degraded,
            GatewayVendorHealthStatus::Open,
            GatewayVendorHealthStatus::HalfOpen,
        ] {
            assert_eq!(
                GatewayVendorHealthStatus::from_str_lossy(status.as_str()),
                status
            );
        }
        assert_eq!(
            GatewayVendorHealthStatus::from_str_lossy("garbage"),
            GatewayVendorHealthStatus::Healthy
        );
        assert_eq!(
            serde_json::to_string(&GatewayVendorHealthStatus::HalfOpen).unwrap(),
            "\"half-open\""
        );
    }

    #[test]
    fn outcome_parsing_is_conservative_for_unknown_values() {
        assert_eq!(GatewayOutcome::from_str_lossy("ok"), GatewayOutcome::Ok);
        assert_eq!(
            GatewayOutcome::from_str_lossy("client-aborted"),
            GatewayOutcome::ClientAborted
        );
        // 未知结果一律按失败统计，绝不能默认成成功。
        assert_eq!(GatewayOutcome::from_str_lossy("???"), GatewayOutcome::Error);
        assert!(GatewayOutcome::Ok.is_success());
        assert!(!GatewayOutcome::Timeout.is_success());
    }

    #[test]
    fn rule_requires_a_scope() {
        let error = rule(&FailoverRuleInput {
            pattern: "x".to_owned(),
            ..FailoverRuleInput::default()
        })
        .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "Gateway 故障规则作用域无效。");
    }

    #[test]
    fn rule_requires_at_least_one_condition() {
        let error = rule(&global_rule("   ")).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "请至少配置错误文本或 HTTP 状态码。");

        // 只有状态码也可以。
        let normalized = rule(&FailoverRuleInput {
            scope: Some(FailoverRuleScope::Global),
            status_codes: Some(vec![503]),
            ..FailoverRuleInput::default()
        })
        .expect("状态码足以构成条件");
        assert_eq!(normalized.status_codes, vec![503]);
    }

    #[test]
    fn rule_collapses_whitespace_in_patterns() {
        let normalized = rule(&global_rule("  rate\nlimit\tnow  ")).expect("normalize");
        assert_eq!(normalized.pattern, "rate limit now");
    }

    #[test]
    fn rule_deduplicates_and_sorts_status_codes() {
        let normalized = rule(&FailoverRuleInput {
            scope: Some(FailoverRuleScope::Global),
            status_codes: Some(vec![503, 429, 503, 500]),
            ..FailoverRuleInput::default()
        })
        .expect("normalize");
        assert_eq!(normalized.status_codes, vec![429, 500, 503]);
    }

    #[test]
    fn rule_rejects_out_of_range_status_codes() {
        for code in [200, 399, 600] {
            let error = rule(&FailoverRuleInput {
                scope: Some(FailoverRuleScope::Global),
                status_codes: Some(vec![code]),
                ..FailoverRuleInput::default()
            })
            .expect_err("必须拒绝");
            assert_eq!(
                error.to_string(),
                "HTTP 状态码必须是 400 到 599 之间的整数。"
            );
        }
    }

    #[test]
    fn rule_rejects_overlong_patterns_by_character_count() {
        let long = "字".repeat(GATEWAY_FAILOVER_RULE_LIMITS.max_pattern_length + 1);
        let error = rule(&global_rule(&long)).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "Gateway 故障规则不能超过 200 个字符。");

        // 恰好 200 个字符必须接受（边界含等号）。
        let exact = "字".repeat(GATEWAY_FAILOVER_RULE_LIMITS.max_pattern_length);
        assert!(rule(&global_rule(&exact)).is_ok());
    }

    #[test]
    fn custom_response_requires_both_status_and_body() {
        let mut input = global_rule("x");
        input.custom_response_status = Some(418);
        let error = rule(&input).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "自定义返回状态码和内容必须同时配置。");

        let mut input = global_rule("x");
        input.custom_response_body = Some("{\"a\":1}".to_owned());
        let error = rule(&input).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "自定义返回状态码和内容必须同时配置。");
    }

    #[test]
    fn custom_response_status_range_differs_from_failover_status_range() {
        // 故障状态码限定 400–599，而自定义返回状态码允许 100–599。
        let mut input = global_rule("x");
        input.custom_response_status = Some(200);
        input.custom_response_body = Some("{}".to_owned());
        assert!(rule(&input).is_ok(), "200 是合法的自定义返回状态码");

        let mut input = global_rule("x");
        input.custom_response_status = Some(99);
        input.custom_response_body = Some("{}".to_owned());
        let error = rule(&input).expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "自定义返回状态码必须是 100 到 599 之间的整数。"
        );
    }

    #[test]
    fn custom_response_body_must_be_valid_json_and_bounded() {
        let mut input = global_rule("x");
        input.custom_response_status = Some(429);
        input.custom_response_body = Some("not json".to_owned());
        let error = rule(&input).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "自定义返回内容必须是合法 JSON。");

        let mut input = global_rule("x");
        input.custom_response_status = Some(429);
        input.custom_response_body = Some(format!("{{\"pad\":\"{}\"}}", "x".repeat(40 * 1024)));
        let error = rule(&input).expect_err("必须拒绝");
        assert!(error.to_string().contains("32 KB"), "{error}");
    }

    #[test]
    fn rule_scope_constraints_are_enforced() {
        // 全局规则不得指定平台或供应商。
        let mut input = global_rule("x");
        input.provider_id = Some(ProviderId::Codex);
        assert_eq!(
            rule(&input).expect_err("必须拒绝").to_string(),
            "全局规则不能指定 Provider 或供应商。"
        );

        // 平台级规则必须指定平台。
        let input = FailoverRuleInput {
            scope: Some(FailoverRuleScope::Provider),
            pattern: "x".to_owned(),
            ..FailoverRuleInput::default()
        };
        assert_eq!(
            rule(&input).expect_err("必须拒绝").to_string(),
            "Provider 级和供应商级规则必须指定 Provider。"
        );

        // 平台级规则不得指定供应商。
        let input = FailoverRuleInput {
            scope: Some(FailoverRuleScope::Provider),
            provider_id: Some(ProviderId::Claude),
            vendor_id: Some(VendorId::from("v1")),
            pattern: "x".to_owned(),
            ..FailoverRuleInput::default()
        };
        assert_eq!(
            rule(&input).expect_err("必须拒绝").to_string(),
            "Provider 级规则不能指定供应商。"
        );

        // 供应商级规则必须指定供应商。
        let input = FailoverRuleInput {
            scope: Some(FailoverRuleScope::Vendor),
            provider_id: Some(ProviderId::Codex),
            pattern: "x".to_owned(),
            ..FailoverRuleInput::default()
        };
        assert_eq!(
            rule(&input).expect_err("必须拒绝").to_string(),
            "供应商级规则必须指定供应商。"
        );
    }

    #[test]
    fn enabled_defaults_to_true_unless_explicitly_false() {
        assert!(rule(&global_rule("x")).expect("normalize").enabled);
        assert!(
            rule(&FailoverRuleInput {
                enabled: Some(true),
                ..global_rule("x")
            })
            .expect("normalize")
            .enabled
        );
        assert!(
            !rule(&FailoverRuleInput {
                enabled: Some(false),
                ..global_rule("x")
            })
            .expect("normalize")
            .enabled
        );
    }

    #[test]
    fn blank_rule_id_normalizes_to_new_rule() {
        let normalized = rule(&FailoverRuleInput {
            id: Some("   ".to_owned()),
            ..global_rule("x")
        })
        .expect("normalize");
        assert_eq!(normalized.id, None);
    }

    fn stored_rule(scope: FailoverRuleScope, pattern: &str) -> FailoverRule {
        FailoverRule {
            id: "r1".to_owned(),
            scope,
            provider_id: Some(ProviderId::Codex),
            vendor_id: Some(VendorId::from("v1")),
            pattern: pattern.to_owned(),
            status_codes: Some(vec![429, 503]),
            custom_response_status: None,
            custom_response_body: None,
            enabled: true,
            created_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
            updated_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
        }
    }

    #[test]
    fn rule_scope_matching_follows_the_three_level_hierarchy() {
        let global = stored_rule(FailoverRuleScope::Global, "x");
        assert!(global.applies_to(ProviderId::Qoder, None));

        let provider = stored_rule(FailoverRuleScope::Provider, "x");
        assert!(provider.applies_to(ProviderId::Codex, None));
        assert!(!provider.applies_to(ProviderId::Gemini, None));

        let vendor = stored_rule(FailoverRuleScope::Vendor, "x");
        let vendor_id = VendorId::from("v1");
        assert!(vendor.applies_to(ProviderId::Codex, Some(&vendor_id)));
        // 供应商级规则在没有供应商上下文时不适用。
        assert!(!vendor.applies_to(ProviderId::Codex, None));
        assert!(!vendor.applies_to(ProviderId::Codex, Some(&VendorId::from("other"))));
    }

    #[test]
    fn rule_matching_is_case_insensitive_substring() {
        let rule = stored_rule(FailoverRuleScope::Global, "Rate Limit");
        assert!(rule.matches(None, Some("upstream said RATE LIMIT exceeded")));
        assert!(!rule.matches(None, Some("all good")));
        // 没有消息时不得命中。
        assert!(!rule.matches(None, None));
        // 状态码命中即可，不需要消息。
        assert!(rule.matches(Some(429), None));
        assert!(!rule.matches(Some(200), None));
    }

    #[test]
    fn empty_pattern_rule_only_matches_by_status_code() {
        let mut rule = stored_rule(FailoverRuleScope::Global, "");
        rule.status_codes = Some(vec![500]);
        assert!(rule.matches(Some(500), Some("anything")));
        assert!(!rule.matches(Some(200), Some("anything")));
    }

    /// 命中时**要能说出是被哪一项命中的**。
    ///
    /// 「测试规则」功能靠它：只回布尔值时，用户看到「没命中」不知道该改状态码还是改文本。
    #[test]
    fn match_rule_reports_which_condition_hit() {
        let codes = [400u16, 429];
        // 状态码精确命中。
        assert_eq!(
            match_rule("boom", Some(&codes), Some(429), None),
            Some(RuleHit::Status)
        );
        // 文本包含命中（不区分大小写）。
        assert_eq!(
            match_rule(
                "rate limit",
                None,
                None,
                Some("Upstream RATE LIMIT exceeded")
            ),
            Some(RuleHit::Pattern)
        );
        // 两者都不会命中。
        assert_eq!(
            match_rule("boom", Some(&codes), Some(200), Some("all good")),
            None
        );
        // **两者都命中时报状态码**：它是精确匹配，对用户更有用。
        assert_eq!(
            match_rule("boom", Some(&codes), Some(400), Some("boom happened")),
            Some(RuleHit::Status)
        );
    }

    /// 空模式只按状态码判；没有消息时文本这一项不得命中。
    #[test]
    fn match_rule_edge_cases() {
        assert_eq!(match_rule("", None, Some(500), Some("x")), None);
        assert_eq!(match_rule("x", None, None, None), None);
        assert_eq!(match_rule("x", None, Some(500), None), None);
        // 状态码列表为空等价于「不按状态码判」。
        assert_eq!(match_rule("x", Some(&[]), Some(500), None), None);
    }

    /// **`FailoverRule::matches` 与 `match_rule` 必须同源**。
    ///
    /// 这条是这一组里最要紧的：转发路径用前者、「测试规则」用后者。两份判断一旦
    /// 分叉，就会出现「测试说会命中、实际没切换」——那是最难查的一类偏差。
    #[test]
    fn the_forwarding_path_and_the_test_command_agree() {
        let mut rule = stored_rule(FailoverRuleScope::Global, "Rate Limit");
        rule.status_codes = Some(vec![429]);
        let cases: [(Option<u16>, Option<&str>); 6] = [
            (Some(429), None),
            (Some(429), Some("anything")),
            (Some(500), Some("rate limit hit")),
            (Some(500), Some("all good")),
            (None, Some("RATE LIMIT")),
            (None, None),
        ];
        for (status, message) in cases {
            let via_rule = rule.matches(status, message);
            let via_function =
                match_rule(&rule.pattern, rule.status_codes.as_deref(), status, message).is_some();
            assert_eq!(
                via_rule, via_function,
                "status={status:?} message={message:?}"
            );
        }
    }

    #[test]
    fn usage_merge_takes_per_field_maximum_not_a_sum() {
        // 同一请求多次上报时累加会重复计数，必须取最大值。
        let mut usage = GatewayUsage {
            input_tokens: Some(100),
            output_tokens: Some(10),
            total_tokens: Some(110),
            ..GatewayUsage::default()
        };
        usage.merge(&GatewayUsage {
            input_tokens: Some(150),
            output_tokens: Some(5),
            total_tokens: Some(155),
            ..GatewayUsage::default()
        });

        assert_eq!(usage.input_tokens, Some(150));
        // 后报的 output 更小，必须保留较大的旧值。
        assert_eq!(usage.output_tokens, Some(10));
        assert_eq!(usage.total_tokens, Some(155));
    }

    #[test]
    fn usage_merge_fills_missing_fields_and_keeps_the_latest_model() {
        let mut usage = GatewayUsage::default();
        usage.merge(&GatewayUsage {
            input_tokens: Some(1),
            model: Some("gpt-5".to_owned()),
            ..GatewayUsage::default()
        });
        assert_eq!(usage.input_tokens, Some(1));
        assert_eq!(usage.model.as_deref(), Some("gpt-5"));

        usage.merge(&GatewayUsage {
            model: Some("gpt-5.1".to_owned()),
            ..GatewayUsage::default()
        });
        assert_eq!(usage.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(usage.input_tokens, Some(1), "合并不得清掉已有字段");
    }

    #[test]
    fn empty_usage_is_reported_as_empty() {
        assert!(GatewayUsage::default().is_empty());
        assert!(
            !GatewayUsage {
                cost_usd: Some(0.0),
                ..GatewayUsage::default()
            }
            .is_empty()
        );
    }

    #[test]
    fn error_messages_are_collapsed_and_truncated() {
        assert_eq!(sanitize_error_message("a\nb\tc"), "a b c");
        let long = "x".repeat(1000);
        let sanitized = sanitize_error_message(&long);
        assert_eq!(sanitized.chars().count(), REQUEST_ERROR_MAX_LENGTH - 1 + 3);
    }

    #[test]
    fn retry_buffer_window_matches_the_configured_limit() {
        assert!(!is_retry_bufferable(0), "未知长度不得缓冲");
        assert!(is_retry_bufferable(1));
        assert!(is_retry_bufferable(
            GATEWAY_REQUEST_LIMITS.retry_buffer_bytes
        ));
        assert!(!is_retry_bufferable(
            GATEWAY_REQUEST_LIMITS.retry_buffer_bytes + 1
        ));
    }
}
