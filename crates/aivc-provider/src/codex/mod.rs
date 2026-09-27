//! Codex 平台实现。
//!
//! Codex 是本工程里最复杂的一环：官方 `app-server` 分支、32 MB 读取上限、
//! 独立缓存体系与 `.visual-console-trash` 回收站。现网刻意**不**让它与其余三家共享
//! 控制流（DEV-GUIDE 已记录该结论），这里保持一致。
//!
//! # 子模块
//!
//! - [`parser`]：`rollout-*.jsonl` 解析（列表/详情两种模式）。
//! - [`listing`]：列表构建（缓存优先 + 有界并发）。
//! - [`paging`]：详情分页折叠（首屏语义、字节锚点、去重）。
//! - [`paths`]：存储布局、变更路径计算与目录边界围栏。

pub mod appserver;
pub mod branch;
pub mod listing;
pub mod parser;
pub mod paths;
pub mod provider;
