//! 网关相关 IPC 命令。
//!
//! # 覆盖范围
//!
//! 健康状态、故障规则、请求日志、用量报表与失败诊断。这些命令全部落在已有仓储上，
//! 因此本模块没有算法，只有「参数 → 仓储调用 → 线上形状」的映射。
//!
//! # 三处需要注意的取值约定
//!
//! 1. **端口 0 表示自动分配**，不是「非法」。界面上「自动」这一档就传 0。
//! 2. **用量区间是必填的**。缺省区间会让聚合查询扫全表，因此由调用方明确给出。
//! 3. **诊断分页的页码从 1 开始**，且每页上限 50——这是仓储的钳制，命令面不重复实现，
//!    但要把真实的钳制规则写在文档里，避免调用方以为传 100 会拿到 100 条。

use serde_json::Value;

use aivc_core::error::Result;
use aivc_core::gateway::{FailoverRule, FailoverRuleInput, GatewayOutcome, GatewayVendorHealth};
use aivc_core::gateway_usage::{
    FailureDiagnosticsPage, FailureOutcomeFilter, UsageReport, UsageSummary,
};
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;
use aivc_infra::db::{LogEntry, LogFilter};
use aivc_provider::ProviderContext;

use crate::validate::{
    require_boolean, require_gateway_circuit_duration_seconds,
    require_gateway_circuit_failure_threshold, require_gateway_failure_threshold,
    require_gateway_port, require_positive_integer, require_string,
};

/// 诊断分页每页上限（与仓储一致）。
pub const DIAGNOSTICS_PAGE_SIZE_MAX: u32 = 50;

/// 网关健康命令。
pub struct GatewayHealth;

impl GatewayHealth {
    /// 列出全部健康状态。
    ///
    /// # Errors
    ///
    /// 数据库读取失败。
    pub async fn get_vendor_health(ctx: &ProviderContext) -> Result<Vec<GatewayVendorHealth>> {
        ctx.gateway_health().load_all().await
    }

    /// 重置健康状态。
    ///
    /// 不传 `vendorId` 表示重置全部（界面上「重置全部」按钮走这条路）。
    ///
    /// # Errors
    ///
    /// 数据库写入失败。
    pub async fn reset_vendor_health(ctx: &ProviderContext, args: &Value) -> Result<u64> {
        match args
            .get("vendorId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
        {
            Some(vendor_id) => {
                let removed = ctx
                    .gateway_health()
                    .delete(&VendorId::from(vendor_id))
                    .await?;
                Ok(u64::from(removed))
            }
            None => ctx
                .gateway_health()
                .delete_all()
                .await
                .map(|count| count as u64),
        }
    }
}

/// 故障规则命令。
pub struct GatewayRules;

impl GatewayRules {
    /// 列出全部规则。
    ///
    /// # Errors
    ///
    /// 数据库读取失败。
    pub async fn list_failover_rules(ctx: &ProviderContext) -> Result<Vec<FailoverRule>> {
        ctx.failover_rule().list().await
    }

    /// 保存规则（新建或更新）。
    ///
    /// # Errors
    ///
    /// 作用域、匹配条件或自定义响应非法时返回领域错误。
    pub async fn save_failover_rule(ctx: &ProviderContext, args: &Value) -> Result<FailoverRule> {
        let input: FailoverRuleInput = serde_json::from_value(args.clone()).map_err(|error| {
            aivc_core::Error::Ipc(aivc_core::error::IpcError::CommandFailed {
                message: format!("参数无效：gateway failover rule（{error}）"),
            })
        })?;
        ctx.failover_rule().save(&input).await
    }

    /// 删除规则。
    ///
    /// # Errors
    ///
    /// 数据库写入失败。
    pub async fn delete_failover_rule(ctx: &ProviderContext, args: &Value) -> Result<bool> {
        let id = require_string(args.get("id"), "id")?;
        ctx.failover_rule().delete(&id).await
    }

    /// 启用或停用规则。
    ///
    /// # Errors
    ///
    /// 数据库写入失败。
    pub async fn set_failover_rule_enabled(ctx: &ProviderContext, args: &Value) -> Result<bool> {
        let id = require_string(args.get("id"), "id")?;
        let enabled = require_boolean(args.get("enabled"), "enabled")?;
        ctx.failover_rule().set_enabled(&id, enabled).await
    }
}

/// 用量与日志命令。
pub struct GatewayLogs;

impl GatewayLogs {
    /// 用量汇总。
    ///
    /// # Errors
    ///
    /// 区间缺失或数据库读取失败。
    pub async fn get_usage_summary(ctx: &ProviderContext, args: &Value) -> Result<UsageSummary> {
        let (start, end) = period(args)?;
        ctx.gateway_request_log().usage_summary(&start, &end).await
    }

    /// 用量报表（按供应商分组）。
    ///
    /// # Errors
    ///
    /// 区间缺失或数据库读取失败。
    pub async fn get_usage_report(ctx: &ProviderContext, args: &Value) -> Result<UsageReport> {
        let (start, end) = period(args)?;
        ctx.gateway_request_log().usage_report(&start, &end).await
    }

    /// 最近失败。
    ///
    /// # Errors
    ///
    /// 数据库读取失败。
    pub async fn get_recent_failures(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let failures = match args
            .get("limit")
            .and_then(Value::as_f64)
            .filter(|value| value.fract() == 0.0 && *value > 0.0)
        {
            Some(limit) => {
                // 上限 100：失败列表只用于界面上的一小块区域。
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let limit = (limit as u32).min(100);
                ctx.gateway_request_log()
                    .recent_failures_limited(limit)
                    .await?
            }
            None => ctx.gateway_request_log().recent_failures().await?,
        };
        serde_json::to_value(failures).map_err(|error| serialize_error(&error))
    }

    /// 失败诊断分页。
    ///
    /// 页码从 1 开始；每页上限 [`DIAGNOSTICS_PAGE_SIZE_MAX`]（由仓储钳制）。
    ///
    /// # Errors
    ///
    /// 参数非法或数据库读取失败。
    pub async fn get_failure_diagnostics(
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<FailureDiagnosticsPage> {
        let page = require_positive_integer(args.get("page"), "page")?;
        let page_size = require_positive_integer(args.get("pageSize"), "pageSize")?;
        let vendor_id = args
            .get("vendorId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(VendorId::from);
        let provider_id = args
            .get("providerId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ProviderId::from_target_id);
        let outcome = match args.get("outcome").and_then(Value::as_str) {
            Some("error") => FailureOutcomeFilter::Error,
            Some("timeout") => FailureOutcomeFilter::Timeout,
            // 未知值表示「不筛选」，而不是「按失败筛选」——筛选条件不该悄悄收紧。
            _ => FailureOutcomeFilter::Any,
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let page = page as u32;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let page_size = page_size as u32;

        ctx.gateway_request_log()
            .failure_diagnostics_page(
                page,
                page_size,
                vendor_id,
                outcome,
                args.get("periodStart").and_then(timestamp),
                args.get("periodEnd").and_then(timestamp),
                provider_id,
            )
            .await
    }

    /// 查询可清理的日志条目。
    ///
    /// # Errors
    ///
    /// 数据库读取失败。
    pub async fn query_logs(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let entries = ctx
            .gateway_request_log()
            .cleanup_entries(&filter(args))
            .await?;
        // `LogEntry` 是仓储的读取模型，未实现 Serialize。这里显式映射成线上形状：
        // 顺带把字段名固定下来，避免以后有人给它加 derive 就悄悄改变了契约。
        let items: Vec<Value> = entries.iter().map(log_entry_value).collect();
        Ok(Value::Array(items))
    }

    /// 删除指定请求编号的日志。
    ///
    /// # Errors
    ///
    /// 参数非法或数据库写入失败。
    pub async fn delete_log_entries(ctx: &ProviderContext, args: &Value) -> Result<u64> {
        let Some(items) = args.get("requestIds").and_then(Value::as_array) else {
            return Err(invalid("requestIds"));
        };
        let ids: Vec<String> = items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        ctx.gateway_request_log().delete_entries(&ids).await
    }

    /// 按筛选条件清空日志。
    ///
    /// # Errors
    ///
    /// 数据库写入失败。
    pub async fn clear_logs(ctx: &ProviderContext, args: &Value) -> Result<u64> {
        ctx.gateway_request_log()
            .delete_by_filter(&filter(args))
            .await
    }
}

/// 网关设置命令。
///
/// 读写都走 [`SettingsStore`]：读用 `snapshot()`（返回归一化快照，逐字段容错），
/// 写用 `update()`（在锁内做读-改-写，避免并发覆盖）。
pub struct GatewaySettingsCommands;

impl GatewaySettingsCommands {
    /// 读取全部网关设置。
    ///
    /// # Errors
    ///
    /// 设置读取失败。
    pub async fn get_settings(ctx: &ProviderContext) -> Result<Value> {
        let settings = ctx.settings().snapshot().await;
        Ok(serde_json::json!({
            "port": settings.gateway_port,
            "enabled": settings.gateway_enabled,
            "externalApiEnabled": settings.gateway_external_api_enabled,
            // 只暴露「是否已设置令牌」，绝不返回令牌或其摘要。
            "hasExternalToken": !settings.gateway_external_api_token_hash.is_empty(),
            "failureThreshold": settings.gateway_failure_threshold,
            "circuitFailureThreshold": settings.gateway_circuit_failure_threshold,
            "circuitDurationSeconds": settings.gateway_circuit_duration_seconds,
        }))
    }

    /// 读取网关端口。
    ///
    /// # Errors
    ///
    /// 设置读取失败。
    pub async fn get_port(ctx: &ProviderContext) -> Result<Value> {
        let settings = ctx.settings().snapshot().await;
        Ok(serde_json::json!({ "port": settings.gateway_port }))
    }

    /// 设置网关端口；`0` 表示自动分配。
    ///
    /// # Errors
    ///
    /// 端口越界或设置写入失败。
    pub async fn set_port(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let port = require_gateway_port(args.get("port"))?;
        ctx.settings()
            .update(|raw| raw.set_gateway_port(port))
            .await?;
        Ok(serde_json::json!({ "port": port }))
    }

    /// 设置网关启用状态。
    ///
    /// # Errors
    ///
    /// 参数非法或设置写入失败。
    pub async fn set_enabled(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let enabled = require_boolean(args.get("enabled"), "enabled")?;
        ctx.settings()
            .update(|raw| raw.set_gateway_enabled(enabled))
            .await?;
        Ok(serde_json::json!({ "enabled": enabled }))
    }

    /// 读取外部 API 状态。
    ///
    /// # Errors
    ///
    /// 设置读取失败。
    pub async fn get_external_api_status(ctx: &ProviderContext) -> Result<Value> {
        let settings = ctx.settings().snapshot().await;
        Ok(serde_json::json!({
            "enabled": settings.gateway_external_api_enabled,
            "hasToken": !settings.gateway_external_api_token_hash.is_empty(),
        }))
    }

    /// 设置外部 API 启用状态。
    ///
    /// # Errors
    ///
    /// 参数非法或设置写入失败。
    pub async fn set_external_api_enabled(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let enabled = require_boolean(args.get("enabled"), "enabled")?;
        ctx.settings()
            .update(|raw| raw.set_gateway_external_api_enabled(enabled))
            .await?;
        Ok(serde_json::json!({ "enabled": enabled }))
    }

    /// 轮换外部访问令牌。
    ///
    /// 生成一个新令牌，**只在这里返回一次**；库里只落 SHA-256 摘要。调用方必须立刻把它
    /// 显示给用户并提示保存——之后再也取不回来。
    ///
    /// # Errors
    ///
    /// 设置写入失败。
    pub async fn rotate_external_api_token(ctx: &ProviderContext) -> Result<Value> {
        let token = generate_external_token();
        let hash = hex::encode(<sha2::Sha256 as sha2::Digest>::digest(token.as_bytes()));
        ctx.settings()
            .update(|raw| raw.set_gateway_external_api_token_hash(&hash))
            .await?;
        Ok(serde_json::json!({ "token": token }))
    }

    /// 设置异常切换阈值（1–10）。
    ///
    /// # Errors
    ///
    /// 越界或设置写入失败。
    pub async fn set_failure_threshold(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let value = require_gateway_failure_threshold(args.get("value"))?;
        #[allow(clippy::cast_possible_truncation)]
        let threshold = value as u8;
        ctx.settings()
            .update(|raw| raw.set_gateway_failure_threshold(threshold))
            .await?;
        Ok(serde_json::json!({ "value": threshold }))
    }

    /// 设置熔断次数（1–20）。
    ///
    /// # Errors
    ///
    /// 越界或设置写入失败。
    pub async fn set_circuit_failure_threshold(
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Value> {
        let value = require_gateway_circuit_failure_threshold(args.get("value"))?;
        #[allow(clippy::cast_possible_truncation)]
        let threshold = value as u8;
        ctx.settings()
            .update(|raw| raw.set_gateway_circuit_failure_threshold(threshold))
            .await?;
        Ok(serde_json::json!({ "value": threshold }))
    }

    /// 设置熔断持续时长（10–86400 秒）。
    ///
    /// # Errors
    ///
    /// 越界或设置写入失败。
    pub async fn set_circuit_duration_seconds(
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Value> {
        let value = require_gateway_circuit_duration_seconds(args.get("value"))?;
        #[allow(clippy::cast_possible_truncation)]
        let seconds = value as u32;
        ctx.settings()
            .update(|raw| raw.set_gateway_circuit_duration_seconds(seconds))
            .await?;
        Ok(serde_json::json!({ "value": seconds }))
    }
}

/// 生成外部访问令牌。
///
/// 用两个 UUID v4 拼接：122 位随机量对局域网场景足够，且不需要引入新的随机源依赖。
/// 令牌只在生成时返回一次，库里只存它的 SHA-256。
fn generate_external_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// 把仓储的日志条目映射成线上形状。
fn log_entry_value(entry: &LogEntry) -> Value {
    serde_json::json!({
        "requestId": entry.request_id,
        "createdAt": entry.created_at,
        "vendorId": entry.vendor_id,
        "providerId": entry.provider_id,
        "outcome": entry.outcome,
        "upstreamStatus": entry.upstream_status,
        // **没转发的请求也要能看出来。**
        //
        // 「路由不存在 / 鉴权失败」记的是 `outcome: ok`（那不是供应商的错），
        // 于是它与一次真正转发成功的记录只差这一个字段。少了它，
        // 查「CLI 为什么失败」的人在日志里只能看到一串 `ok`。
        "errorCode": entry.error_code,
    })
}
/// 解析用量区间（必填）。
fn period(args: &Value) -> Result<(Timestamp, Timestamp)> {
    let start = args
        .get("periodStart")
        .and_then(timestamp)
        .ok_or_else(|| invalid("periodStart"))?;
    let end = args
        .get("periodEnd")
        .and_then(timestamp)
        .ok_or_else(|| invalid("periodEnd"))?;
    Ok((start, end))
}

/// 解析日志筛选条件（全部可选）。
fn filter(args: &Value) -> LogFilter {
    LogFilter {
        vendor_id: args
            .get("vendorId")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(VendorId::from),
        // 未知取值表示「不限结果」，而不是「按失败筛选」：筛选条件不该悄悄收紧。
        // 因此这里显式列举，不用有损解析（后者会把未知值回退成 `Error`）。
        outcome: match args.get("outcome").and_then(Value::as_str) {
            Some("ok") => Some(GatewayOutcome::Ok),
            Some("error") => Some(GatewayOutcome::Error),
            Some("timeout") => Some(GatewayOutcome::Timeout),
            Some("client-aborted") => Some(GatewayOutcome::ClientAborted),
            _ => None,
        },
        period_start: args.get("periodStart").and_then(timestamp),
        period_end: args.get("periodEnd").and_then(timestamp),
    }
}

/// 解析时间戳（接受 ISO 字符串或 Unix 毫秒整数）。
fn timestamp(value: &Value) -> Option<Timestamp> {
    match value {
        Value::String(text) if !text.trim().is_empty() => Some(Timestamp::from_raw(text)),
        Value::Number(number) => number.as_i64().map(Timestamp::from_unix_millis),
        _ => None,
    }
}

/// 参数非法。
fn invalid(name: &str) -> aivc_core::Error {
    aivc_core::error::IpcError::InvalidParameter {
        name: name.to_owned(),
    }
    .into()
}

/// 序列化失败。
fn serialize_error(error: &serde_json::Error) -> aivc_core::Error {
    aivc_core::error::IpcError::CommandFailed {
        message: format!("序列化返回值失败：{error}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::gateway::FailoverRuleScope;
    use aivc_core::target::Platform;
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
    use aivc_provider::ProviderRepositories;
    use serde_json::json;
    use std::sync::Arc;

    async fn context() -> (ProviderContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let wsl = WslAdapter::new(
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
            WslConfig {
                use_worker: false,
                ..WslConfig::default()
            },
        );
        let ctx = ProviderContext::new(
            paths,
            SettingsStore::new(dir.path().join("runtime/settings.json")),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );
        (ctx, dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vendor_health_starts_empty_and_can_be_reset() {
        let (ctx, _dir) = context().await;
        assert!(
            GatewayHealth::get_vendor_health(&ctx)
                .await
                .expect("list")
                .is_empty()
        );
        // 重置全部在空表上返回 0，不报错。
        assert_eq!(
            GatewayHealth::reset_vendor_health(&ctx, &json!({}))
                .await
                .expect("reset"),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failover_rules_round_trip_through_the_command_surface() {
        let (ctx, _dir) = context().await;
        let saved = GatewayRules::save_failover_rule(
            &ctx,
            &json!({
                "scope": "global",
                "pattern": "insufficient quota",
                "statusCodes": [429],
                "enabled": true
            }),
        )
        .await
        .expect("save");
        assert_eq!(saved.scope, FailoverRuleScope::Global);

        let rules = GatewayRules::list_failover_rules(&ctx).await.expect("list");
        assert_eq!(rules.len(), 1);

        assert!(
            GatewayRules::set_failover_rule_enabled(
                &ctx,
                &json!({ "id": saved.id, "enabled": false })
            )
            .await
            .expect("toggle")
        );
        let rules = GatewayRules::list_failover_rules(&ctx).await.expect("list");
        assert!(!rules[0].enabled);

        assert!(
            GatewayRules::delete_failover_rule(&ctx, &json!({ "id": saved.id }))
                .await
                .expect("delete")
        );
        assert!(
            GatewayRules::list_failover_rules(&ctx)
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_invalid_rule_scope_is_rejected() {
        // 作用域校验在领域层，命令面只负责把参数搬过去。
        let (ctx, _dir) = context().await;
        let error = GatewayRules::save_failover_rule(&ctx, &json!({ "scope": "nope" }))
            .await
            .expect_err("必须拒绝");
        assert!(
            error.to_string().contains("参数无效") || error.to_string().contains("作用域"),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn usage_queries_require_an_explicit_period() {
        // 缺省区间会让聚合查询扫全表，因此必须显式给出。
        let (ctx, _dir) = context().await;
        let error = GatewayLogs::get_usage_summary(&ctx, &json!({}))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：periodStart");

        let error = GatewayLogs::get_usage_report(&ctx, &json!({}))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：periodStart");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn usage_queries_accept_iso_periods() {
        let (ctx, _dir) = context().await;
        let args = json!({
            "periodStart": "2026-01-01T00:00:00.000Z",
            "periodEnd": "2026-12-31T23:59:59.999Z"
        });
        let summary = GatewayLogs::get_usage_summary(&ctx, &args)
            .await
            .expect("summary");
        assert_eq!(summary.request_count, 0);
        let report = GatewayLogs::get_usage_report(&ctx, &args)
            .await
            .expect("report");
        assert!(report.vendors.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn diagnostics_paging_requires_a_page_and_size() {
        let (ctx, _dir) = context().await;
        let error = GatewayLogs::get_failure_diagnostics(&ctx, &json!({ "pageSize": 10 }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：page");

        let error = GatewayLogs::get_failure_diagnostics(&ctx, &json!({ "page": 1 }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：pageSize");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn diagnostics_paging_clamps_the_page_size() {
        // 每页上限由仓储钳制，命令面不重复实现，但行为要可观测。
        let (ctx, _dir) = context().await;
        let page =
            GatewayLogs::get_failure_diagnostics(&ctx, &json!({ "page": 1, "pageSize": 500 }))
                .await
                .expect("page");
        assert!(page.page_size <= DIAGNOSTICS_PAGE_SIZE_MAX);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn log_queries_return_an_array_on_an_empty_database() {
        let (ctx, _dir) = context().await;
        let entries = GatewayLogs::query_logs(&ctx, &json!({}))
            .await
            .expect("query");
        assert!(entries.as_array().expect("数组").is_empty());

        let recent = GatewayLogs::get_recent_failures(&ctx, &json!({}))
            .await
            .expect("recent");
        assert!(recent.as_array().expect("数组").is_empty());

        assert_eq!(
            GatewayLogs::clear_logs(&ctx, &json!({}))
                .await
                .expect("clear"),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_log_entries_requires_an_array() {
        let (ctx, _dir) = context().await;
        let error = GatewayLogs::delete_log_entries(&ctx, &json!({ "requestIds": "nope" }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：requestIds");
        // 空数组是合法的。
        assert_eq!(
            GatewayLogs::delete_log_entries(&ctx, &json!({ "requestIds": [] }))
                .await
                .expect("delete"),
            0
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gateway_port_round_trips_and_zero_means_auto() {
        let (ctx, _dir) = context().await;
        // 端口 0 表示自动分配，不是非法值。
        let value = GatewaySettingsCommands::set_port(&ctx, &json!({ "port": 0 }))
            .await
            .expect("set");
        assert_eq!(value["port"], 0);
        assert_eq!(
            GatewaySettingsCommands::get_port(&ctx).await.expect("get")["port"],
            0
        );

        let error = GatewaySettingsCommands::set_port(&ctx, &json!({ "port": 70000 }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn external_api_status_never_exposes_the_token() {
        let (ctx, _dir) = context().await;
        let status = GatewaySettingsCommands::get_external_api_status(&ctx)
            .await
            .expect("status");
        // 只暴露「是否存在令牌」，绝不返回令牌本身。
        assert!(status.get("token").is_none());
        assert_eq!(status["hasToken"], false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resilience_settings_enforce_their_ranges() {
        let (ctx, _dir) = context().await;
        assert_eq!(
            GatewaySettingsCommands::set_failure_threshold(&ctx, &json!({ "value": 3 }))
                .await
                .expect("threshold")["value"],
            3
        );
        let error = GatewaySettingsCommands::set_failure_threshold(&ctx, &json!({ "value": 0 }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "异常切换阈值必须是 1 到 10 之间的整数。");

        let error =
            GatewaySettingsCommands::set_circuit_duration_seconds(&ctx, &json!({ "value": 5 }))
                .await
                .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "熔断持续时间必须是 10 到 86400 秒之间的整数。"
        );
    }

    #[test]
    fn timestamps_accept_both_strings_and_millis() {
        assert!(timestamp(&json!("2026-01-01T00:00:00.000Z")).is_some());
        assert!(timestamp(&json!(1_767_225_600_000i64)).is_some());
        // 空串与浮点都不是合法时间戳。
        assert!(timestamp(&json!("")).is_none());
        assert!(timestamp(&json!(1.5)).is_none());
        assert!(timestamp(&json!(null)).is_none());
    }
}
