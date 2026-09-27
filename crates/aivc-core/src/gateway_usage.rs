//! 网关用量：解析、合并与聚合。
//!
//! 这里集中三件互不相同、但极易混淆的操作：
//!
//! | 操作 | 语义 | 用在何处 |
//! | --- | --- | --- |
//! | [`GatewayUsage::merge`]（在 [`crate::gateway`]） | **逐项取最大值** | 同一请求内多次上报（SSE 中途 + 结束） |
//! | [`aggregate_usage`] | **逐项求和** | 跨请求汇总报表 |
//! | [`parse_gateway_usage`] | 从上游响应体提取 | 请求结束时捕获 |
//!
//! 把「合并」写成求和会让单次请求的 token 翻倍；把「聚合」写成取最大值会让报表
//! 只反映最大的一次请求。两者都必须在测试里固定住。
//!
//! # 三套字段族
//!
//! 上游协议不统一，同一个量在不同平台叫不同名字，因此每项都要按别名依次探测：
//!
//! | 量 | 别名（按优先级） |
//! | --- | --- |
//! | 输入 | `input_tokens`、`prompt_tokens`、`inputTokenCount` |
//! | 输出 | `output_tokens`、`completion_tokens`、`outputTokenCount` |
//! | 缓存输入 | `input_tokens_details.cached_tokens`、`prompt_tokens_details.cached_tokens`、`cached_tokens` |
//! | 推理 | `completion_tokens_details.reasoning_tokens`、`reasoning_tokens`、`thoughtsTokenCount` |
//! | 总量 | `total_tokens`、`totalTokenCount`（缺失时回退为输入 + 输出） |

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::gateway::{GatewayOutcome, GatewayUsage};
use crate::id::VendorId;
use crate::target::ProviderId;
use crate::time::Timestamp;

// ---------------------------------------------------------------------------
// 报表类型
// ---------------------------------------------------------------------------

/// 用量总览。
///
/// 刻意**不**实现 `Default`：总览必须携带统计区间，缺省构造出的零值总览没有意义，
/// 只会掩盖「忘记填区间」这类错误。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    /// 请求数。
    pub request_count: u64,
    /// 成功数。
    pub success_count: u64,
    /// 失败数。
    pub failure_count: u64,
    /// 发生故障切换的请求数。
    pub switched_count: u64,
    /// 重试总次数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_count: Option<u64>,
    /// 平均耗时（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_duration_ms: Option<f64>,
    /// 输入 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// 输出 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// 缓存输入 token。
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
    /// 统计区间起点。
    pub period_start: Timestamp,
    /// 统计区间终点。
    pub period_end: Timestamp,
}

/// 单个维度的用量汇总。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageDimension {
    /// 维度键。
    pub key: String,
    /// 展示名。
    pub label: String,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 请求数。
    pub request_count: u64,
    /// 成功数。
    pub success_count: u64,
    /// 失败数。
    pub failure_count: u64,
    /// 发生故障切换的请求数。
    pub switched_count: u64,
    /// 重试总次数。
    pub retry_count: u64,
    /// 平均耗时（毫秒）。
    pub average_duration_ms: f64,
    /// 输入 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// 输出 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// 缓存输入 token。
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

/// 用量报表：总览 + 按供应商 + 按模型。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageReport {
    /// 总览。
    pub summary: UsageSummary,
    /// 按供应商汇总。
    pub vendors: Vec<UsageDimension>,
    /// 按模型汇总。
    pub models: Vec<UsageDimension>,
}

/// 聚合输入：一条请求记录的统计视图。
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    /// 供应商编号。
    pub vendor_id: VendorId,
    /// 供应商展示名（可能已被删除，此时由调用方填入占位文案）。
    pub vendor_name: String,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 模型名。
    pub model: Option<String>,
    /// 结果。
    pub outcome: GatewayOutcome,
    /// 耗时（毫秒）。
    pub duration_ms: u64,
    /// 重试次数。
    pub retry_count: u32,
    /// 是否发生故障切换。
    pub switched: bool,
    /// 已存储的用量 JSON。
    pub usage_json: Option<String>,
}

/// 模型维度在未标注模型名时使用的标签。
pub const UNLABELLED_MODEL: &str = "未标注模型";

// ---------------------------------------------------------------------------
// 解析
// ---------------------------------------------------------------------------

/// 从上游响应体提取用量。
///
/// 依次尝试 `root.usage`、`root.response.usage`、`root` 三个位置，任一命中即可。
/// 四项 token 全为空时返回 [`None`]（表示「这份响应没有用量信息」）。
#[must_use]
pub fn parse_gateway_usage(body: &Value) -> Option<GatewayUsage> {
    let root = body.as_object()?;
    let response = root.get("response").and_then(Value::as_object);
    let usage = root
        .get("usage")
        .and_then(Value::as_object)
        .or_else(|| {
            response
                .and_then(|item| item.get("usage"))
                .and_then(Value::as_object)
        })
        .unwrap_or(root);

    let usage = Value::Object(usage.clone());
    let input_tokens = number_at(
        &usage,
        &["input_tokens", "prompt_tokens", "inputTokenCount"],
    );
    let output_tokens = number_at(
        &usage,
        &["output_tokens", "completion_tokens", "outputTokenCount"],
    );
    let cached_input_tokens = number_at(
        &usage,
        &[
            "input_tokens_details.cached_tokens",
            "prompt_tokens_details.cached_tokens",
            "cached_tokens",
        ],
    );
    let reasoning_tokens = number_at(
        &usage,
        &[
            "completion_tokens_details.reasoning_tokens",
            "reasoning_tokens",
            "thoughtsTokenCount",
        ],
    );

    if input_tokens.is_none()
        && output_tokens.is_none()
        && cached_input_tokens.is_none()
        && reasoning_tokens.is_none()
    {
        return None;
    }

    let total_tokens = number_at(&usage, &["total_tokens", "totalTokenCount"]).or_else(|| {
        (input_tokens.is_some() || output_tokens.is_some())
            .then(|| input_tokens.unwrap_or(0) + output_tokens.unwrap_or(0))
    });

    Some(GatewayUsage {
        model: None,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        reasoning_tokens,
        total_tokens,
        cost_usd: None,
    })
}

/// 从流式响应分块中提取用量：逐行取 `data:` 载荷解析，保留**最后一条**成功解析的结果。
///
/// 分块可能跨越多次网络读取，因此当前分块不是完整 JSON 时忽略而不是报错。
#[must_use]
pub fn parse_usage_from_chunk(chunk: &str) -> Option<GatewayUsage> {
    let mut latest = None;
    for line in chunk.split(['\n', '\r']) {
        let payload = line.trim().strip_prefix("data:").unwrap_or(line).trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(parsed) = serde_json::from_str::<Value>(payload)
            && let Some(usage) = parse_gateway_usage(&parsed)
        {
            latest = Some(usage);
        }
    }
    latest
}

/// 从已存储的 `usage_json` 解析用量。
#[must_use]
pub fn parse_stored_usage(value: Option<&str>) -> Option<GatewayUsage> {
    let value = value?;
    serde_json::from_str::<GatewayUsage>(value).ok()
}

/// 按点号路径取第一个有限的数值，并转换为 token 计数。
///
/// 非有限值（`NaN` / `Infinity`）与负数按「未提供」处理；小数向零截断
/// （token 计数出现小数说明上游数据异常，截断比丢弃更有用）。
///
/// 下方已确认数值有限且非负，因此截断转换不会丢符号。
#[allow(clippy::cast_sign_loss)]
fn number_at(root: &Value, paths: &[&str]) -> Option<u64> {
    for path in paths {
        let mut current = root;
        for segment in path.split('.') {
            match current.get(segment) {
                Some(next) => current = next,
                None => break,
            }
        }
        if let Some(number) = current.as_f64()
            && number.is_finite()
            && number >= 0.0
        {
            return Some(number.trunc() as u64);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// 聚合
// ---------------------------------------------------------------------------

/// 把请求记录聚合为总览、按供应商、按模型三个维度。
///
/// 统计规则与现网一致：token 与费用**求和**，耗时求和后除以请求数得到平均值。
#[must_use]
pub fn aggregate_usage(
    rows: &[UsageRow],
    period_start: &Timestamp,
    period_end: &Timestamp,
) -> UsageReport {
    let mut summary = Accumulator::new("summary", "全部请求", ProviderId::Codex);
    let mut vendors: Vec<Accumulator> = Vec::new();
    let mut models: Vec<Accumulator> = Vec::new();

    for row in rows {
        let usage = parse_stored_usage(row.usage_json.as_deref());
        summary.add(row, usage.as_ref());

        upsert_dimension(
            &mut vendors,
            row.vendor_id.as_str(),
            &row.vendor_name,
            row.provider_id,
            row,
            usage.as_ref(),
        );

        // 模型维度键包含平台，避免不同平台的同名模型被合并。
        let model_label = row
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(UNLABELLED_MODEL);
        upsert_dimension(
            &mut models,
            &format!("{}:{model_label}", row.provider_id.as_str()),
            model_label,
            row.provider_id,
            row,
            usage.as_ref(),
        );
    }

    UsageReport {
        summary: summary.into_summary(period_start, period_end),
        vendors: finish_dimensions(vendors),
        models: finish_dimensions(models),
    }
}

/// 累加器。
#[derive(Debug, Clone)]
struct Accumulator {
    key: String,
    label: String,
    provider_id: ProviderId,
    request_count: u64,
    success_count: u64,
    failure_count: u64,
    switched_count: u64,
    retry_count: u64,
    duration_ms: u64,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cached_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    total_tokens: Option<u64>,
    cost_usd: Option<f64>,
}

impl Accumulator {
    fn new(key: &str, label: &str, provider_id: ProviderId) -> Self {
        Self {
            key: key.to_owned(),
            label: label.to_owned(),
            provider_id,
            request_count: 0,
            success_count: 0,
            failure_count: 0,
            switched_count: 0,
            retry_count: 0,
            duration_ms: 0,
            input_tokens: None,
            output_tokens: None,
            cached_input_tokens: None,
            reasoning_tokens: None,
            total_tokens: None,
            cost_usd: None,
        }
    }

    fn add(&mut self, row: &UsageRow, usage: Option<&GatewayUsage>) {
        self.request_count += 1;
        if row.outcome.is_success() {
            self.success_count += 1;
        } else {
            self.failure_count += 1;
        }
        if row.switched {
            self.switched_count += 1;
        }
        self.retry_count += u64::from(row.retry_count);
        self.duration_ms += row.duration_ms;

        let Some(usage) = usage else {
            return;
        };
        add_optional(&mut self.input_tokens, usage.input_tokens);
        add_optional(&mut self.output_tokens, usage.output_tokens);
        add_optional(&mut self.cached_input_tokens, usage.cached_input_tokens);
        add_optional(&mut self.reasoning_tokens, usage.reasoning_tokens);
        add_optional(&mut self.total_tokens, usage.total_tokens);
        if let Some(cost) = usage.cost_usd {
            self.cost_usd = Some(self.cost_usd.unwrap_or(0.0) + cost);
        }
    }

    fn into_summary(self, period_start: &Timestamp, period_end: &Timestamp) -> UsageSummary {
        let average = average_duration(self.duration_ms, self.request_count);
        UsageSummary {
            request_count: self.request_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            switched_count: self.switched_count,
            retry_count: Some(self.retry_count),
            average_duration_ms: Some(average),
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_input_tokens: self.cached_input_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cost_usd: self.cost_usd,
            period_start: period_start.clone(),
            period_end: period_end.clone(),
        }
    }

    fn into_dimension(self) -> UsageDimension {
        let average = average_duration(self.duration_ms, self.request_count);
        UsageDimension {
            key: self.key,
            label: self.label,
            provider_id: self.provider_id,
            request_count: self.request_count,
            success_count: self.success_count,
            failure_count: self.failure_count,
            switched_count: self.switched_count,
            retry_count: self.retry_count,
            average_duration_ms: average,
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cached_input_tokens: self.cached_input_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            cost_usd: self.cost_usd,
        }
    }
}

/// 累加可选数值。
fn add_optional(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or(0) + value);
    }
}

/// 平均耗时；无请求时为 0。
fn average_duration(total_ms: u64, request_count: u64) -> f64 {
    if request_count == 0 {
        0.0
    } else {
        total_ms as f64 / request_count as f64
    }
}

/// 查找或新建维度累加器并累加。
fn upsert_dimension(
    dimensions: &mut Vec<Accumulator>,
    key: &str,
    label: &str,
    provider_id: ProviderId,
    row: &UsageRow,
    usage: Option<&GatewayUsage>,
) {
    let index = if let Some(index) = dimensions.iter().position(|item| item.key == key) {
        index
    } else {
        dimensions.push(Accumulator::new(key, label, provider_id));
        dimensions.len() - 1
    };
    if let Some(item) = dimensions.get_mut(index) {
        item.add(row, usage);
    }
}

/// 转为维度列表并排序：请求数降序，其次按标签升序。
fn finish_dimensions(mut dimensions: Vec<Accumulator>) -> Vec<UsageDimension> {
    let mut finished: Vec<UsageDimension> = dimensions
        .drain(..)
        .map(Accumulator::into_dimension)
        .collect();
    finished.sort_by(|left, right| {
        right
            .request_count
            .cmp(&left.request_count)
            .then_with(|| left.label.cmp(&right.label))
    });
    finished
}

// ---------------------------------------------------------------------------
// 失败诊断
// ---------------------------------------------------------------------------

/// 异常类型筛选。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FailureOutcomeFilter {
    /// 不筛选。
    #[serde(rename = "")]
    Any,
    /// 仅失败。
    Error,
    /// 仅超时。
    Timeout,
}

impl FailureOutcomeFilter {
    /// 序列化短名（`Any` 为空串，与现网一致）。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Any => "",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }

    /// 由短名解析；未知值回退为「不筛选」。
    #[must_use]
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "error" => Self::Error,
            "timeout" => Self::Timeout,
            _ => Self::Any,
        }
    }
}

/// 工作台展示用的近期异常（只含安全摘要）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentFailure {
    /// 供应商编号。
    pub vendor_id: VendorId,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 结果（只可能是失败或超时）。
    pub outcome: FailureOutcomeFilter,
    /// 上游 HTTP 状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    /// 记录时间。
    pub created_at: Timestamp,
}

/// 诊断明细：在近期异常基础上补充重试、耗时与受控错误文本。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureDiagnostic {
    /// 供应商编号。
    pub vendor_id: VendorId,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 结果。
    pub outcome: FailureOutcomeFilter,
    /// 上游 HTTP 状态码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_status: Option<u16>,
    /// 重试次数。
    pub retry_count: u32,
    /// 耗时（毫秒）。
    pub duration_ms: u64,
    /// 错误代码。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    /// 错误摘要（已截断、已清洗）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// 记录时间。
    pub created_at: Timestamp,
}

/// 诊断明细分页。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureDiagnosticsPage {
    /// 本页条目。
    pub items: Vec<FailureDiagnostic>,
    /// 满足条件的总数。
    pub total: u64,
    /// 当前页码（1 起）。
    pub page: u32,
    /// 每页条数。
    pub page_size: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::{PAGINATION_DEFAULT_PAGE_SIZE, PAGINATION_MAX_PAGE_SIZE};

    fn json(text: &str) -> Value {
        serde_json::from_str(text).expect("valid json")
    }

    fn row(
        vendor: &str,
        provider: ProviderId,
        outcome: GatewayOutcome,
        usage: Option<&str>,
    ) -> UsageRow {
        UsageRow {
            vendor_id: VendorId::from(vendor),
            vendor_name: vendor.to_owned(),
            provider_id: provider,
            model: None,
            outcome,
            duration_ms: 100,
            retry_count: 0,
            switched: false,
            usage_json: usage.map(str::to_owned),
        }
    }

    #[test]
    fn parses_openai_style_usage() {
        let usage = parse_gateway_usage(&json(
            r#"{"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}"#,
        ))
        .expect("usage");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(50));
        assert_eq!(usage.total_tokens, Some(150));
    }

    #[test]
    fn parses_anthropic_style_usage() {
        let usage = parse_gateway_usage(&json(
            r#"{"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5}}"#,
        ))
        .expect("usage");
        assert_eq!(usage.input_tokens, Some(10));
        assert_eq!(usage.output_tokens, Some(20));
    }

    #[test]
    fn parses_gemini_style_usage_with_camel_case_aliases() {
        let usage = parse_gateway_usage(&json(
            r#"{"usageMetadata":{"promptTokenCount":7,"candidatesTokenCount":3,"thoughtsTokenCount":2}}"#,
        ));
        // usageMetadata 不是受支持的容器名，必须返回 None 而不是瞎猜。
        assert!(usage.is_none());

        let usage = parse_gateway_usage(&json(
            r#"{"usage":{"inputTokenCount":7,"outputTokenCount":3,"thoughtsTokenCount":2}}"#,
        ))
        .expect("usage");
        assert_eq!(usage.input_tokens, Some(7));
        assert_eq!(usage.output_tokens, Some(3));
        assert_eq!(usage.reasoning_tokens, Some(2));
    }

    #[test]
    fn parses_cached_and_reasoning_detail_paths() {
        let usage = parse_gateway_usage(&json(
            r#"{"usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":80},"completion_tokens_details":{"reasoning_tokens":40}}}"#,
        ))
        .expect("usage");
        assert_eq!(usage.cached_input_tokens, Some(80));
        assert_eq!(usage.reasoning_tokens, Some(40));
    }

    #[test]
    fn total_falls_back_to_input_plus_output() {
        let usage =
            parse_gateway_usage(&json(r#"{"usage":{"input_tokens":10,"output_tokens":5}}"#))
                .expect("usage");
        assert_eq!(usage.total_tokens, Some(15));

        // 显式 total 优先。
        let usage = parse_gateway_usage(&json(
            r#"{"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":999}}"#,
        ))
        .expect("usage");
        assert_eq!(usage.total_tokens, Some(999));
    }

    #[test]
    fn reads_usage_from_a_nested_response_object() {
        let usage = parse_gateway_usage(&json(
            r#"{"response":{"usage":{"input_tokens":1,"output_tokens":2}}}"#,
        ))
        .expect("usage");
        assert_eq!(usage.input_tokens, Some(1));
    }

    #[test]
    fn reads_usage_from_the_root_when_no_usage_object_exists() {
        let usage =
            parse_gateway_usage(&json(r#"{"input_tokens":3,"output_tokens":4}"#)).expect("usage");
        assert_eq!(usage.input_tokens, Some(3));
    }

    #[test]
    fn returns_none_when_no_token_fields_are_present() {
        assert!(parse_gateway_usage(&json(r#"{"usage":{}}"#)).is_none());
        assert!(parse_gateway_usage(&json(r#"{"id":"x","model":"m"}"#)).is_none());
        assert!(parse_gateway_usage(&json("[]")).is_none());
        assert!(parse_gateway_usage(&json("null")).is_none());
    }

    #[test]
    fn ignores_non_numeric_negative_and_non_finite_token_values() {
        let usage = parse_gateway_usage(&json(
            r#"{"usage":{"input_tokens":"100","output_tokens":-5,"cached_tokens":null}}"#,
        ));
        // 全部非法 → 视为没有用量信息。
        assert!(usage.is_none());

        let usage =
            parse_gateway_usage(&json(r#"{"usage":{"input_tokens":12.9}}"#)).expect("usage");
        // 小数向零截断，而不是丢弃。
        assert_eq!(usage.input_tokens, Some(12));
    }

    #[test]
    fn integral_floats_are_accepted() {
        let usage =
            parse_gateway_usage(&json(r#"{"usage":{"input_tokens":100.0}}"#)).expect("usage");
        assert_eq!(usage.input_tokens, Some(100));
    }

    #[test]
    fn parses_usage_from_sse_chunks_taking_the_last_valid_entry() {
        let chunk = concat!(
            "data: {\"usage\":{\"input_tokens\":10}}\n\n",
            "data: {\"usage\":{\"input_tokens\":20,\"output_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        let usage = parse_usage_from_chunk(chunk).expect("usage");
        assert_eq!(usage.input_tokens, Some(20));
        assert_eq!(usage.output_tokens, Some(5));
    }

    #[test]
    fn sse_parsing_skips_incomplete_and_non_usage_lines() {
        let chunk = concat!(
            "event: message\n",
            "data: {\"usage\":{\"inp\n", // 不完整 JSON
            "data: not json\n",
            "data: {\"usage\":{\"output_tokens\":7}}\n",
        );
        let usage = parse_usage_from_chunk(chunk).expect("usage");
        assert_eq!(usage.output_tokens, Some(7));

        assert!(parse_usage_from_chunk("data: [DONE]\n").is_none());
        assert!(parse_usage_from_chunk("").is_none());
    }

    #[test]
    fn stored_usage_parsing_tolerates_corrupt_json() {
        assert!(parse_stored_usage(Some("not json")).is_none());
        assert!(parse_stored_usage(None).is_none());
        let usage = parse_stored_usage(Some(r#"{"inputTokens":5}"#)).expect("usage");
        assert_eq!(usage.input_tokens, Some(5));
    }

    #[test]
    fn aggregation_sums_across_requests_rather_than_taking_the_maximum() {
        // 这是与「请求内合并」最容易混淆的地方：跨请求必须求和。
        let rows = vec![
            row(
                "v1",
                ProviderId::Codex,
                GatewayOutcome::Ok,
                Some(r#"{"inputTokens":10,"outputTokens":5,"totalTokens":15}"#),
            ),
            row(
                "v1",
                ProviderId::Codex,
                GatewayOutcome::Ok,
                Some(r#"{"inputTokens":20,"outputTokens":8,"totalTokens":28}"#),
            ),
        ];
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let report = aggregate_usage(&rows, &start, &end);

        assert_eq!(report.summary.request_count, 2);
        assert_eq!(report.summary.input_tokens, Some(30), "跨请求必须求和");
        assert_eq!(report.summary.output_tokens, Some(13));
        assert_eq!(report.summary.total_tokens, Some(43));
    }

    #[test]
    fn aggregation_counts_success_failure_switched_and_retries() {
        let mut failing = row("v1", ProviderId::Codex, GatewayOutcome::Error, None);
        failing.retry_count = 3;
        failing.switched = true;
        let mut timing_out = row("v1", ProviderId::Codex, GatewayOutcome::Timeout, None);
        timing_out.retry_count = 1;
        timing_out.duration_ms = 300;

        let rows = vec![
            row("v1", ProviderId::Codex, GatewayOutcome::Ok, None),
            failing,
            timing_out,
            // 客户端主动断开也计入失败。
            row("v1", ProviderId::Codex, GatewayOutcome::ClientAborted, None),
        ];
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let report = aggregate_usage(&rows, &start, &end);

        assert_eq!(report.summary.request_count, 4);
        assert_eq!(report.summary.success_count, 1);
        assert_eq!(report.summary.failure_count, 3);
        assert_eq!(report.summary.switched_count, 1);
        assert_eq!(report.summary.retry_count, Some(4));
        // 平均耗时 = (100+100+300+100)/4 = 150
        assert_eq!(report.summary.average_duration_ms, Some(150.0));
    }

    #[test]
    fn empty_input_yields_zeroed_summary_and_no_dimensions() {
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let report = aggregate_usage(&[], &start, &end);

        assert_eq!(report.summary.request_count, 0);
        assert_eq!(
            report.summary.average_duration_ms,
            Some(0.0),
            "无请求时平均值为 0"
        );
        assert_eq!(report.summary.retry_count, Some(0));
        assert!(report.vendors.is_empty());
        assert!(report.models.is_empty());
        assert_eq!(report.summary.period_start, start);
        assert_eq!(report.summary.period_end, end);
    }

    #[test]
    fn model_dimension_key_includes_the_provider() {
        // 不同平台的同名模型不能被合并成一条。
        let mut codex = row("v1", ProviderId::Codex, GatewayOutcome::Ok, None);
        codex.model = Some("shared-model".to_owned());
        let mut claude = row("v2", ProviderId::Claude, GatewayOutcome::Ok, None);
        claude.model = Some("shared-model".to_owned());

        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let report = aggregate_usage(&[codex, claude], &start, &end);

        assert_eq!(report.models.len(), 2, "同名模型必须按平台分开");
        let keys: Vec<&str> = report.models.iter().map(|item| item.key.as_str()).collect();
        assert!(keys.contains(&"codex:shared-model"));
        assert!(keys.contains(&"claude:shared-model"));
    }

    #[test]
    fn missing_model_name_uses_the_unlabelled_label() {
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let report = aggregate_usage(
            &[row("v1", ProviderId::Codex, GatewayOutcome::Ok, None)],
            &start,
            &end,
        );
        assert_eq!(report.models.len(), 1);
        assert_eq!(report.models[0].label, UNLABELLED_MODEL);
        assert_eq!(report.models[0].key, "codex:未标注模型");
    }

    #[test]
    fn dimensions_are_sorted_by_request_count_then_label() {
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let mut rows = Vec::new();
        for _ in 0..3 {
            rows.push(row("busy", ProviderId::Codex, GatewayOutcome::Ok, None));
        }
        rows.push(row("quiet-b", ProviderId::Codex, GatewayOutcome::Ok, None));
        rows.push(row("quiet-a", ProviderId::Codex, GatewayOutcome::Ok, None));

        let report = aggregate_usage(&rows, &start, &end);
        let labels: Vec<&str> = report
            .vendors
            .iter()
            .map(|item| item.label.as_str())
            .collect();
        assert_eq!(labels, vec!["busy", "quiet-a", "quiet-b"]);
    }

    #[test]
    fn corrupt_usage_json_does_not_produce_phantom_tokens() {
        // 损坏的 usage 只应让该条请求没有 token 数据，不能凭空造数。
        let rows = vec![
            row(
                "v1",
                ProviderId::Codex,
                GatewayOutcome::Ok,
                Some("not json"),
            ),
            row("v1", ProviderId::Codex, GatewayOutcome::Ok, None),
            row(
                "v1",
                ProviderId::Codex,
                GatewayOutcome::Ok,
                Some(r#"{"inputTokens":7}"#),
            ),
        ];
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");
        let report = aggregate_usage(&rows, &start, &end);

        assert_eq!(report.summary.request_count, 3);
        assert_eq!(report.summary.input_tokens, Some(7));
        assert_eq!(report.summary.output_tokens, None, "无数据时不得输出 0");
    }

    #[test]
    fn cost_is_summed_and_absent_when_never_reported() {
        let start = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let end = Timestamp::from_raw("2026-01-02T00:00:00.000Z");

        let report = aggregate_usage(
            &[row(
                "v1",
                ProviderId::Codex,
                GatewayOutcome::Ok,
                Some(r#"{"inputTokens":1}"#),
            )],
            &start,
            &end,
        );
        assert_eq!(report.summary.cost_usd, None);

        let report = aggregate_usage(
            &[
                row(
                    "v1",
                    ProviderId::Codex,
                    GatewayOutcome::Ok,
                    Some(r#"{"costUsd":0.5}"#),
                ),
                row(
                    "v1",
                    ProviderId::Codex,
                    GatewayOutcome::Ok,
                    Some(r#"{"costUsd":1.25}"#),
                ),
            ],
            &start,
            &end,
        );
        assert_eq!(report.summary.cost_usd, Some(1.75));
    }

    #[test]
    fn failure_outcome_filter_round_trips_and_defaults_to_any() {
        assert_eq!(FailureOutcomeFilter::Any.as_str(), "");
        assert_eq!(
            FailureOutcomeFilter::from_str_lossy(""),
            FailureOutcomeFilter::Any
        );
        assert_eq!(
            FailureOutcomeFilter::from_str_lossy("error"),
            FailureOutcomeFilter::Error
        );
        assert_eq!(
            FailureOutcomeFilter::from_str_lossy("timeout"),
            FailureOutcomeFilter::Timeout
        );
        assert_eq!(
            FailureOutcomeFilter::from_str_lossy("???"),
            FailureOutcomeFilter::Any
        );
    }

    #[test]
    fn pagination_constants_are_sane_for_diagnostics() {
        // 诊断分页与通用分页共用同一组常量，这里做一次交叉确认。
        const {
            assert!(PAGINATION_DEFAULT_PAGE_SIZE <= PAGINATION_MAX_PAGE_SIZE);
        }
        assert_eq!(PAGINATION_DEFAULT_PAGE_SIZE, 10);
    }
}
