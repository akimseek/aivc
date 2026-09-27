//! 会话摘要缓存仓储。
//!
//! # 缓存失效键
//!
//! 主键是 `(cache_key, file_path)`，但**是否复用**取决于 `(mtime_ms, change_ms, size)`
//! 三元组是否全等。现网刻意如此：Windows 与 WSL 之间复制文件会保留 mtime 与大小，
//! 只有 ctime（`change_ms`）能反映内容已变。只比对 mtime + size 会让详情页复用旧 JSONL。
//!
//! # 淘汰策略
//!
//! 上限 4,000 条 / 64 MB，单条 1 MB；超限时按 `cached_at` 升序（同时间按 rowid）淘汰。
//! 淘汰与写入在**同一事务**内完成，避免并发写入把刚淘汰的空间又填满。
//!
//! # 与现网的一处刻意改进
//!
//! 现网的总量统计用 `LENGTH(session_json)`，而 SQLite 的 `LENGTH()` 对 TEXT 返回的是
//! **字符数**而非字节数。会话正文多为中文（UTF-8 下 3 字节/字符），因此现网 64 MB 的
//! 上限实际允许落盘约 192 MB。这里改用 `LENGTH(CAST(session_json AS BLOB))` 统计真实
//! 字节数。这是资源上限的修正，不改变任何用户可见行为或数据格式。

use std::collections::BTreeMap;

use aivc_core::error::Result;
use aivc_core::session::Session;
use aivc_core::time::Timestamp;
use rusqlite::{Connection, params};

use crate::db::Db;
use crate::db::migrations::sqlite_error;

/// 单条缓存体积上限（字节）。超过则跳过该条，不写入。
pub const MAX_ENTRY_BYTES: usize = 1024 * 1024;

/// 缓存总条数上限。
pub const MAX_ENTRIES: i64 = 4_000;

/// 缓存总字节上限。
pub const MAX_TOTAL_BYTES: i64 = 64 * 1024 * 1024;

/// 一条缓存记录。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionCacheEntry {
    /// 会话文件绝对路径。
    pub file_path: String,
    /// 文件修改时间（毫秒）。
    pub mtime_ms: f64,
    /// 文件内容变更时间（毫秒）。
    pub change_ms: f64,
    /// 文件大小（字节）。
    pub size: u64,
    /// 解析后的会话摘要。
    pub session: Session,
}

impl SessionCacheEntry {
    /// 判断缓存是否仍然新鲜。
    ///
    /// 三个字段必须**全部**相等：只看 mtime 与 size 会在跨环境复制文件时误判。
    ///
    /// 这里刻意使用精确比较：这是缓存失效键，不是浮点测量。用误差范围比较会让
    /// 「文件已改但 mtime 只差几微秒」的场景被误判为命中，从而展示过期内容。
    #[must_use]
    #[allow(clippy::float_cmp)]
    pub fn is_fresh_for(&self, mtime_ms: f64, change_ms: f64, size: u64) -> bool {
        self.mtime_ms == mtime_ms && self.change_ms == change_ms && self.size == size
    }
}

/// 缓存统计。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// 记录条数。
    pub entries: i64,
    /// 不同缓存键数量。
    pub cache_keys: i64,
    /// 载荷总字节数。
    pub payload_bytes: i64,
}

/// 会话缓存仓储。
#[derive(Debug, Clone)]
pub struct SessionCacheRepo {
    db: Db,
}

impl SessionCacheRepo {
    /// 由数据库句柄构造。
    #[must_use]
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// 读取某个缓存键下的全部记录。
    ///
    /// 单条 JSON 损坏时跳过该条而不是整体失败：损坏只可能来自外部改写，
    /// 下一次扫描会自动重建，让整个列表加载失败是不可接受的。
    pub async fn read(&self, cache_key: &str) -> Result<BTreeMap<String, SessionCacheEntry>> {
        let cache_key = cache_key.to_owned();
        self.db
            .read(move |connection| {
                let mut statement = connection
                    .prepare(
                        "SELECT file_path, mtime_ms, change_ms, size, session_json
                         FROM session_cache WHERE cache_key = ?1",
                    )
                    .map_err(sqlite_error)?;
                let rows = statement
                    .query_map(params![cache_key], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, f64>(1)?,
                            row.get::<_, f64>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                        ))
                    })
                    .map_err(sqlite_error)?;

                let mut entries = BTreeMap::new();
                for row in rows {
                    let (file_path, mtime_ms, change_ms, size, json) = row.map_err(sqlite_error)?;
                    let Ok(session) = serde_json::from_str::<Session>(&json) else {
                        continue;
                    };
                    entries.insert(
                        file_path.clone(),
                        SessionCacheEntry {
                            file_path,
                            mtime_ms,
                            change_ms,
                            size: u64::try_from(size).unwrap_or(0),
                            session,
                        },
                    );
                }
                Ok(entries)
            })
            .await
    }

    /// 用给定记录整体替换某个缓存键的内容，并在同一事务内执行淘汰。
    ///
    /// 返回 `(写入条数, 淘汰条数)`。超过 [`MAX_ENTRY_BYTES`] 的单条会被跳过。
    pub async fn replace(
        &self,
        cache_key: &str,
        entries: &[SessionCacheEntry],
    ) -> Result<(usize, i64)> {
        let cache_key = cache_key.to_owned();

        // 按 file_path 去重（后写覆盖先写），避免同一批里出现重复主键。
        let mut deduplicated: BTreeMap<String, (f64, f64, u64, String)> = BTreeMap::new();
        for entry in entries {
            let Ok(json) = serde_json::to_string(&entry.session) else {
                continue;
            };
            // `String::len()` 是字节数，与现网的 `Buffer.byteLength(json, "utf8")` 等价。
            if json.len() > MAX_ENTRY_BYTES {
                continue;
            }
            deduplicated.insert(
                entry.file_path.clone(),
                (entry.mtime_ms, entry.change_ms, entry.size, json),
            );
        }

        self.db
            .write(move |connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;
                transaction
                    .execute(
                        "DELETE FROM session_cache WHERE cache_key = ?1",
                        params![cache_key],
                    )
                    .map_err(sqlite_error)?;

                let cached_at = Timestamp::now();
                {
                    let mut statement = transaction
                        .prepare(
                            "INSERT INTO session_cache
                               (cache_key, file_path, mtime_ms, change_ms, size, session_json, cached_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        )
                        .map_err(sqlite_error)?;
                    for (file_path, (mtime_ms, change_ms, size, json)) in &deduplicated {
                        // SQLite 的整数是 i64，rusqlite 未为 u64 实现 ToSql。
                        // 文件大小不可能接近 i64::MAX，溢出时饱和而不是回绕成负数。
                        let size = i64::try_from(*size).unwrap_or(i64::MAX);
                        statement
                            .execute(params![
                                cache_key,
                                file_path,
                                mtime_ms,
                                change_ms,
                                size,
                                json,
                                cached_at.as_str()
                            ])
                            .map_err(sqlite_error)?;
                    }
                }

                let pruned = prune(&transaction)?;
                transaction.commit().map_err(sqlite_error)?;
                Ok((deduplicated.len(), pruned))
            })
            .await
    }

    /// 从某个缓存键中移除指定文件的记录（文件已删除或变更时调用）。
    pub async fn remove_entries(&self, cache_key: &str, file_paths: &[String]) -> Result<()> {
        if file_paths.is_empty() {
            return Ok(());
        }
        let cache_key = cache_key.to_owned();
        let file_paths = file_paths.to_vec();
        self.db
            .write(move |connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;
                {
                    let mut statement = transaction
                        .prepare(
                            "DELETE FROM session_cache WHERE cache_key = ?1 AND file_path = ?2",
                        )
                        .map_err(sqlite_error)?;
                    for file_path in &file_paths {
                        statement
                            .execute(params![cache_key, file_path])
                            .map_err(sqlite_error)?;
                    }
                }
                transaction.commit().map_err(sqlite_error)?;
                Ok(())
            })
            .await
    }

    /// 按上限淘汰缓存，返回淘汰条数。
    pub async fn prune(&self) -> Result<i64> {
        self.db
            .write(|connection| {
                let transaction = connection.transaction().map_err(sqlite_error)?;
                let removed = prune(&transaction)?;
                transaction.commit().map_err(sqlite_error)?;
                Ok(removed)
            })
            .await
    }

    /// 读取缓存统计（诊断用）。
    pub async fn stats(&self) -> Result<CacheStats> {
        self.db
            .read(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*),
                                COUNT(DISTINCT cache_key),
                                COALESCE(SUM(LENGTH(CAST(session_json AS BLOB))), 0)
                         FROM session_cache",
                        [],
                        |row| {
                            Ok(CacheStats {
                                entries: row.get(0)?,
                                cache_keys: row.get(1)?,
                                payload_bytes: row.get(2)?,
                            })
                        },
                    )
                    .map_err(sqlite_error)
            })
            .await
    }
}

/// 在给定事务/连接上执行淘汰。
fn prune(connection: &Connection) -> Result<i64> {
    let stats = connection
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(LENGTH(CAST(session_json AS BLOB))), 0) FROM session_cache",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(sqlite_error)?;

    let (mut entries, mut bytes) = stats;
    if entries <= MAX_ENTRIES && bytes <= MAX_TOTAL_BYTES {
        return Ok(0);
    }

    // 最旧优先。`cached_at` 相同时用 rowid 保证顺序稳定。
    let mut statement = connection
        .prepare(
            "SELECT rowid, LENGTH(CAST(session_json AS BLOB)) AS payload_bytes
             FROM session_cache
             ORDER BY cached_at ASC, rowid ASC",
        )
        .map_err(sqlite_error)?;
    let candidates = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(sqlite_error)?;

    let mut removed = 0;
    let mut delete = connection
        .prepare("DELETE FROM session_cache WHERE rowid = ?1")
        .map_err(sqlite_error)?;
    for candidate in candidates {
        if entries <= MAX_ENTRIES && bytes <= MAX_TOTAL_BYTES {
            break;
        }
        let (rowid, payload_bytes) = candidate.map_err(sqlite_error)?;
        delete.execute(params![rowid]).map_err(sqlite_error)?;
        entries -= 1;
        bytes -= payload_bytes;
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    // 缓存指纹是精确键值，测试中的断言同样必须用精确比较。
    #![allow(clippy::float_cmp)]

    use super::*;
    use crate::db::Db;
    use aivc_core::id::SessionId;
    use aivc_core::session::{Message, Role};

    async fn repo() -> (tempfile::TempDir, SessionCacheRepo) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, SessionCacheRepo::new(db))
    }

    fn session(id: &str, title: &str) -> Session {
        Session {
            id: SessionId::from(id),
            title: title.to_owned(),
            source_title: None,
            cwd: Some("/work".to_owned()),
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
            preview: vec![Message {
                role: Role::User,
                text: title.to_owned(),
                timestamp: None,
                source_line: None,
            }],
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    fn entry(id: &str, title: &str, mtime: f64, change: f64, size: u64) -> SessionCacheEntry {
        SessionCacheEntry {
            file_path: format!("/sessions/{id}.jsonl"),
            mtime_ms: mtime,
            change_ms: change,
            size,
            session: session(id, title),
        }
    }

    #[test]
    fn freshness_requires_all_three_fingerprint_fields() {
        let item = entry("a", "标题", 100.0, 200.0, 300);
        assert!(item.is_fresh_for(100.0, 200.0, 300));
        // 任一字段不同都必须判定为过期——尤其是 change_ms：
        // 跨环境复制会保留 mtime 与 size，只有 ctime 能反映内容已变。
        assert!(!item.is_fresh_for(101.0, 200.0, 300));
        assert!(!item.is_fresh_for(100.0, 201.0, 300));
        assert!(!item.is_fresh_for(100.0, 200.0, 301));
    }

    #[tokio::test]
    async fn read_returns_empty_for_unknown_cache_key() {
        let (_dir, repo) = repo().await;
        assert!(repo.read("missing").await.expect("read").is_empty());
    }

    #[tokio::test]
    async fn replace_then_read_round_trips_all_fields() {
        let (_dir, repo) = repo().await;
        let entries = vec![
            entry("a", "第一条", 1.0, 2.0, 30),
            entry("b", "第二条", 3.0, 4.0, 40),
        ];

        let (written, pruned) = repo.replace("local", &entries).await.expect("replace");
        assert_eq!(written, 2);
        assert_eq!(pruned, 0);

        let read = repo.read("local").await.expect("read");
        assert_eq!(read.len(), 2);
        let first = read.get("/sessions/a.jsonl").expect("entry a");
        assert_eq!(first.session.title, "第一条");
        assert_eq!(first.mtime_ms, 1.0);
        assert_eq!(first.change_ms, 2.0);
        assert_eq!(first.size, 30);
        assert_eq!(first.session.preview.len(), 1);
    }

    #[tokio::test]
    async fn replace_is_scoped_to_one_cache_key() {
        let (_dir, repo) = repo().await;
        repo.replace("local", &[entry("a", "本机", 1.0, 1.0, 1)])
            .await
            .expect("replace local");
        repo.replace("wsl:Ubuntu", &[entry("b", "WSL", 1.0, 1.0, 1)])
            .await
            .expect("replace wsl");

        assert_eq!(repo.read("local").await.expect("read").len(), 1);
        assert_eq!(repo.read("wsl:Ubuntu").await.expect("read").len(), 1);
        // 替换某个键不得影响另一个键。
        repo.replace("local", &[]).await.expect("clear local");
        assert!(repo.read("local").await.expect("read").is_empty());
        assert_eq!(repo.read("wsl:Ubuntu").await.expect("read").len(), 1);
    }

    #[tokio::test]
    async fn replace_removes_entries_that_are_no_longer_present() {
        let (_dir, repo) = repo().await;
        repo.replace(
            "local",
            &[
                entry("a", "旧", 1.0, 1.0, 1),
                entry("b", "保留", 1.0, 1.0, 1),
            ],
        )
        .await
        .expect("first");
        // 第二次只提交 b：a 必须消失（整体替换语义，不是增量合并）。
        repo.replace("local", &[entry("b", "保留", 1.0, 1.0, 1)])
            .await
            .expect("second");

        let read = repo.read("local").await.expect("read");
        assert_eq!(read.len(), 1);
        assert!(read.contains_key("/sessions/b.jsonl"));
    }

    #[tokio::test]
    async fn duplicate_file_paths_within_one_batch_do_not_fail() {
        let (_dir, repo) = repo().await;
        // 同一批里出现重复主键时，后者覆盖前者而不是让整批写入失败。
        let (written, _) = repo
            .replace(
                "local",
                &[
                    entry("a", "旧标题", 1.0, 1.0, 1),
                    entry("a", "新标题", 2.0, 2.0, 2),
                ],
            )
            .await
            .expect("replace");
        assert_eq!(written, 1);

        let read = repo.read("local").await.expect("read");
        assert_eq!(
            read.get("/sessions/a.jsonl").expect("entry").session.title,
            "新标题"
        );
    }

    #[tokio::test]
    async fn oversized_entries_are_skipped_without_failing_the_batch() {
        let (_dir, repo) = repo().await;
        let mut huge = entry("huge", "小标题", 1.0, 1.0, 1);
        // 用超长预览把序列化体积推过单条上限。
        huge.session.preview = vec![Message {
            role: Role::User,
            text: "字".repeat(MAX_ENTRY_BYTES),
            timestamp: None,
            source_line: None,
        }];

        let (written, _) = repo
            .replace("local", &[huge, entry("ok", "正常", 1.0, 1.0, 1)])
            .await
            .expect("replace");
        assert_eq!(written, 1, "超限条目应被跳过");

        let read = repo.read("local").await.expect("read");
        assert!(read.contains_key("/sessions/ok.jsonl"));
        assert!(!read.contains_key("/sessions/huge.jsonl"));
    }

    #[tokio::test]
    async fn remove_entries_deletes_only_the_named_files_in_the_named_key() {
        let (_dir, repo) = repo().await;
        repo.replace(
            "local",
            &[entry("a", "A", 1.0, 1.0, 1), entry("b", "B", 1.0, 1.0, 1)],
        )
        .await
        .expect("replace");

        repo.remove_entries("local", &["/sessions/a.jsonl".to_owned()])
            .await
            .expect("remove");

        let read = repo.read("local").await.expect("read");
        assert_eq!(read.len(), 1);
        assert!(read.contains_key("/sessions/b.jsonl"));
    }

    #[tokio::test]
    async fn remove_entries_with_empty_input_is_a_no_op() {
        let (_dir, repo) = repo().await;
        repo.replace("local", &[entry("a", "A", 1.0, 1.0, 1)])
            .await
            .expect("replace");
        repo.remove_entries("local", &[]).await.expect("no-op");
        assert_eq!(repo.read("local").await.expect("read").len(), 1);
    }

    #[tokio::test]
    async fn corrupted_rows_are_skipped_individually() {
        let (dir, repo) = repo().await;
        repo.replace(
            "local",
            &[entry("a", "A", 1.0, 1.0, 1), entry("b", "B", 1.0, 1.0, 1)],
        )
        .await
        .expect("replace");

        // 外部把某一条 JSON 写坏：只有该条消失，其余照常返回。
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        db.write(|connection| {
            connection
                .execute(
                    "UPDATE session_cache SET session_json = 'not json' WHERE file_path = '/sessions/a.jsonl'",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("corrupt");

        let read = repo.read("local").await.expect("read");
        assert_eq!(read.len(), 1);
        assert!(read.contains_key("/sessions/b.jsonl"));
    }

    #[tokio::test]
    async fn stats_report_entries_keys_and_byte_payload() {
        let (_dir, repo) = repo().await;
        repo.replace("local", &[entry("a", "中文标题", 1.0, 1.0, 1)])
            .await
            .expect("replace");
        repo.replace("wsl:Ubuntu", &[entry("b", "另一个", 1.0, 1.0, 1)])
            .await
            .expect("replace");

        let stats = repo.stats().await.expect("stats");
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.cache_keys, 2);
        // 按字节统计：中文标题的载荷必须显著大于其字符数。
        assert!(stats.payload_bytes > 100, "载荷字节数异常：{stats:?}");
    }

    #[tokio::test]
    async fn prune_evicts_the_oldest_entries_first() {
        let (_dir, repo) = repo().await;

        // 直接构造超限数据：写入 4,050 条极小的记录。
        let entries: Vec<SessionCacheEntry> = (0..(MAX_ENTRIES + 50))
            .map(|index| entry(&format!("s{index}"), "t", 1.0, 1.0, 1))
            .collect();
        repo.replace("local", &entries).await.expect("replace");

        let stats = repo.stats().await.expect("stats");
        assert!(
            stats.entries <= MAX_ENTRIES,
            "淘汰后仍超过条数上限：{stats:?}"
        );

        // 同一批写入的 cached_at 相同，淘汰按 rowid 升序，因此留下的是**后**写入的。
        let read = repo.read("local").await.expect("read");
        assert!(read.contains_key(&format!("/sessions/s{}.jsonl", MAX_ENTRIES + 49)));
        assert!(!read.contains_key("/sessions/s0.jsonl"));
    }

    #[tokio::test]
    async fn explicit_prune_is_a_no_op_when_under_limits() {
        let (_dir, repo) = repo().await;
        repo.replace("local", &[entry("a", "A", 1.0, 1.0, 1)])
            .await
            .expect("replace");
        assert_eq!(repo.prune().await.expect("prune"), 0);
    }

    #[tokio::test]
    async fn concurrent_replaces_for_different_keys_do_not_interfere() {
        let (_dir, repo) = repo().await;
        let repo = std::sync::Arc::new(repo);

        let mut handles = Vec::new();
        for index in 0..16 {
            let repo = std::sync::Arc::clone(&repo);
            handles.push(tokio::spawn(async move {
                let key = format!("key-{index}");
                repo.replace(&key, &[entry(&format!("s{index}"), "t", 1.0, 1.0, 1)])
                    .await
                    .expect("replace");
                repo.read(&key).await.expect("read").len()
            }));
        }
        for handle in handles {
            assert_eq!(handle.await.expect("join"), 1);
        }

        let stats = repo.stats().await.expect("stats");
        assert_eq!(stats.entries, 16);
        assert_eq!(stats.cache_keys, 16);
    }
}
