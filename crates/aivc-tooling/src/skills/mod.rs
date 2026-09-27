//! Codex Skill 管理。
//!
//! 目录布局：`$CODEX_HOME/skills/<name>`；回收站 `<home>/.visual-console-trash/skills/<name>-<时间戳>`。
//!
//! 必须保持的现网语义：
//!
//! - 停用 = 目录重命名追加 `.disabled`，恢复时剥掉后缀；
//! - 导入要求目录内含 `SKILL.md`（≤2 MB），且必须为 YAML frontmatter 并含 `name` / `description`；
//! - 导入先写临时目录再原子换入，避免中途失败留下半个 Skill。

pub mod core;
pub mod import;
pub mod store;

pub use core::{
    MAX_SKILL_MD_BYTES, MAX_SKILL_NAME_CHARS, SkillMetadata, is_safe_skill_name,
    read_skill_metadata, require_safe_skill_name, skill_directory_name, strip_disabled_suffix,
    trash_skill_active_name,
};
