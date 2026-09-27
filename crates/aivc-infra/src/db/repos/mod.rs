//! 仓储：按聚合把 SQL 访问收敛到独立模块。
//!
//! # 为什么是具体类型而不是端口 trait
//!
//! 架构文档最初为每个聚合列了端口 trait。落地时改为具体结构体，理由是：
//!
//! 1. 目前只有 SQLite 一种实现，抽象没有第二个实现者；
//! 2. 测试用**真实 SQLite**（`tempfile` + 真实 schema）比内存替身更有价值——
//!    它同时验证 SQL、索引、约束与事务语义，而替身会把这些全部绕过；
//! 3. 现网 DEV-GUIDE 已明确记录过同类结论：抽象必须能减少实际缺陷或改善可测性，
//!    否则只是增加回调与条件分支。
//!
//! 若将来确实需要第二实现（例如把会话缓存换成嵌入式 KV），再抽取 trait 也不迟——
//! 届时调用点已经稳定，抽取成本更低。

pub mod failover_rule;
pub mod gateway_health;
pub mod gateway_request_log;
pub mod message_index;
pub mod preset;
pub mod session_cache;
pub mod session_metadata;
pub mod vendor;
