//! 基于仓储的网关依赖来源。
//!
//! # 它把三件事接起来
//!
//! | 网关需要 | 来自 |
//! | --- | --- |
//! | 供应商候选池 | `VendorRepo` |
//! | 故障识别规则 | `FailoverRuleRepo` |
//! | 请求记录落库 | `GatewayRequestLogRepo` |
//! | 运行时设置 | `SettingsStore` |
//!
//! # 设置为什么要缓存
//!
//! [`GatewaySource::settings`] 是同步的，而 `SettingsStore::snapshot()` 是异步的
//! （要读文件、做逐字段容错解析）。若每个请求都异步读一次设置，等于把磁盘 I/O 放进
//! 转发热路径。
//!
//! 因此设置以快照形式缓存：启动时读一次，设置变更后由上层调用 [`RepositorySource::refresh_settings`]
//! 刷新。网关读到的可能短暂滞后——这是可接受的，因为网关设置（端口、阈值）本来就不该
//! 在转发中途变化。

use std::sync::Arc;

use arc_swap::ArcSwap;

use aivc_core::error::Result;
use aivc_core::gateway::{FailoverRule, GatewayAttempt, GatewayRequestRecord};
use aivc_core::ports::BoxFuture;
use aivc_core::vendor::ApiVendor;
use aivc_infra::config::SettingsStore;
use aivc_infra::db::{FailoverRuleRepo, GatewayRequestLogRepo, VendorRepo};

use crate::failover::CircuitConfig;
use crate::server::{GatewaySettings, GatewaySource};

/// 仓储支撑的网关来源。
/// 请求记录写库后的回调。
///
/// 用回调而不是让网关直接依赖事件机制：网关是第 2 层，而事件要送到第 4 层的窗口；
/// 让低层依赖高层会破坏分层。回调由组装根注入，网关只管「记完通知一声」。
///
/// **回调抛错不影响记录**：事件送不到不该让已经完成的转发变成失败。
pub type RecordHook = Arc<dyn Fn(&GatewayRequestRecord) + Send + Sync>;

/// 基于仓储的网关数据来源。
pub struct RepositorySource {
    vendor: VendorRepo,
    /// 记录钩子；`None` 表示不需要通知（测试与无界面场合）。
    record_hook: Option<RecordHook>,
    failover_rule: FailoverRuleRepo,
    request_log: GatewayRequestLogRepo,
    settings_store: SettingsStore,
    settings: ArcSwap<GatewaySettings>,
}

impl std::fmt::Debug for RepositorySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepositorySource").finish_non_exhaustive()
    }
}

impl RepositorySource {
    /// 构造并立即读入一次设置快照。
    ///
    /// # Errors
    ///
    /// 设置读取失败时返回错误。启动阶段失败应当显式暴露——带着错误的端口与阈值启动
    /// 比拒绝启动更难排查。
    pub async fn new(
        vendor: VendorRepo,
        failover_rule: FailoverRuleRepo,
        request_log: GatewayRequestLogRepo,
        settings_store: SettingsStore,
    ) -> Result<Self> {
        let settings = read_settings(&settings_store).await;
        Ok(Self {
            vendor,
            failover_rule,
            request_log,
            settings_store,
            settings: ArcSwap::from_pointee(settings),
            // 默认不通知：无界面场合（测试、CLI）不需要事件。
            record_hook: None,
        })
    }

    /// 装上请求记录钩子。
    ///
    /// 返回 `self` 以便链式构造。钩子在记录**写库成功或失败之后**都会被调用——
    /// 界面需要看到这次请求，而记录失败是另一回事（已经记了日志）。
    #[must_use]
    pub fn with_record_hook(mut self, hook: RecordHook) -> Self {
        self.record_hook = Some(hook);
        self
    }

    /// 重新读取设置快照。
    ///
    /// 设置变更后由上层调用；不必每请求刷新，见模块文档。
    pub async fn refresh_settings(&self) {
        let next = read_settings(&self.settings_store).await;
        self.settings.store(Arc::new(next));
    }
}

/// 由设置存储读出网关设置快照。
async fn read_settings(store: &SettingsStore) -> GatewaySettings {
    let raw = store.snapshot().await;
    GatewaySettings {
        enabled: raw.gateway_enabled,
        port: raw.gateway_port,
        failure_threshold: usize::from(raw.gateway_failure_threshold),
        circuit: CircuitConfig {
            failure_threshold: usize::from(raw.gateway_circuit_failure_threshold),
            duration: std::time::Duration::from_secs(u64::from(
                raw.gateway_circuit_duration_seconds,
            )),
        },
        external_enabled: raw.gateway_external_api_enabled,
        // 空串表示「尚未生成令牌」，而不是「令牌是空串」。
        external_token_hash: (!raw.gateway_external_api_token_hash.is_empty())
            .then_some(raw.gateway_external_api_token_hash),
        upstream_timeout: std::time::Duration::from_millis(crate::proxy::upstream_timeout_ms()),
        request_timeout: std::time::Duration::from_millis(crate::proxy::upstream_timeout_ms()),
    }
}

impl GatewaySource for RepositorySource {
    fn vendors(&self) -> BoxFuture<'_, Vec<ApiVendor>> {
        Box::pin(async move {
            match self.vendor.list(None).await {
                Ok(vendors) => vendors,
                Err(error) => {
                    // 供应商列表读不出来时返回空池：网关会给出「没有可用供应商」，
                    // 比让整个请求失败在数据库层更容易理解。
                    tracing::warn!(%error, "读取供应商列表失败");
                    Vec::new()
                }
            }
        })
    }

    fn failover_rules(&self) -> BoxFuture<'_, Vec<FailoverRule>> {
        Box::pin(async move {
            match self.failover_rule.list().await {
                Ok(rules) => rules,
                Err(error) => {
                    // 规则读不出来时退回内置判定（空规则集），而不是让请求失败。
                    tracing::warn!(%error, "读取故障规则失败");
                    Vec::new()
                }
            }
        })
    }

    fn settings(&self) -> GatewaySettings {
        self.settings.load().as_ref().clone()
    }

    fn refresh_settings(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let next = read_settings(&self.settings_store).await;
            self.settings.store(Arc::new(next));
        })
    }

    fn record(&self, record: GatewayRequestRecord) -> BoxFuture<'_, ()> {
        self.record_with_attempts(record, Vec::new())
    }

    fn record_with_attempts(
        &self,
        record: GatewayRequestRecord,
        attempts: Vec<GatewayAttempt>,
    ) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // 记录失败不影响已经完成的转发，因此只记日志。
            if !self
                .request_log
                .record_with_attempts(&record, &attempts)
                .await
            {
                tracing::warn!(
                    request_id = record.request_id.as_str(),
                    "写入网关请求记录失败"
                );
            }
            // 通知界面：工作台靠这个事件实时更新，**不轮询**。
            if let Some(hook) = &self.record_hook {
                hook(&record);
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::gateway::{GatewayOutcome, GatewayUsage};
    use aivc_core::id::{RequestId, VendorId};
    use aivc_core::target::ProviderId;
    use aivc_core::time::Timestamp;
    use aivc_core::vendor::ApiVendorInput;
    use aivc_infra::config::AppPaths;
    use aivc_infra::db::Db;

    async fn source() -> (RepositorySource, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let store = SettingsStore::new(dir.path().join("runtime/settings.json"));
        let source = RepositorySource::new(
            VendorRepo::new(db.clone()),
            FailoverRuleRepo::new(db.clone()),
            GatewayRequestLogRepo::new(db),
            store,
        )
        .await
        .expect("source");
        (source, dir)
    }

    fn sample_record() -> GatewayRequestRecord {
        GatewayRequestRecord {
            request_id: RequestId::from("req-1"),
            route_id: "r1".to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: VendorId::from("v1"),
            method: "POST".to_owned(),
            path: "/gateway/codex/r1/v1/messages".to_owned(),
            model: Some("gpt-5".to_owned()),
            upstream_status: Some(200),
            outcome: GatewayOutcome::Ok,
            duration_ms: 120,
            bytes_in: 100,
            bytes_out: 2_000,
            retry_count: 0,
            switched: false,
            error_code: None,
            error_message: None,
            usage: Some(GatewayUsage {
                input_tokens: Some(10),
                output_tokens: Some(20),
                ..GatewayUsage::default()
            }),
            created_at: Timestamp::now(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_defaults_come_from_the_store() {
        let (source, _dir) = source().await;
        let settings = source.settings();
        // 默认端口是 0（自动分配），网关默认开启。
        assert_eq!(settings.port, 0);
        assert!(settings.enabled);
        assert!(!settings.external_enabled);
        assert!(
            settings.external_token_hash.is_none(),
            "未生成令牌时不该有哈希"
        );
        assert!(settings.failure_threshold >= 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_are_reread_on_refresh() {
        let (source, _dir) = source().await;
        assert_eq!(source.settings().port, 0);
        // 直接改设置文件，再刷新。
        source
            .settings_store
            .update(|raw| {
                raw.set_gateway_port(9_999);
                raw.set_gateway_failure_threshold(7);
            })
            .await
            .expect("update");
        source.refresh_settings().await;
        let settings = source.settings();
        assert_eq!(settings.port, 9_999);
        assert_eq!(settings.failure_threshold, 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_generated_token_hash_is_exposed() {
        let (source, _dir) = source().await;
        source
            .settings_store
            .update(|raw| raw.set_gateway_external_api_token_hash("abc123"))
            .await
            .expect("update");
        source.refresh_settings().await;
        assert_eq!(
            source.settings().external_token_hash.as_deref(),
            Some("abc123")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_vendor_table_yields_an_empty_pool() {
        let (source, _dir) = source().await;
        assert!(source.vendors().await.is_empty());
        assert!(source.failover_rules().await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn vendors_come_from_the_repository() {
        let (source, _dir) = source().await;
        source
            .vendor
            .save(&ApiVendorInput {
                provider_id: ProviderId::Codex,
                name: "主供应商".to_owned(),
                api_key: "sk-test".to_owned(),
                api_base_url: "https://api.example.com".to_owned(),
                ..ApiVendorInput::default()
            })
            .await
            .expect("save");
        let vendors = source.vendors().await;
        assert_eq!(vendors.len(), 1);
        assert_eq!(vendors[0].name, "主供应商");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failover_rules_come_from_the_repository() {
        let (source, _dir) = source().await;
        source
            .failover_rule
            .save(&aivc_core::gateway::FailoverRuleInput {
                scope: Some(aivc_core::gateway::FailoverRuleScope::Global),
                pattern: "insufficient quota".to_owned(),
                status_codes: Some(vec![429]),
                ..aivc_core::gateway::FailoverRuleInput::default()
            })
            .await
            .expect("save");
        assert_eq!(source.failover_rules().await.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn recording_persists_the_request() {
        // 这是这一层的核心职责：网关记下的请求必须真的进库。
        let (source, _dir) = source().await;
        source.record(sample_record()).await;

        // 先用不带任何筛选的计数确认行确实写进去了——`usage_summary` 还带时间区间，
        // 两者混在一起会让「没写进去」和「区间不含它」难以区分。
        let total = source
            .request_log
            .count(&aivc_infra::db::LogFilter {
                vendor_id: None,
                outcome: None,
                period_start: None,
                period_end: None,
            })
            .await
            .expect("count");
        assert_eq!(total, 1, "记录没有落库");

        let summary = source
            .request_log
            .usage_summary(
                &Timestamp::from_unix_millis(0),
                // 上界必须是能正常序列化的时间：区间比较是按 ISO 字符串做的，
                // 传一个荒谬的年份会得到无法比较的值。
                &Timestamp::from_unix_millis(4_102_444_800_000),
            )
            .await
            .expect("summary");
        assert_eq!(summary.request_count, 1, "用量汇总没有统计到刚写入的记录");

        // 用量与模型名也要落进去。
        let report = source
            .request_log
            .usage_report(
                &Timestamp::from_unix_millis(0),
                // 上界必须是能正常序列化的时间：区间比较是按 ISO 字符串做的，
                // 传一个荒谬的年份会得到无法比较的值。
                &Timestamp::from_unix_millis(4_102_444_800_000),
            )
            .await
            .expect("report");
        assert_eq!(report.vendors.len(), 1);
        // 报表按维度聚合，供应商标识在 `key` 上。
        assert_eq!(report.vendors[0].key, "v1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn attempts_and_summary_are_saved_as_one_usage_record() {
        let (source, _dir) = source().await;
        let attempt = GatewayAttempt {
            number: 1,
            vendor_id: VendorId::from("v1"),
            vendor_name: "mock supplier".into(),
            upstream_status: Some(200),
            duration_ms: 120,
            error_message: None,
        };
        source
            .record_with_attempts(sample_record(), vec![attempt.clone()])
            .await;
        let page = source
            .request_log
            .cleanup_entries_page(&aivc_infra::db::LogFilter::default(), 1, 20)
            .await
            .expect("page");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].attempts, vec![attempt]);
        assert_eq!(page.items[0].retry_count, 0);
        assert!(!page.items[0].switched);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_record_is_not_fatal() {
        // 记录失败不该影响已经完成的转发：`record` 不返回错误。
        let (source, _dir) = source().await;
        let mut record = sample_record();
        record.request_id = RequestId::from("req-2");
        source.record(record).await;
        // 再写一条相同编号：主键冲突，但调用方拿不到错误。
        let mut duplicate = sample_record();
        duplicate.request_id = RequestId::from("req-2");
        source.record(duplicate).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_survive_a_reload_from_disk() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let store = SettingsStore::new(dir.path().join("runtime/settings.json"));
        store
            .update(|raw| raw.set_gateway_port(8_080))
            .await
            .expect("update");

        // 新建一个来源（模拟重启），应当读到落盘的值。
        let source = RepositorySource::new(
            VendorRepo::new(db.clone()),
            FailoverRuleRepo::new(db.clone()),
            GatewayRequestLogRepo::new(db),
            store,
        )
        .await
        .expect("source");
        assert_eq!(source.settings().port, 8_080);
    }
}
