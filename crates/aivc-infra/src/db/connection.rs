//! 数据库连接建立与 PRAGMA 配置。

use std::path::Path;

use aivc_core::error::Result;
use rusqlite::Connection;

use crate::db::migrations::sqlite_error;

/// SQLite 忙等超时（毫秒）。
///
/// 与现网一致。WAL 下读不阻塞写，但多个写者仍会竞争，超时避免直接抛 `SQLITE_BUSY`。
pub const BUSY_TIMEOUT_MS: i64 = 5_000;

/// 打开连接并应用 PRAGMA。
///
/// `journal_mode = WAL` 是**持久化**属性（写入数据库文件头），只需设置一次；
/// `busy_timeout` 与 `foreign_keys` 是**每连接**属性，因此每次打开都要设置。
pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path).map_err(sqlite_error)?;
    configure(&connection)?;
    Ok(connection)
}

/// 打开内存库（测试与诊断用）。
pub fn open_in_memory() -> Result<Connection> {
    let connection = Connection::open_in_memory().map_err(sqlite_error)?;
    configure(&connection)?;
    Ok(connection)
}

/// 应用 PRAGMA。
///
/// `journal_mode` 需要读取返回行（SQLite 会回显生效的模式），因此用 `query_row`；
/// 另外两个不返回行，用 `execute_batch`。内存库不支持 WAL，会回显 `memory`，
/// 这不是错误，因此不校验回显值。
pub fn configure(connection: &Connection) -> Result<()> {
    let _mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(sqlite_error)?;
    connection
        .execute_batch(&format!(
            "PRAGMA busy_timeout = {BUSY_TIMEOUT_MS};
             PRAGMA foreign_keys = ON;"
        ))
        .map_err(sqlite_error)?;
    Ok(())
}

/// 读取当前生效的 journal 模式（诊断用）。
pub fn journal_mode(connection: &Connection) -> Result<String> {
    connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(sqlite_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;

    #[test]
    fn file_database_uses_wal_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let connection = open(&dir.path().join("test.db")).expect("open");
        assert_eq!(
            journal_mode(&connection).expect("mode").to_lowercase(),
            "wal"
        );
    }

    #[test]
    fn open_creates_missing_parent_directories() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("a").join("b").join("test.db");
        open(&nested).expect("open");
        assert!(nested.exists());
    }

    #[test]
    fn foreign_keys_are_enabled_on_every_connection() {
        // 每连接属性：不设置的话级联删除不会生效。
        let dir = tempfile::tempdir().expect("tempdir");
        let connection = open(&dir.path().join("test.db")).expect("open");
        let enabled: i64 = connection
            .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
            .expect("pragma");
        assert_eq!(enabled, 1);
    }

    #[test]
    fn memory_database_is_supported_for_tests() {
        let mut connection = open_in_memory().expect("open memory");
        migrations::apply(&mut connection).expect("migrate");
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert!(tables >= 11);
    }
}
