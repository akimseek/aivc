//! Qoder CN 平台实现。
//!
//! 存储布局：`<config_dir>/projects/<项目键>/<会话编号>.jsonl`，回收站在
//! `<config_dir>/.visual-console-trash/projects/…` 下保持同样的结构。
//!
//! # 一份会话由多个产物组成
//!
//! 与其余三个平台不同，Qoder 的一份会话除了 transcript 还有若干**关联目录**
//! （`tasks/`、`file-history/`、`logs/sessions/`）。删除与恢复必须把它们作为一组
//! 原子处理，否则会留下孤儿目录。
//!
//! # 子模块
//!
//! - [`parser`]：会话 JSONL 解析与用量累加。

pub mod parser;
pub mod paths;
pub mod provider;
