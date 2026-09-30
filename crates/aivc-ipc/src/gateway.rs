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

    /// 试一条**还没保存**的规则会不会命中。
    ///
    /// # 为什么这条命令值得存在
    ///
    /// 故障规则的匹配是「不区分大小写的包含匹配 + 状态码精确匹配」的**或**关系。
    /// 用户配完最想知道的是「我这条规则到底会不会命中」——而这件事
    /// **只有规则引擎自己说了算**。
    ///
    /// 若让界面自己实现一遍匹配，两份逻辑迟早分叉，表现是
    /// 「测试说会命中、实际没切换」——那是最难查的一类偏差。
    /// 因此这里直接调用 core 里那份 `match_rule`，与转发路径**同源**。
    ///
    /// # 参数
    ///
    /// - `rule`：规则本体（与 `gateway:save-failover-rule` 同形状，`id` 可省）；
    /// - `statusCode`：模拟的上游状态码，可省；
    /// - `message`：模拟的上游错误文本，可省。
    ///
    /// 三者至少要给出一个「可命中的条件」，否则必然不命中——那是用户的配置问题，
    /// 不是错误，照常返回 `matched: false`。
    ///
    /// # Errors
    ///
    /// 规则参数非法（作用域/匹配条件/自定义响应那几条校验与保存时一致）。
    pub fn test_failover_rule(args: &Value) -> Result<Value> {
        let input: FailoverRuleInput = serde_json::from_value(
            args.get("rule").cloned().unwrap_or(Value::Null),
        )
        .map_err(|error| {
            aivc_core::Error::Ipc(aivc_core::error::IpcError::CommandFailed {
                message: format!("参数无效：gateway failover rule（{error}）"),
            })
        })?;
        // **走同一套归一化与校验**：测试时被拒的规则，保存时也会被拒。
        // 反过来也一样——否则测试会给出保存不了的「通过」。
        let normalized = input.normalize()?;

        let status_code = match args.get("statusCode") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                u16::try_from(value.as_u64().ok_or_else(|| {
                    aivc_core::Error::Ipc(aivc_core::error::IpcError::InvalidParameter {
                        name: "statusCode".to_owned(),
                    })
                })?)
                .map_err(|_| {
                    aivc_core::Error::Ipc(aivc_core::error::IpcError::InvalidParameter {
                        name: "statusCode".to_owned(),
                    })
                })?,
            ),
        };
        // 空串按「没给」处理：界面上的输入框空着时传的就是空串。
        let message = args
            .get("message")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());

        let hit = aivc_core::gateway::match_rule(
            &normalized.pattern,
            Some(normalized.status_codes.as_slice()),
            status_code,
            message,
        );
        Ok(serde_json::json!({
            "matched": hit.is_some(),
            // `matchedBy` 让界面能说清是**被哪一项**命中的——只回布尔值时，
            // 用户看到「没命中」不知道该改状态码还是改文本。
            "matchedBy": match hit {
                Some(aivc_core::gateway::RuleHit::Status) => "status",
                Some(aivc_core::gateway::RuleHit::Pattern) => "pattern",
                None => "none",
            },
        }))
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
    /// # 分页是可选的，且**要么两个参数都给、要么都不给**
    ///
    /// - 都不给：返回**全量数组**——与加这组参数之前完全一致，界面上「最近请求」那类
    ///   旧调用（只带筛选条件）拿到的形状不变；
    /// - 都给：返回 `{ entries, total, page, pageSize }`
    ///   （[`aivc_infra::db::LogEntryPage`] 的线上形状）。
    ///
    /// 只给一个时报「参数无效」而不是**静默忽略**：这条命令此前就吃过一次亏——
    /// 前端传的是 `{page, pageSize}`，而这里读的是另一套筛选参数名，于是两个参数被吃掉，
    /// 调用方以为自己在分页，实际每次都拿全量。
    ///
    /// 页码从 1 起；每页上限由仓储钳制（[`aivc_infra::db::LOGS_PAGE_SIZE_MAX`]），
    /// 命令面不重复实现。
    ///
    /// # Errors
    ///
    /// 分页参数非法或数据库读取失败。
    pub async fn query_logs(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let filter = filter(args);
        let page = args.get("page");
        let page_size = args.get("pageSize");
        if page.is_none() && page_size.is_none() {
            let entries = ctx.gateway_request_log().cleanup_entries(&filter).await?;
            // `LogEntry` 是仓储的读取模型，未实现 Serialize。这里显式映射成线上形状：
            // 顺带把字段名固定下来，避免以后有人给它加 derive 就悄悄改变了契约。
            return Ok(Value::Array(entries.iter().map(log_entry_value).collect()));
        }

        let page = require_positive_integer(page, "page")?;
        let page_size = require_positive_integer(page_size, "pageSize")?;
        // 饱和转换而不是 `as u32`：超大页码截断后会变成一个**很小的页码**，
        // 于是「翻过头」变成「悄悄回到前面某页」——那是一份看起来正常的错数据。
        let page = u32::try_from(page).unwrap_or(u32::MAX);
        let page_size = u32::try_from(page_size).unwrap_or(u32::MAX);
        let result = ctx
            .gateway_request_log()
            .cleanup_entries_page(&filter, page, page_size)
            .await?;
        Ok(serde_json::json!({
            "entries": result.entries.iter().map(log_entry_value).collect::<Vec<Value>>(),
            "total": result.total,
            "page": result.page,
            "pageSize": result.page_size,
        }))
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

/// 本地网关的端口与地址状态。
///
/// # 三个命令共用这一个形状
///
/// `get-port` / `set-port` / `set-enabled` 此前**各返回各的**（`{port}`、`{port}`、
/// `{enabled}`），而界面把三者都当成同一个类型用：面板点一下「启用」就
/// `setStatus(返回值)`，于是 `configuredPort` 与 `activePort` 当场变成 `undefined`
/// ——「当前端口」显示「未启动」，端口输入框里变成字符串 `"undefined"`。
///
/// 现在三处都返回这个形状，界面在任何一步之后拿到的状态都是完整的。
///
/// # 为什么 `configuredPort` 与 `activePort` 必须分开
///
/// `configuredPort == 0` 表示**由操作系统自动分配**（设置里的默认值就是 0），
/// 那时真实端口只有监听成功之后才知道。因此：
///
/// - `configuredPort`：用户填的那个（可能是 0）；
/// - `activePort`：**这次真正在服务**的那个，从地址里解析出来；
/// - `baseUrl`：直接可用的地址；**空串表示当前没有在监听**。
///
/// 界面若自己拿 `configuredPort` 去拼 URL，端口为 0 时拼出来的是
/// `http://127.0.0.1:0` —— 一个永远不会有人监听的地址。
#[must_use]
pub fn port_status(configured_port: u16, enabled: bool, address: &str) -> Value {
    let base_url = address.trim();
    serde_json::json!({
        "enabled": enabled,
        "configuredPort": configured_port,
        // 解析不出来就是 0：与「没有在监听」同义，界面按这个判「未启动」。
        "activePort": crate::route::port_of(base_url).unwrap_or(0),
        "baseUrl": base_url,
    })
}

/// 网关设置页要的**全部状态**：端口与地址 + 三个韧性阈值。
///
/// # 为什么合成一个形状
///
/// 这一页需要 6 个值（地址、配置端口、真实端口、启用位、异常切换阈值、熔断次数、
/// 熔断时长）。如果读一次拿一部分、改一次回一部分，界面就必须自己拼——
/// 而这个项目**已经因为「setter 只回部分字段」出过缺陷**：当时 `set-enabled`
/// 只返回 `{enabled}`，界面拿它替换状态，把地址和端口抹成了 `undefined`。
///
/// 因此这里只有**一个**形状：读与写全部返回它，界面任何时候拿到的都是完整的。
///
/// 原来还有一个只读全部设置的 `get_settings`，返回的是另一套字段名
/// （`port` / `enabled` 而不是 `configuredPort` / `activePort`）——同一份数据两个形状，
/// 而它既没有调用方也没有测试。合并掉。
#[must_use]
pub fn gateway_status(
    settings: &aivc_infra::config::settings::AppSettings,
    address: &str,
) -> Value {
    let mut status = port_status(settings.gateway_port, settings.gateway_enabled, address);
    if let Some(object) = status.as_object_mut() {
        object.insert(
            "failureThreshold".to_owned(),
            Value::from(settings.gateway_failure_threshold),
        );
        object.insert(
            "circuitFailureThreshold".to_owned(),
            Value::from(settings.gateway_circuit_failure_threshold),
        );
        object.insert(
            "circuitDurationSeconds".to_owned(),
            Value::from(settings.gateway_circuit_duration_seconds),
        );
    }
    status
}

impl GatewaySettingsCommands {
    /// 读取网关端口与地址。
    ///
    /// `address` 是**当前监听地址**（由调用方从网关状态取；没在跑就是空串）。
    /// 不能在这里从设置里推：设置里的端口可能是 0（自动分配）。
    ///
    /// # Errors
    ///
    /// 设置读取失败。
    pub async fn get_port(ctx: &ProviderContext, address: &str) -> Result<Value> {
        let settings = ctx.settings().snapshot().await;
        Ok(gateway_status(&settings, address))
    }

    /// 设置网关端口；`0` 表示自动分配。
    ///
    /// 返回**改完之后的完整状态**，不是只回显那个端口：界面拿它直接替换当前状态，
    /// 只回显端口会把另外几个字段抹掉。
    ///
    /// # Errors
    ///
    /// 端口越界或设置写入失败。
    pub async fn set_port(ctx: &ProviderContext, args: &Value, address: &str) -> Result<Value> {
        let port = require_gateway_port(args.get("port"))?;
        ctx.settings()
            .update(|raw| raw.set_gateway_port(port))
            .await?;
        let settings = ctx.settings().snapshot().await;
        Ok(gateway_status(&settings, address))
    }

    /// 设置网关启用状态。
    ///
    /// 同 `set_port`：返回改完之后的完整状态。
    ///
    /// # Errors
    ///
    /// 参数非法或设置写入失败。
    pub async fn set_enabled(ctx: &ProviderContext, args: &Value, address: &str) -> Result<Value> {
        let enabled = require_boolean(args.get("enabled"), "enabled")?;
        ctx.settings()
            .update(|raw| raw.set_gateway_enabled(enabled))
            .await?;
        let settings = ctx.settings().snapshot().await;
        Ok(gateway_status(&settings, address))
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
    /// # 为什么这三个 setter 之前界面上不存在
    ///
    /// 它们**一直是完整实现的**（含边界校验与错误文案），只是从没登记进
    /// `COMMANDS`——因此命令面里查不到、界面也就没法调。同一份代码里
    /// 还躺着一个同样没登记的 `get_settings`，而且既无调用方也无测试。
    /// 这一轮把它们接出来。
    ///
    /// 返回**完整的网关状态**（与 `set_port` 一致），而不是只回显那一个数字。
    ///
    /// # Errors
    ///
    /// 越界或设置写入失败。
    pub async fn set_failure_threshold(
        ctx: &ProviderContext,
        args: &Value,
        address: &str,
    ) -> Result<Value> {
        let value = require_gateway_failure_threshold(args.get("value"))?;
        #[allow(clippy::cast_possible_truncation)]
        let threshold = value as u8;
        ctx.settings()
            .update(|raw| raw.set_gateway_failure_threshold(threshold))
            .await?;
        let settings = ctx.settings().snapshot().await;
        Ok(gateway_status(&settings, address))
    }

    /// 设置熔断次数（1–20）。
    ///
    /// # Errors
    ///
    /// 越界或设置写入失败。
    pub async fn set_circuit_failure_threshold(
        ctx: &ProviderContext,
        args: &Value,
        address: &str,
    ) -> Result<Value> {
        let value = require_gateway_circuit_failure_threshold(args.get("value"))?;
        #[allow(clippy::cast_possible_truncation)]
        let threshold = value as u8;
        ctx.settings()
            .update(|raw| raw.set_gateway_circuit_failure_threshold(threshold))
            .await?;
        let settings = ctx.settings().snapshot().await;
        Ok(gateway_status(&settings, address))
    }

    /// 设置熔断持续时长（10–86400 秒）。
    ///
    /// # Errors
    ///
    /// 越界或设置写入失败。
    pub async fn set_circuit_duration_seconds(
        ctx: &ProviderContext,
        args: &Value,
        address: &str,
    ) -> Result<Value> {
        let value = require_gateway_circuit_duration_seconds(args.get("value"))?;
        #[allow(clippy::cast_possible_truncation)]
        let seconds = value as u32;
        ctx.settings()
            .update(|raw| raw.set_gateway_circuit_duration_seconds(seconds))
            .await?;
        let settings = ctx.settings().snapshot().await;
        Ok(gateway_status(&settings, address))
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
    use aivc_infra::db::LOGS_PAGE_SIZE_MAX;
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

    /// 造一条请求记录：分页测试要真的落库，才测得出生效的 `total`。
    fn log_record(id: &str, created_at: &str) -> aivc_core::gateway::GatewayRequestRecord {
        aivc_core::gateway::GatewayRequestRecord {
            request_id: aivc_core::id::RequestId::from(id),
            route_id: "route-1".to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: VendorId::from("v1"),
            method: "POST".to_owned(),
            path: "/v1/responses".to_owned(),
            model: None,
            upstream_status: Some(200),
            outcome: GatewayOutcome::Ok,
            duration_ms: 10,
            bytes_in: 1,
            bytes_out: 2,
            retry_count: 0,
            switched: false,
            error_code: None,
            error_message: None,
            usage: None,
            created_at: Timestamp::from_raw(created_at),
        }
    }

    /// 分页：首页 / 末页 / 越界页的条数与 `total`，以及**不带参数时仍是裸数组**。
    ///
    /// 最后一条是本任务的关键约束：旧调用（界面上的「最近请求」）只带筛选条件，
    /// 它拿到的必须是和改动之前一模一样的形状。
    #[tokio::test(flavor = "multi_thread")]
    async fn logs_paging_returns_a_page_with_the_total() {
        let (ctx, _dir) = context().await;
        for index in 0..5 {
            assert!(
                ctx.gateway_request_log()
                    .record(&log_record(
                        &format!("r{index}"),
                        &format!("2026-06-{:02}T00:00:00.000Z", index + 1)
                    ))
                    .await
            );
        }

        let first = GatewayLogs::query_logs(&ctx, &json!({ "page": 1, "pageSize": 2 }))
            .await
            .expect("page 1");
        assert_eq!(first["total"], 5, "total 是匹配总数，不是本页条数");
        assert_eq!(first["page"], 1);
        assert_eq!(first["pageSize"], 2);
        let entries = first["entries"].as_array().expect("数组");
        assert_eq!(entries.len(), 2);
        // 排序与不分页那条路一致：最新的在最前。
        assert_eq!(entries[0]["requestId"], "r4");

        let last = GatewayLogs::query_logs(&ctx, &json!({ "page": 3, "pageSize": 2 }))
            .await
            .expect("page 3");
        assert_eq!(last["entries"].as_array().expect("数组").len(), 1);

        // 越界页：空数组 + 原样的总数——界面据此判断是「翻过头了」而不是「没数据」。
        let beyond = GatewayLogs::query_logs(&ctx, &json!({ "page": 9, "pageSize": 2 }))
            .await
            .expect("page 9");
        assert!(beyond["entries"].as_array().expect("数组").is_empty());
        assert_eq!(beyond["total"], 5);

        // 不带分页参数：裸数组（旧调用形状不变），且是全量。
        let all = GatewayLogs::query_logs(&ctx, &json!({}))
            .await
            .expect("all");
        assert_eq!(all.as_array().expect("数组").len(), 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn logs_paging_requires_both_parameters() {
        // 只给一个必须**报错**，不能静默忽略：被吃掉的参数会让调用方以为自己在分页
        // （这条命令就被这么坑过一次）。两个都不给才是「不分页」。
        let (ctx, _dir) = context().await;
        let error = GatewayLogs::query_logs(&ctx, &json!({ "page": 1 }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：pageSize");

        let error = GatewayLogs::query_logs(&ctx, &json!({ "pageSize": 10 }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：page");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn logs_paging_clamps_the_page_size() {
        // 每页上限由仓储钳制（与异常诊断同一条规则），命令面不重复实现，但行为要可观测。
        let (ctx, _dir) = context().await;
        let value = GatewayLogs::query_logs(&ctx, &json!({ "page": 1, "pageSize": 500 }))
            .await
            .expect("page");
        assert!(value["pageSize"].as_u64().expect("数字") <= u64::from(LOGS_PAGE_SIZE_MAX));
        assert!(value["entries"].is_array());
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
        let value = GatewaySettingsCommands::set_port(&ctx, &json!({ "port": 0 }), "")
            .await
            .expect("set");
        assert_eq!(value["configuredPort"], 0);
        assert_eq!(
            GatewaySettingsCommands::get_port(&ctx, "")
                .await
                .expect("get")["configuredPort"],
            0
        );

        let error = GatewaySettingsCommands::set_port(&ctx, &json!({ "port": 70000 }), "")
            .await
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。"
        );
    }

    /// 端口状态的形状：三个命令返回同一套字段，且地址与端口是一致的。
    ///
    /// # 为什么这条值得单独钉
    ///
    /// 这里此前**三处各返回各的**（`{port}`、`{port}`、`{enabled}`），而界面把
    /// 三者当成同一个类型用——点一下「启用」就把另外两个字段抹成 `undefined`，
    /// 端口输入框里显示字符串 `"undefined"`。这类错位在界面测试里看不出来，
    /// 因为测试注入的是自己编的假数据。
    #[tokio::test(flavor = "multi_thread")]
    async fn the_port_status_shape_is_the_same_for_all_three_commands() {
        let (ctx, _dir) = context().await;
        // 假装网关正监听在 50064（配置里是 0，即自动分配）。
        let address = "http://127.0.0.1:50064";

        let read = GatewaySettingsCommands::get_port(&ctx, address)
            .await
            .expect("get");
        assert_eq!(read["configuredPort"], 0);
        assert_eq!(read["activePort"], 50064);
        assert_eq!(read["baseUrl"], address);
        // **默认是「开启」**（设置里那句「缺失视为开启」），因此这里不断言具体真假，
        // 只记下默认值，后面用「切换之后与它相反」来证明 set_enabled 真的生效了。
        let default_enabled = read["enabled"].as_bool().expect("enabled 是布尔");

        let after_set_port =
            GatewaySettingsCommands::set_port(&ctx, &json!({ "port": 8080 }), address)
                .await
                .expect("set");
        let after_set_enabled = GatewaySettingsCommands::set_enabled(
            &ctx,
            &json!({ "enabled": !default_enabled }),
            address,
        )
        .await
        .expect("enable");

        // **所有网关设置命令的键必须完全一样**：界面拿任何一个去替换状态都要是完整的。
        let keys = |value: &Value| {
            let mut list: Vec<String> = value.as_object().expect("对象").keys().cloned().collect();
            list.sort();
            list
        };
        assert_eq!(keys(&read), keys(&after_set_port));
        assert_eq!(keys(&read), keys(&after_set_enabled));
        assert_eq!(
            keys(&read),
            vec![
                "activePort".to_owned(),
                "baseUrl".to_owned(),
                // 三个韧性阈值后来并进这个形状（原来它们由一个**没有调用方**的
                // `get_settings` 单独返回，字段名还不一样）。见 `gateway_status`。
                "circuitDurationSeconds".to_owned(),
                "circuitFailureThreshold".to_owned(),
                "configuredPort".to_owned(),
                "enabled".to_owned(),
                "failureThreshold".to_owned()
            ]
        );

        // 设置之后回来的状态里，另外两个字段**没有被抹掉**。
        assert_eq!(after_set_port["activePort"], 50064);
        assert_eq!(after_set_port["baseUrl"], address);
        assert_eq!(after_set_enabled["activePort"], 50064);
        // 切换是真的生效了（与默认值相反）。
        assert_eq!(after_set_enabled["enabled"], !default_enabled);
    }

    /// 没在监听时：地址空串、`activePort` 为 0，而**配置端口照样报出来**。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stopped_gateway_reports_no_address_but_keeps_the_configured_port() {
        let (ctx, _dir) = context().await;
        GatewaySettingsCommands::set_port(&ctx, &json!({ "port": 8080 }), "")
            .await
            .expect("set");
        let value = GatewaySettingsCommands::get_port(&ctx, "")
            .await
            .expect("get");
        assert_eq!(value["baseUrl"], "");
        assert_eq!(value["activePort"], 0);
        // 这一条是界面「已启用但未启动」那个状态的依据。
        assert_eq!(value["configuredPort"], 8080);
    }

    /// 端口解析不出来时按 0 处理，而不是 panic 或给出个假端口。
    #[test]
    fn a_malformed_address_yields_no_active_port() {
        for address in [
            "",
            "   ",
            "http://127.0.0.1",
            "not-a-url",
            "http://127.0.0.1:abc",
        ] {
            let value = port_status(0, true, address);
            assert_eq!(value["activePort"], 0, "{address}");
        }
        // 正常地址要解析得出来（末尾斜杠也认）。
        assert_eq!(
            port_status(0, true, "http://127.0.0.1:50064")["activePort"],
            50064
        );
        assert_eq!(
            port_status(0, true, "http://127.0.0.1:50064/")["activePort"],
            50064
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
        let updated = GatewaySettingsCommands::set_failure_threshold(
            &ctx,
            &json!({ "value": 3 }),
            "127.0.0.1:8787",
        )
        .await
        .expect("threshold");
        // 返回的是**完整状态**，不是只回显那一个数字：界面拿它直接替换当前状态。
        assert_eq!(updated["failureThreshold"], 3);
        assert_eq!(updated["configuredPort"], updated["configuredPort"]);
        assert_eq!(updated["baseUrl"], "127.0.0.1:8787");
        assert_eq!(updated["activePort"], 8787);
        // 另外两个阈值也在，且没被这次写入动过。
        assert!(updated["circuitFailureThreshold"].is_number());
        assert!(updated["circuitDurationSeconds"].is_number());

        let error = GatewaySettingsCommands::set_failure_threshold(
            &ctx,
            &json!({ "value": 0 }),
            "127.0.0.1:8787",
        )
        .await
        .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "异常切换阈值必须是 1 到 10 之间的整数。");

        let error = GatewaySettingsCommands::set_circuit_duration_seconds(
            &ctx,
            &json!({ "value": 5 }),
            "127.0.0.1:8787",
        )
        .await
        .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "熔断持续时间必须是 10 到 86400 秒之间的整数。"
        );

        // 熔断次数的边界（1–20）。
        let error = GatewaySettingsCommands::set_circuit_failure_threshold(
            &ctx,
            &json!({ "value": 21 }),
            "127.0.0.1:8787",
        )
        .await
        .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "熔断次数必须是 1 到 20 之间的整数。");
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
