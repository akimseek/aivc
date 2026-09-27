//! 工作区预设仓储。
//!
//! # 去重语义
//!
//! 保存时若已有**相同 `cwd` 且相同 `target_kind`** 的预设，旧记录会被删除。
//! 现网把这个规则写在 `saveWorkspacePreset` 里：同一工作目录只保留最新一条，
//! 否则「最近使用的工作区」列表会被同一目录的多次保存刷满。
//!
//! 注意 `target_kind` 的比较是**严格相等**：`None`（不限）与 `Some(Local)` 是不同槽位。

use aivc_core::error::Result;
use aivc_core::id::PresetId;
use aivc_core::preset::WorkspacePreset;
use aivc_core::target::TargetKind;
use aivc_core::time::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 工作区预设仓储。
#[derive(Debug, Clone)]
pub struct WorkspacePresetRepo {
    db: Db,
}

impl WorkspacePresetRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 按更新时间倒序列出全部预设。
    pub async fn list(&self) -> Result<Vec<WorkspacePreset>> {
        self.db
            .read(|connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT id, name, cwd, target_kind, prompt, cli_args, updated_at
                         FROM workspace_presets
                         ORDER BY updated_at DESC",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map([], row_to_preset)
                    .map_err(sqlite_error)?;

                let mut presets = Vec::new();
                for row in rows {
                    presets.push(row.map_err(sqlite_error)?);
                }
                Ok(presets)
            })
            .await
    }

    /// 读取单个预设。
    pub async fn get(&self, id: &PresetId) -> Result<Option<WorkspacePreset>> {
        let id = id.clone();
        self.db
            .read(move |connection| {
                connection
                    .query_row(
                        "SELECT id, name, cwd, target_kind, prompt, cli_args, updated_at
                         FROM workspace_presets WHERE id = ?1",
                        params![id.as_str()],
                        row_to_preset,
                    )
                    .optional()
                    .map_err(sqlite_error)
            })
            .await
    }

    /// 保存预设，并在同一事务内按「同 `cwd` + 同 `target_kind`」去重。
    ///
    /// 返回被删除的重复预设编号（若有），便于调用方同步内存状态。
    pub async fn save(&self, preset: &WorkspacePreset) -> Result<Option<PresetId>> {
        let preset = preset.clone();
        self.db
            .write(move |connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;

                let duplicate: Option<String> = transaction
                    .query_row(
                        "SELECT id FROM workspace_presets
                         WHERE id <> ?1 AND cwd = ?2
                           AND ((target_kind IS NULL AND ?3 IS NULL) OR target_kind = ?3)
                         LIMIT 1",
                        params![
                            preset.id.as_str(),
                            preset.cwd,
                            target_kind_to_sql(preset.target_kind)
                        ],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(sqlite_error)?;

                if let Some(duplicate_id) = &duplicate {
                    transaction
                        .execute(
                            "DELETE FROM workspace_presets WHERE id = ?1",
                            params![duplicate_id],
                        )
                        .map_err(sqlite_error)?;
                }

                upsert(&transaction, &preset)?;
                transaction.commit().map_err(sqlite_error)?;
                Ok(duplicate.map(PresetId::from))
            })
            .await
    }

    /// 删除预设，返回是否确实删除了记录。
    pub async fn delete(&self, id: &PresetId) -> Result<bool> {
        let id = id.clone();
        self.db
            .write(move |connection| {
                let removed = connection
                    .execute(
                        "DELETE FROM workspace_presets WHERE id = ?1",
                        params![id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                Ok(removed > 0)
            })
            .await
    }

    /// 预设总数（诊断与测试用）。
    pub async fn count(&self) -> Result<i64> {
        self.db
            .read(|connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM workspace_presets", [], |row| {
                        row.get(0)
                    })
                    .map_err(sqlite_error)
            })
            .await
    }
}

/// 写入一条预设（upsert）。
fn upsert(connection: &Connection, preset: &WorkspacePreset) -> Result<()> {
    connection
        .execute(
            "INSERT INTO workspace_presets (id, name, cwd, target_kind, prompt, cli_args, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               name = excluded.name,
               cwd = excluded.cwd,
               target_kind = excluded.target_kind,
               prompt = excluded.prompt,
               cli_args = excluded.cli_args,
               updated_at = excluded.updated_at",
            params![
                preset.id.as_str(),
                preset.name,
                preset.cwd,
                target_kind_to_sql(preset.target_kind),
                preset.prompt,
                preset.cli_args,
                preset.updated_at.as_str()
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

/// 把一行映射为预设。
fn row_to_preset(row: &rusqlite::Row<'_>) -> rusqlite::Result<WorkspacePreset> {
    let target_kind: Option<String> = row.get(3)?;
    Ok(WorkspacePreset {
        id: PresetId::from(row.get::<_, String>(0)?),
        name: row.get(1)?,
        cwd: row.get(2)?,
        target_kind: target_kind_from_sql(target_kind.as_deref()),
        prompt: row.get(4)?,
        cli_args: row.get(5)?,
        updated_at: Timestamp::from_raw(row.get::<_, String>(6)?),
    })
}

/// 目标类型 → SQL 值。
fn target_kind_to_sql(kind: Option<TargetKind>) -> Option<&'static str> {
    match kind {
        Some(TargetKind::Local) => Some("local"),
        Some(TargetKind::Wsl) => Some("wsl"),
        None => None,
    }
}

/// SQL 值 → 目标类型；未知值视为不限（不因脏数据让整行读取失败）。
fn target_kind_from_sql(value: Option<&str>) -> Option<TargetKind> {
    match value {
        Some("local") => Some(TargetKind::Local),
        Some("wsl") => Some(TargetKind::Wsl),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> (tempfile::TempDir, WorkspacePresetRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, WorkspacePresetRepo::new(db))
    }

    fn preset(
        id: &str,
        name: &str,
        cwd: &str,
        kind: Option<TargetKind>,
        at: &str,
    ) -> WorkspacePreset {
        WorkspacePreset {
            id: PresetId::from(id),
            name: name.to_owned(),
            cwd: cwd.to_owned(),
            target_kind: kind,
            prompt: None,
            cli_args: None,
            updated_at: Timestamp::from_raw(at),
        }
    }

    #[tokio::test]
    async fn empty_database_lists_nothing() {
        let (_dir, repo) = repo().await;
        assert!(repo.list().await.expect("list").is_empty());
        assert_eq!(repo.count().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn save_then_list_round_trips_all_fields() {
        let (_dir, repo) = repo().await;
        let mut item = preset(
            "p1",
            "我的工作区",
            "/work/a",
            Some(TargetKind::Wsl),
            "2026-01-01T00:00:00.000Z",
        );
        item.prompt = Some("帮我重构".to_owned());
        item.cli_args = Some("--model gpt-5".to_owned());

        assert_eq!(repo.save(&item).await.expect("save"), None);

        let listed = repo.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], item);
    }

    #[tokio::test]
    async fn list_is_ordered_by_updated_at_descending() {
        let (_dir, repo) = repo().await;
        repo.save(&preset("old", "旧", "/a", None, "2026-01-01T00:00:00.000Z"))
            .await
            .expect("save");
        repo.save(&preset("new", "新", "/b", None, "2026-06-01T00:00:00.000Z"))
            .await
            .expect("save");
        repo.save(&preset("mid", "中", "/c", None, "2026-03-01T00:00:00.000Z"))
            .await
            .expect("save");

        let ids: Vec<String> = repo
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|item| item.id.into_string())
            .collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn saving_the_same_working_directory_replaces_the_previous_entry() {
        let (_dir, repo) = repo().await;
        repo.save(&preset(
            "p1",
            "旧名",
            "/work",
            None,
            "2026-01-01T00:00:00.000Z",
        ))
        .await
        .expect("first");
        let duplicate = repo
            .save(&preset(
                "p2",
                "新名",
                "/work",
                None,
                "2026-02-01T00:00:00.000Z",
            ))
            .await
            .expect("second");

        // 返回被替换的编号，便于调用方同步内存状态。
        assert_eq!(duplicate, Some(PresetId::from("p1")));
        let listed = repo.list().await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "新名");
    }

    #[tokio::test]
    async fn deduplication_is_scoped_by_target_kind() {
        let (_dir, repo) = repo().await;
        // 同一 cwd 但目标类型不同 → 是不同槽位，必须都保留。
        repo.save(&preset(
            "local",
            "本机",
            "/work",
            Some(TargetKind::Local),
            "2026-01-01T00:00:00.000Z",
        ))
        .await
        .expect("local");
        repo.save(&preset(
            "wsl",
            "WSL",
            "/work",
            Some(TargetKind::Wsl),
            "2026-01-02T00:00:00.000Z",
        ))
        .await
        .expect("wsl");
        repo.save(&preset(
            "any",
            "不限",
            "/work",
            None,
            "2026-01-03T00:00:00.000Z",
        ))
        .await
        .expect("any");

        assert_eq!(repo.count().await.expect("count"), 3);
    }

    #[tokio::test]
    async fn deduplication_does_not_treat_none_and_local_as_the_same_slot() {
        // None（不限）与 Some(Local) 是不同槽位，`IS NULL` 的比较必须正确。
        let (_dir, repo) = repo().await;
        repo.save(&preset(
            "any",
            "不限",
            "/work",
            None,
            "2026-01-01T00:00:00.000Z",
        ))
        .await
        .expect("any");
        let duplicate = repo
            .save(&preset(
                "local",
                "本机",
                "/work",
                Some(TargetKind::Local),
                "2026-01-02T00:00:00.000Z",
            ))
            .await
            .expect("local");

        assert_eq!(duplicate, None, "不限与本机不应互相去重");
        assert_eq!(repo.count().await.expect("count"), 2);
    }

    #[tokio::test]
    async fn saving_the_same_id_updates_in_place() {
        let (_dir, repo) = repo().await;
        repo.save(&preset("p1", "旧", "/a", None, "2026-01-01T00:00:00.000Z"))
            .await
            .expect("first");
        repo.save(&preset("p1", "新", "/b", None, "2026-02-01T00:00:00.000Z"))
            .await
            .expect("second");

        assert_eq!(repo.count().await.expect("count"), 1);
        let item = repo
            .get(&PresetId::from("p1"))
            .await
            .expect("get")
            .expect("present");
        assert_eq!(item.name, "新");
        assert_eq!(item.cwd, "/b");
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_id() {
        let (_dir, repo) = repo().await;
        assert!(
            repo.get(&PresetId::from("missing"))
                .await
                .expect("get")
                .is_none()
        );
    }

    #[tokio::test]
    async fn delete_reports_whether_a_row_was_removed() {
        let (_dir, repo) = repo().await;
        repo.save(&preset("p1", "n", "/a", None, "2026-01-01T00:00:00.000Z"))
            .await
            .expect("save");

        assert!(repo.delete(&PresetId::from("p1")).await.expect("delete"));
        assert!(
            !repo
                .delete(&PresetId::from("p1"))
                .await
                .expect("delete again")
        );
        assert_eq!(repo.count().await.expect("count"), 0);
    }

    #[tokio::test]
    async fn unknown_target_kind_degrades_to_unset_instead_of_failing() {
        // 脏数据不应让整行读取失败。
        let (dir, repo) = repo().await;
        repo.save(&preset("p1", "n", "/a", None, "2026-01-01T00:00:00.000Z"))
            .await
            .expect("save");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            connection
                .execute(
                    "UPDATE workspace_presets SET target_kind = 'container' WHERE id = 'p1'",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("write dirty kind");

        let item = repo
            .get(&PresetId::from("p1"))
            .await
            .expect("get")
            .expect("present");
        assert_eq!(item.target_kind, None);
    }

    #[tokio::test]
    async fn concurrent_saves_of_the_same_slot_leave_exactly_one_row() {
        // 去重与写入在同一事务内完成，因此并发保存同一槽位不会留下两条。
        let (_dir, repo) = repo().await;
        let repo = std::sync::Arc::new(repo);

        let mut handles = Vec::new();
        for index in 0..8 {
            let repo = std::sync::Arc::clone(&repo);
            handles.push(tokio::spawn(async move {
                repo.save(&preset(
                    &format!("p{index}"),
                    "n",
                    "/same",
                    None,
                    "2026-01-01T00:00:00.000Z",
                ))
                .await
            }));
        }
        for handle in handles {
            handle.await.expect("join").expect("save");
        }

        assert_eq!(repo.count().await.expect("count"), 1);
    }
}
