//! Claude 存储布局与变更路径。
//!
//! # 布局
//!
//! ```text
//! <config_dir>/                      默认 ~/.claude
//! ├── projects/<编码后的工作目录>/<会话编号>.jsonl
//! └── .visual-console-trash/
//!     └── projects/<编码后的工作目录>/<会话编号>.jsonl
//! ```
//!
//! 会话文件位于 `projects` 下的**第二层**（`<项目目录>/<文件>`），因此扫描时限定
//! `mindepth 2 / maxdepth 2`——项目目录名是工作目录的编码形式，可能包含任意字符，
//! 递归下去没有意义，只会误收备份文件。
//!
//! # 与 Codex 的差异
//!
//! Codex 的回收站保留 `sessions/` 前缀（相对 `codex_home`），Claude 保留的是
//! `projects/` 前缀（相对 `config_dir`）。两者的相对基准不同，但规则一致：
//! **保持源根目录下的相对结构**。
//!
//! # 拒绝文案
//!
//! Claude 的四类操作各有专属文案（与现网逐字一致），因此不能用共享的固定错误，
//! 必须把文案传给 [`relocate_session_path`]。

use aivc_core::error::Result;
use aivc_core::ports::StoreKind;

use crate::common::{assert_session_file_inside, join_path, relative_to, relocate_session_path};

/// 会话目录名。
pub const PROJECTS_DIR_NAME: &str = "projects";
/// 回收站目录名。
pub const TRASH_DIR_NAME: &str = ".visual-console-trash";
/// 会话文件扩展名。
pub const SESSION_FILE_SUFFIX: &str = ".jsonl";
/// 会话文件通配模式。
pub const SESSION_FILE_GLOB: &str = "*.jsonl";

/// 通用操作围栏文案（读取、删除等）。
pub const MESSAGE_OPERATE: &str = "拒绝操作 Claude 会话目录之外的文件";
/// 移入回收站的围栏文案。
pub const MESSAGE_TRASH: &str = "拒绝移动 Claude projects 目录之外的文件";
/// 恢复的围栏文案。
pub const MESSAGE_RESTORE: &str = "拒绝恢复 Claude 回收站之外的文件";

/// Claude 的存储布局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeLayout {
    config_dir: String,
    kind: StoreKind,
}

impl ClaudeLayout {
    /// 构造。
    #[must_use]
    pub fn new(config_dir: impl Into<String>, kind: StoreKind) -> Self {
        Self {
            config_dir: config_dir.into(),
            kind,
        }
    }

    /// 配置根目录（默认 `~/.claude`）。
    #[must_use]
    pub fn config_dir(&self) -> &str {
        &self.config_dir
    }

    /// 存储后端类型。
    #[must_use]
    pub const fn kind(&self) -> StoreKind {
        self.kind
    }

    /// 活跃会话根目录：`<config_dir>/projects`。
    #[must_use]
    pub fn projects_root(&self) -> String {
        join_path(self.kind, &self.config_dir, PROJECTS_DIR_NAME)
    }

    /// 回收站根目录：`<config_dir>/.visual-console-trash`。
    #[must_use]
    pub fn trash_root(&self) -> String {
        join_path(self.kind, &self.config_dir, TRASH_DIR_NAME)
    }

    /// 回收站会话根目录：`<config_dir>/.visual-console-trash/projects`。
    #[must_use]
    pub fn trash_projects_root(&self) -> String {
        join_path(self.kind, &self.trash_root(), PROJECTS_DIR_NAME)
    }

    /// 断言路径可用于通用操作（读取、复制源等）。
    pub fn assert_operable(&self, path: &str) -> Result<()> {
        assert_session_file_inside(path, &self.projects_root(), self.kind, MESSAGE_OPERATE)
    }

    /// 断言路径可读：活跃目录或回收站目录之内。
    pub fn assert_readable(&self, path: &str) -> Result<()> {
        if crate::common::is_strictly_inside(path, &self.projects_root(), self.kind)
            || crate::common::is_strictly_inside(path, &self.trash_projects_root(), self.kind)
        {
            return Ok(());
        }
        Err(aivc_core::error::SessionError::SessionPathNotAllowed {
            message: MESSAGE_OPERATE.to_owned(),
        }
        .into())
    }

    /// 会话文件对应的回收站路径。
    pub fn trash_path_for(&self, source: &str) -> Result<String> {
        relocate_session_path(
            source,
            &self.projects_root(),
            &self.trash_projects_root(),
            self.kind,
            MESSAGE_TRASH,
        )
    }

    /// 回收站文件恢复后的原路径。
    pub fn restore_path_for(&self, source: &str) -> Result<String> {
        relocate_session_path(
            source,
            &self.trash_projects_root(),
            &self.projects_root(),
            self.kind,
            MESSAGE_RESTORE,
        )
    }

    /// 分支/复制后的文件路径：与源文件同目录，文件名为 `<新编号>.jsonl`。
    ///
    /// 与 Codex 不同：Claude 的文件名就是会话编号，没有 `rollout-<时间戳>-` 前缀。
    pub fn derived_path_for(&self, source: &str, new_id: &str) -> Result<String> {
        // 源在回收站时，派生文件落到活跃目录的对应位置：分支与复制品不该留在回收站。
        let directory =
            if crate::common::is_strictly_inside(source, &self.projects_root(), self.kind) {
                let relative = relative_to(source, &self.projects_root()).ok_or_else(|| {
                    aivc_core::error::SessionError::SessionPathNotAllowed {
                        message: MESSAGE_OPERATE.to_owned(),
                    }
                })?;
                let parent = parent_of(&relative);
                if parent.is_empty() {
                    self.projects_root()
                } else {
                    join_path(self.kind, &self.projects_root(), &parent)
                }
            } else if crate::common::is_strictly_inside(
                source,
                &self.trash_projects_root(),
                self.kind,
            ) {
                let relative =
                    relative_to(source, &self.trash_projects_root()).ok_or_else(|| {
                        aivc_core::error::SessionError::SessionPathNotAllowed {
                            message: MESSAGE_OPERATE.to_owned(),
                        }
                    })?;
                let parent = parent_of(&relative);
                if parent.is_empty() {
                    self.projects_root()
                } else {
                    join_path(self.kind, &self.projects_root(), &parent)
                }
            } else {
                return Err(aivc_core::error::SessionError::SessionPathNotAllowed {
                    message: MESSAGE_OPERATE.to_owned(),
                }
                .into());
            };
        Ok(join_path(
            self.kind,
            &directory,
            &format!("{new_id}{SESSION_FILE_SUFFIX}"),
        ))
    }

    /// 判断文件名是否是 Claude 会话文件。
    #[must_use]
    pub fn is_session_file_name(name: &str) -> bool {
        name.to_ascii_lowercase().ends_with(SESSION_FILE_SUFFIX)
    }
}

/// 取相对路径的父目录（`a/b/c.jsonl` → `a/b`）。
///
/// 没有分隔符、或分隔符就是首字符（相对路径不该出现）时返回空串，表示「就在根目录下」。
fn parent_of(relative: &str) -> String {
    match relative.rfind(['/', '\\']) {
        Some(index) if index > 0 => relative[..index].to_owned(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wsl() -> ClaudeLayout {
        ClaudeLayout::new("/home/a/.claude", StoreKind::Wsl)
    }

    #[test]
    fn layout_roots_are_derived_from_the_config_directory() {
        let wsl = wsl();
        assert_eq!(wsl.projects_root(), "/home/a/.claude/projects");
        assert_eq!(wsl.trash_root(), "/home/a/.claude/.visual-console-trash");
        assert_eq!(
            wsl.trash_projects_root(),
            "/home/a/.claude/.visual-console-trash/projects"
        );
    }

    #[test]
    fn trash_path_preserves_the_project_directory() {
        let wsl = wsl();
        let source = "/home/a/.claude/projects/-home-a-work/abc-123.jsonl";
        assert_eq!(
            wsl.trash_path_for(source).expect("trash path"),
            "/home/a/.claude/.visual-console-trash/projects/-home-a-work/abc-123.jsonl"
        );
    }

    #[test]
    fn restore_path_inverts_the_trash_path() {
        let wsl = wsl();
        let original = "/home/a/.claude/projects/-home-a-work/abc-123.jsonl";
        let trashed = wsl.trash_path_for(original).expect("trash");
        assert_eq!(wsl.restore_path_for(&trashed).expect("restore"), original);
    }

    #[test]
    fn trash_path_rejects_files_outside_the_projects_root() {
        let wsl = wsl();
        for outside in [
            "/home/a/.claude/settings.json",
            "/home/a/.claude/.visual-console-trash/projects/x/a.jsonl",
            "/home/a/.claude/projects",
            "/etc/passwd",
            "/home/a/.claude/projects-evil/a.jsonl",
        ] {
            let error = wsl.trash_path_for(outside).expect_err("必须拒绝");
            assert_eq!(error.to_string(), MESSAGE_TRASH, "输入 {outside}");
        }
    }

    #[test]
    fn restore_path_rejects_files_outside_the_trash() {
        let wsl = wsl();
        for outside in [
            "/home/a/.claude/projects/x/a.jsonl",
            "/home/a/.claude/.visual-console-trash/projects",
            "/home/a/.claude/.visual-console-trash/settings.json",
        ] {
            let error = wsl.restore_path_for(outside).expect_err("必须拒绝");
            assert_eq!(error.to_string(), MESSAGE_RESTORE);
        }
    }

    #[test]
    fn the_root_directories_themselves_are_rejected() {
        // 目录自身也算「在根之内」；不收紧就能把整个 projects/ 移进回收站。
        let wsl = wsl();
        assert!(wsl.trash_path_for("/home/a/.claude/projects").is_err());
        assert!(
            wsl.restore_path_for("/home/a/.claude/.visual-console-trash/projects")
                .is_err()
        );
    }

    #[test]
    fn operable_fence_uses_the_generic_message() {
        let wsl = wsl();
        assert!(
            wsl.assert_operable("/home/a/.claude/projects/x/a.jsonl")
                .is_ok()
        );
        let error = wsl
            .assert_operable("/home/a/.claude/settings.json")
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), MESSAGE_OPERATE);
    }

    #[test]
    fn readable_fence_accepts_both_roots() {
        let wsl = wsl();
        assert!(
            wsl.assert_readable("/home/a/.claude/projects/x/a.jsonl")
                .is_ok()
        );
        assert!(
            wsl.assert_readable("/home/a/.claude/.visual-console-trash/projects/x/a.jsonl")
                .is_ok()
        );
        assert!(
            wsl.assert_readable("/home/a/.claude/settings.json")
                .is_err()
        );
    }

    #[test]
    fn derived_paths_keep_the_project_directory() {
        let wsl = wsl();
        let source = "/home/a/.claude/projects/-home-a-work/old-id.jsonl";
        assert_eq!(
            wsl.derived_path_for(source, "new-id").expect("derived"),
            "/home/a/.claude/projects/-home-a-work/new-id.jsonl"
        );
    }

    #[test]
    fn derived_paths_from_the_trash_land_in_the_active_tree() {
        let wsl = wsl();
        let source = "/home/a/.claude/.visual-console-trash/projects/-home-a-work/old-id.jsonl";
        assert_eq!(
            wsl.derived_path_for(source, "new-id").expect("derived"),
            "/home/a/.claude/projects/-home-a-work/new-id.jsonl"
        );
    }

    #[test]
    fn derived_paths_reject_sources_outside_both_roots() {
        let wsl = wsl();
        assert!(wsl.derived_path_for("/etc/passwd", "x").is_err());
    }

    #[test]
    fn session_file_names_end_with_jsonl() {
        assert!(ClaudeLayout::is_session_file_name("abc.jsonl"));
        assert!(ClaudeLayout::is_session_file_name("ABC.JSONL"));
        assert!(!ClaudeLayout::is_session_file_name("abc.json"));
        assert!(!ClaudeLayout::is_session_file_name("abc.jsonl.bak"));
    }

    #[test]
    fn parent_of_handles_flat_and_nested_paths() {
        assert_eq!(parent_of("a/b/c.jsonl"), "a/b");
        assert_eq!(parent_of("c.jsonl"), "");
    }

    #[test]
    fn local_layout_uses_the_host_separator() {
        let local = ClaudeLayout::new("C:\\Users\\akim\\.claude", StoreKind::Local);
        assert!(local.projects_root().ends_with("projects"));
        if cfg!(windows) {
            assert!(
                !local.projects_root().contains('/'),
                "{}",
                local.projects_root()
            );
        }
    }
}
