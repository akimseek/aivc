//! 本地文件系统 `SessionStore` 适配器。
//!
//! 实现 [`aivc_core::ports::SessionStore`]：本机路径语义、`tokio::fs` 异步 I/O、
//! 流式逐行读取（支持从字节偏移或行号起读，且可提前停止，避免大 JSONL 整体载入内存）。
//!
//! 对应现网 `core/fs-utils.ts`、`core/line-reader.ts` 与 `providers/session-storage.ts`
//! 的本机分支。
//!
//! # 子模块
//!
//! - [`local`]：[`LocalSessionStore`]——端口实现。
//! - [`line_reader`]：同步逐行读取与头部读取，供 [`local`] 在阻塞上下文中调用。

pub mod line_reader;
pub mod local;

pub use line_reader::{read_head_blocking, read_lines_blocking};
pub use local::LocalSessionStore;
