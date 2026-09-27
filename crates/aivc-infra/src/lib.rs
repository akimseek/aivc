//! 基础设施适配器：SQLite、本地文件系统、WSL、HTTP 客户端、配置与日志。
//!
//! 本 crate 实现 [`aivc_core::ports`] 定义的端口，并把外部世界的细节挡在领域层之外。
//! 它是**唯一**允许直接使用 `rusqlite` / `reqwest` / `wsl.exe` / 文件系统的内部 crate
//! （除 `aivc-terminal` 需要 `portable-pty` 之外）。
//!
//! # 模块与内部依赖规则
//!
//! ```text
//! logging  ← 无内部依赖（只被 lib.rs 的初始化函数使用）
//! config   ← 无内部依赖
//! fs       ← config（路径语义）
//! wsl      ← config
//! db       ← 无内部依赖
//! http     ← config
//! ```
//!
//! **规则：`fs` / `wsl` / `db` / `http` 之间不得互相依赖。** 需要跨适配器协作时，
//! 由上层（`aivc-provider` / `aivc-gateway`）编排，而不是让适配器彼此调用——否则
//! 会出现「文件系统适配器里藏着数据库访问」这类难以审计的耦合。
//!
//! 这条规则是**模块级**的，`cargo xtask check-layers` 只校验 crate 级依赖，
//! 因此由代码评审把关；把 `fs` 与 `db` 合并进同一 crate 正是为了减少 crate 数量，
//! 代价就是这条规则需要靠评审而非编译器保证。
//!
//! # 与现网的对应关系
//!
//! | 本模块 | 现网文件 |
//! | --- | --- |
//! | [`config`] | `core/application-paths.ts`、`core/main-helpers.ts`、`core/settings.ts` |
//! | [`db`] | `core/app-database.ts`、`vendors/vendor-manager.ts`（schema 部分）、`gateway/*-store.ts` |
//! | [`fs`] | `core/fs-utils.ts`、`core/line-reader.ts`、`providers/session-storage.ts`（本机分支） |
//! | [`wsl`] | `core/wsl.ts` |
//! | [`http`] | `gateway/vendor-balance.ts`、`vendors/vendor-manager.ts`（模型查询部分） |
//! | [`logging`] | `core/performance.ts`、`gateway/gateway-log.ts`（文件部分） |

#![forbid(unsafe_code)]

pub mod config;
pub mod db;
pub mod fs;
pub mod http;
pub mod logging;
pub mod wsl;

pub use config::{AppPaths, AppSettings, RawSettings, RuntimeRoot, RuntimeShape, SettingsStore};
pub use db::Db;
pub use fs::LocalSessionStore;
pub use wsl::{WslAdapter, WslConfig, WslSessionStore};
