//! Claude Code 平台实现。
//!
//! 存储布局：`~/.claude/projects/<编码后的工作目录>/<会话编号>.jsonl`，
//! 回收站在 `<配置目录>/.visual-console-trash/projects/…` 下保持同样的结构。
//!
//! # 子模块
//!
//! - [`parser`]：会话 JSONL 解析、用量累加与上下文窗口推断。

pub mod branch;
pub mod parser;
pub mod paths;
pub mod provider;
