//! 会话元数据仓储。
//!
//! 用户级状态（自定义标题、分支来源、收藏、最近打开）与 CLI 私有存储无关，
//! 只存在本应用的 `session_metadata` 表中。四个字段共用一条记录，因此**任何一次
//! 更新都必须是「读当前 → 合并单字段 → 写回」**，不能整体覆盖（现网 DEV-GUIDE
//! 明确要求「在同一元数据记录中合并更新，彼此不覆盖」）。
//!
//! 合并与归一化的纯逻辑在 [`aivc_core::metadata`]，本模块只负责 SQL 与事务。

use std::collections::BTreeMap;

use aivc_core::error::Result;
use aivc_core::id::{SessionId, TargetId};
use aivc_core::metadata::{self, RawSessionMetadata};
use aivc_core::session::{BranchMetadata, Session, SessionMetadata};
use aivc_core::time::Timestamp;
use rusqlite::{Connection, OptionalExtension, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 一条带身份的元数据记录（用于「收藏 / 最近打开」快速访问区）。
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataEntry {
    /// 目标标识。
    pub target_id: TargetId,
    /// 会话编号。
    pub session_id: SessionId,
    /// 归一化后的元数据。
    pub metadata: SessionMetadata,
}

/// 会话元数据仓储。
#[derive(Debug, Clone)]
pub struct SessionMetadataRepo {
    db: Db,
}

impl SessionMetadataRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 读取单条元数据（已归一化）。不存在时返回空元数据。
    pub async fn get(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
    ) -> Result<SessionMetadata> {
        let target_id = target_id.clone();
        let session_id = session_id.clone();
        self.db
            .read(move |connection| read_one(connection, &target_id, &session_id))
            .await
    }

    /// 读取某个目标下的全部元数据（按会话编号索引）。
    pub async fn get_map(&self, target_id: &TargetId) -> Result<BTreeMap<String, SessionMetadata>> {
        let target_id = target_id.clone();
        self.db
            .read(move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT session_id, metadata_json FROM session_metadata WHERE target_id = ?1",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(params![target_id.as_str()], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(sqlite_error)?;

                let mut map = BTreeMap::new();
                for row in rows {
                    let (session_id, json) = row.map_err(sqlite_error)?;
                    map.insert(session_id, parse_metadata(&json));
                }
                Ok(map)
            })
            .await
    }

    /// 列出全部元数据记录（快速访问区用）。
    pub async fn list_all(&self) -> Result<Vec<MetadataEntry>> {
        self.db
            .read(|connection| {
                let mut statement = connection
                    .prepare("SELECT target_id, session_id, metadata_json FROM session_metadata")
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                        ))
                    })
                    .map_err(sqlite_error)?;

                let mut entries = Vec::new();
                for row in rows {
                    let (target_id, session_id, json) = row.map_err(sqlite_error)?;
                    entries.push(MetadataEntry {
                        target_id: TargetId::from(target_id),
                        session_id: SessionId::from(session_id),
                        metadata: parse_metadata(&json),
                    });
                }
                Ok(entries)
            })
            .await
    }

    /// 反查某会话的全部分支子会话编号。
    ///
    /// 走 `idx_session_metadata_parent` 表达式索引，不做全表扫描。
    pub async fn ids_by_parent(
        &self,
        target_id: &TargetId,
        parent_session_id: &SessionId,
    ) -> Result<Vec<SessionId>> {
        let target_id = target_id.clone();
        let parent_session_id = parent_session_id.clone();
        self.db
            .read(move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT session_id FROM session_metadata
                         WHERE target_id = ?1
                           AND json_extract(metadata_json, '$.branch.parentSessionId') = ?2",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(
                        params![target_id.as_str(), parent_session_id.as_str()],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(sqlite_error)?;

                let mut ids = Vec::new();
                for row in rows {
                    ids.push(SessionId::from(row.map_err(sqlite_error)?));
                }
                Ok(ids)
            })
            .await
    }

    /// 写入单条元数据（整体覆盖）。
    pub async fn save(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
        metadata: &SessionMetadata,
    ) -> Result<()> {
        let target_id = target_id.clone();
        let session_id = session_id.clone();
        let updated_at = metadata.updated_at.clone().unwrap_or_else(Timestamp::now);
        let json = serde_json::to_string(metadata).map_err(|error| {
            aivc_core::error::DatabaseError::Sqlite {
                message: format!("序列化会话元数据失败：{error}"),
            }
        })?;
        self.db
            .write(move |connection| {
                upsert(connection, &target_id, &session_id, &json, &updated_at)
            })
            .await
    }

    /// 删除单条元数据。
    pub async fn delete(&self, target_id: &TargetId, session_id: &SessionId) -> Result<()> {
        let target_id = target_id.clone();
        let session_id = session_id.clone();
        self.db
            .write(move |connection| {
                connection
                    .execute(
                        "DELETE FROM session_metadata WHERE target_id = ?1 AND session_id = ?2",
                        params![target_id.as_str(), session_id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await
    }

    /// 设置自定义标题。归一化后若元数据为空则删除整条记录。
    pub async fn set_custom_title(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
        custom_title: &str,
    ) -> Result<SessionMetadata> {
        // 闭包要跨线程进入写线程，因此必须持有所有权而不是借用入参。
        let custom_title = custom_title.to_owned();
        self.update(target_id, session_id, move |current| {
            metadata::merge_custom_title(current, &custom_title, Timestamp::now())
        })
        .await
    }

    /// 设置收藏状态。取消收藏且无其他元数据时会删除整条记录。
    pub async fn set_favorite(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
        favorite: bool,
    ) -> Result<SessionMetadata> {
        self.update(target_id, session_id, move |current| {
            metadata::merge_favorite(current, favorite, Timestamp::now())
        })
        .await
    }

    /// 记录「最近打开」。总是落库（现网语义）。
    pub async fn mark_opened(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
    ) -> Result<SessionMetadata> {
        self.update(target_id, session_id, |current| {
            (
                metadata::merge_last_opened(current, Timestamp::now()),
                false,
            )
        })
        .await
    }

    /// 写入分支来源。总是落库（现网语义）。
    pub async fn set_branch(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
        branch: &BranchMetadata,
    ) -> Result<SessionMetadata> {
        let branch = branch.clone();
        self.update(target_id, session_id, move |current| {
            (
                metadata::merge_branch(current, &branch, Timestamp::now()),
                false,
            )
        })
        .await
    }

    /// 把元数据附着到单个会话上。
    pub async fn attach(&self, target_id: &TargetId, session: Session) -> Result<Session> {
        let metadata = self.get(target_id, &session.id).await?;
        Ok(metadata::attach(session, &metadata))
    }

    /// 批量附着元数据。
    ///
    /// 只读一次元数据表（而不是逐会话查询），这是列表加载的关键路径。
    pub async fn attach_list(
        &self,
        target_id: &TargetId,
        sessions: Vec<Session>,
    ) -> Result<Vec<Session>> {
        if sessions.is_empty() {
            return Ok(sessions);
        }
        let map = self.get_map(target_id).await?;
        Ok(sessions
            .into_iter()
            .map(|session| {
                let metadata = map.get(session.id.as_str()).cloned().unwrap_or_default();
                metadata::attach(session, &metadata)
            })
            .collect())
    }

    /// 在**写连接内**完成「读当前 → 合并 → 落库」。
    ///
    /// 这三步必须在同一次写操作里完成，否则并发更新同一会话的不同字段会互相覆盖：
    /// 两个任务都读到同一份旧值，各自写回，后者覆盖前者。写连接由单写者 actor 串行化，
    /// 因此把整段逻辑放进 `write` 即可获得原子性。
    ///
    /// 现网是两次独立 `await`（`readSessionMetadata` 然后 `saveSessionMetadata`），
    /// 存在同样的竞态；这里刻意修正。
    ///
    /// `mutate` 返回 `(新元数据, 是否应删除记录)`。归一化后为空时必须删除整条记录，
    /// 而不是写入一个空对象——否则「取消收藏」会留下垃圾行。
    async fn update<F>(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
        mutate: F,
    ) -> Result<SessionMetadata>
    where
        F: FnOnce(&SessionMetadata) -> (SessionMetadata, bool) + Send + 'static,
    {
        let target_id = target_id.clone();
        let session_id = session_id.clone();
        self.db
            .write(move |connection| {
                let current = read_one(connection, &target_id, &session_id)?;
                let (next, empty) = mutate(&current);
                if empty {
                    connection
                        .execute(
                            "DELETE FROM session_metadata WHERE target_id = ?1 AND session_id = ?2",
                            params![target_id.as_str(), session_id.as_str()],
                        )
                        .map_err(sqlite_error)?;
                    return Ok(next);
                }
                let json = serde_json::to_string(&next).map_err(|error| {
                    aivc_core::error::DatabaseError::Sqlite {
                        message: format!("序列化会话元数据失败：{error}"),
                    }
                })?;
                let updated_at = next.updated_at.clone().unwrap_or_else(Timestamp::now);
                upsert(connection, &target_id, &session_id, &json, &updated_at)?;
                Ok(next)
            })
            .await
    }
}

/// 读取单条元数据（在给定连接上）。
fn read_one(
    connection: &Connection,
    target_id: &TargetId,
    session_id: &SessionId,
) -> Result<SessionMetadata> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT metadata_json FROM session_metadata
             WHERE target_id = ?1 AND session_id = ?2",
            params![target_id.as_str(), session_id.as_str()],
            |row| row.get(0),
        )
        .optional()
        .map_err(sqlite_error)?;
    Ok(raw.map_or_else(SessionMetadata::default, |json| parse_metadata(&json)))
}

/// 解析元数据 JSON；损坏时降级为空元数据。
///
/// 用 [`RawSessionMetadata`] 而不是直接反序列化到强类型：单个字段类型不符
/// 只应丢弃该字段，而不是让用户丢掉自定义标题。
fn parse_metadata(json: &str) -> SessionMetadata {
    serde_json::from_str::<RawSessionMetadata>(json)
        .map_or_else(|_| SessionMetadata::default(), |raw| raw.normalize())
}

/// 写入一条元数据（upsert）。
fn upsert(
    connection: &Connection,
    target_id: &TargetId,
    session_id: &SessionId,
    json: &str,
    updated_at: &Timestamp,
) -> Result<()> {
    connection
        .execute(
            "INSERT INTO session_metadata (target_id, session_id, metadata_json, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(target_id, session_id) DO UPDATE SET
               metadata_json = excluded.metadata_json,
               updated_at = excluded.updated_at",
            params![
                target_id.as_str(),
                session_id.as_str(),
                json,
                updated_at.as_str()
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::session::Message;

    async fn repo() -> (tempfile::TempDir, SessionMetadataRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, SessionMetadataRepo::new(db))
    }

    fn target() -> TargetId {
        TargetId::from("local")
    }

    fn session(id: &str, title: &str) -> Session {
        Session {
            id: SessionId::from(id),
            title: title.to_owned(),
            source_title: None,
            cwd: None,
            created_at: None,
            updated_at: None,
            model: None,
            model_status: None,
            cli_version: None,
            file_path: format!("/sessions/{id}.jsonl"),
            file_mtime_ms: None,
            file_change_ms: None,
            file_size: None,
            message_count: 1,
            message_count_is_lower_bound: false,
            preview: Vec::<Message>::new(),
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    #[tokio::test]
    async fn get_returns_empty_metadata_for_unknown_session() {
        let (_dir, repo) = repo().await;
        let metadata = repo
            .get(&target(), &SessionId::from("missing"))
            .await
            .expect("get");
        assert_eq!(metadata, SessionMetadata::default());
    }

    #[tokio::test]
    async fn custom_title_round_trips_and_is_trimmed() {
        let (_dir, repo) = repo().await;
        let saved = repo
            .set_custom_title(&target(), &SessionId::from("s1"), "  我的标题  ")
            .await
            .expect("set title");
        assert_eq!(saved.custom_title.as_deref(), Some("我的标题"));

        let read = repo
            .get(&target(), &SessionId::from("s1"))
            .await
            .expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("我的标题"));
    }

    #[tokio::test]
    async fn favorite_and_title_do_not_overwrite_each_other() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_custom_title(&target(), &id, "标题")
            .await
            .expect("title");
        repo.set_favorite(&target(), &id, true).await.expect("fav");

        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("标题"));
        assert_eq!(read.favorite, Some(true));

        // 反向顺序也必须互不覆盖。
        repo.set_custom_title(&target(), &id, "新标题")
            .await
            .expect("title again");
        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("新标题"));
        assert_eq!(read.favorite, Some(true));
    }

    #[tokio::test]
    async fn unsetting_favorite_removes_the_record_when_nothing_else_remains() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_favorite(&target(), &id, true).await.expect("fav");
        repo.set_favorite(&target(), &id, false)
            .await
            .expect("unfav");

        // 记录应被删除而不是留一个空对象。
        let entries = repo.list_all().await.expect("list");
        assert!(entries.is_empty(), "空元数据记录应被删除：{entries:?}");
    }

    #[tokio::test]
    async fn unsetting_favorite_keeps_the_record_when_title_remains() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_custom_title(&target(), &id, "标题")
            .await
            .expect("title");
        repo.set_favorite(&target(), &id, true).await.expect("fav");
        repo.set_favorite(&target(), &id, false)
            .await
            .expect("unfav");

        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("标题"));
        assert_eq!(read.favorite, None);
    }

    #[tokio::test]
    async fn mark_opened_sets_both_last_opened_and_updated_at() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("s1");
        let metadata = repo.mark_opened(&target(), &id).await.expect("open");
        assert!(metadata.last_opened_at.is_some());
        assert!(metadata.updated_at.is_some());

        let read = repo.get(&target(), &id).await.expect("get");
        assert!(read.last_opened_at.is_some());
    }

    #[tokio::test]
    async fn branch_metadata_round_trips_and_coexists_with_favorite() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("child");
        repo.set_favorite(&target(), &id, true).await.expect("fav");
        repo.set_branch(
            &target(),
            &id,
            &BranchMetadata {
                parent_target_id: Some(target()),
                parent_session_id: Some(SessionId::from("parent")),
                parent_message_index: None,
                parent_message_line: Some(42),
                created_by: Some(aivc_core::session::BranchOrigin::Branch),
            },
        )
        .await
        .expect("branch");

        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.favorite, Some(true));
        let branch = read.branch.expect("branch present");
        assert_eq!(
            branch.parent_session_id.as_ref().map(AsRef::as_ref),
            Some("parent")
        );
        assert_eq!(branch.parent_message_line, Some(42));
    }

    #[tokio::test]
    async fn ids_by_parent_finds_children() {
        let (_dir, repo) = repo().await;
        for child in ["c1", "c2"] {
            repo.set_branch(
                &target(),
                &SessionId::from(child),
                &BranchMetadata {
                    parent_target_id: Some(target()),
                    parent_session_id: Some(SessionId::from("p1")),
                    parent_message_index: None,
                    parent_message_line: Some(1),
                    created_by: None,
                },
            )
            .await
            .expect("branch");
        }
        // 另一个父会话的子会话不应混入。
        repo.set_branch(
            &target(),
            &SessionId::from("other"),
            &BranchMetadata {
                parent_target_id: Some(target()),
                parent_session_id: Some(SessionId::from("p2")),
                parent_message_index: None,
                parent_message_line: Some(1),
                created_by: None,
            },
        )
        .await
        .expect("branch");

        let mut children: Vec<String> = repo
            .ids_by_parent(&target(), &SessionId::from("p1"))
            .await
            .expect("children")
            .into_iter()
            .map(|id| id.as_str().to_owned())
            .collect();
        children.sort();
        assert_eq!(children, vec!["c1", "c2"]);

        assert!(
            repo.ids_by_parent(&target(), &SessionId::from("nobody"))
                .await
                .expect("children")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn ids_by_parent_is_scoped_to_one_target() {
        let (_dir, repo) = repo().await;
        repo.set_branch(
            &TargetId::from("wsl:Ubuntu"),
            &SessionId::from("c1"),
            &BranchMetadata {
                parent_target_id: Some(TargetId::from("wsl:Ubuntu")),
                parent_session_id: Some(SessionId::from("p1")),
                parent_message_index: None,
                parent_message_line: None,
                created_by: None,
            },
        )
        .await
        .expect("branch");

        assert!(
            repo.ids_by_parent(&target(), &SessionId::from("p1"))
                .await
                .expect("children")
                .is_empty(),
            "不同目标下的同名父会话不得混入"
        );
    }

    #[tokio::test]
    async fn attach_freezes_the_cli_title_and_applies_custom_title() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_custom_title(&target(), &id, "我的标题")
            .await
            .expect("title");

        let attached = repo
            .attach(&target(), session("s1", "首条用户消息"))
            .await
            .expect("attach");
        assert_eq!(attached.title, "我的标题");
        assert_eq!(attached.source_title.as_deref(), Some("首条用户消息"));
        assert!(attached.metadata.is_some());
    }

    #[tokio::test]
    async fn attach_list_leaves_sessions_without_metadata_untouched() {
        let (_dir, repo) = repo().await;
        repo.set_custom_title(&target(), &SessionId::from("s1"), "有元数据")
            .await
            .expect("title");

        let sessions = vec![session("s1", "原始一"), session("s2", "原始二")];
        let attached = repo
            .attach_list(&target(), sessions)
            .await
            .expect("attach list");

        assert_eq!(attached[0].title, "有元数据");
        assert_eq!(attached[0].source_title.as_deref(), Some("原始一"));
        // 无元数据的会话不得被写入 sourceTitle 或 metadata。
        assert_eq!(attached[1].title, "原始二");
        assert!(attached[1].source_title.is_none());
        assert!(attached[1].metadata.is_none());
    }

    #[tokio::test]
    async fn attach_list_with_empty_input_skips_the_query() {
        let (_dir, repo) = repo().await;
        assert!(
            repo.attach_list(&target(), Vec::new())
                .await
                .expect("empty")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn malformed_json_is_rejected_by_the_expression_index() {
        // `idx_session_metadata_parent` 是 json_extract 表达式索引，SQLite 在写入时
        // 必须重新求值该表达式，因此非法 JSON 会直接被拒绝。
        //
        // 这是相对现网的**刻意改进**：现网没有这个索引，垃圾可以落库，读取时再靠
        // 容错逻辑兜住。让数据库拒绝非法值比事后容错更可靠——该列只由本应用写入，
        // 正常情况下永远不会触发。
        let (dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_custom_title(&target(), &id, "标题")
            .await
            .expect("title");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        let outcome = db
            .write(|connection| {
                connection
                    .execute(
                        "UPDATE session_metadata SET metadata_json = '{ broken' WHERE session_id = 's1'",
                        [],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await;
        assert!(outcome.is_err(), "非法 JSON 必须被索引拒绝");

        // 原有记录不受影响。
        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("标题"));
    }

    #[tokio::test]
    async fn valid_json_with_the_wrong_shape_degrades_to_empty() {
        // 合法 JSON 但不是对象（例如数字）可以通过索引，此时读取侧必须降级而不是报错。
        let (dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_custom_title(&target(), &id, "标题")
            .await
            .expect("title");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            connection
                .execute(
                    "UPDATE session_metadata SET metadata_json = '123' WHERE session_id = 's1'",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("write non-object json");

        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read, SessionMetadata::default());
    }

    #[tokio::test]
    async fn partially_invalid_metadata_keeps_the_valid_fields() {
        let (dir, repo) = repo().await;
        let id = SessionId::from("s1");
        repo.set_custom_title(&target(), &id, "标题")
            .await
            .expect("title");

        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            // favorite 类型不符、branch 是数字：两者应被丢弃，标题必须保留。
            connection
                .execute(
                    r#"UPDATE session_metadata
                       SET metadata_json = '{"customTitle":"标题","favorite":"oops","branch":7}'
                       WHERE session_id = 's1'"#,
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("corrupt");

        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("标题"));
        assert_eq!(read.favorite, None);
        assert!(read.branch.is_none());
    }

    #[tokio::test]
    async fn save_then_delete_round_trips() {
        let (_dir, repo) = repo().await;
        let id = SessionId::from("s1");
        let metadata = SessionMetadata {
            custom_title: Some("标题".to_owned()),
            ..SessionMetadata::default()
        };
        repo.save(&target(), &id, &metadata).await.expect("save");
        assert_eq!(
            repo.get(&target(), &id)
                .await
                .expect("get")
                .custom_title
                .as_deref(),
            Some("标题")
        );

        repo.delete(&target(), &id).await.expect("delete");
        assert_eq!(
            repo.get(&target(), &id).await.expect("get"),
            SessionMetadata::default()
        );
    }

    #[tokio::test]
    async fn get_map_is_scoped_to_one_target() {
        let (_dir, repo) = repo().await;
        repo.set_custom_title(&target(), &SessionId::from("a"), "本机")
            .await
            .expect("title");
        repo.set_custom_title(&TargetId::from("wsl:Ubuntu"), &SessionId::from("b"), "WSL")
            .await
            .expect("title");

        let map = repo.get_map(&target()).await.expect("map");
        assert_eq!(map.len(), 1);
        assert!(map.contains_key("a"));
    }

    #[tokio::test]
    async fn concurrent_updates_to_different_fields_converge() {
        // 两个字段的并发更新都必须保留——依赖仓储内部的「读-改-写」不丢字段。
        let (_dir, repo) = repo().await;
        let repo = std::sync::Arc::new(repo);
        let id = SessionId::from("s1");

        let first = {
            let repo = std::sync::Arc::clone(&repo);
            let id = id.clone();
            tokio::spawn(async move { repo.set_custom_title(&target(), &id, "标题").await })
        };
        let second = {
            let repo = std::sync::Arc::clone(&repo);
            let id = id.clone();
            tokio::spawn(async move { repo.set_favorite(&target(), &id, true).await })
        };
        first.await.expect("join").expect("title");
        second.await.expect("join").expect("favorite");

        let read = repo.get(&target(), &id).await.expect("get");
        assert_eq!(read.custom_title.as_deref(), Some("标题"));
        assert_eq!(read.favorite, Some(true));
    }
}
