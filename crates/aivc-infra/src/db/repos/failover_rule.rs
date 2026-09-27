//! 网关故障识别规则仓储。
//!
//! 内置默认规则是**编译期常量**（见 [`aivc_core::constants`]），不落库；本表只存用户
//! 自定义规则。因此数据库不可用时默认规则依然生效——这是现网刻意的降级设计。
//!
//! # 排序语义
//!
//! 按 `created_at ASC, id ASC` 读取。规则之间是**或**关系（任一命中即触发切换），
//! 因此顺序不影响结果，但固定顺序能让 `list` 的结果稳定、便于测试与对拍。

use aivc_core::constants::GATEWAY_FAILOVER_RULE_LIMITS;
use aivc_core::error::{GatewayError, Result};
use aivc_core::gateway::{FailoverRule, FailoverRuleInput, FailoverRuleScope};
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 故障规则仓储。
#[derive(Debug, Clone)]
pub struct FailoverRuleRepo {
    db: Db,
}

impl FailoverRuleRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 列出全部自定义规则。
    pub async fn list(&self) -> Result<Vec<FailoverRule>> {
        self.db
            .read(|connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT id, scope, provider_id, vendor_id, pattern, status_codes,
                                custom_response_status, custom_response_body, enabled,
                                created_at, updated_at
                         FROM gateway_failover_rules
                         ORDER BY created_at ASC, id ASC",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement.query_map([], row_to_rule).map_err(sqlite_error)?;

                let mut rules = Vec::new();
                for row in rows {
                    rules.push(row.map_err(sqlite_error)?);
                }
                Ok(rules)
            })
            .await
    }

    /// 读取单条规则。
    pub async fn get(&self, id: &str) -> Result<Option<FailoverRule>> {
        let id = id.to_owned();
        self.db
            .read(move |connection| {
                connection
                    .query_row(
                        "SELECT id, scope, provider_id, vendor_id, pattern, status_codes,
                                custom_response_status, custom_response_body, enabled,
                                created_at, updated_at
                         FROM gateway_failover_rules WHERE id = ?1",
                        params![id],
                        row_to_rule,
                    )
                    .optional()
                    .map_err(sqlite_error)
            })
            .await
    }

    /// 保存规则（新建或更新），返回保存后的完整记录。
    ///
    /// 新建时若已达条数上限则拒绝；更新已有规则不受上限约束（否则用户无法修改已有规则）。
    pub async fn save(&self, input: &FailoverRuleInput) -> Result<FailoverRule> {
        let normalized = input.normalize()?;
        self.db
            .write(move |connection| {
                let existing: Option<(String, String)> = match normalized.id.as_deref() {
                    Some(id) => connection
                        .query_row(
                            "SELECT id, created_at FROM gateway_failover_rules WHERE id = ?1",
                            params![id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .optional()
                        .map_err(sqlite_error)?,
                    None => None,
                };

                if existing.is_none() {
                    let count: i64 = connection
                        .query_row("SELECT COUNT(*) FROM gateway_failover_rules", [], |row| {
                            row.get(0)
                        })
                        .map_err(sqlite_error)?;
                    if usize::try_from(count).unwrap_or(usize::MAX)
                        >= GATEWAY_FAILOVER_RULE_LIMITS.max_rule_count
                    {
                        return Err(GatewayError::FailoverRuleLimit {
                            limit: GATEWAY_FAILOVER_RULE_LIMITS.max_rule_count,
                        }
                        .into());
                    }
                }

                let now = Timestamp::now();
                let rule = FailoverRule {
                    id: existing
                        .as_ref()
                        .map_or_else(|| uuid::Uuid::new_v4().to_string(), |(id, _)| id.clone()),
                    scope: normalized.scope,
                    provider_id: normalized.provider_id,
                    vendor_id: normalized.vendor_id.clone(),
                    pattern: normalized.pattern.clone(),
                    status_codes: if normalized.status_codes.is_empty() {
                        None
                    } else {
                        Some(normalized.status_codes.clone())
                    },
                    custom_response_status: normalized.custom_response_status,
                    custom_response_body: normalized.custom_response_body.clone(),
                    enabled: normalized.enabled,
                    // 更新时间保留原创建时间，避免列表顺序在编辑后跳变。
                    created_at: existing.as_ref().map_or_else(
                        || now.clone(),
                        |(_, created)| Timestamp::from_raw(created.clone()),
                    ),
                    updated_at: now,
                };

                upsert(connection, &rule)?;
                Ok(rule)
            })
            .await
    }

    /// 删除规则，返回是否确实删除了记录。
    pub async fn delete(&self, id: &str) -> Result<bool> {
        let id = id.trim().to_owned();
        if id.is_empty() {
            return Err(GatewayError::FailoverRuleIdEmpty.into());
        }
        self.db
            .write(move |connection| {
                let removed = connection
                    .execute(
                        "DELETE FROM gateway_failover_rules WHERE id = ?1",
                        params![id],
                    )
                    .map_err(sqlite_error)?;
                Ok(removed > 0)
            })
            .await
    }

    /// 启用 / 停用规则。
    pub async fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool> {
        let id = id.trim().to_owned();
        if id.is_empty() {
            return Err(GatewayError::FailoverRuleIdEmpty.into());
        }
        self.db
            .write(move |connection| {
                let updated = connection
                    .execute(
                        "UPDATE gateway_failover_rules SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
                        params![i64::from(enabled), Timestamp::now().as_str(), id],
                    )
                    .map_err(sqlite_error)?;
                if updated == 0 {
                    return Err(GatewayError::FailoverRuleNotFound.into());
                }
                Ok(true)
            })
            .await
    }

    /// 规则总数。
    pub async fn count(&self) -> Result<i64> {
        self.db
            .read(|connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM gateway_failover_rules", [], |row| {
                        row.get(0)
                    })
                    .map_err(sqlite_error)
            })
            .await
    }
}

/// 写入一条规则（upsert）。
fn upsert(connection: &Connection, rule: &FailoverRule) -> Result<()> {
    let status_codes = serde_json::to_string(&rule.status_codes.clone().unwrap_or_default())
        .unwrap_or_else(|_| "[]".to_owned());
    connection
        .execute(
            "INSERT INTO gateway_failover_rules
               (id, scope, provider_id, vendor_id, pattern, status_codes,
                custom_response_status, custom_response_body, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
             ON CONFLICT(id) DO UPDATE SET
               scope = excluded.scope,
               provider_id = excluded.provider_id,
               vendor_id = excluded.vendor_id,
               pattern = excluded.pattern,
               status_codes = excluded.status_codes,
               custom_response_status = excluded.custom_response_status,
               custom_response_body = excluded.custom_response_body,
               enabled = excluded.enabled,
               updated_at = excluded.updated_at",
            params![
                rule.id,
                scope_to_sql(rule.scope),
                rule.provider_id.map(ProviderId::as_str),
                rule.vendor_id.as_ref().map(VendorId::as_str),
                rule.pattern,
                status_codes,
                rule.custom_response_status,
                rule.custom_response_body,
                i64::from(rule.enabled),
                rule.created_at.as_str(),
                rule.updated_at.as_str()
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

/// 把一行映射为规则。
fn row_to_rule(row: &rusqlite::Row<'_>) -> rusqlite::Result<FailoverRule> {
    let provider_id: Option<String> = row.get(2)?;
    let vendor_id: Option<String> = row.get(3)?;
    let status_codes: Option<String> = row.get(5)?;
    let custom_response_status: Option<i64> = row.get(6)?;
    Ok(FailoverRule {
        id: row.get(0)?,
        scope: scope_from_sql(&row.get::<_, String>(1)?),
        provider_id: provider_id.as_deref().and_then(provider_from_sql),
        vendor_id: vendor_id.map(VendorId::from),
        pattern: row.get(4)?,
        status_codes: parse_status_codes(status_codes.as_deref()),
        custom_response_status: custom_response_status.and_then(|value| u16::try_from(value).ok()),
        custom_response_body: row.get(7)?,
        enabled: row.get::<_, i64>(8)? != 0,
        created_at: Timestamp::from_raw(row.get::<_, String>(9)?),
        updated_at: Timestamp::from_raw(row.get::<_, String>(10)?),
    })
}

/// 作用域 → SQL 值。
fn scope_to_sql(scope: FailoverRuleScope) -> &'static str {
    match scope {
        FailoverRuleScope::Global => "global",
        FailoverRuleScope::Provider => "provider",
        FailoverRuleScope::Vendor => "vendor",
    }
}

/// SQL 值 → 作用域；未知值回退为 `global`（保守：作用域越大越不会误伤，
/// 而且真正决定命中与否的是 `pattern` 与 `status_codes`）。
fn scope_from_sql(value: &str) -> FailoverRuleScope {
    match value {
        "provider" => FailoverRuleScope::Provider,
        "vendor" => FailoverRuleScope::Vendor,
        _ => FailoverRuleScope::Global,
    }
}

/// SQL 值 → 平台标识。
fn provider_from_sql(value: &str) -> Option<ProviderId> {
    ProviderId::ALL
        .into_iter()
        .find(|item| item.as_str() == value)
}

/// 解析状态码 JSON 数组，过滤掉越界值。
fn parse_status_codes(value: Option<&str>) -> Option<Vec<u16>> {
    let parsed: Vec<u16> = serde_json::from_str::<Vec<u16>>(value?).ok()?;
    let filtered: Vec<u16> = parsed
        .into_iter()
        .filter(|code| (400..=599).contains(code))
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> (tempfile::TempDir, FailoverRuleRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, FailoverRuleRepo::new(db))
    }

    fn global(pattern: &str) -> FailoverRuleInput {
        FailoverRuleInput {
            scope: Some(FailoverRuleScope::Global),
            pattern: pattern.to_owned(),
            ..FailoverRuleInput::default()
        }
    }

    #[tokio::test]
    async fn empty_database_lists_nothing() {
        let (_dir, repo) = repo().await;
        assert!(repo.list().await.expect("list").is_empty());
        assert_eq!(repo.count().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn save_assigns_an_id_and_round_trips_all_fields() {
        let (_dir, repo) = repo().await;
        let mut input = global("rate limit");
        input.status_codes = Some(vec![429, 503]);
        input.custom_response_status = Some(418);
        input.custom_response_body = Some(r#"{"error":"teapot"}"#.to_owned());

        let saved = repo.save(&input).await.expect("save");
        assert!(!saved.id.is_empty());
        assert!(saved.enabled, "缺省应启用");

        let listed = repo.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], saved);
        assert_eq!(listed[0].status_codes, Some(vec![429, 503]));
        assert_eq!(listed[0].custom_response_status, Some(418));
    }

    #[tokio::test]
    async fn saving_with_an_existing_id_updates_in_place_and_keeps_created_at() {
        let (_dir, repo) = repo().await;
        let saved = repo.save(&global("first")).await.expect("save");

        let updated = repo
            .save(&FailoverRuleInput {
                id: Some(saved.id.clone()),
                pattern: "second".to_owned(),
                ..global("ignored")
            })
            .await
            .expect("update");

        assert_eq!(updated.id, saved.id);
        assert_eq!(updated.pattern, "second");
        // 创建时间必须保留，否则列表顺序会在编辑后跳变。
        assert_eq!(updated.created_at, saved.created_at);
        assert_eq!(repo.count().await.expect("count"), 1);
    }

    #[tokio::test]
    async fn scope_and_provider_round_trip() {
        let (_dir, repo) = repo().await;
        let saved = repo
            .save(&FailoverRuleInput {
                scope: Some(FailoverRuleScope::Provider),
                provider_id: Some(ProviderId::Claude),
                pattern: "overloaded".to_owned(),
                ..FailoverRuleInput::default()
            })
            .await
            .expect("save");

        assert_eq!(saved.scope, FailoverRuleScope::Provider);
        assert_eq!(saved.provider_id, Some(ProviderId::Claude));

        let listed = repo.list().await.expect("list");
        assert_eq!(listed[0].scope, FailoverRuleScope::Provider);
        assert_eq!(listed[0].provider_id, Some(ProviderId::Claude));
    }

    #[tokio::test]
    async fn vendor_scope_round_trips() {
        let (_dir, repo) = repo().await;
        let saved = repo
            .save(&FailoverRuleInput {
                scope: Some(FailoverRuleScope::Vendor),
                provider_id: Some(ProviderId::Codex),
                vendor_id: Some(VendorId::from("v1")),
                pattern: "capacity".to_owned(),
                ..FailoverRuleInput::default()
            })
            .await
            .expect("save");

        assert_eq!(saved.vendor_id, Some(VendorId::from("v1")));
        let listed = repo.list().await.expect("list");
        assert_eq!(listed[0].vendor_id, Some(VendorId::from("v1")));
    }

    #[tokio::test]
    async fn invalid_input_is_rejected_before_touching_the_database() {
        let (_dir, repo) = repo().await;
        // 空条件必须被拒绝，且不得留下记录。
        assert!(repo.save(&global("   ")).await.is_err());
        assert_eq!(repo.count().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn rule_count_limit_blocks_new_rules_but_not_updates() {
        let (_dir, repo) = repo().await;

        // 填满到上限。
        let mut last_id = String::new();
        for index in 0..GATEWAY_FAILOVER_RULE_LIMITS.max_rule_count {
            let saved = repo
                .save(&global(&format!("pattern-{index}")))
                .await
                .expect("save");
            last_id = saved.id;
        }
        assert_eq!(
            repo.count().await.expect("count"),
            i64::try_from(GATEWAY_FAILOVER_RULE_LIMITS.max_rule_count).unwrap_or(i64::MAX)
        );

        // 新建必须被拒绝，且错误文案与现网一致。
        let error = repo
            .save(&global("one-too-many"))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "Gateway 故障规则最多保存 100 条。");
        assert_eq!(
            repo.count().await.expect("count"),
            i64::try_from(GATEWAY_FAILOVER_RULE_LIMITS.max_rule_count).unwrap_or(i64::MAX)
        );

        // 更新已有规则不受上限约束，否则用户无法修改已有规则。
        assert!(
            repo.save(&FailoverRuleInput {
                id: Some(last_id),
                ..global("updated")
            })
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn set_enabled_toggles_and_reports_missing_rules() {
        let (_dir, repo) = repo().await;
        let saved = repo.save(&global("x")).await.expect("save");

        repo.set_enabled(&saved.id, false).await.expect("disable");
        assert!(
            !repo
                .get(&saved.id)
                .await
                .expect("get")
                .expect("present")
                .enabled
        );

        repo.set_enabled(&saved.id, true).await.expect("enable");
        assert!(
            repo.get(&saved.id)
                .await
                .expect("get")
                .expect("present")
                .enabled
        );

        let error = repo
            .set_enabled("missing", false)
            .await
            .expect_err("必须报错");
        assert_eq!(error.to_string(), "Gateway 故障规则不存在。");
    }

    #[tokio::test]
    async fn empty_rule_id_is_rejected() {
        let (_dir, repo) = repo().await;
        assert_eq!(
            repo.delete("   ").await.expect_err("必须拒绝").to_string(),
            "规则 ID 不能为空。"
        );
        assert_eq!(
            repo.set_enabled("", true)
                .await
                .expect_err("必须拒绝")
                .to_string(),
            "规则 ID 不能为空。"
        );
    }

    #[tokio::test]
    async fn delete_reports_whether_a_row_was_removed() {
        let (_dir, repo) = repo().await;
        let saved = repo.save(&global("x")).await.expect("save");
        assert!(repo.delete(&saved.id).await.expect("delete"));
        assert!(!repo.delete(&saved.id).await.expect("delete again"));
        assert_eq!(repo.count().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn list_is_ordered_by_creation_time_then_id() {
        let (_dir, repo) = repo().await;
        let first = repo.save(&global("a")).await.expect("a");
        let second = repo.save(&global("b")).await.expect("b");
        let third = repo.save(&global("c")).await.expect("c");

        let ids: Vec<String> = repo
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|rule| rule.id)
            .collect();
        assert_eq!(ids, vec![first.id, second.id, third.id]);
    }

    #[tokio::test]
    async fn corrupt_status_codes_degrade_to_none_without_failing_the_row() {
        let (dir, repo) = repo().await;
        let saved = repo.save(&global("x")).await.expect("save");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        let id = saved.id.clone();
        db.write(move |connection| {
            connection
                .execute(
                    "UPDATE gateway_failover_rules SET status_codes = 'not json' WHERE id = ?1",
                    params![id],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("corrupt");

        // 状态码损坏不影响整条规则读取，pattern 仍然可用。
        let listed = repo.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pattern, "x");
        assert_eq!(listed[0].status_codes, None);
    }

    #[tokio::test]
    async fn out_of_range_status_codes_in_storage_are_filtered_out() {
        let (dir, repo) = repo().await;
        let saved = repo.save(&global("x")).await.expect("save");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        let id = saved.id.clone();
        db.write(move |connection| {
            connection
                .execute(
                    "UPDATE gateway_failover_rules SET status_codes = '[200,429,999]' WHERE id = ?1",
                    params![id],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("write");

        let listed = repo.list().await.expect("list");
        assert_eq!(listed[0].status_codes, Some(vec![429]));
    }

    #[tokio::test]
    async fn concurrent_saves_produce_distinct_rules() {
        let (_dir, repo) = repo().await;
        let repo = std::sync::Arc::new(repo);

        let mut handles = Vec::new();
        for index in 0..8 {
            let repo = std::sync::Arc::clone(&repo);
            handles.push(tokio::spawn(async move {
                repo.save(&global(&format!("pattern-{index}"))).await
            }));
        }
        for handle in handles {
            handle.await.expect("join").expect("save");
        }
        assert_eq!(repo.count().await.expect("count"), 8);
    }
}
