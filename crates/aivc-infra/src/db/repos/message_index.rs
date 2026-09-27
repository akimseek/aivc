//! 消息分页锚点仓储。
//!
//! 锚点把「可见消息序号」映射到文件内的**字节位置**，让详情分页可以直接 `seek` 定位，
//! 而不必从文件头逐行读过去。
//!
//! # 表设计要点
//!
//! 主键是 `(target_id, session_id, file_path, mtime_ms, size, message_offset)`，而
//! **失效判断**使用四元组 `(file_path, mtime_ms, change_ms, size)`：
//!
//! - `change_ms`（ctime）不在主键里，但参与失效判断。Windows 与 WSL 之间复制文件会
//!   保留 mtime 与大小，只有 ctime 能反映内容已变——这正是现网踩过的坑；
//! - 保存时先删除「指纹不同」的旧行，因此不会累积历史指纹的锚点。
//!
//! # 与现网的差异
//!
//! 现网锚点存 `line_number`，本工程存 `byte_offset`。查询语义完全一致
//! （取 `message_offset <= ?` 的最近一条），只是定位方式从线性变成随机访问。

use aivc_core::error::Result;
use aivc_core::id::{SessionId, TargetId};
use aivc_core::session::{MessageAnchor, Session};
use rusqlite::{OptionalExtension, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 会话文件的指纹，用于锚点失效判断。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionIndexKey {
    /// 目标标识。
    pub target_id: TargetId,
    /// 会话编号。
    pub session_id: SessionId,
    /// 会话文件绝对路径。
    pub file_path: String,
    /// 文件修改时间（毫秒）。
    pub mtime_ms: f64,
    /// 文件内容变更时间（毫秒）。
    pub change_ms: f64,
    /// 文件大小（字节）。
    pub size: u64,
}

impl SessionIndexKey {
    /// 由会话摘要构造指纹。
    ///
    /// `file_mtime_ms` 缺失时回退到 `updated_at` 的解析值，`file_change_ms` 缺失时
    /// 回退到 `mtime_ms`——与现网一致，保证老数据也能形成稳定指纹。
    #[must_use]
    pub fn from_session(target_id: &TargetId, session: &Session) -> Self {
        let mtime_ms = session
            .file_mtime_ms
            .or_else(|| {
                session
                    .updated_at
                    .as_ref()
                    .and_then(aivc_core::time::Timestamp::to_unix_millis)
                    .map(|value| value as f64)
            })
            .unwrap_or(0.0);
        Self {
            target_id: target_id.clone(),
            session_id: session.id.clone(),
            file_path: session.file_path.clone(),
            mtime_ms,
            change_ms: session.file_change_ms.unwrap_or(mtime_ms),
            size: session.file_size.unwrap_or(0),
        }
    }
}

/// 一次锚点写入。
#[derive(Debug, Clone, PartialEq)]
pub struct MessageIndexEntry {
    /// 文件指纹。
    pub key: SessionIndexKey,
    /// 锚点集合（按 `message_offset` 升序）。
    pub anchors: Vec<MessageAnchor>,
    /// 解析到的可见消息总数。
    pub message_count: u64,
}

/// 消息锚点仓储。
#[derive(Debug, Clone)]
pub struct MessageIndexRepo {
    db: Db,
}

impl MessageIndexRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 写入锚点集合，返回实际写入条数。
    ///
    /// 锚点为空时不做任何事（现网语义：没有锚点就没有可索引的位置，
    /// 写空集合只会白白清掉旧数据）。
    pub async fn save(&self, entry: &MessageIndexEntry) -> Result<usize> {
        if entry.anchors.is_empty() {
            return Ok(0);
        }
        let key = entry.key.clone();
        let anchors = entry.anchors.clone();
        let message_count = entry.message_count;

        self.db
            .write(move |connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;
                // 先清理指纹不同的历史行，避免同一会话累积多份锚点。
                transaction
                    .execute(
                        "DELETE FROM session_message_index
                         WHERE target_id = ?1 AND session_id = ?2
                           AND (file_path <> ?3 OR mtime_ms <> ?4 OR change_ms <> ?5 OR size <> ?6)",
                        params![
                            key.target_id.as_str(),
                            key.session_id.as_str(),
                            key.file_path,
                            key.mtime_ms,
                            key.change_ms,
                            to_i64(key.size)
                        ],
                    )
                    .map_err(sqlite_error)?;

                let indexed_at = aivc_core::time::Timestamp::now();
                {
                    let mut statement = transaction
                        .prepare(
                            "INSERT INTO session_message_index
                               (target_id, session_id, file_path, mtime_ms, change_ms, size,
                                message_offset, byte_offset, line_number, message_count, indexed_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                             ON CONFLICT(target_id, session_id, file_path, mtime_ms, size, message_offset)
                             DO UPDATE SET
                               byte_offset = excluded.byte_offset,
                               line_number = excluded.line_number,
                               change_ms = excluded.change_ms,
                               message_count = excluded.message_count,
                               indexed_at = excluded.indexed_at",
                        )
                        .map_err(sqlite_error)?;
                    for anchor in &anchors {
                        statement
                            .execute(params![
                                key.target_id.as_str(),
                                key.session_id.as_str(),
                                key.file_path,
                                key.mtime_ms,
                                key.change_ms,
                                to_i64(key.size),
                                to_i64(anchor.message_offset),
                                to_i64(anchor.byte_offset),
                                to_i64(anchor.line_number),
                                to_i64(message_count),
                                indexed_at.as_str()
                            ])
                            .map_err(sqlite_error)?;
                    }
                }
                transaction.commit().map_err(sqlite_error)?;
                Ok(anchors.len())
            })
            .await
    }

    /// 查询「序号不超过 `message_offset`」的最近锚点。
    ///
    /// 找不到（或指纹已变）时返回 [`None`]，调用方应回退到从头读取。
    pub async fn find_anchor_at_or_before(
        &self,
        key: &SessionIndexKey,
        message_offset: u64,
    ) -> Result<Option<MessageAnchor>> {
        let key = key.clone();
        let offset = to_i64(message_offset);
        self.db
            .read(move |connection| {
                connection
                    .query_row(
                        "SELECT message_offset, byte_offset, line_number, message_count
                         FROM session_message_index
                         WHERE target_id = ?1 AND session_id = ?2 AND file_path = ?3
                           AND mtime_ms = ?4 AND change_ms = ?5 AND size = ?6
                           AND message_offset <= ?7
                         ORDER BY message_offset DESC
                         LIMIT 1",
                        params![
                            key.target_id.as_str(),
                            key.session_id.as_str(),
                            key.file_path,
                            key.mtime_ms,
                            key.change_ms,
                            to_i64(key.size),
                            offset
                        ],
                        |row| {
                            Ok(MessageAnchor {
                                message_offset: from_i64(row.get(0)?),
                                byte_offset: from_i64(row.get(1)?),
                                line_number: from_i64(row.get(2)?),
                                message_count: from_i64(row.get(3)?),
                            })
                        },
                    )
                    .optional()
                    .map_err(sqlite_error)
            })
            .await
    }

    /// 删除某会话的全部锚点（会话被删除或永久清除时调用）。
    pub async fn clear_for_session(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
    ) -> Result<()> {
        let target_id = target_id.clone();
        let session_id = session_id.clone();
        self.db
            .write(move |connection| {
                connection
                    .execute(
                        "DELETE FROM session_message_index WHERE target_id = ?1 AND session_id = ?2",
                        params![target_id.as_str(), session_id.as_str()],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await
    }

    /// 某会话当前的锚点数量（诊断与测试用）。
    pub async fn count_for_session(
        &self,
        target_id: &TargetId,
        session_id: &SessionId,
    ) -> Result<i64> {
        let target_id = target_id.clone();
        let session_id = session_id.clone();
        self.db
            .read(move |connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM session_message_index
                         WHERE target_id = ?1 AND session_id = ?2",
                        params![target_id.as_str(), session_id.as_str()],
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)
            })
            .await
    }
}

/// `u64` → `i64`：SQLite 整数是 i64，溢出时饱和（文件大小与偏移不可能接近上限）。
fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `i64` → `u64`：列定义非负，负值视为 0 而不是回绕。
fn from_i64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    // 文件指纹是精确键值（mtime/ctime/size），断言必须用精确比较。
    #![allow(clippy::float_cmp)]

    use super::*;
    use aivc_core::session::Message;

    async fn repo() -> (tempfile::TempDir, MessageIndexRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, MessageIndexRepo::new(db))
    }

    fn session(id: &str, mtime: f64, change: f64, size: u64) -> Session {
        Session {
            id: SessionId::from(id),
            title: "t".to_owned(),
            source_title: None,
            cwd: None,
            created_at: None,
            updated_at: None,
            model: None,
            model_status: None,
            cli_version: None,
            file_path: format!("/sessions/{id}.jsonl"),
            file_mtime_ms: Some(mtime),
            file_change_ms: Some(change),
            file_size: Some(size),
            message_count: 1,
            message_count_is_lower_bound: false,
            preview: Vec::<Message>::new(),
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    fn key(id: &str, mtime: f64, change: f64, size: u64) -> SessionIndexKey {
        SessionIndexKey::from_session(&TargetId::from("local"), &session(id, mtime, change, size))
    }

    fn anchor(offset: u64, byte_offset: u64) -> MessageAnchor {
        MessageAnchor {
            message_offset: offset,
            byte_offset,
            line_number: offset + 1,
            message_count: 500,
        }
    }

    fn entry(key: SessionIndexKey, anchors: Vec<MessageAnchor>) -> MessageIndexEntry {
        MessageIndexEntry {
            key,
            anchors,
            message_count: 500,
        }
    }

    #[test]
    fn key_falls_back_when_fingerprint_fields_are_missing() {
        // file_mtime_ms 缺失 → 回退到 updated_at；file_change_ms 缺失 → 回退到 mtime。
        let mut raw = session("s1", 0.0, 0.0, 0);
        raw.file_mtime_ms = None;
        raw.file_change_ms = None;
        raw.file_size = None;
        raw.updated_at = Some(aivc_core::time::Timestamp::from_raw(
            "2026-09-16T12:34:56.789Z",
        ));

        let key = SessionIndexKey::from_session(&TargetId::from("local"), &raw);
        assert_eq!(key.mtime_ms, 1_789_562_096_789.0);
        assert_eq!(key.change_ms, key.mtime_ms, "change_ms 缺失应回退到 mtime");
        assert_eq!(key.size, 0);
    }

    #[test]
    fn key_uses_the_file_change_time_when_present() {
        // 这是现网踩过的坑：跨环境复制保留 mtime 与 size，只有 ctime 变了。
        let key = key("s1", 100.0, 200.0, 300);
        assert_eq!(key.mtime_ms, 100.0);
        assert_eq!(key.change_ms, 200.0);
        assert_eq!(key.size, 300);
    }

    #[tokio::test]
    async fn saving_no_anchors_is_a_no_op() {
        let (_dir, repo) = repo().await;
        let written = repo
            .save(&entry(key("s1", 1.0, 1.0, 1), Vec::new()))
            .await
            .expect("save");
        assert_eq!(written, 0);
        assert_eq!(
            repo.count_for_session(&TargetId::from("local"), &SessionId::from("s1"))
                .await
                .expect("count"),
            0
        );
    }

    #[tokio::test]
    async fn anchors_round_trip_and_are_looked_up_by_offset() {
        let (_dir, repo) = repo().await;
        let k = key("s1", 10.0, 11.0, 12);
        repo.save(&entry(
            k.clone(),
            vec![anchor(0, 0), anchor(100, 4096), anchor(200, 8192)],
        ))
        .await
        .expect("save");

        // 恰好命中锚点。
        let found = repo
            .find_anchor_at_or_before(&k, 100)
            .await
            .expect("find")
            .expect("anchor");
        assert_eq!(found.message_offset, 100);
        assert_eq!(found.byte_offset, 4096);

        // 落在两个锚点之间时取更早的那个。
        let between = repo
            .find_anchor_at_or_before(&k, 150)
            .await
            .expect("find")
            .expect("anchor");
        assert_eq!(between.message_offset, 100);

        // offset 0 应命中 message_offset=0 的锚点。
        assert!(
            repo.find_anchor_at_or_before(&k, 0)
                .await
                .expect("find")
                .is_some()
        );

        // 超出最后一个锚点 → 取最后一个。
        let last = repo
            .find_anchor_at_or_before(&k, 9_999)
            .await
            .expect("find")
            .expect("anchor");
        assert_eq!(last.message_offset, 200);
    }

    #[tokio::test]
    async fn anchor_lookup_returns_none_when_the_fingerprint_changed() {
        let (_dir, repo) = repo().await;
        repo.save(&entry(
            key("s1", 10.0, 11.0, 12),
            vec![anchor(0, 0), anchor(100, 4096)],
        ))
        .await
        .expect("save");

        // mtime 变化 → 指纹不匹配 → 必须回退到从头读取。
        assert!(
            repo.find_anchor_at_or_before(&key("s1", 99.0, 11.0, 12), 100)
                .await
                .expect("find")
                .is_none()
        );
        // ctime 变化 → 同上。这是跨环境复制文件的场景。
        assert!(
            repo.find_anchor_at_or_before(&key("s1", 10.0, 99.0, 12), 100)
                .await
                .expect("find")
                .is_none()
        );
        // size 变化 → 同上。
        assert!(
            repo.find_anchor_at_or_before(&key("s1", 10.0, 11.0, 99), 100)
                .await
                .expect("find")
                .is_none()
        );
    }

    #[tokio::test]
    async fn saving_a_new_fingerprint_replaces_the_old_anchors() {
        let (_dir, repo) = repo().await;
        let target = TargetId::from("local");
        let id = SessionId::from("s1");

        repo.save(&entry(
            key("s1", 10.0, 11.0, 12),
            vec![anchor(0, 0), anchor(100, 4096)],
        ))
        .await
        .expect("first");
        assert_eq!(
            repo.count_for_session(&target, &id).await.expect("count"),
            2
        );

        // 文件变了（mtime 与 ctime 都变）→ 旧指纹的锚点必须被清掉。
        let fresh = key("s1", 20.0, 21.0, 22);
        repo.save(&entry(fresh.clone(), vec![anchor(0, 0)]))
            .await
            .expect("second");
        assert_eq!(
            repo.count_for_session(&target, &id).await.expect("count"),
            1,
            "旧指纹的锚点应被清理"
        );
        assert!(
            repo.find_anchor_at_or_before(&fresh, 0)
                .await
                .expect("find")
                .is_some()
        );
    }

    #[tokio::test]
    async fn saving_the_same_offset_again_updates_instead_of_duplicating() {
        let (_dir, repo) = repo().await;
        let target = TargetId::from("local");
        let id = SessionId::from("s1");
        let k = key("s1", 10.0, 11.0, 12);

        repo.save(&entry(k.clone(), vec![anchor(0, 0), anchor(100, 4096)]))
            .await
            .expect("first");
        // 同一指纹、同一 message_offset，但字节位置被修正（例如重新解析）。
        repo.save(&entry(k.clone(), vec![anchor(0, 0), anchor(100, 5000)]))
            .await
            .expect("second");

        assert_eq!(
            repo.count_for_session(&target, &id).await.expect("count"),
            2
        );
        let found = repo
            .find_anchor_at_or_before(&k, 100)
            .await
            .expect("find")
            .expect("anchor");
        assert_eq!(found.byte_offset, 5000, "同 offset 应更新而不是新增");
    }

    #[tokio::test]
    async fn anchors_are_scoped_to_target_session_and_file() {
        let (_dir, repo) = repo().await;
        let k = key("s1", 1.0, 1.0, 1);
        repo.save(&entry(k.clone(), vec![anchor(0, 0)]))
            .await
            .expect("save");

        // 不同会话、不同目标都不得命中。
        assert!(
            repo.find_anchor_at_or_before(&key("s2", 1.0, 1.0, 1), 0)
                .await
                .expect("find")
                .is_none()
        );
        let mut other_target = k.clone();
        other_target.target_id = TargetId::from("wsl:Ubuntu");
        assert!(
            repo.find_anchor_at_or_before(&other_target, 0)
                .await
                .expect("find")
                .is_none()
        );
        let mut other_file = k.clone();
        other_file.file_path = "/elsewhere.jsonl".to_owned();
        assert!(
            repo.find_anchor_at_or_before(&other_file, 0)
                .await
                .expect("find")
                .is_none()
        );
    }

    #[tokio::test]
    async fn clear_for_session_removes_only_that_session() {
        let (_dir, repo) = repo().await;
        let target = TargetId::from("local");
        repo.save(&entry(key("s1", 1.0, 1.0, 1), vec![anchor(0, 0)]))
            .await
            .expect("save s1");
        repo.save(&entry(key("s2", 1.0, 1.0, 1), vec![anchor(0, 0)]))
            .await
            .expect("save s2");

        repo.clear_for_session(&target, &SessionId::from("s1"))
            .await
            .expect("clear");

        assert_eq!(
            repo.count_for_session(&target, &SessionId::from("s1"))
                .await
                .expect("count"),
            0
        );
        assert_eq!(
            repo.count_for_session(&target, &SessionId::from("s2"))
                .await
                .expect("count"),
            1
        );
    }

    #[tokio::test]
    async fn very_large_offsets_saturate_instead_of_wrapping() {
        // SQLite 整数是 i64。超过 i64::MAX 的偏移必须**饱和**而不是回绕：
        // 回绕会变成负数，让 `ORDER BY message_offset DESC` 把它排到最后，
        // 直接破坏分页定位；饱和则至少保持「这是最大偏移」这一语义。
        let (_dir, repo) = repo().await;
        let k = key("s1", 1.0, 1.0, 1);
        let huge = MessageAnchor {
            message_offset: u64::MAX,
            byte_offset: u64::MAX,
            line_number: u64::MAX,
            message_count: u64::MAX,
        };
        repo.save(&entry(k.clone(), vec![huge]))
            .await
            .expect("save");

        let found = repo
            .find_anchor_at_or_before(&k, u64::MAX)
            .await
            .expect("find")
            .expect("anchor");
        let saturated = u64::try_from(i64::MAX).expect("i64::MAX 可表示为 u64");
        assert_eq!(found.byte_offset, saturated);
        assert!(found.message_offset > 0, "偏移不得回绕成 0 或负数");
    }
}
