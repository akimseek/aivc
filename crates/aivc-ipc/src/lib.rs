//! IPC 层：命令面与前端适配。
//!
//! # 职责
//!
//! - [`validate`]：所有入参的校验。IPC 是进程边界，渲染层的任何值都可能到达这里，
//!   因此**先校验再执行**。
//!
//! # 与前端的关系
//!
//! 前端（React + xterm.js）**不做改动**：命令名、参数形状与返回结构都与现网一致，
//! 由本层承担全部适配。因此这里的类型必须与 `src/shared/types.ts` 的线上形状逐字对齐。

#![forbid(unsafe_code)]

pub use sessions::{SessionCommands, TargetLookup};

pub use commands::{COMMAND_COUNT, CommandDomain, command_names, commands_of, domain_of};

/// 命令名是否已注册。
///
/// 供适配层校验「桥写的方法映射到了真实命令」——写错命令名不会在编译期暴露。
#[must_use]
pub fn is_known_command(command: &str) -> bool {
    commands::is_known(command)
}

pub mod cli;
pub mod commands;
pub use router::{CommandRouter, PENDING};

pub mod gateway;
pub mod http_fetcher;
pub mod route;
pub mod router;
pub mod sessions;
pub mod settings;
pub mod shell;
pub mod skills;
pub mod terminal;
pub mod validate;
pub mod vendor;
