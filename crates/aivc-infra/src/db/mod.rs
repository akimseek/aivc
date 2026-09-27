//! SQLite 适配器：全新 schema、版本化迁移、单写者 actor、读连接池、仓储。
//!
//! 全新库 `data/aivc.db`，schema 从 `V1` 起干净设计（**不**迁移旧工程的 `app.db`，
//! 两者并存互不干扰）。因此这里没有任何历史兼容列，也没有现网那种
//! 「`ALTER TABLE` 失败即清空重建」的隐式升级路径。
//!
//! # 子模块
//!
//! - [`connection`]：连接建立与 PRAGMA（WAL / `busy_timeout` / 外键）。
//! - [`migrations`]：版本化迁移与 V1 schema。
//! - [`facade`]：门面。写入经单一 writer actor 串行化（与现网「全局串行写队列」
//!   语义同构），读取走连接池；两者都在阻塞线程上执行，不再阻塞异步运行时。
//! - [`repos`]：按聚合划分的仓储实现。
//!
//! # 相对现网的结构改进
//!
//! - `session_message_index` 以 `byte_offset` 为唯一锚点，随机定位不再需要线性读取；
//! - 补齐 `gateway_request_logs` 的 Provider 与 outcome 索引（现网诊断查询缺索引）；
//! - 分支父会话反查改用表达式索引，取代现网的 `json_extract` 全表扫描。
//!   副作用是 SQLite 会拒绝写入非法 JSON，等于给该列加了一道完整性护栏；
//! - 缓存总量按**字节**统计（现网用 `LENGTH()` 得到字符数，中文场景下实际放宽约 3 倍）；
//! - 元数据的「读-改-写」在写连接内一次完成，是原子的；现网是两次独立 `await`，
//!   并发更新同一会话的不同字段时会互相覆盖。

pub mod connection;
pub mod facade;
pub mod migrations;
pub mod repos;

pub use facade::Db;
pub use migrations::{MIGRATIONS, Migration, applied_versions, apply, latest_version};
pub use repos::failover_rule::FailoverRuleRepo;
pub use repos::gateway_health::GatewayHealthRepo;
pub use repos::gateway_request_log::{
    GatewayRequestLogRepo, LogEntry, LogFilter, RECENT_FAILURE_LIMIT,
};
pub use repos::message_index::{MessageIndexEntry, MessageIndexRepo, SessionIndexKey};
pub use repos::preset::WorkspacePresetRepo;
pub use repos::session_cache::{CacheStats, SessionCacheEntry, SessionCacheRepo};
pub use repos::session_metadata::{MetadataEntry, SessionMetadataRepo};
pub use repos::vendor::VendorRepo;
