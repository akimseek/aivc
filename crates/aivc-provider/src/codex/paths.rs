//! Codex 存储布局与变更路径计算。
//!
//! # 布局
//!
//! ```text
//! <codex_home>/
//! ├── sessions/<年>/<月>/<日>/rollout-<时间戳>-<uuid>.jsonl
//! └── .visual-console-trash/
//!     └── sessions/<年>/<月>/<日>/rollout-<时间戳>-<uuid>.jsonl
//! ```
//!
//! 回收站**保持相对 `codex_home` 的目录结构**（含 `sessions/` 前缀），因此恢复时只需
//! 反向拼一次路径，不需要额外索引。这是现网的设计，也是「恢复后原位置已存在同名文件」
//! 这条冲突判断能成立的前提。
//!
//! # 路径围栏
//!
//! 所有路径计算前都必须先通过目录边界断言。渲染层可以传 `file_path` 提示（列表里已经
//! 拿到路径，省掉一次全量扫描），但**它来自渲染层**，因此每条变更路径都要重新校验：
//!
//! - 删除：源文件必须在 `sessions` 之内；
//! - 恢复：源文件必须在 `.visual-console-trash/sessions` 之内；
//! - 清除：源文件必须在 `.visual-console-trash/sessions` 之内。
//!
//! 断言顺序也重要：**先断言、再算相对路径**。若先算相对路径，越界输入会得到一个
//! 看似合理的垃圾路径，错误就变成静默的数据损坏而不是明确的拒绝。
//!
//! # 一处刻意统一的文案
//!
//! 现网同一个「删除到回收站」操作在本机与 WSL 下给出**不同**的错误文案
//! （`codex-store.ts` 是「拒绝将 sessions 目录之外的文件移动到回收站」，
//! `codex-targets.ts` 是「拒绝移动 sessions 目录之外的文件」）。同一逻辑操作不应因
//! 后端不同而给出不同提示，这里统一采用信息更完整的前者。

use aivc_core::error::{Result, SessionError};
use aivc_core::path::{is_inside_local_path, is_inside_posix_dir};
use aivc_core::ports::StoreKind;
use aivc_core::text::format_session_stamp;
use aivc_core::time::Timestamp;

/// 会话文件名的前缀与后缀。
pub const SESSION_FILE_PREFIX: &str = "rollout-";
/// 会话文件扩展名。
pub const SESSION_FILE_SUFFIX: &str = ".jsonl";
/// 会话文件通配模式（供 `find` 使用）。
pub const SESSION_FILE_GLOB: &str = "rollout-*.jsonl";
/// 回收站目录名。
pub const TRASH_DIR_NAME: &str = ".visual-console-trash";
/// 会话子目录名。
pub const SESSIONS_DIR_NAME: &str = "sessions";

/// Codex 的存储布局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexLayout {
    codex_home: String,
    kind: StoreKind,
}

impl CodexLayout {
    /// 构造。
    #[must_use]
    pub fn new(codex_home: impl Into<String>, kind: StoreKind) -> Self {
        Self {
            codex_home: codex_home.into(),
            kind,
        }
    }

    /// 配置根目录。
    #[must_use]
    pub fn codex_home(&self) -> &str {
        &self.codex_home
    }

    /// 存储后端类型。
    #[must_use]
    pub const fn kind(&self) -> StoreKind {
        self.kind
    }

    /// 活跃会话根目录：`<home>/sessions`。
    #[must_use]
    pub fn sessions_root(&self) -> String {
        self.join(SESSIONS_DIR_NAME)
    }

    /// 回收站根目录：`<home>/.visual-console-trash`。
    #[must_use]
    pub fn trash_root(&self) -> String {
        self.join(TRASH_DIR_NAME)
    }

    /// 回收站会话根目录：`<home>/.visual-console-trash/sessions`。
    #[must_use]
    pub fn trash_sessions_root(&self) -> String {
        self.join(&format!("{TRASH_DIR_NAME}/{SESSIONS_DIR_NAME}"))
    }

    /// 判断路径是否位于某个根目录之内。
    #[must_use]
    pub fn is_inside(&self, path: &str, root: &str) -> bool {
        match self.kind {
            StoreKind::Local => is_inside_local_path(path, root),
            StoreKind::Wsl => is_inside_posix_dir(path, root),
        }
    }

    /// 判断路径是否**严格**位于某个根目录之内（排除根目录本身）。
    ///
    /// # 为什么必须排除根目录本身
    ///
    /// `is_inside` 对「目录自身」返回真，因此 `sessions` 根目录本身也会通过围栏。
    /// 若不做这层收紧，一条伪造的 `filePath` 就能算出
    /// `<home>/.visual-console-trash/sessions`，把**整个会话目录**移进回收站。
    /// 现网存在这个洞（`isInsidePosixDir` 对目录自身返回真，随后直接切片算相对路径），
    /// 这里刻意收紧。
    #[must_use]
    pub fn is_strictly_inside(&self, path: &str, root: &str) -> bool {
        self.is_inside(path, root) && relative_to(path, root).is_some()
    }

    /// 断言路径严格位于活跃会话根目录之内（删除操作的围栏）。
    pub fn assert_deletable(&self, path: &str) -> Result<()> {
        if self.is_strictly_inside(path, &self.sessions_root()) {
            return Ok(());
        }
        Err(SessionError::DeleteOutsideSessionsRoot.into())
    }

    /// 断言路径严格位于回收站会话根目录之内（恢复操作的围栏）。
    pub fn assert_in_trash(&self, path: &str) -> Result<()> {
        if self.is_strictly_inside(path, &self.trash_sessions_root()) {
            return Ok(());
        }
        Err(SessionError::RestoreOutsideTrashRoot.into())
    }

    /// 断言路径严格位于回收站会话根目录之内（永久删除操作的围栏）。
    pub fn assert_purgeable(&self, path: &str) -> Result<()> {
        if self.is_strictly_inside(path, &self.trash_sessions_root()) {
            return Ok(());
        }
        Err(SessionError::PurgeOutsideTrashRoot.into())
    }

    /// 断言路径可读：严格位于活跃会话目录或回收站目录之内。
    pub fn assert_readable(&self, path: &str) -> Result<()> {
        if self.is_strictly_inside(path, &self.sessions_root())
            || self.is_strictly_inside(path, &self.trash_sessions_root())
        {
            return Ok(());
        }
        Err(SessionError::ReadOutsideCodexSessionsRoot.into())
    }

    /// 会话文件对应的回收站路径。
    ///
    /// 保持相对 `codex_home` 的结构，因此 `sessions/2026/09/16/x.jsonl` 会变成
    /// `<home>/.visual-console-trash/sessions/2026/09/16/x.jsonl`。
    pub fn trash_path_for(&self, source: &str) -> Result<String> {
        self.assert_deletable(source)?;
        let relative = self
            .relative_to_home(source)
            .ok_or(SessionError::PathNotAllowed)?;
        Ok(self.join(&format!("{TRASH_DIR_NAME}/{relative}")))
    }

    /// 回收站文件恢复后的原路径。
    pub fn restore_path_for(&self, source: &str) -> Result<String> {
        self.assert_in_trash(source)?;
        // 相对路径基于**回收站根目录**（不含 `.visual-console-trash/` 前缀），
        // 因此拼回 `codex_home` 即可得到原位置。
        let relative = self
            .relative_to_trash_root(source)
            .ok_or(SessionError::PathNotAllowed)?;
        Ok(self.join(&relative))
    }

    /// 分支会话文件路径。
    ///
    /// 源文件在活跃目录时，分支与它同目录；源文件在回收站时，分支落到活跃目录的
    /// 对应相对位置（分支不应留在回收站）。
    pub fn branch_path_for(
        &self,
        source: &str,
        branch_id: &str,
        now: &Timestamp,
    ) -> Result<String> {
        let file_name = build_session_file_name(now, branch_id);
        if self.is_strictly_inside(source, &self.sessions_root()) {
            return Ok(self.join_under(source, &file_name));
        }
        if self.is_strictly_inside(source, &self.trash_sessions_root()) {
            let relative =
                relative_to(source, &self.trash_root()).ok_or(SessionError::PathNotAllowed)?;
            let directory = dir_name(&relative, self.kind);
            return Ok(self.join(&format!("{directory}/{file_name}")));
        }
        Err(SessionError::BranchSourceNotAllowed.into())
    }

    /// 计算路径相对回收站根目录的部分。
    fn relative_to_trash_root(&self, path: &str) -> Option<String> {
        relative_to(path, &self.trash_root())
    }

    /// 用后端对应的分隔符拼接路径。
    #[must_use]
    pub fn join(&self, relative: &str) -> String {
        join_path(self.kind, &self.codex_home, relative)
    }

    /// 在给定文件所在目录下拼接文件名。
    #[must_use]
    pub fn join_under(&self, file_path: &str, name: &str) -> String {
        join_path(self.kind, &dir_name(file_path, self.kind), name)
    }

    /// 计算路径相对 `codex_home` 的部分。
    fn relative_to_home(&self, path: &str) -> Option<String> {
        relative_to(path, &self.codex_home)
    }
}

/// 计算路径相对某个根目录的部分。
///
/// 路径等于根目录本身时返回 [`None`]——这正是「严格位于之内」的判据，用于排除
/// 「把整个 `sessions/` 移进回收站」这类越界目标。
fn relative_to(path: &str, root: &str) -> Option<String> {
    let root = root.trim_end_matches(['/', '\\']);
    let rest = path.strip_prefix(root)?;
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_owned())
    }
}

/// 构造会话文件名：`rollout-<时间戳>-<编号>.jsonl`。
///
/// 时间戳形如 `2026-09-16T12-34-56`（`toISOString()` 截到秒并把冒号换成短横）。
#[must_use]
pub fn build_session_file_name(now: &Timestamp, session_id: &str) -> String {
    format!(
        "{SESSION_FILE_PREFIX}{}{SESSION_FILE_SUFFIX}",
        format_session_stamp_with_id(now, session_id)
    )
}

/// `rollout-` 与 `.jsonl` 之间的部分。
fn format_session_stamp_with_id(now: &Timestamp, session_id: &str) -> String {
    format!("{}-{session_id}", format_session_stamp(now))
}

/// 按后端拼接路径。
fn join_path(kind: StoreKind, base: &str, rest: &str) -> String {
    match kind {
        StoreKind::Local => std::path::Path::new(base)
            .join(rest)
            .to_string_lossy()
            .into_owned(),
        StoreKind::Wsl => {
            let base = base.trim_end_matches('/');
            let rest = rest.trim_start_matches('/');
            if base.is_empty() {
                rest.to_owned()
            } else if rest.is_empty() {
                base.to_owned()
            } else {
                format!("{base}/{rest}")
            }
        }
    }
}

/// 取路径所在目录（按后端语义）。
#[must_use]
pub fn dir_name(path: &str, kind: StoreKind) -> String {
    match kind {
        StoreKind::Local => std::path::Path::new(path)
            .parent()
            .map_or_else(String::new, |parent| parent.to_string_lossy().into_owned()),
        StoreKind::Wsl => match path.rfind('/') {
            Some(0) => "/".to_owned(),
            Some(index) => path[..index].to_owned(),
            None => String::new(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> CodexLayout {
        CodexLayout::new("C:\\Users\\akim\\.codex", StoreKind::Local)
    }

    fn wsl() -> CodexLayout {
        CodexLayout::new("/home/akim/.codex", StoreKind::Wsl)
    }

    fn stamp() -> Timestamp {
        Timestamp::from_raw("2026-09-16T12:34:56.789Z")
    }

    #[test]
    fn layout_roots_are_derived_from_the_home_directory() {
        let wsl = wsl();
        assert_eq!(wsl.sessions_root(), "/home/akim/.codex/sessions");
        assert_eq!(wsl.trash_root(), "/home/akim/.codex/.visual-console-trash");
        assert_eq!(
            wsl.trash_sessions_root(),
            "/home/akim/.codex/.visual-console-trash/sessions"
        );
    }

    #[test]
    fn local_layout_uses_the_host_separator() {
        let local = local();
        assert!(local.sessions_root().ends_with("sessions"));
        assert!(local.sessions_root().starts_with("C:\\Users\\akim\\.codex"));
        // 本机路径不得出现正斜杠（Windows 上）。
        if cfg!(windows) {
            assert!(
                !local.sessions_root().contains('/'),
                "{}",
                local.sessions_root()
            );
        }
    }

    #[test]
    fn session_file_name_matches_the_rollout_convention() {
        assert_eq!(
            build_session_file_name(&stamp(), "0191f2a3-0000-7000-8000-000000000000"),
            "rollout-2026-09-16T12-34-56-0191f2a3-0000-7000-8000-000000000000.jsonl"
        );
    }

    #[test]
    fn trash_path_preserves_the_relative_structure() {
        let wsl = wsl();
        let source = "/home/akim/.codex/sessions/2026/09/16/rollout-x.jsonl";
        assert_eq!(
            wsl.trash_path_for(source).expect("trash path"),
            "/home/akim/.codex/.visual-console-trash/sessions/2026/09/16/rollout-x.jsonl"
        );
    }

    #[test]
    fn trash_path_rejects_files_outside_the_sessions_root() {
        let wsl = wsl();
        for outside in [
            "/home/akim/.codex/config.toml",
            "/home/akim/.codex/.visual-console-trash/sessions/2026/x.jsonl",
            "/etc/passwd",
            "/home/akim/.codex/sessions-evil/x.jsonl",
            "/home/akim/.codex/sessions/../../etc/passwd",
        ] {
            let error = wsl.trash_path_for(outside).expect_err("必须拒绝");
            assert_eq!(
                error.to_string(),
                "拒绝将 sessions 目录之外的文件移动到回收站",
                "输入 {outside}"
            );
        }
    }

    #[test]
    fn restore_path_inverts_the_trash_path() {
        let wsl = wsl();
        let original = "/home/akim/.codex/sessions/2026/09/16/rollout-x.jsonl";
        let trashed = wsl.trash_path_for(original).expect("trash");
        assert_eq!(wsl.restore_path_for(&trashed).expect("restore"), original);
    }

    #[test]
    fn restore_path_rejects_files_outside_the_trash() {
        let wsl = wsl();
        for outside in [
            "/home/akim/.codex/sessions/2026/x.jsonl",
            "/home/akim/.codex/.visual-console-trash/config.toml",
            "/etc/passwd",
        ] {
            let error = wsl.restore_path_for(outside).expect_err("必须拒绝");
            assert_eq!(error.to_string(), "拒绝恢复回收站目录之外的文件");
        }
    }

    #[test]
    fn purge_fence_has_its_own_message() {
        let wsl = wsl();
        let error = wsl
            .assert_purgeable("/home/akim/.codex/sessions/x.jsonl")
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "拒绝删除回收站目录之外的文件");
    }

    #[test]
    fn readable_fence_accepts_both_active_and_trash() {
        let wsl = wsl();
        assert!(
            wsl.assert_readable("/home/akim/.codex/sessions/2026/x.jsonl")
                .is_ok()
        );
        assert!(
            wsl.assert_readable("/home/akim/.codex/.visual-console-trash/sessions/2026/x.jsonl")
                .is_ok()
        );
        let error = wsl
            .assert_readable("/home/akim/.codex/config.toml")
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "拒绝读取 Codex 会话目录之外的文件。");
    }

    #[test]
    fn branch_from_an_active_session_lands_in_the_same_directory() {
        let wsl = wsl();
        let source = "/home/akim/.codex/sessions/2026/09/16/rollout-x.jsonl";
        assert_eq!(
            wsl.branch_path_for(source, "branch-1", &stamp())
                .expect("branch path"),
            "/home/akim/.codex/sessions/2026/09/16/rollout-2026-09-16T12-34-56-branch-1.jsonl"
        );
    }

    #[test]
    fn branch_from_a_trashed_session_lands_in_the_active_tree() {
        // 分支不应留在回收站：源在回收站时，分支落到活跃目录的对应位置。
        let wsl = wsl();
        let source = "/home/akim/.codex/.visual-console-trash/sessions/2026/09/16/rollout-x.jsonl";
        assert_eq!(
            wsl.branch_path_for(source, "branch-1", &stamp())
                .expect("branch path"),
            "/home/akim/.codex/sessions/2026/09/16/rollout-2026-09-16T12-34-56-branch-1.jsonl"
        );
    }

    #[test]
    fn branch_rejects_sources_outside_both_roots() {
        let wsl = wsl();
        let error = wsl
            .branch_path_for("/etc/passwd", "b", &stamp())
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "分支只能从会话目录或回收站内的会话生成。"
        );
    }

    #[test]
    fn posix_dir_name_handles_root_and_bare_names() {
        assert_eq!(dir_name("/a/b/c.jsonl", StoreKind::Wsl), "/a/b");
        assert_eq!(dir_name("/a", StoreKind::Wsl), "/");
        assert_eq!(dir_name("bare.jsonl", StoreKind::Wsl), "");
    }

    #[test]
    fn wsl_joining_normalizes_redundant_separators() {
        let layout = CodexLayout::new("/home/a/.codex/", StoreKind::Wsl);
        assert_eq!(layout.sessions_root(), "/home/a/.codex/sessions");
        assert_eq!(layout.trash_root(), "/home/a/.codex/.visual-console-trash");
    }

    #[test]
    fn local_fences_reject_traversal_too() {
        let local = local();
        let root = local.sessions_root();
        let escape = format!("{root}\\..\\..\\Windows\\System32\\x.jsonl");
        assert!(local.trash_path_for(&escape).is_err());
    }

    #[test]
    fn directory_roots_themselves_are_rejected() {
        // 目录自身也算「在根之内」，若不收紧就能算出把整个 sessions/ 移进回收站的路径。
        let wsl = wsl();
        assert!(wsl.trash_path_for("/home/akim/.codex/sessions").is_err());
        assert!(wsl.assert_deletable("/home/akim/.codex/sessions").is_err());
        assert!(
            wsl.assert_in_trash("/home/akim/.codex/.visual-console-trash/sessions")
                .is_err()
        );
        assert!(
            wsl.assert_purgeable("/home/akim/.codex/.visual-console-trash/sessions")
                .is_err()
        );
        assert!(wsl.assert_readable("/home/akim/.codex/sessions").is_err());
        assert!(
            wsl.branch_path_for("/home/akim/.codex/sessions", "b", &stamp())
                .is_err()
        );
    }
}
