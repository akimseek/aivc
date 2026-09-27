//! Gemini CLI 平台实现。
//!
//! 存储布局：`<config_dir>/tmp/<项目标识>/chats/session-<时间戳>-<8 位十六进制>.jsonl`，
//! 回收站在 `<config_dir>/.visual-console-trash/tmp/…` 下保持同样的结构。
//!
//! # 子模块
//!
//! - [`parser`]：三种记录形态（元数据 / `$set` 补丁 / 单条消息）的解析与用量累加。

pub mod branch;
pub mod parser;
pub mod paths;
pub mod provider;
