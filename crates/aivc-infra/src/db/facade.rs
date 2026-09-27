//! 数据库门面：单写者 actor + 读连接池。
//!
//! # 并发模型
//!
//! ```text
//!   调用方 ──write()──► mpsc<WriteJob> ──► 专用 OS 线程（唯一写连接）
//!          ──read()───► Semaphore ──────► 读连接池（WAL 并发读）
//! ```
//!
//! 现网用一条全局 Promise 队列串行化全部写入（`BEGIN IMMEDIATE` / `COMMIT`），
//! 并且 `node:sqlite` 是**同步 API**，长事务会阻塞主进程事件循环，与网关热路径争用。
//!
//! 这里保留「写入全串行」的语义（唯一写连接 + 专用线程），但把它移出异步运行时：
//! 写线程用 `blocking_recv` 等待任务，读走连接池且与写不互斥（WAL 允许读写并发）。
//!
//! 写入串行化是**功能要求**，不是实现细节：现网的 `updateSettings`、
//! `replaceSessionCache` 等都依赖「读当前值 → 应用变更 → 落盘」不会互相覆盖。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use aivc_core::error::{DatabaseError, Result};
use rusqlite::Connection;
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::db::connection;
use crate::db::migrations;

/// 读连接池大小。
///
/// 桌面应用并发读很少（会话缓存、元数据、供应商、日志查询），4 条足以覆盖
/// 「列表加载 + 工作台统计 + 诊断弹窗」同时发生的场景，同时避免无谓的 fd 占用。
const READ_POOL_SIZE: usize = 4;

/// 写任务通道容量。写操作本身是短事务，少量缓冲即可。
const WRITE_CHANNEL_CAPACITY: usize = 64;

/// 写任务：在写连接上执行闭包。结果经 [`oneshot`] 回传，因此这里不带返回值。
type WriteJob = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// 数据库门面。
///
/// 克隆 [`Db`] 只克隆句柄（`mpsc::Sender` 与 `Arc<ReadPool>` 都是共享的），
/// 因此各仓储可以各自持有一份，无需外层再包 `Arc`。
#[derive(Debug, Clone)]
pub struct Db {
    path: PathBuf,
    writer: mpsc::Sender<WriteJob>,
    readers: Arc<ReadPool>,
}

impl Db {
    /// 打开数据库、执行迁移，并启动写线程与读池。
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();

        // 先在阻塞线程上建连接并迁移：失败要在这里暴露，而不是在写线程里静默死掉。
        let writer_path = path.clone();
        let writer_connection = tokio::task::spawn_blocking(move || {
            let mut connection = connection::open(&writer_path)?;
            migrations::apply(&mut connection)?;
            Ok::<Connection, aivc_core::Error>(connection)
        })
        .await
        .map_err(|error| DatabaseError::Sqlite {
            message: format!("初始化数据库线程失败：{error}"),
        })??;

        let (sender, mut receiver) = mpsc::channel::<WriteJob>(WRITE_CHANNEL_CAPACITY);
        // 写线程独立于 tokio 运行时：SQLite 是阻塞 API，放在工作线程上会拖慢整个运行时。
        std::thread::Builder::new()
            .name("aivc-db-writer".to_owned())
            .spawn(move || {
                let mut connection = writer_connection;
                while let Some(job) = receiver.blocking_recv() {
                    job(&mut connection);
                }
                // 通道关闭（Db 全部释放）后线程自然退出。
            })
            .map_err(|error| DatabaseError::Sqlite {
                message: format!("无法启动数据库写线程：{error}"),
            })?;

        Ok(Self {
            readers: Arc::new(ReadPool::new(path.clone())),
            path,
            writer: sender,
        })
    }

    /// 数据库文件路径。
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 在唯一写连接上执行闭包，返回其值。
    ///
    /// 闭包在专用写线程上运行，因此**不得**在其中执行异步操作或长时间阻塞。
    pub async fn write<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    {
        let (sender, receiver) = oneshot::channel::<Result<T>>();
        let job: WriteJob = Box::new(move |connection| {
            // 接收端被丢弃（调用方取消）时忽略发送失败。
            let _ = sender.send(operation(connection));
        });
        self.writer
            .send(job)
            .await
            .map_err(|_| DatabaseError::WriterClosed)?;
        receiver.await.map_err(|_| DatabaseError::WriterClosed)?
    }

    /// 在读连接上执行闭包。
    pub async fn read<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        self.readers.read(operation).await
    }

    /// 已应用的迁移版本。
    pub async fn schema_version(&self) -> Result<u32> {
        self.read(|connection| {
            let versions = migrations::applied_versions(connection)?;
            Ok(versions.into_iter().max().unwrap_or(0))
        })
        .await
    }
}

/// 读连接池。
///
/// 用空闲栈 + 信号量实现：信号量限制并发读数量，栈复用已打开的连接。
/// `rusqlite::Connection` 是 `!Sync`，因此必须用 `Mutex` 包住栈（而非包住连接本身），
/// 这样不同读任务可以真正并行使用各自的连接。
#[derive(Debug)]
struct ReadPool {
    path: PathBuf,
    idle: Mutex<Vec<Connection>>,
    permits: Semaphore,
}

impl ReadPool {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            idle: Mutex::new(Vec::new()),
            permits: Semaphore::new(READ_POOL_SIZE),
        }
    }

    async fn read<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let permit = self
            .permits
            .acquire()
            .await
            .map_err(|_| DatabaseError::WriterClosed)?;

        let reused = self.take_idle();
        let path = self.path.clone();

        let (returned, result) = tokio::task::spawn_blocking(move || {
            let connection = match reused {
                Some(connection) => connection,
                None => match connection::open(&path) {
                    Ok(connection) => connection,
                    Err(error) => return (None, Err(error)),
                },
            };
            let outcome = operation(&connection);
            // 读失败时连接状态未知，丢弃而不归还池，避免污染后续读取。
            let returned = outcome.is_ok().then_some(connection);
            (returned, outcome)
        })
        .await
        .map_err(|error| DatabaseError::Sqlite {
            message: format!("数据库读线程失败：{error}"),
        })?;

        if let Some(connection) = returned {
            self.put_idle(connection);
        }
        drop(permit);
        result
    }

    fn take_idle(&self) -> Option<Connection> {
        // 锁中毒说明某个持有者 panic 了；空闲栈本身仍是良构的，取回内部值继续用。
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }

    fn put_idle(&self, connection: Connection) {
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        // 极端情况下（并发下降）可能超池，直接丢弃多余连接即可。
        if idle.len() < READ_POOL_SIZE {
            idle.push(connection);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::latest_version;
    use crate::db::migrations::sqlite_error;
    use rusqlite::params;

    async fn file_db() -> (tempfile::TempDir, Db) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open(dir.path().join("aivc.db")).await.expect("open");
        (dir, db)
    }

    #[tokio::test]
    async fn open_runs_migrations_and_reports_schema_version() {
        let (dir, db) = file_db().await;
        assert_eq!(
            db.schema_version().await.expect("version"),
            latest_version()
        );
        assert!(dir.path().join("aivc.db").exists());
        assert_eq!(db.path(), dir.path().join("aivc.db"));
    }

    #[tokio::test]
    async fn write_returns_the_closure_value() {
        let (_dir, db) = file_db().await;
        let inserted = db
            .write(|connection| {
                connection
                    .execute(
                        "INSERT INTO workspace_presets (id, name, cwd, updated_at)
                         VALUES (?1, ?2, ?3, ?4)",
                        params!["p1", "预设", "/work", "2026-01-01T00:00:00.000Z"],
                    )
                    .map_err(sqlite_error)?;
                Ok(42)
            })
            .await
            .expect("write");
        assert_eq!(inserted, 42);
    }

    #[tokio::test]
    async fn read_sees_committed_writes() {
        let (_dir, db) = file_db().await;
        db.write(|connection| {
            connection
                .execute(
                    "INSERT INTO workspace_presets (id, name, cwd, updated_at)
                     VALUES ('p1', '预设', '/work', '2026-01-01T00:00:00.000Z')",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("write");

        let name: String = db
            .read(|connection| {
                connection
                    .query_row(
                        "SELECT name FROM workspace_presets WHERE id = 'p1'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)
            })
            .await
            .expect("read");
        assert_eq!(name, "预设");
    }

    #[tokio::test]
    async fn write_errors_are_propagated_and_do_not_kill_the_writer() {
        let (_dir, db) = file_db().await;

        // 违反唯一约束必须把错误回传给调用方。
        let first = db
            .write(|connection| {
                connection
                    .execute(
                        "INSERT INTO workspace_presets (id, name, cwd, updated_at) VALUES ('dup', 'a', '/a', 't')",
                        [],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await;
        assert!(first.is_ok());

        let second = db
            .write(|connection| {
                connection
                    .execute(
                        "INSERT INTO workspace_presets (id, name, cwd, updated_at) VALUES ('dup', 'b', '/b', 't')",
                        [],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await;
        assert!(second.is_err(), "重复主键必须失败");

        // 写线程必须仍然可用——现网的队列也要求单次失败不影响后续写入。
        let after = db
            .write(|connection| {
                connection
                    .execute(
                        "INSERT INTO workspace_presets (id, name, cwd, updated_at) VALUES ('ok', 'c', '/c', 't')",
                        [],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await;
        assert!(after.is_ok(), "写线程在错误后应继续工作");
    }

    #[tokio::test]
    async fn read_errors_are_propagated() {
        let (_dir, db) = file_db().await;
        let outcome = db
            .read(|connection| {
                connection
                    .query_row("SELECT * FROM table_that_does_not_exist", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .map_err(sqlite_error)
            })
            .await;
        assert!(outcome.is_err());
    }

    #[tokio::test]
    async fn concurrent_writes_are_serialized_without_lost_updates() {
        let (_dir, db) = file_db().await;
        db.write(|connection| {
            connection
                .execute(
                    "INSERT INTO workspace_presets (id, name, cwd, updated_at) VALUES ('counter', '0', '/c', 't')",
                    [],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
        .await
        .expect("seed");

        // 200 次「读-改-写」并发提交：若没有串行化，增量必然丢失。
        let db = Arc::new(db);
        let mut handles = Vec::new();
        for _ in 0..200 {
            let db = Arc::clone(&db);
            handles.push(tokio::spawn(async move {
                db.write(|connection| {
                    connection
                        .execute(
                            "UPDATE workspace_presets
                         SET name = CAST(CAST(name AS INTEGER) + 1 AS TEXT)
                         WHERE id = 'counter'",
                            [],
                        )
                        .map_err(sqlite_error)?;
                    Ok(())
                })
                .await
            }));
        }
        for handle in handles {
            handle.await.expect("join").expect("write");
        }

        let value: i64 = db
            .read(|connection| {
                connection
                    .query_row(
                        "SELECT CAST(name AS INTEGER) FROM workspace_presets WHERE id = 'counter'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)
            })
            .await
            .expect("read counter");
        assert_eq!(value, 200, "并发写入发生丢失更新");
    }

    #[tokio::test]
    async fn reads_run_concurrently_and_reuse_pooled_connections() {
        let (_dir, db) = file_db().await;
        let db = Arc::new(db);

        let mut handles = Vec::new();
        for _ in 0..32 {
            let db = Arc::clone(&db);
            handles.push(tokio::spawn(async move {
                db.read(|connection| {
                    connection
                        .query_row("SELECT 1", [], |row| row.get::<_, i64>(0))
                        .map_err(sqlite_error)
                })
                .await
            }));
        }
        for handle in handles {
            assert_eq!(handle.await.expect("join").expect("read"), 1);
        }
    }

    #[tokio::test]
    async fn reopening_an_existing_database_keeps_data() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("aivc.db");

        {
            let db = Db::open(&path).await.expect("open");
            db.write(|connection| {
                connection
                    .execute(
                        "INSERT INTO workspace_presets (id, name, cwd, updated_at) VALUES ('p1', 'n', '/w', 't')",
                        [],
                    )
                    .map_err(sqlite_error)?;
                Ok(())
            })
            .await
            .expect("write");
        }

        let db = Db::open(&path).await.expect("reopen");
        // 二次打开会重跑迁移流程，必须是幂等的。
        assert_eq!(
            db.schema_version().await.expect("version"),
            latest_version()
        );
        let count: i64 = db
            .read(|connection| {
                connection
                    .query_row("SELECT COUNT(*) FROM workspace_presets", [], |row| {
                        row.get(0)
                    })
                    .map_err(sqlite_error)
            })
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn every_read_gets_a_migrated_schema() {
        // 读池会按需新开连接，新连接必须已经能看到全部表（迁移在文件层面已落盘）。
        let (_dir, db) = file_db().await;
        let db = Arc::new(db);

        // 先占满读池，迫使后续读请求新建连接。
        let mut handles = Vec::new();
        for _ in 0..(READ_POOL_SIZE + 2) {
            let db = Arc::clone(&db);
            handles.push(tokio::spawn(async move {
                db.read(|connection| {
                    connection
                        .query_row(
                            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                            [],
                            |row| row.get::<_, i64>(0),
                        )
                        .map_err(sqlite_error)
                })
                .await
            }));
        }
        for handle in handles {
            let table_count = handle.await.expect("join").expect("read");
            assert!(table_count >= 11, "新连接缺少表：{table_count}");
        }
    }
}
