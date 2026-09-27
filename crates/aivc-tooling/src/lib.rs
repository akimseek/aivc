//! Skill 管理与 CLI 环境探测/安装。
//!
//! 合并自原先独立的 `aivc-skills` 与 `aivc-cli-env`：两者都只依赖领域层与基础设施层，
//! 都没有重量级或独特的外部依赖，单独成 crate 只是增加样板而不带来边界收益。
//!
//! - [`skills`]：Codex Skill 的列表、导入、启停、回收站。对应现网 `skills/skills.ts`。
//! - [`cli_env`]：node / npm / nvm / 各平台 CLI 的版本探测与安装。
//!   对应现网 `cli/cli-installer.ts`。
//!
//! 两者共同的约束：WSL 侧必须用**登录 shell** 探测，因为 `nvm` / `fnm` 只在登录 shell
//! 注入 PATH。

#![forbid(unsafe_code)]

pub mod cli_env;
pub mod performance;
pub mod qoder_models;
pub mod skills;
pub mod vendor;
pub mod vendor_balance;
pub mod vendor_models;
pub mod vendor_query;
