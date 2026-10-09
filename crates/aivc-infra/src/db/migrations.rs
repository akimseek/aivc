//! 版本化迁移与全新 schema。
//!
//! # 为什么自建迁移执行器而不用 `refinery`
//!
//! `refinery` 的 `rusqlite` 特性绑定特定 `rusqlite` 版本；一旦两者不匹配，Cargo 会
//! 引入第二个 `rusqlite` 副本，表现为难以定位的类型不兼容。本工程是**全新库**，
//! 当前只有 V1，迁移执行器不到百行且完全可控，因此不引入该依赖。
//!
//! # 与旧工程的关系
//!
//! 已确认决策：**不做旧库迁移**。本工程使用 `data/aivc.db`，与旧工程的 `app.db`
//! 并存互不干扰。因此 schema 从 V1 起就是干净设计，**没有**任何历史兼容列，也没有
//! 现网那种「`ALTER TABLE` 失败即清空重建」的隐式升级路径。
//!
//! # 相对现网的结构改进
//!
//! | 改进 | 理由 |
//! | --- | --- |
//! | `session_message_index` 以 `byte_offset` 为唯一锚点 | 现网以行号为锚，随机定位仍需从文件头逐行读到起始行；字节偏移可 `seek` 直接定位 |
//! | 新增 `(provider_id, created_at DESC)` 索引 | 诊断弹窗按 Provider 筛选，现网只有 `created_at` 与 `(vendor_id, created_at)` |
//! | 新增 `(outcome, created_at DESC)` 索引 | 异常诊断固定过滤 `outcome IN ('error','timeout')` |
//! | 新增分支父会话表达式索引 | 现网靠 `json_extract` 全表扫描反查子会话 |

use aivc_core::error::{DatabaseError, Result};
use aivc_core::time::Timestamp;
use rusqlite::{Connection, params};

/// 一条迁移。
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    /// 版本号，从 1 起严格递增。
    pub version: u32,
    /// 说明，写入 `schema_version` 便于事后审计。
    pub description: &'static str,
    /// 迁移 SQL，在单个事务内执行。
    pub sql: &'static str,
}

/// 全部迁移。**只允许追加**，已发布的条目不得修改。
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        description: "初始 schema：会话缓存/元数据/分页锚点、工作区预设、供应商、网关健康/请求日志/故障规则",
        sql: V1_INITIAL,
    },
    Migration {
        version: 2,
        description: "供应商余额快照表（现网在 vendor-balance.ts 中惰性创建）",
        sql: V2_VENDOR_BALANCE,
    },
    Migration {
        version: 3,
        description: "分页锚点补记绝对行号：字节偏移用于定位，行号是消息身份",
        sql: V3_ANCHOR_LINE_NUMBER,
    },
    Migration {
        version: 4,
        description: "网关请求日志增加上游尝试明细，保留一次外部请求一条统计记录",
        sql: "ALTER TABLE gateway_request_logs ADD COLUMN attempts_json TEXT NOT NULL DEFAULT '[]';",
    },
];

/// 当前最新 schema 版本。
#[must_use]
pub fn latest_version() -> u32 {
    MIGRATIONS.last().map_or(0, |migration| migration.version)
}

/// 在当前连接上执行全部未应用的迁移。
///
/// 每条迁移在独立事务内执行：失败即整条回滚，不会留下半套 schema。
pub fn apply(conn: &mut Connection) -> Result<()> {
    validate_migration_table()?;
    ensure_version_table(conn)?;

    let applied = applied_versions(conn)?;
    for migration in MIGRATIONS {
        if applied.contains(&migration.version) {
            continue;
        }
        let transaction = conn
            .transaction()
            .map_err(|error| migration_error(migration.version, error))?;
        transaction
            .execute_batch(migration.sql)
            .map_err(|error| migration_error(migration.version, error))?;
        transaction
            .execute(
                "INSERT INTO schema_version (version, description, applied_at) VALUES (?1, ?2, ?3)",
                params![
                    migration.version,
                    migration.description,
                    Timestamp::now().as_str()
                ],
            )
            .map_err(|error| migration_error(migration.version, error))?;
        transaction
            .commit()
            .map_err(|error| migration_error(migration.version, error))?;
    }
    Ok(())
}

/// 已应用的版本集合。
pub fn applied_versions(conn: &Connection) -> Result<std::collections::BTreeSet<u32>> {
    let mut statement = conn
        .prepare("SELECT version FROM schema_version")
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([], |row| row.get::<_, u32>(0))
        .map_err(sqlite_error)?;
    let mut versions = std::collections::BTreeSet::new();
    for row in rows {
        versions.insert(row.map_err(sqlite_error)?);
    }
    Ok(versions)
}

/// 校验迁移表本身是良构的：版本从 1 起、严格递增、无重复。
///
/// 这是编译期常量表的自检，能在启动阶段立刻发现有人插错顺序或改了历史版本号。
fn validate_migration_table() -> Result<()> {
    let mut previous = 0;
    for migration in MIGRATIONS {
        if migration.version != previous + 1 {
            return Err(DatabaseError::Migration {
                version: migration.version,
                message: format!(
                    "迁移版本必须从 1 起严格递增：期望 {}，实际 {}",
                    previous + 1,
                    migration.version
                ),
            }
            .into());
        }
        previous = migration.version;
    }
    Ok(())
}

fn ensure_version_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version     INTEGER PRIMARY KEY,
            description TEXT NOT NULL,
            applied_at  TEXT NOT NULL
        );",
    )
    .map_err(sqlite_error)
}

// 这两个函数专用于 `map_err` 适配器。`map_err` 按定义以值传递错误，
// 若改为借用签名，每个调用点都要包一层闭包（`.map_err(|e| f(&e))`），
// 可读性下降而收益为零，因此显式豁免 `needless_pass_by_value`。
#[allow(clippy::needless_pass_by_value)]
fn migration_error(version: u32, error: rusqlite::Error) -> aivc_core::Error {
    DatabaseError::Migration {
        version,
        message: error.to_string(),
    }
    .into()
}

/// 把 `rusqlite` 错误收敛为领域错误。
///
/// 领域层不能依赖 `rusqlite`，因此无法用 `From` 实现自动转换，只能在此手工收敛。
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn sqlite_error(error: rusqlite::Error) -> aivc_core::Error {
    DatabaseError::Sqlite {
        message: error.to_string(),
    }
    .into()
}

/// V1：初始 schema。
const V1_INITIAL: &str = r"
-- 会话摘要缓存：列表页只读摘要，详情按需读取。
-- 缓存失效键是 (mtime_ms, change_ms, size) 三元组：Windows/WSL 之间复制文件会保留
-- mtime 与大小，只有 ctime 能反映内容已变，因此三者必须同时比对。
CREATE TABLE session_cache (
    cache_key    TEXT    NOT NULL,
    file_path    TEXT    NOT NULL,
    mtime_ms     REAL    NOT NULL,
    change_ms    REAL    NOT NULL DEFAULT 0,
    size         INTEGER NOT NULL,
    session_json TEXT    NOT NULL,
    cached_at    TEXT    NOT NULL,
    PRIMARY KEY (cache_key, file_path)
);
CREATE INDEX idx_session_cache_key ON session_cache(cache_key);
-- 淘汰策略按 cached_at 升序，需要该索引避免每次全表排序。
CREATE INDEX idx_session_cache_cached_at ON session_cache(cached_at);

-- 用户级会话元数据：自定义标题、分支来源、收藏、最近打开。
-- 只记录轻量状态，不保存会话正文，也不创建第二套全文索引。
CREATE TABLE session_metadata (
    target_id     TEXT NOT NULL,
    session_id    TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (target_id, session_id)
);
CREATE INDEX idx_session_metadata_updated_at ON session_metadata(updated_at);
-- 反查某会话的全部分支子会话，避免现网那种 json_extract 全表扫描。
CREATE INDEX idx_session_metadata_parent
    ON session_metadata(target_id, json_extract(metadata_json, '$.branch.parentSessionId'));

-- 消息分页锚点：只保存文件指纹与消息位置，不保存正文。
-- byte_offset 是唯一锚点（现网用行号，随机定位仍需线性读取）。
CREATE TABLE session_message_index (
    target_id      TEXT    NOT NULL,
    session_id     TEXT    NOT NULL,
    file_path      TEXT    NOT NULL,
    mtime_ms       REAL    NOT NULL,
    change_ms      REAL    NOT NULL DEFAULT 0,
    size           INTEGER NOT NULL,
    message_offset INTEGER NOT NULL,
    byte_offset    INTEGER NOT NULL,
    message_count  INTEGER NOT NULL,
    indexed_at     TEXT    NOT NULL,
    PRIMARY KEY (target_id, session_id, file_path, mtime_ms, size, message_offset)
);
CREATE INDEX idx_session_message_index_session
    ON session_message_index(target_id, session_id, indexed_at DESC);

-- 工作区预设。
CREATE TABLE workspace_presets (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    cwd         TEXT NOT NULL,
    target_kind TEXT,
    prompt      TEXT,
    cli_args    TEXT,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_workspace_presets_updated_at ON workspace_presets(updated_at);

-- 供应商。name_norm 与 sort 在**同一 Provider 内**唯一：不同 Provider 的排序互不干扰，
-- 也不会因为重名而混淆协议与认证头。
CREATE TABLE api_vendors (
    id              TEXT PRIMARY KEY,
    provider_id     TEXT NOT NULL,
    name            TEXT NOT NULL,
    name_norm       TEXT NOT NULL,
    api_key         TEXT NOT NULL,
    api_base_url    TEXT NOT NULL,
    input_price_usd REAL,
    output_price_usd REAL,
    sort            INTEGER NOT NULL DEFAULT 0,
    enabled         INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    last_enabled_at TEXT
);
CREATE UNIQUE INDEX idx_api_vendors_provider_name_norm ON api_vendors(provider_id, name_norm);
CREATE UNIQUE INDEX idx_api_vendors_provider_sort ON api_vendors(provider_id, sort);

-- 供应商要写入用户 CLI 的配置模板（启用时落盘，写前备份）。
CREATE TABLE api_vendor_configs (
    id          TEXT PRIMARY KEY,
    vendor_id   TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    label       TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    target_path TEXT NOT NULL,
    content     TEXT NOT NULL,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (vendor_id) REFERENCES api_vendors(id) ON DELETE CASCADE
);
CREATE INDEX idx_api_vendor_configs_vendor_id ON api_vendor_configs(vendor_id, sort_order);

-- 模型列表与余额查询配置（按供应商一条）。
CREATE TABLE api_vendor_query_configs (
    vendor_id          TEXT PRIMARY KEY,
    model_query_json   TEXT,
    balance_query_json TEXT,
    FOREIGN KEY (vendor_id) REFERENCES api_vendors(id) ON DELETE CASCADE
);

-- 网关供应商健康状态：熔断状态机的持久化视图。
-- 现网语义是「内存 Map 为准 + 节流落盘」，本表同样只是快照。
CREATE TABLE gateway_vendor_health (
    vendor_id           TEXT PRIMARY KEY,
    provider_id         TEXT NOT NULL,
    status              TEXT NOT NULL DEFAULT 'healthy',
    failure_count       INTEGER NOT NULL DEFAULT 0,
    success_count       INTEGER NOT NULL DEFAULT 0,
    failure_rate        REAL NOT NULL DEFAULT 0,
    last_failure_at     TEXT,
    last_success_at     TEXT,
    circuit_until       TEXT,
    last_failure_reason TEXT
);
CREATE INDEX idx_gateway_vendor_health_provider ON gateway_vendor_health(provider_id, status);

-- 网关请求日志：用量与费用统计的唯一数据源（不引入第二套统计）。
CREATE TABLE gateway_request_logs (
    request_id      TEXT PRIMARY KEY,
    route_id        TEXT NOT NULL,
    provider_id     TEXT NOT NULL,
    vendor_id       TEXT NOT NULL,
    method          TEXT NOT NULL,
    path            TEXT NOT NULL,
    model           TEXT,
    upstream_status INTEGER,
    outcome         TEXT NOT NULL,
    duration_ms     INTEGER NOT NULL,
    bytes_in        INTEGER NOT NULL DEFAULT 0,
    bytes_out       INTEGER NOT NULL DEFAULT 0,
    retry_count     INTEGER NOT NULL DEFAULT 0,
    switched        INTEGER NOT NULL DEFAULT 0,
    error_code      TEXT,
    error_message   TEXT,
    usage_json      TEXT,
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_gateway_request_logs_created_at ON gateway_request_logs(created_at DESC);
CREATE INDEX idx_gateway_request_logs_vendor ON gateway_request_logs(vendor_id, created_at DESC);
-- 现网缺失：诊断弹窗按 Provider 筛选。
CREATE INDEX idx_gateway_request_logs_provider ON gateway_request_logs(provider_id, created_at DESC);
-- 现网缺失：异常诊断固定过滤 outcome IN ('error','timeout')。
CREATE INDEX idx_gateway_request_logs_outcome ON gateway_request_logs(outcome, created_at DESC);

-- 网关故障识别规则。内置默认规则是编译期常量，本表只存用户自定义规则。
CREATE TABLE gateway_failover_rules (
    id                   TEXT PRIMARY KEY,
    scope                TEXT NOT NULL,
    provider_id          TEXT,
    vendor_id            TEXT,
    pattern              TEXT NOT NULL DEFAULT '',
    status_codes         TEXT NOT NULL DEFAULT '',
    custom_response_status INTEGER,
    custom_response_body TEXT,
    enabled              INTEGER NOT NULL DEFAULT 1,
    created_at           TEXT NOT NULL,
    updated_at           TEXT NOT NULL
);
CREATE INDEX idx_gateway_failover_rules_scope
    ON gateway_failover_rules(scope, provider_id, vendor_id, enabled);
";

/// V2：供应商余额快照。
///
/// 现网把这张表放在 `vendor-balance.ts` 里惰性创建（`ensureBalanceSchema`），导致
/// 「表是否存在」取决于是否调用过余额功能——`deleteApiVendor` 甚至必须 try/catch。
/// 纳入版本化迁移后，这种隐式依赖被彻底消除。
const V2_VENDOR_BALANCE: &str = r"
CREATE TABLE api_vendor_balance_snapshots (
    vendor_id     TEXT PRIMARY KEY,
    remaining     REAL,
    total         REAL,
    used          REAL,
    unit          TEXT,
    plan_name     TEXT,
    is_valid      INTEGER NOT NULL DEFAULT 1,
    status        TEXT NOT NULL DEFAULT 'idle',
    error_message TEXT,
    queried_at    TEXT
);
CREATE INDEX idx_vendor_balance_queried_at ON api_vendor_balance_snapshots(queried_at);
";

/// V3：分页锚点补记**绝对行号**。
///
/// V1 只存了 `byte_offset`（用于 `seek` 定位），但详情返回的 `source_line` 必须是原始
/// JSONL 的绝对行号——「从此处分支」依赖它定位轮次。若从字节偏移起读后按相对值计数
/// 行号，分页越靠后偏移越大，分支位置就会漂移。因此行号与字节偏移都要存：
/// 前者是身份，后者是定位手段。
const V3_ANCHOR_LINE_NUMBER: &str = r"
ALTER TABLE session_message_index ADD COLUMN line_number INTEGER NOT NULL DEFAULT 1;
";

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn memory_connection() -> Connection {
        Connection::open_in_memory().expect("open in-memory db")
    }

    #[test]
    fn migration_table_starts_at_one_and_is_contiguous() {
        assert_eq!(MIGRATIONS[0].version, 1);
        validate_migration_table().expect("迁移表必须良构");
        assert_eq!(latest_version(), 4);
    }

    #[test]
    fn apply_creates_every_table_and_records_the_version() {
        let mut conn = memory_connection();
        apply(&mut conn).expect("apply migrations");

        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .expect("prepare");
        let tables: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .filter_map(std::result::Result::ok)
            .collect();

        for expected in [
            "api_vendor_configs",
            "api_vendor_query_configs",
            "api_vendors",
            "gateway_failover_rules",
            "gateway_request_logs",
            "gateway_vendor_health",
            "schema_version",
            "session_cache",
            "session_metadata",
            "session_message_index",
            "workspace_presets",
        ] {
            assert!(
                tables.iter().any(|name| name == expected),
                "缺少表 {expected}，实际：{tables:?}"
            );
        }

        let versions = applied_versions(&conn).expect("applied versions");
        assert_eq!(versions, std::collections::BTreeSet::from([1, 2, 3, 4]));
    }

    #[test]
    fn apply_is_idempotent() {
        let mut conn = memory_connection();
        apply(&mut conn).expect("first apply");
        // 二次执行不得报错，也不得重复写入版本记录。
        apply(&mut conn).expect("second apply");
        let versions = applied_versions(&conn).expect("applied versions");
        assert_eq!(versions.len(), MIGRATIONS.len());
    }

    #[test]
    fn attempt_details_migration_keeps_existing_request_logs() {
        let conn = memory_connection();
        conn.execute_batch(V1_INITIAL).expect("v1");
        conn.execute("INSERT INTO gateway_request_logs (request_id, route_id, provider_id, vendor_id, method, path, upstream_status, outcome, duration_ms, created_at) VALUES ('existing', 'external-claude', 'claude', 'v1', 'POST', '/claude/v1/messages', 400, 'ok', 100, '2026-10-08T01:30:00.000Z')", []).expect("log");
        conn.execute_batch(MIGRATIONS[3].sql).expect("v4");
        let (outcome, status, attempts): (String, u16, String) = conn.query_row("SELECT outcome, upstream_status, attempts_json FROM gateway_request_logs WHERE request_id = 'existing'", [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).expect("preserved");
        assert_eq!(outcome, "ok");
        assert_eq!(status, 400);
        assert_eq!(attempts, "[]");
    }

    #[test]
    fn message_index_stores_both_byte_offset_and_absolute_line_number() {
        // 两者职责不同，缺一不可：
        // - byte_offset 是**定位手段**（本地文件可 seek，分页 O(1)）；
        // - line_number 是**消息身份**（详情返回的 sourceLine 必须是绝对行号，
        //   「从此处分支」依赖它定位轮次）。
        // 现网只有行号，因此定位是线性的。
        let mut conn = memory_connection();
        apply(&mut conn).expect("apply migrations");

        let mut statement = conn
            .prepare("PRAGMA table_info(session_message_index)")
            .expect("prepare");
        let columns: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query")
            .filter_map(std::result::Result::ok)
            .collect();

        assert!(columns.iter().any(|name| name == "byte_offset"));
        assert!(
            columns.iter().any(|name| name == "line_number"),
            "锚点必须同时记录绝对行号：{columns:?}"
        );
    }

    #[test]
    fn vendor_uniqueness_is_scoped_per_provider() {
        let mut conn = memory_connection();
        apply(&mut conn).expect("apply migrations");

        let insert = |conn: &Connection, id: &str, provider: &str, name_norm: &str, sort: i64| {
            conn.execute(
                "INSERT INTO api_vendors
                   (id, provider_id, name, name_norm, api_key, api_base_url, sort, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, '', '', ?5, 0, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                params![id, provider, name_norm, name_norm, sort],
            )
        };

        insert(&conn, "a", "codex", "same", 1).expect("first codex vendor");
        // 同一 Provider 内重名必须被拒绝。
        assert!(insert(&conn, "b", "codex", "same", 2).is_err());
        // 同一 Provider 内 sort 重复必须被拒绝。
        assert!(insert(&conn, "c", "codex", "other", 1).is_err());
        // 不同 Provider 可以使用相同的名字与排序。
        insert(&conn, "d", "claude", "same", 1).expect("claude vendor may reuse name and sort");
    }

    #[test]
    fn branch_parent_lookup_uses_an_expression_index() {
        let mut conn = memory_connection();
        apply(&mut conn).expect("apply migrations");

        // 索引必须真实存在（表达式索引依赖 SQLite 允许 json_extract 作为索引表达式）。
        let index_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_session_metadata_parent'",
                [],
                |row| row.get(0),
            )
            .expect("index lookup");
        assert_eq!(index_exists, 1, "父会话表达式索引未创建");

        // 插入足够多的行，让成本模型有理由选择索引而不是全表扫描
        // （单行表上全表扫描确实更便宜，此时断言计划没有意义）。
        let transaction = conn.transaction().expect("begin");
        {
            let mut statement = transaction
                .prepare(
                    "INSERT INTO session_metadata (target_id, session_id, metadata_json, updated_at)
                     VALUES (?1, ?2, ?3, '2026-01-01T00:00:00.000Z')",
                )
                .expect("prepare");
            for index in 0..600 {
                let metadata = if index == 599 {
                    r#"{"branch":{"parentSessionId":"parent-1"}}"#.to_owned()
                } else {
                    format!(r#"{{"branch":{{"parentSessionId":"parent-{index}"}}}}"#)
                };
                statement
                    .execute(params!["local", format!("child-{index}"), metadata])
                    .expect("insert");
            }
        }
        transaction.commit().expect("commit");

        // 反查子会话必须走索引而不是全表扫描。
        // 注意：`EXPLAIN QUERY PLAN` 的输出列数随 SQLite 版本变化
        // （3.24 起为 id/parent/notused/detail），因此按列名读取而不是下标。
        let plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT session_id FROM session_metadata
                 WHERE target_id = 'local'
                   AND json_extract(metadata_json, '$.branch.parentSessionId') = 'parent-1'",
                [],
                |row| row.get("detail"),
            )
            .expect("query plan");
        assert!(
            plan.contains("idx_session_metadata_parent"),
            "未命中父会话索引：{plan}"
        );

        let child: String = conn
            .query_row(
                "SELECT session_id FROM session_metadata
                 WHERE target_id = 'local'
                   AND json_extract(metadata_json, '$.branch.parentSessionId') = 'parent-1'",
                [],
                |row| row.get(0),
            )
            .expect("lookup child");
        assert_eq!(child, "child-1");
    }

    #[test]
    fn diagnostics_indexes_exist_for_provider_and_outcome_filters() {
        let mut conn = memory_connection();
        apply(&mut conn).expect("apply migrations");

        let mut statement = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'gateway_request_logs'")
            .expect("prepare");
        let indexes: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query")
            .filter_map(std::result::Result::ok)
            .collect();

        assert!(indexes.iter().any(|name| name.contains("provider")));
        assert!(indexes.iter().any(|name| name.contains("outcome")));
    }

    #[test]
    fn vendor_configs_cascade_on_vendor_delete() {
        let mut conn = memory_connection();
        apply(&mut conn).expect("apply migrations");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("enable foreign keys");

        conn.execute(
            "INSERT INTO api_vendors
               (id, provider_id, name, name_norm, api_key, api_base_url, sort, enabled, created_at, updated_at)
             VALUES ('v1', 'codex', 'n', 'n', 'k', 'https://x', 1, 1, 't', 't')",
            [],
        )
        .expect("insert vendor");
        conn.execute(
            "INSERT INTO api_vendor_configs (id, vendor_id, provider_id, target_path, content)
             VALUES ('c1', 'v1', 'codex', '~/.codex/config.toml', 'x')",
            [],
        )
        .expect("insert config");

        conn.execute("DELETE FROM api_vendors WHERE id = 'v1'", [])
            .expect("delete vendor");

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM api_vendor_configs", [], |row| {
                row.get(0)
            })
            .expect("count");
        assert_eq!(remaining, 0, "删除供应商应级联删除其配置模板");
    }
}
