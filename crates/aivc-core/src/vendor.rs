//! 供应商领域模型与输入归一化。
//!
//! 供应商是本应用唯一会**写入用户 CLI 配置文件**的功能，因此这里的归一化与校验
//! 是安全边界的一部分：任何放宽都可能把内容写到 `~/.codex` 之外。
//!
//! # 必须逐字保持的校验语义
//!
//! | 字段 | 规则 | 失败文案 |
//! | --- | --- | --- |
//! | `name` | `trim` 后非空，截断 80 字符 | `供应商名称不能为空。` |
//! | `api_key` | `trim` 后非空（更新时允许留空表示沿用旧值） | `API Key 不能为空。` |
//! | `api_base_url` | `trim` 后非空 | `API 请求地址不能为空。` |
//! | 费率 | 有限、非负、最多 2 位小数 | `{输入/输出}费率必须是非负数，且最多保留 2 位小数。` |
//! | 认证方式 | 6 种枚举之一 | `查询认证方式无效。` |
//! | 自定义 Header 名 | 符合 token 字符集，≤100 | `自定义认证 Header 名称无效。` |
//! | 自定义请求头 | ≤32 条，值 ≤4000 且不含换行 | `自定义请求头无效。` |
//! | 查询配置 | 序列化后 ≤32 KB | `供应商查询配置过大。` |
//! | 余额模板 | 4 种之一 | `余额查询模板无效。` |
//! | 余额倍率 | `(0, 1_000_000]` | `余额换算倍率必须大于 0 且不超过 1000000。` |
//!
//! # 与现网的两处差异
//!
//! - **长度按字符而非 UTF-16 码元**（现网用 `String.prototype.slice`）。原因同
//!   [`crate::text`]：JS 的 slice 可能把代理对切成孤立代理，Rust 的 `String` 无法表示。
//! - **`name_norm` 用 `to_lowercase` 而非 `toLocaleLowerCase`**。两者在 `en-US` 下一致；
//!   差异只出现在土耳其语等特殊 locale，而现网进程 locale 不可控，因此选确定性的那个。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::constants::{
    QUERY_CONFIG_MAX_JSON_BYTES, VENDOR_CONFIG_LABEL_MAX_LENGTH, VENDOR_NAME_MAX_LENGTH,
};
use crate::error::{Result, VendorError};
use crate::gateway::GatewayVendorHealth;
use crate::id::VendorId;
use crate::target::ProviderId;
use crate::time::Timestamp;

/// 查询接口使用的认证方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryAuthMode {
    /// `Authorization: Bearer <key>`。
    Bearer,
    /// `x-api-key`。
    XApiKey,
    /// `x-goog-api-key`。
    XGoogApiKey,
    /// `api-key`。
    ApiKey,
    /// 把密钥放入 `key` / `api_key` 查询参数。
    Query,
    /// 不附加认证。
    None,
}

impl QueryAuthMode {
    /// 全部取值。
    pub const ALL: [Self; 6] = [
        Self::Bearer,
        Self::XApiKey,
        Self::XGoogApiKey,
        Self::ApiKey,
        Self::Query,
        Self::None,
    ];

    /// 序列化短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bearer => "bearer",
            Self::XApiKey => "x-api-key",
            Self::XGoogApiKey => "x-goog-api-key",
            Self::ApiKey => "api-key",
            Self::Query => "query",
            Self::None => "none",
        }
    }
}

/// HTTP 方法（余额查询用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// GET。
    Get,
    /// POST。
    Post,
}

/// 模型列表查询配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelQueryConfig {
    /// 查询地址（相对或绝对，≤2000 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// 认证方式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<QueryAuthMode>,
    /// 自定义认证 Header 名（≤100 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header_name: Option<String>,
    /// 查询参数形式的认证键名（≤100 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_query_name: Option<String>,
    /// 附加请求头。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
}

impl ModelQueryConfig {
    /// 是否至少有一个字段有值。
    #[must_use]
    pub fn has_values(&self) -> bool {
        self.endpoint.is_some()
            || self.auth_mode.is_some()
            || self.auth_header_name.is_some()
            || self.auth_query_name.is_some()
            || self
                .headers
                .as_ref()
                .is_some_and(|headers| !headers.is_empty())
    }
}

/// 余额查询模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BalanceQueryTemplate {
    /// 自动探测。
    Auto,
    /// 通用模板。
    Generic,
    /// New API 站点。
    NewApi,
    /// 自定义。
    Custom,
}

impl BalanceQueryTemplate {
    /// 全部取值。
    pub const ALL: [Self; 4] = [Self::Auto, Self::Generic, Self::NewApi, Self::Custom];

    /// 序列化短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Generic => "generic",
            Self::NewApi => "new-api",
            Self::Custom => "custom",
        }
    }
}

/// 余额查询配置。路径字段使用点号分隔的 JSON Path，例如 `data.quota`。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceQueryConfig {
    /// 模板。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<BalanceQueryTemplate>,
    /// 基础地址（≤2000 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// 查询路径（≤2000 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// HTTP 方法。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<HttpMethod>,
    /// 认证方式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_mode: Option<QueryAuthMode>,
    /// 自定义认证 Header 名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header_name: Option<String>,
    /// 查询参数形式的认证键名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_query_name: Option<String>,
    /// 附加请求头。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    /// 独立访问令牌（≤4000 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    /// 用户编号（≤200 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    /// 剩余额度字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_path: Option<String>,
    /// 总额度字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_path: Option<String>,
    /// 已用额度字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_path: Option<String>,
    /// 单位字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit_path: Option<String>,
    /// 套餐名字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_path: Option<String>,
    /// 有效性字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_path: Option<String>,
    /// 状态字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_path: Option<String>,
    /// 失效说明字段路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalid_message_path: Option<String>,
    /// 余额换算倍率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<f64>,
}

impl BalanceQueryConfig {
    /// 字段数量（不含 `template`）。
    fn value_field_count(&self) -> usize {
        [
            self.base_url.is_some(),
            self.endpoint.is_some(),
            self.method.is_some(),
            self.auth_mode.is_some(),
            self.auth_header_name.is_some(),
            self.auth_query_name.is_some(),
            self.headers.as_ref().is_some_and(|h| !h.is_empty()),
            self.access_token.is_some(),
            self.user_id.is_some(),
            self.remaining_path.is_some(),
            self.total_path.is_some(),
            self.used_path.is_some(),
            self.unit_path.is_some(),
            self.plan_path.is_some(),
            self.valid_path.is_some(),
            self.status_path.is_some(),
            self.invalid_message_path.is_some(),
            self.multiplier.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
    }
}

/// 供应商费率（按百万 token 计价的美元费率）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorPricing {
    /// 输入费率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_per_million_usd: Option<f64>,
    /// 输出费率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_per_million_usd: Option<f64>,
}

impl VendorPricing {
    /// 是否两个费率都未设置。
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.input_per_million_usd.is_none() && self.output_per_million_usd.is_none()
    }
}

/// 要写入用户 CLI 的配置模板。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorConfigTemplate {
    /// 模板编号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 展示标签（≤60 字符）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// 是否启用。
    pub enabled: bool,
    /// 目标路径（必须在白名单目录内）。
    pub target_path: String,
    /// 配置内容，支持 `{{API_KEY}}` / `{{BASE_URL}}` / `{{VENDOR_NAME}}` 占位符。
    pub content: String,
}

/// 一个供应商。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiVendor {
    /// 供应商编号。
    pub id: VendorId,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 展示名。
    pub name: String,
    /// API Key（明文存储；网关需要直接使用）。
    pub api_key: String,
    /// API 基础地址。
    pub api_base_url: String,
    /// 候选池排序（同平台内唯一）。
    pub sort: i64,
    /// 费率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<VendorPricing>,
    /// 模型查询配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_query: Option<ModelQueryConfig>,
    /// 余额查询配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_query: Option<BalanceQueryConfig>,
    /// 配置模板。
    #[serde(default)]
    pub configs: Vec<VendorConfigTemplate>,
    /// 是否参与候选池。
    pub enabled: bool,
    /// 创建时间。
    pub created_at: Timestamp,
    /// 更新时间。
    pub updated_at: Timestamp,
    /// 最近一次启用时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_enabled_at: Option<Timestamp>,
    /// 余额快照（读时关联）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<BalanceSnapshot>,
    /// 余额查询状态（读时关联）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_status: Option<BalanceStatus>,
    /// 余额查询错误（读时关联）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_error: Option<String>,
    /// 余额查询时间（读时关联）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_queried_at: Option<Timestamp>,
    /// 网关健康状态（读时关联）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_health: Option<GatewayVendorHealth>,
}

impl ApiVendor {
    /// 是否已具备参与网关候选池的完整条件。
    #[must_use]
    pub fn is_route_eligible(&self) -> bool {
        self.enabled && !self.api_key.trim().is_empty() && !self.api_base_url.trim().is_empty()
    }
}

/// 余额快照。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceSnapshot {
    /// 剩余额度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    /// 总额度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    /// 已用额度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    /// 单位。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// 套餐名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
    /// 额度是否有效。
    pub is_valid: bool,
}

/// 余额查询状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BalanceStatus {
    /// 空闲（尚未查询）。
    Idle,
    /// 查询中。
    Loading,
    /// 成功。
    Success,
    /// 失败。
    Error,
}

impl BalanceStatus {
    /// 序列化短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::Success => "success",
            Self::Error => "error",
        }
    }

    /// 由短名解析；未知值回退为 [`Self::Idle`]。
    #[must_use]
    pub fn from_str_lossy(value: &str) -> Self {
        match value {
            "loading" => Self::Loading,
            "success" => Self::Success,
            "error" => Self::Error,
            _ => Self::Idle,
        }
    }
}

/// 模型列表中的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorModel {
    /// 模型编号。
    pub id: String,
    /// 对象类型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    /// 归属方。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owned_by: Option<String>,
    /// 创建时间（Unix 秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<i64>,
    /// 描述。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// 计费倍率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_multiplier: Option<f64>,
    /// 标签。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// 保存供应商时的输入。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiVendorInput {
    /// 供应商编号；为空表示新建。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<VendorId>,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 展示名。
    #[serde(default)]
    pub name: String,
    /// API Key。
    #[serde(default)]
    pub api_key: String,
    /// API 基础地址。
    #[serde(default)]
    pub api_base_url: String,
    /// 排序。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<i64>,
    /// 费率。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<VendorPricing>,
    /// 模型查询配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_query: Option<ModelQueryConfig>,
    /// 余额查询配置。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance_query: Option<BalanceQueryConfig>,
    /// 是否启用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// 配置模板。
    #[serde(default)]
    pub configs: Vec<VendorConfigTemplate>,
}

/// 归一化后的供应商输入。
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedVendorInput {
    /// 供应商编号；`None` 表示新建。
    pub id: Option<VendorId>,
    /// 所属平台。
    pub provider_id: ProviderId,
    /// 已 `trim` 并截断的名称。
    pub name: String,
    /// 已 `trim` 的 API Key（可能为空，表示沿用旧值）。
    pub api_key: String,
    /// 已 `trim` 的基础地址。
    pub api_base_url: String,
    /// 排序（可能为 `None`，由仓储决定）。
    pub sort: Option<i64>,
    /// 费率。
    pub pricing: Option<VendorPricing>,
    /// 模型查询配置。
    pub model_query: Option<ModelQueryConfig>,
    /// 余额查询配置。
    pub balance_query: Option<BalanceQueryConfig>,
    /// 是否启用。
    pub enabled: bool,
    /// 配置模板。
    pub configs: Vec<VendorConfigTemplate>,
}

impl ApiVendorInput {
    /// 归一化并校验。
    ///
    /// `allow_empty_api_key` 为真时允许留空（更新场景：留空表示沿用已保存的 Key）。
    pub fn normalize(&self, allow_empty_api_key: bool) -> Result<NormalizedVendorInput> {
        let name = self.name.trim();
        if name.is_empty() {
            return Err(VendorError::NameRequired.into());
        }
        let api_key = self.api_key.trim();
        if api_key.is_empty() && !allow_empty_api_key {
            return Err(VendorError::ApiKeyRequired.into());
        }
        let api_base_url = self.api_base_url.trim();
        if api_base_url.is_empty() {
            return Err(VendorError::ApiBaseUrlRequired.into());
        }

        let configs = self
            .configs
            .iter()
            .map(|config| VendorConfigTemplate {
                // 配置行编号是全局主键，**不能复用渲染层默认模板里的固定 ID**，
                // 否则复制或导入供应商时会撞主键。这里一律重新生成。
                id: Some(uuid::Uuid::new_v4().to_string()),
                provider_id: config.provider_id,
                label: clamp_optional(config.label.as_deref(), VENDOR_CONFIG_LABEL_MAX_LENGTH),
                enabled: config.enabled,
                target_path: config.target_path.trim().to_owned(),
                content: config.content.clone(),
            })
            .collect();

        Ok(NormalizedVendorInput {
            id: self.id.clone(),
            provider_id: self.provider_id,
            name: clamp(name, VENDOR_NAME_MAX_LENGTH),
            api_key: api_key.to_owned(),
            api_base_url: api_base_url.to_owned(),
            sort: self.sort,
            pricing: normalize_pricing(self.pricing.as_ref())?,
            model_query: normalize_model_query(self.model_query.as_ref())?,
            balance_query: normalize_balance_query(self.balance_query.as_ref())?,
            // 现网是 `input.enabled !== false`：缺省视为启用。
            enabled: self.enabled != Some(false),
            configs,
        })
    }
}

/// 供应商名称归一化键（同一平台内唯一）。
#[must_use]
pub fn normalize_vendor_name(value: &str) -> String {
    value.trim().to_lowercase()
}

/// 费率归一化：有限、非负、最多 2 位小数，并四舍五入到 2 位。
fn normalize_pricing(pricing: Option<&VendorPricing>) -> Result<Option<VendorPricing>> {
    let Some(pricing) = pricing else {
        return Ok(None);
    };
    let normalized = VendorPricing {
        input_per_million_usd: normalize_rate(pricing.input_per_million_usd, "输入费率")?,
        output_per_million_usd: normalize_rate(pricing.output_per_million_usd, "输出费率")?,
    };
    Ok(if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    })
}

/// 单个费率归一化。
///
/// # 为什么用**带容差**的比较，而不是精确浮点相等
///
/// 这里判断的是「乘以 100 之后是不是整数」，也就是「小数位是否超过 2 位」。
/// 但 `0.55 * 100.0` 在 IEEE 754 里是 `55.00000000000001`——精确相等会把它判成
/// 「超过 2 位小数」，而 **0.55 恰恰是最常见的那种价格**；`2.2` 同理
/// （`220.00000000000003`）。实测就是被这条拒掉的。
///
/// 容差取 `1e-9`：它比浮点乘法的舍入误差（这个量级约 `1e-14`）大好几个数量级，
/// 又比「真的多了一位小数」小得多——`1.234 * 100.0 = 123.4`，与最近整数的距离是
/// `0.4`。因此 `1.234` 照样被拒。
///
/// 原先那句注释担心「用误差范围比较会让 1.234 被误判为合法」——按上面的量级，
/// 那个担心不成立，而它付出的代价是拒绝 `0.55`。
#[allow(clippy::float_cmp)]
fn normalize_rate(value: Option<f64>, label: &'static str) -> Result<Option<f64>> {
    /// 判定「乘以 100 后是不是整数」的容差。
    const SCALE_TOLERANCE: f64 = 1e-9;
    let Some(value) = value else {
        return Ok(None);
    };
    if !value.is_finite() || value < 0.0 {
        return Err(VendorError::PricingInvalid { label }.into());
    }
    let scaled = value * 100.0;
    if (scaled - scaled.round()).abs() > SCALE_TOLERANCE {
        return Err(VendorError::PricingInvalid { label }.into());
    }
    // 取整后再除，落库的是干净的两小数位值。
    Ok(Some(scaled.round() / 100.0))
}

/// 模型查询配置归一化。全部字段为空时返回 [`None`]。
fn normalize_model_query(config: Option<&ModelQueryConfig>) -> Result<Option<ModelQueryConfig>> {
    let Some(config) = config else {
        return Ok(None);
    };
    let normalized = ModelQueryConfig {
        endpoint: query_text(config.endpoint.as_deref(), 2_000),
        // 枚举类型本身已排除非法值，无需额外校验。
        auth_mode: config.auth_mode,
        auth_header_name: header_name(config.auth_header_name.as_deref())?,
        auth_query_name: query_text(config.auth_query_name.as_deref(), 100),
        headers: normalize_headers(config.headers.as_ref())?,
    };
    assert_query_config_size(&normalized)?;
    Ok(if normalized.has_values() {
        Some(normalized)
    } else {
        None
    })
}

/// 余额查询配置归一化。
///
/// 只有 `template` 且其为 `auto` 时视为「未配置」，返回 [`None`]——与现网一致。
fn normalize_balance_query(
    config: Option<&BalanceQueryConfig>,
) -> Result<Option<BalanceQueryConfig>> {
    let Some(config) = config else {
        return Ok(None);
    };
    let template = config.template.unwrap_or(BalanceQueryTemplate::Auto);
    let normalized = BalanceQueryConfig {
        template: Some(template),
        base_url: query_text(config.base_url.as_deref(), 2_000),
        endpoint: query_text(config.endpoint.as_deref(), 2_000),
        method: config.method,
        // 枚举类型本身已排除非法值，无需额外校验。
        auth_mode: config.auth_mode,
        auth_header_name: header_name(config.auth_header_name.as_deref())?,
        auth_query_name: query_text(config.auth_query_name.as_deref(), 100),
        headers: normalize_headers(config.headers.as_ref())?,
        access_token: query_text(config.access_token.as_deref(), 4_000),
        user_id: query_text(config.user_id.as_deref(), 200),
        remaining_path: query_text(config.remaining_path.as_deref(), 300),
        total_path: query_text(config.total_path.as_deref(), 300),
        used_path: query_text(config.used_path.as_deref(), 300),
        unit_path: query_text(config.unit_path.as_deref(), 300),
        plan_path: query_text(config.plan_path.as_deref(), 300),
        valid_path: query_text(config.valid_path.as_deref(), 300),
        status_path: query_text(config.status_path.as_deref(), 300),
        invalid_message_path: query_text(config.invalid_message_path.as_deref(), 300),
        multiplier: config.multiplier,
    };

    if let Some(multiplier) = normalized.multiplier
        && (!multiplier.is_finite() || multiplier <= 0.0 || multiplier > 1_000_000.0)
    {
        return Err(VendorError::BalanceMultiplierInvalid.into());
    }
    assert_query_config_size(&normalized)?;

    // 只有 template=auto 且没有任何其他字段时视为未配置。
    Ok(
        if template == BalanceQueryTemplate::Auto && normalized.value_field_count() == 0 {
            None
        } else {
            Some(normalized)
        },
    )
}

/// 归一化自定义 Header 名并校验字符集。
fn header_name(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = query_text(value, 100) else {
        return Ok(None);
    };
    if !is_valid_header_name(&value) {
        return Err(VendorError::AuthHeaderNameInvalid.into());
    }
    Ok(Some(value))
}

/// 归一化自定义请求头集合。
fn normalize_headers(
    headers: Option<&BTreeMap<String, String>>,
) -> Result<Option<BTreeMap<String, String>>> {
    let Some(headers) = headers else {
        return Ok(None);
    };
    let mut normalized = BTreeMap::new();
    for (name, value) in headers.iter().take(32) {
        if !is_valid_header_name(name) || value.len() > 4_000 || value.contains(['\r', '\n']) {
            return Err(VendorError::HeaderInvalid.into());
        }
        normalized.insert(name.clone(), value.clone());
    }
    Ok(if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    })
}

/// HTTP token 字符集校验（对应现网 `QUERY_HEADER_NAME_PATTERN`）。
fn is_valid_header_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > 100 {
        return false;
    }
    bytes.iter().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'.'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'|'
                    | b'~'
                    | b'-'
            )
    })
}

/// 归一化查询文本：空白视为未设置，否则 `trim` 并截断。
fn query_text(value: Option<&str>, max_length: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(clamp(trimmed, max_length))
    }
}

/// 按字符数截断。
fn clamp(value: &str, max_length: usize) -> String {
    value.chars().take(max_length).collect()
}

/// 可选字段截断，空白视为未设置。
fn clamp_optional(value: Option<&str>, max_length: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(clamp(trimmed, max_length))
    }
}

/// 校验查询配置的 JSON 体积。
fn assert_query_config_size<T: Serialize>(value: &T) -> Result<()> {
    let bytes = serde_json::to_string(value).map_or(0, |text| text.len());
    if bytes > QUERY_CONFIG_MAX_JSON_BYTES {
        return Err(VendorError::QueryConfigTooLarge.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str) -> ApiVendorInput {
        ApiVendorInput {
            provider_id: ProviderId::Codex,
            name: name.to_owned(),
            api_key: "sk-test".to_owned(),
            api_base_url: "https://api.example.com".to_owned(),
            ..ApiVendorInput::default()
        }
    }

    #[test]
    fn required_fields_are_validated_in_the_existing_order() {
        // 名称优先于 Key 优先于地址——顺序决定用户先看到哪条错误。
        let mut raw = input("  ");
        raw.api_key = "  ".to_owned();
        raw.api_base_url = "  ".to_owned();
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "供应商名称不能为空。"
        );

        let mut raw = input("名字");
        raw.api_key = "  ".to_owned();
        raw.api_base_url = "  ".to_owned();
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "API Key 不能为空。"
        );

        let mut raw = input("名字");
        raw.api_base_url = "  ".to_owned();
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "API 请求地址不能为空。"
        );
    }

    #[test]
    fn empty_api_key_is_allowed_when_updating() {
        let mut raw = input("名字");
        raw.api_key = "   ".to_owned();
        let normalized = raw.normalize(true).expect("更新时允许留空");
        assert_eq!(normalized.api_key, "");
    }

    #[test]
    fn name_and_url_are_trimmed_and_name_is_truncated() {
        let mut raw = input("  名字  ");
        raw.api_base_url = "  https://x.example  ".to_owned();
        let normalized = raw.normalize(false).expect("normalize");
        assert_eq!(normalized.name, "名字");
        assert_eq!(normalized.api_base_url, "https://x.example");

        let long = input(&"n".repeat(200));
        assert_eq!(
            long.normalize(false)
                .expect("normalize")
                .name
                .chars()
                .count(),
            VENDOR_NAME_MAX_LENGTH
        );
    }

    #[test]
    fn enabled_defaults_to_true_unless_explicitly_false() {
        assert!(input("n").normalize(false).expect("normalize").enabled);
        let mut raw = input("n");
        raw.enabled = Some(false);
        assert!(!raw.normalize(false).expect("normalize").enabled);
    }

    #[test]
    fn pricing_accepts_up_to_two_decimals_and_rounds() {
        let mut raw = input("n");
        raw.pricing = Some(VendorPricing {
            input_per_million_usd: Some(1.25),
            output_per_million_usd: Some(0.0),
        });
        let pricing = raw
            .normalize(false)
            .expect("normalize")
            .pricing
            .expect("pricing");
        assert_eq!(pricing.input_per_million_usd, Some(1.25));
        // 0 是有效费率，不能被当成「未设置」。
        assert_eq!(pricing.output_per_million_usd, Some(0.0));
    }

    /// **常见价格必须被接受**——包括那些二进制表示不精确的。
    ///
    /// # 这条测试是怎么来的
    ///
    /// 上一条测试用的是 `1.25`，而 `1.25` 恰好能被二进制精确表示
    /// （`1.25 * 100.0 == 125.0` 精确成立），于是它**在缺陷存在时也是绿的**。
    ///
    /// 而 `0.55 * 100.0` 是 `55.00000000000001`。原先的判断是
    /// `scaled.round() != scaled`（精确相等），因此 **`0.55` 和 `2.2` 这类最常见
    /// 的价格会被拒**，报的还是「必须是非负数，且最多保留 2 位小数」——
    /// 用户看着 0.55 完全不知道哪里错了。
    ///
    /// 探针写了一条 `pricing.inputPerMillionUsd = 0.55` 的供应商才把它撞出来。
    #[test]
    fn pricing_accepts_ordinary_prices_that_are_not_binary_exact() {
        for price in [0.55, 2.2, 0.29, 1.15, 3.75, 0.01, 15.0] {
            let mut raw = input("n");
            raw.pricing = Some(VendorPricing {
                input_per_million_usd: Some(price),
                output_per_million_usd: None,
            });
            let pricing = raw
                .normalize(false)
                .unwrap_or_else(|error| panic!("{price} 必须被接受，实际报错：{error}"))
                .pricing
                .expect("pricing");
            assert_eq!(
                pricing.input_per_million_usd,
                Some(price),
                "{price} 归一化后不该变"
            );
        }
    }

    /// 容差是 `1e-9`，比它大得多的偏差仍然要拒——**放开 0.55 不等于放开 1.234**。
    #[test]
    fn the_tolerance_still_rejects_a_real_third_decimal() {
        for bad in [1.234, 0.555, 9.999, 0.001] {
            let mut raw = input("n");
            raw.pricing = Some(VendorPricing {
                input_per_million_usd: Some(bad),
                output_per_million_usd: None,
            });
            assert!(
                raw.normalize(false).is_err(),
                "{bad} 有真实的三位小数，必须拒绝"
            );
        }
    }

    #[test]
    fn pricing_rejects_more_than_two_decimals_negative_and_non_finite() {
        for bad in [1.234, -1.0, f64::NAN, f64::INFINITY] {
            let mut raw = input("n");
            raw.pricing = Some(VendorPricing {
                input_per_million_usd: Some(bad),
                output_per_million_usd: None,
            });
            let error = raw.normalize(false).expect_err("必须拒绝");
            assert_eq!(
                error.to_string(),
                "输入费率必须是非负数，且最多保留 2 位小数。"
            );
        }
    }

    #[test]
    fn pricing_label_follows_the_offending_field() {
        let mut raw = input("n");
        raw.pricing = Some(VendorPricing {
            input_per_million_usd: None,
            output_per_million_usd: Some(9.999),
        });
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "输出费率必须是非负数，且最多保留 2 位小数。"
        );
    }

    #[test]
    fn empty_pricing_normalizes_to_none() {
        let mut raw = input("n");
        raw.pricing = Some(VendorPricing::default());
        assert!(raw.normalize(false).expect("normalize").pricing.is_none());
    }

    #[test]
    fn model_query_normalizes_text_and_drops_fully_empty_configs() {
        let mut raw = input("n");
        raw.model_query = Some(ModelQueryConfig {
            endpoint: Some("  /v1/models  ".to_owned()),
            ..ModelQueryConfig::default()
        });
        let config = raw
            .normalize(false)
            .expect("normalize")
            .model_query
            .expect("config");
        assert_eq!(config.endpoint.as_deref(), Some("/v1/models"));

        // 全部字段为空 → 视为未配置。
        let mut raw = input("n");
        raw.model_query = Some(ModelQueryConfig {
            endpoint: Some("   ".to_owned()),
            ..ModelQueryConfig::default()
        });
        assert!(
            raw.normalize(false)
                .expect("normalize")
                .model_query
                .is_none()
        );
    }

    #[test]
    fn header_names_must_match_the_http_token_charset() {
        let mut raw = input("n");
        raw.model_query = Some(ModelQueryConfig {
            auth_header_name: Some("X-Custom-Key".to_owned()),
            ..ModelQueryConfig::default()
        });
        assert!(raw.normalize(false).is_ok());

        for bad in ["X Custom", "X:Key", "X\nKey", ""] {
            let mut raw = input("n");
            raw.model_query = Some(ModelQueryConfig {
                auth_header_name: Some(bad.to_owned()),
                ..ModelQueryConfig::default()
            });
            // 空白名会被归一化为「未设置」而不是报错。
            if bad.trim().is_empty() {
                assert!(raw.normalize(false).is_ok());
            } else {
                assert_eq!(
                    raw.normalize(false).expect_err("必须拒绝").to_string(),
                    "自定义认证 Header 名称无效。"
                );
            }
        }
    }

    #[test]
    fn custom_headers_reject_newlines_and_oversized_values() {
        let mut headers = BTreeMap::new();
        headers.insert("X-Ok".to_owned(), "value".to_owned());
        let mut raw = input("n");
        raw.model_query = Some(ModelQueryConfig {
            headers: Some(headers.clone()),
            ..ModelQueryConfig::default()
        });
        assert!(raw.normalize(false).is_ok());

        // 换行会被拒绝（Header 注入防护）。
        let mut injected = BTreeMap::new();
        injected.insert("X-Bad".to_owned(), "a\r\nX-Evil: 1".to_owned());
        let mut raw = input("n");
        raw.model_query = Some(ModelQueryConfig {
            headers: Some(injected),
            ..ModelQueryConfig::default()
        });
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "自定义请求头无效。"
        );

        // 值超长同样拒绝。
        let mut oversized = BTreeMap::new();
        oversized.insert("X-Big".to_owned(), "v".repeat(4_001));
        let mut raw = input("n");
        raw.model_query = Some(ModelQueryConfig {
            headers: Some(oversized),
            ..ModelQueryConfig::default()
        });
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "自定义请求头无效。"
        );
    }

    #[test]
    fn balance_query_defaults_to_auto_template() {
        let mut raw = input("n");
        raw.balance_query = Some(BalanceQueryConfig {
            endpoint: Some("/api/user/self".to_owned()),
            ..BalanceQueryConfig::default()
        });
        let config = raw
            .normalize(false)
            .expect("normalize")
            .balance_query
            .expect("config");
        assert_eq!(config.template, Some(BalanceQueryTemplate::Auto));
    }

    #[test]
    fn balance_query_with_only_auto_template_is_treated_as_unset() {
        let mut raw = input("n");
        raw.balance_query = Some(BalanceQueryConfig {
            template: Some(BalanceQueryTemplate::Auto),
            ..BalanceQueryConfig::default()
        });
        assert!(
            raw.normalize(false)
                .expect("normalize")
                .balance_query
                .is_none()
        );

        // 非 auto 模板即使没有其他字段也要保留。
        let mut raw = input("n");
        raw.balance_query = Some(BalanceQueryConfig {
            template: Some(BalanceQueryTemplate::NewApi),
            ..BalanceQueryConfig::default()
        });
        assert!(
            raw.normalize(false)
                .expect("normalize")
                .balance_query
                .is_some()
        );
    }

    #[test]
    fn balance_multiplier_range_is_enforced() {
        for good in [0.5, 1.0, 1_000_000.0] {
            let mut raw = input("n");
            raw.balance_query = Some(BalanceQueryConfig {
                multiplier: Some(good),
                ..BalanceQueryConfig::default()
            });
            assert!(raw.normalize(false).is_ok(), "{good} 应当合法");
        }
        for bad in [0.0, -1.0, 1_000_001.0, f64::NAN] {
            let mut raw = input("n");
            raw.balance_query = Some(BalanceQueryConfig {
                multiplier: Some(bad),
                ..BalanceQueryConfig::default()
            });
            assert_eq!(
                raw.normalize(false).expect_err("必须拒绝").to_string(),
                "余额换算倍率必须大于 0 且不超过 1000000。"
            );
        }
    }

    #[test]
    fn query_config_size_limit_is_enforced() {
        // 单个字段会被截断到各自上限，因此只能靠字段组合突破 32 KB：
        // 32 个请求头 × 4000 字符 ≈ 128 KB，远超上限。
        let mut headers = BTreeMap::new();
        for index in 0..32 {
            headers.insert(format!("X-Pad-{index}"), "v".repeat(4_000));
        }
        let mut raw = input("n");
        raw.balance_query = Some(BalanceQueryConfig {
            headers: Some(headers),
            ..BalanceQueryConfig::default()
        });
        assert_eq!(
            raw.normalize(false).expect_err("必须拒绝").to_string(),
            "供应商查询配置过大。"
        );
    }

    #[test]
    fn config_templates_get_fresh_ids_and_trimmed_paths() {
        let mut raw = input("n");
        raw.configs = vec![
            VendorConfigTemplate {
                id: Some("renderer-template-id".to_owned()),
                provider_id: ProviderId::Codex,
                label: Some("  标签  ".to_owned()),
                enabled: true,
                target_path: "  ~/.codex/config.toml  ".to_owned(),
                content: "x".to_owned(),
            },
            VendorConfigTemplate {
                id: Some("renderer-template-id".to_owned()),
                provider_id: ProviderId::Codex,
                label: None,
                enabled: true,
                target_path: "~/.codex/other.toml".to_owned(),
                content: "y".to_owned(),
            },
        ];
        let normalized = raw.normalize(false).expect("normalize");
        assert_eq!(normalized.configs.len(), 2);
        assert_eq!(normalized.configs[0].target_path, "~/.codex/config.toml");
        assert_eq!(normalized.configs[0].label.as_deref(), Some("标签"));
        // 两行的编号必须不同，否则会撞全局主键。
        assert_ne!(normalized.configs[0].id, normalized.configs[1].id);
        assert_ne!(
            normalized.configs[0].id.as_deref(),
            Some("renderer-template-id")
        );
    }

    #[test]
    fn vendor_name_key_is_trimmed_and_lowercased() {
        assert_eq!(normalize_vendor_name("  My Vendor  "), "my vendor");
        assert_eq!(normalize_vendor_name("DEEPSEEK"), "deepseek");
    }

    #[test]
    fn route_eligibility_requires_enabled_key_and_url() {
        let mut vendor = ApiVendor {
            id: VendorId::from("v1"),
            provider_id: ProviderId::Codex,
            name: "n".to_owned(),
            api_key: "k".to_owned(),
            api_base_url: "https://x".to_owned(),
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
        };
        assert!(vendor.is_route_eligible());

        vendor.enabled = false;
        assert!(!vendor.is_route_eligible());
        vendor.enabled = true;
        vendor.api_key = "   ".to_owned();
        assert!(!vendor.is_route_eligible(), "空白 Key 不算配置完整");
        vendor.api_key = "k".to_owned();
        vendor.api_base_url = String::new();
        assert!(!vendor.is_route_eligible());
    }

    #[test]
    fn auth_modes_and_templates_serialize_to_their_short_names() {
        assert_eq!(
            serde_json::to_string(&QueryAuthMode::XGoogApiKey).unwrap(),
            "\"x-goog-api-key\""
        );
        assert_eq!(
            serde_json::to_string(&QueryAuthMode::ApiKey).unwrap(),
            "\"api-key\""
        );
        assert_eq!(
            serde_json::to_string(&BalanceQueryTemplate::NewApi).unwrap(),
            "\"new-api\""
        );
        assert_eq!(
            serde_json::to_string(&HttpMethod::Post).unwrap(),
            "\"POST\""
        );
        assert_eq!(
            BalanceStatus::from_str_lossy("loading"),
            BalanceStatus::Loading
        );
        assert_eq!(BalanceStatus::from_str_lossy("???"), BalanceStatus::Idle);
    }
}
