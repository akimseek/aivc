//! 网关供应商健康状态仓储。
//!
//! # 本表只是快照，不是权威来源
//!
//! 现网与这里都遵循同一设计：熔断状态机的**权威状态在内存**，本表只做节流落盘，
//! 供进程重启后恢复。因此：
//!
//! - 读取失败或数据库不可用时，网关必须继续用内存状态工作，不能拒绝请求；
//! - 成功路径的重复写入被节流（状态变化立即写），避免每个成功请求都写一次磁盘。
//!
//! # 读取时的状态转换
//!
//! 读出 `open` 且 `circuit_until` 已过期时，必须归一化为 `half-open`——否则进程重启后
//! 会把一个已经过期的熔断当成仍在熔断，把可用供应商永久排除在候选池外。

use aivc_core::error::Result;
use aivc_core::gateway::{GatewayVendorHealth, GatewayVendorHealthStatus};
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 网关健康状态仓储。
#[derive(Debug, Clone)]
pub struct GatewayHealthRepo {
    db: Db,
}

impl GatewayHealthRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 读取全部健康状态。
    ///
    /// 顺序固定为 `provider_id, vendor_id`，便于状态快照与测试对拍。
    pub async fn load_all(&self) -> Result<Vec<GatewayVendorHealth>> {
        self.db
            .read(|connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT vendor_id, provider_id, status, failure_count, success_count,
                                failure_rate, last_failure_at, last_success_at, circuit_until,
                                last_failure_reason
                         FROM gateway_vendor_health
                         ORDER BY provider_id, vendor_id",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map([], row_to_health)
                    .map_err(sqlite_error)?;

                let mut health = Vec::new();
                for row in rows {
                    health.push(row.map_err(sqlite_error)?);
                }
                Ok(health)
            })
            .await
    }

    /// 读取单个供应商的健康状态。
    pub async fn get(&self, vendor_id: &VendorId) -> Result<Option<GatewayVendorHealth>> {
        let vendor_id = vendor_id.clone();
        self.db
            .read(move |connection| {
                connection
                    .query_row(
                        "SELECT vendor_id, provider_id, status, failure_count, success_count,
                                failure_rate, last_failure_at, last_success_at, circuit_until,
                                last_failure_reason
                         FROM gateway_vendor_health WHERE vendor_id = ?1",
                        params![vendor_id.as_str()],
                        row_to_health,
                    )
                    .optional()
                    .map_err(sqlite_error)
            })
            .await
    }

    /// 写入（或更新）一条健康状态。
    pub async fn upsert(&self, health: &GatewayVendorHealth) -> Result<()> {
        let health = health.clone();
        self.db
            .write(move |connection| upsert(connection, &health))
            .await
    }

    /// 批量写入。健康状态持久化是节流后的批量操作，因此需要批量接口。
    pub async fn upsert_many(&self, entries: &[GatewayVendorHealth]) -> Result<usize> {
        if entries.is_empty() {
            return Ok(0);
        }
        let entries = entries.to_vec();
        self.db
            .write(move |connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;
                for entry in &entries {
                    upsert(&transaction, entry)?;
                }
                transaction.commit().map_err(sqlite_error)?;
                Ok(entries.len())
            })
            .await
    }

    /// 删除单个供应商的健康状态。
    pub async fn delete(&self, vendor_id: &VendorId) -> Result<bool> {
        let vendor_id = vendor_id.clone();
        self.db
            .write(move |connection| {
                let removed = connection
                    .execute(
                        "DELETE FROM gateway_vendor_health WHERE vendor_id = ?1",
                        params![vendor_id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                Ok(removed > 0)
            })
            .await
    }

    /// 清空全部健康状态（「重置熔断」操作）。
    pub async fn delete_all(&self) -> Result<usize> {
        self.db
            .write(|connection| {
                let removed = connection
                    .execute("DELETE FROM gateway_vendor_health", [])
                    .map_err(sqlite_error)?;
                Ok(removed)
            })
            .await
    }
}

/// 写入一条健康状态（upsert）。
fn upsert(connection: &Connection, health: &GatewayVendorHealth) -> Result<()> {
    connection
        .execute(
            "INSERT INTO gateway_vendor_health
               (vendor_id, provider_id, status, failure_count, success_count, failure_rate,
                last_failure_at, last_success_at, circuit_until, last_failure_reason)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(vendor_id) DO UPDATE SET
               provider_id = excluded.provider_id,
               status = excluded.status,
               failure_count = excluded.failure_count,
               success_count = excluded.success_count,
               failure_rate = excluded.failure_rate,
               last_failure_at = excluded.last_failure_at,
               last_success_at = excluded.last_success_at,
               circuit_until = excluded.circuit_until,
               last_failure_reason = excluded.last_failure_reason",
            params![
                health.vendor_id.as_str(),
                health.provider_id.as_str(),
                health.status.as_str(),
                to_i64(health.failure_count),
                to_i64(health.success_count),
                health.failure_rate,
                health.last_failure_at.as_ref().map(Timestamp::as_str),
                health.last_success_at.as_ref().map(Timestamp::as_str),
                health.circuit_until.as_ref().map(Timestamp::as_str),
                health.last_failure_reason,
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

/// 把一行映射为健康状态，并做读取时归一化。
fn row_to_health(row: &rusqlite::Row<'_>) -> rusqlite::Result<GatewayVendorHealth> {
    let provider_id: String = row.get(1)?;
    let status: String = row.get(2)?;
    let circuit_until: Option<String> = row.get(8)?;
    Ok(GatewayVendorHealth {
        vendor_id: VendorId::from(row.get::<_, String>(0)?),
        provider_id: provider_from_sql(&provider_id),
        status: normalize_status(&status, circuit_until.as_deref()),
        failure_count: from_i64(row.get(3)?),
        success_count: from_i64(row.get(4)?),
        failure_rate: row.get(5)?,
        last_failure_at: row.get::<_, Option<String>>(6)?.map(Timestamp::from_raw),
        last_success_at: row.get::<_, Option<String>>(7)?.map(Timestamp::from_raw),
        circuit_until: circuit_until.map(Timestamp::from_raw),
        last_failure_reason: row.get(9)?,
    })
}

/// 读取时归一化状态：熔断已过期则视为 `half-open`。
#[must_use]
pub fn normalize_status(status: &str, circuit_until: Option<&str>) -> GatewayVendorHealthStatus {
    let parsed = GatewayVendorHealthStatus::from_str_lossy(status);
    if parsed != GatewayVendorHealthStatus::Open {
        return parsed;
    }
    let expired = circuit_until
        .map(Timestamp::from_raw)
        .and_then(|until| until.to_unix_millis())
        .is_none_or(|until| until <= Timestamp::now().to_unix_millis().unwrap_or(0));
    if expired {
        GatewayVendorHealthStatus::HalfOpen
    } else {
        GatewayVendorHealthStatus::Open
    }
}

/// SQL 值 → 平台标识；未知值回退为 Codex（与 targetId 解析的兜底一致）。
fn provider_from_sql(value: &str) -> ProviderId {
    ProviderId::ALL
        .into_iter()
        .find(|item| item.as_str() == value)
        .unwrap_or(ProviderId::Codex)
}

/// `u64` → `i64`，溢出饱和。
fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `i64` → `u64`，负值视为 0。
fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> (tempfile::TempDir, GatewayHealthRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, GatewayHealthRepo::new(db))
    }

    fn health(
        vendor: &str,
        provider: ProviderId,
        status: GatewayVendorHealthStatus,
    ) -> GatewayVendorHealth {
        GatewayVendorHealth {
            vendor_id: VendorId::from(vendor),
            provider_id: provider,
            status,
            failure_count: 3,
            success_count: 10,
            failure_rate: 0.23,
            last_failure_at: Some(Timestamp::from_raw("2026-01-01T00:00:00.000Z")),
            last_success_at: Some(Timestamp::from_raw("2026-01-02T00:00:00.000Z")),
            circuit_until: None,
            last_failure_reason: Some("HTTP 503".to_owned()),
        }
    }

    #[tokio::test]
    async fn empty_database_lists_nothing() {
        let (_dir, repo) = repo().await;
        assert!(repo.load_all().await.expect("load").is_empty());
    }

    #[tokio::test]
    async fn upsert_then_load_round_trips_all_fields() {
        let (_dir, repo) = repo().await;
        let entry = health(
            "v1",
            ProviderId::Claude,
            GatewayVendorHealthStatus::Degraded,
        );
        repo.upsert(&entry).await.expect("upsert");

        let loaded = repo.load_all().await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], entry);
    }

    #[tokio::test]
    async fn upsert_updates_in_place() {
        let (_dir, repo) = repo().await;
        let mut entry = health("v1", ProviderId::Codex, GatewayVendorHealthStatus::Healthy);
        repo.upsert(&entry).await.expect("first");

        entry.status = GatewayVendorHealthStatus::Degraded;
        entry.failure_count = 7;
        repo.upsert(&entry).await.expect("second");

        let loaded = repo.load_all().await.expect("load");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].status, GatewayVendorHealthStatus::Degraded);
        assert_eq!(loaded[0].failure_count, 7);
    }

    #[tokio::test]
    async fn expired_open_circuit_is_normalized_to_half_open_on_read() {
        // 关键：进程重启后不得把已过期的熔断当成仍在熔断，否则可用供应商被永久排除。
        let (dir, repo) = repo().await;
        let mut entry = health("v1", ProviderId::Codex, GatewayVendorHealthStatus::Open);
        entry.circuit_until = Some(Timestamp::from_raw("2000-01-01T00:00:00.000Z"));
        repo.upsert(&entry).await.expect("upsert");

        let loaded = repo.load_all().await.expect("load");
        assert_eq!(loaded[0].status, GatewayVendorHealthStatus::HalfOpen);

        // 数据库里的原始值不变（归一化只发生在读取侧）。
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        let raw: String = db
            .read(|connection| {
                connection
                    .query_row(
                        "SELECT status FROM gateway_vendor_health WHERE vendor_id = 'v1'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)
            })
            .await
            .expect("read raw");
        assert_eq!(raw, "open");
    }

    #[tokio::test]
    async fn unexpired_open_circuit_stays_open() {
        let (_dir, repo) = repo().await;
        let mut entry = health("v1", ProviderId::Codex, GatewayVendorHealthStatus::Open);
        // 远未来的截止时间。
        entry.circuit_until = Some(Timestamp::from_raw("2100-01-01T00:00:00.000Z"));
        repo.upsert(&entry).await.expect("upsert");

        assert_eq!(
            repo.load_all().await.expect("load")[0].status,
            GatewayVendorHealthStatus::Open
        );
    }

    #[tokio::test]
    async fn open_without_circuit_until_is_normalized_to_half_open() {
        // 缺少截止时间的熔断无法判断是否过期，保守地视为已过期。
        let (_dir, repo) = repo().await;
        let mut entry = health("v1", ProviderId::Codex, GatewayVendorHealthStatus::Open);
        entry.circuit_until = None;
        repo.upsert(&entry).await.expect("upsert");

        assert_eq!(
            repo.load_all().await.expect("load")[0].status,
            GatewayVendorHealthStatus::HalfOpen
        );
    }

    #[tokio::test]
    async fn load_is_ordered_by_provider_then_vendor() {
        let (_dir, repo) = repo().await;
        repo.upsert(&health(
            "v2",
            ProviderId::Gemini,
            GatewayVendorHealthStatus::Healthy,
        ))
        .await
        .expect("upsert");
        repo.upsert(&health(
            "v1",
            ProviderId::Gemini,
            GatewayVendorHealthStatus::Healthy,
        ))
        .await
        .expect("upsert");
        repo.upsert(&health(
            "v9",
            ProviderId::Claude,
            GatewayVendorHealthStatus::Healthy,
        ))
        .await
        .expect("upsert");

        let order: Vec<(String, String)> = repo
            .load_all()
            .await
            .expect("load")
            .into_iter()
            .map(|item| {
                (
                    item.provider_id.as_str().to_owned(),
                    item.vendor_id.into_string(),
                )
            })
            .collect();
        assert_eq!(
            order,
            vec![
                ("claude".to_owned(), "v9".to_owned()),
                ("gemini".to_owned(), "v1".to_owned()),
                ("gemini".to_owned(), "v2".to_owned()),
            ]
        );
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_vendor() {
        let (_dir, repo) = repo().await;
        assert!(
            repo.get(&VendorId::from("missing"))
                .await
                .expect("get")
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_reports_whether_a_row_was_removed() {
        let (_dir, repo) = repo().await;
        let entry = health("v1", ProviderId::Codex, GatewayVendorHealthStatus::Healthy);
        repo.upsert(&entry).await.expect("upsert");

        assert!(repo.delete(&entry.vendor_id).await.expect("delete"));
        assert!(!repo.delete(&entry.vendor_id).await.expect("delete again"));
        assert!(repo.load_all().await.expect("load").is_empty());
    }

    #[tokio::test]
    async fn delete_all_clears_every_row() {
        let (_dir, repo) = repo().await;
        for index in 0..5 {
            repo.upsert(&health(
                &format!("v{index}"),
                ProviderId::Codex,
                GatewayVendorHealthStatus::Healthy,
            ))
            .await
            .expect("upsert");
        }

        assert_eq!(repo.delete_all().await.expect("delete all"), 5);
        assert!(repo.load_all().await.expect("load").is_empty());
    }

    #[tokio::test]
    async fn upsert_many_is_a_single_transaction() {
        let (_dir, repo) = repo().await;
        let entries: Vec<GatewayVendorHealth> = (0..10)
            .map(|index| {
                health(
                    &format!("v{index}"),
                    ProviderId::Codex,
                    GatewayVendorHealthStatus::Healthy,
                )
            })
            .collect();

        assert_eq!(repo.upsert_many(&entries).await.expect("upsert many"), 10);
        assert_eq!(repo.load_all().await.expect("load").len(), 10);
        // 空批次是 no-op。
        assert_eq!(repo.upsert_many(&[]).await.expect("empty"), 0);
    }

    #[tokio::test]
    async fn unknown_provider_and_status_values_degrade_instead_of_failing() {
        // 脏数据不应让整行读取失败，更不应让网关拒绝请求。
        let (dir, repo) = repo().await;
        repo.upsert(&health(
            "v1",
            ProviderId::Codex,
            GatewayVendorHealthStatus::Healthy,
        ))
        .await
        .expect("upsert");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            connection
                .execute(
                    "UPDATE gateway_vendor_health SET provider_id = 'unknown', status = 'weird' WHERE vendor_id = 'v1'",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("write dirty");

        let loaded = repo.load_all().await.expect("load");
        assert_eq!(loaded.len(), 1);
        // 未知平台回退为 Codex（与 targetId 解析兜底一致），未知状态回退为 healthy。
        assert_eq!(loaded[0].provider_id, ProviderId::Codex);
        assert_eq!(loaded[0].status, GatewayVendorHealthStatus::Healthy);
    }

    #[test]
    fn normalize_status_is_a_pure_function_of_the_stored_value() {
        // 无截止时间的 open 视为过期。
        assert_eq!(
            normalize_status("open", None),
            GatewayVendorHealthStatus::HalfOpen
        );
        // 非 open 状态不受截止时间影响。
        assert_eq!(
            normalize_status("healthy", Some("2100-01-01T00:00:00.000Z")),
            GatewayVendorHealthStatus::Healthy
        );
        assert_eq!(
            normalize_status("degraded", None),
            GatewayVendorHealthStatus::Degraded
        );
    }
}
