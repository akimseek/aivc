//! Gemini 存储布局与变更路径。
//!
//! # 布局
//!
//! ```text
//! <config_dir>/                      默认 ~/.gemini
//! ├── projects.json                  { "projects": { "<工作目录>": "<项目标识>" } }
//! ├── tmp/<项目标识>/chats/session-<YYYY-MM-DDTHH-MM>-<8 位十六进制>.jsonl
//! └── .visual-console-trash/
//!     └── tmp/<项目标识>/chats/session-…jsonl
//! ```
//!
//! # 项目标识与工作目录是**反查**关系
//!
//! `projects.json` 的键是工作目录、值是项目标识（通常是工作目录的哈希）。会话文件里
//! **不含**工作目录，因此 `cwd` 只能靠这张表反查。表缺失时 `cwd` 为空，而不是报错——
//! 用户可能删过 `projects.json`，但历史会话仍然可读。
//!
//! # 文件名时间戳不含秒
//!
//! 与其他平台不同，Gemini 的文件名时间戳是 `YYYY-MM-DDTHH-MM`（16 字符，**不含秒**）。
//! 用带秒的格式会让文件名与解析器的形状校验不匹配，会话编号将无法从文件名回退提取。
//!
//! # 拒绝文案
//!
//! Gemini 有四类专属文案（与现网逐字一致），因此不能用共享的固定错误。

use aivc_core::error::Result;
use aivc_core::ports::StoreKind;
use aivc_core::time::Timestamp;

use crate::common::{assert_session_file_inside, join_path, relative_to};

/// 会话子目录名。
pub const TMP_DIR_NAME: &str = "tmp";
/// 会话文件所在的第三层目录名。
pub const CHATS_DIR_NAME: &str = "chats";
/// 回收站目录名。
pub const TRASH_DIR_NAME: &str = ".visual-console-trash";
/// 项目映射文件名。
pub const PROJECTS_FILE_NAME: &str = "projects.json";
/// 会话文件名前缀。
pub const SESSION_FILE_PREFIX: &str = "session-";
/// 会话文件扩展名。
pub const SESSION_FILE_SUFFIX: &str = ".jsonl";
/// `find` 用的路径模式。
pub const SESSION_PATH_PATTERN: &str = "*/chats/session-*.jsonl";

/// 通用操作围栏文案。
pub const MESSAGE_OPERATE: &str = "拒绝操作 Gemini 会话目录之外的文件";
/// 移入回收站的围栏文案。
pub const MESSAGE_TRASH: &str = "拒绝移动 Gemini tmp 目录之外的文件";
/// 恢复的围栏文案。
pub const MESSAGE_RESTORE: &str = "拒绝恢复 Gemini 回收站之外的文件";
/// 永久删除的围栏文案。
pub const MESSAGE_PURGE: &str = "拒绝删除 Gemini 回收站之外的文件";
/// 分支源不允许时的文案。
pub const MESSAGE_BRANCH_SOURCE: &str = "Gemini 分支只能从会话目录或回收站内的会话生成。";

/// Gemini 的存储布局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeminiLayout {
    config_dir: String,
    kind: StoreKind,
}

impl GeminiLayout {
    /// 构造。
    #[must_use]
    pub fn new(config_dir: impl Into<String>, kind: StoreKind) -> Self {
        Self {
            config_dir: config_dir.into(),
            kind,
        }
    }

    /// 配置根目录（默认 `~/.gemini`）。
    #[must_use]
    pub fn config_dir(&self) -> &str {
        &self.config_dir
    }

    /// 存储后端类型。
    #[must_use]
    pub const fn kind(&self) -> StoreKind {
        self.kind
    }

    /// 活跃会话根目录：`<config_dir>/tmp`。
    #[must_use]
    pub fn active_root(&self) -> String {
        join_path(self.kind, &self.config_dir, TMP_DIR_NAME)
    }

    /// 回收站根目录：`<config_dir>/.visual-console-trash`。
    #[must_use]
    pub fn trash_root(&self) -> String {
        join_path(self.kind, &self.config_dir, TRASH_DIR_NAME)
    }

    /// 回收站内的会话根目录：`<config_dir>/.visual-console-trash/tmp`。
    #[must_use]
    pub fn trash_active_root(&self) -> String {
        join_path(self.kind, &self.trash_root(), TMP_DIR_NAME)
    }

    /// 项目映射文件：`<config_dir>/projects.json`。
    #[must_use]
    pub fn projects_file(&self) -> String {
        join_path(self.kind, &self.config_dir, PROJECTS_FILE_NAME)
    }

    /// 断言路径可用于通用操作。
    pub fn assert_operable(&self, path: &str) -> Result<()> {
        assert_session_file_inside(path, &self.active_root(), self.kind, MESSAGE_OPERATE)
    }

    /// 断言路径可读：活跃目录或回收站目录之内。
    pub fn assert_readable(&self, path: &str) -> Result<()> {
        if crate::common::is_strictly_inside(path, &self.active_root(), self.kind)
            || crate::common::is_strictly_inside(path, &self.trash_active_root(), self.kind)
        {
            return Ok(());
        }
        Err(aivc_core::error::SessionError::SessionPathNotAllowed {
            message: MESSAGE_OPERATE.to_owned(),
        }
        .into())
    }

    /// 会话文件对应的回收站路径。
    ///
    /// 相对基准是**配置根目录**（不是 `tmp`），因此回收站里会保留 `tmp/` 这一层。
    pub fn trash_path_for(&self, source: &str) -> Result<String> {
        assert_session_file_inside(source, &self.active_root(), self.kind, MESSAGE_TRASH)?;
        let relative = relative_to(source, &self.config_dir).ok_or_else(|| {
            aivc_core::error::SessionError::SessionPathNotAllowed {
                message: MESSAGE_TRASH.to_owned(),
            }
        })?;
        Ok(join_path(self.kind, &self.trash_root(), &relative))
    }

    /// 回收站文件恢复后的原路径。
    pub fn restore_path_for(&self, source: &str) -> Result<String> {
        assert_session_file_inside(source, &self.trash_root(), self.kind, MESSAGE_RESTORE)?;
        let relative = relative_to(source, &self.trash_root()).ok_or_else(|| {
            aivc_core::error::SessionError::SessionPathNotAllowed {
                message: MESSAGE_RESTORE.to_owned(),
            }
        })?;
        Ok(join_path(self.kind, &self.config_dir, &relative))
    }

    /// 断言路径可永久删除（回收站之内）。
    pub fn assert_purgeable(&self, path: &str) -> Result<()> {
        assert_session_file_inside(path, &self.trash_active_root(), self.kind, MESSAGE_PURGE)
    }

    /// 分支/复制产出的文件路径。
    ///
    /// 源在活跃目录时同目录产出；源在回收站时落到活跃目录的对应位置——分支与复制品
    /// 不该留在回收站。
    pub fn derived_path_for(&self, source: &str, new_id: &str, now: &Timestamp) -> Result<String> {
        let name = build_session_file_name(now, new_id);
        if crate::common::is_strictly_inside(source, &self.active_root(), self.kind) {
            let parent = crate::common::dir_name(source, self.kind);
            return Ok(join_path(self.kind, &parent, &name));
        }
        if crate::common::is_strictly_inside(source, &self.trash_active_root(), self.kind) {
            // 相对基准是回收站里的 `tmp` 层（不是回收站根），否则会拼出 `tmp/tmp/…`。
            let relative = relative_to(source, &self.trash_active_root()).ok_or_else(|| {
                aivc_core::error::SessionError::SessionPathNotAllowed {
                    message: MESSAGE_BRANCH_SOURCE.to_owned(),
                }
            })?;
            let parent = crate::common::dir_name(&relative, self.kind);
            return Ok(join_path(
                self.kind,
                &join_path(self.kind, &self.active_root(), &parent),
                &name,
            ));
        }
        Err(aivc_core::error::SessionError::SessionPathNotAllowed {
            message: MESSAGE_BRANCH_SOURCE.to_owned(),
        }
        .into())
    }

    /// 从会话文件路径反推项目标识。
    ///
    /// 形状固定为 `<tmp>/<项目标识>/chats/<文件>`，因此取「上上级目录名」。
    #[must_use]
    pub fn project_key_of(&self, file_path: &str) -> String {
        let parent = crate::common::dir_name(file_path, self.kind);
        let grandparent = crate::common::dir_name(&parent, self.kind);
        grandparent
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned()
    }
}

/// 构造会话文件名：`session-<YYYY-MM-DDTHH-MM>-<前 8 位编号>.jsonl`。
#[must_use]
pub fn build_session_file_name(now: &Timestamp, session_id: &str) -> String {
    let suffix: String = session_id.chars().take(8).collect();
    format!(
        "{SESSION_FILE_PREFIX}{}{SESSION_FILE_SUFFIX}",
        format_gemini_stamp_with_id(now, &suffix)
    )
}

/// `session-` 与 `.jsonl` 之间的部分。
fn format_gemini_stamp_with_id(now: &Timestamp, suffix: &str) -> String {
    format!("{}-{suffix}", format_gemini_stamp(now))
}

/// Gemini 的文件名时间戳：`YYYY-MM-DDTHH-MM`（**不含秒**）。
///
/// 从 ISO 8601 取前 16 个字符并把冒号换成短横。ISO 8601 的前 16 字符恰好是
/// `YYYY-MM-DDTHH:MM`，与现网 `toISOString().slice(0, 16)` 等价。
#[must_use]
pub fn format_gemini_stamp(now: &Timestamp) -> String {
    now.as_str()
        .get(..16)
        .map_or_else(String::new, |prefix| prefix.replace(':', "-"))
}

/// 从 `projects.json` 解析「项目标识 → 工作目录」映射。
///
/// 文件里的方向是反的（工作目录 → 项目标识），因此这里要翻转一次。
/// 解析失败或结构不符时返回空表——用户可能删过这个文件，但历史会话仍应可读。
#[must_use]
pub fn parse_projects(content: &str) -> std::collections::BTreeMap<String, String> {
    let mut projects = std::collections::BTreeMap::new();
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(content) else {
        return projects;
    };
    let Some(map) = parsed
        .get("projects")
        .and_then(serde_json::Value::as_object)
    else {
        return projects;
    };
    for (cwd, key) in map {
        if let Some(key) = key.as_str() {
            projects.insert(key.to_owned(), cwd.clone());
        }
    }
    projects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wsl() -> GeminiLayout {
        GeminiLayout::new("/home/a/.gemini", StoreKind::Wsl)
    }

    fn stamp() -> Timestamp {
        Timestamp::from_raw("2026-09-16T12:34:56.789Z")
    }

    #[test]
    fn layout_roots_are_derived_from_the_config_directory() {
        let wsl = wsl();
        assert_eq!(wsl.active_root(), "/home/a/.gemini/tmp");
        assert_eq!(wsl.trash_root(), "/home/a/.gemini/.visual-console-trash");
        assert_eq!(
            wsl.trash_active_root(),
            "/home/a/.gemini/.visual-console-trash/tmp"
        );
        assert_eq!(wsl.projects_file(), "/home/a/.gemini/projects.json");
    }

    #[test]
    fn the_file_name_stamp_has_no_seconds() {
        // 带秒会让解析器的形状校验失败，会话编号将无法从文件名回退提取。
        assert_eq!(format_gemini_stamp(&stamp()), "2026-09-16T12-34");
        assert_eq!(format_gemini_stamp(&stamp()).len(), 16);
    }

    #[test]
    fn the_session_file_name_matches_the_parser_shape() {
        let name = build_session_file_name(&stamp(), "abcdef0123456789");
        assert_eq!(name, "session-2026-09-16T12-34-abcdef01.jsonl");
        // 必须能被解析器的形状校验接受。
        assert_eq!(
            super::super::parser::session_id_from_file_path(&format!("/a/chats/{name}")),
            "abcdef01"
        );
    }

    #[test]
    fn the_project_key_is_the_grandparent_directory() {
        let wsl = wsl();
        assert_eq!(
            wsl.project_key_of("/home/a/.gemini/tmp/proj123/chats/session-x.jsonl"),
            "proj123"
        );
    }

    #[test]
    fn trash_paths_keep_the_tmp_layer() {
        let wsl = wsl();
        let source = "/home/a/.gemini/tmp/proj123/chats/session-2026-09-16T12-34-abcdef01.jsonl";
        assert_eq!(
            wsl.trash_path_for(source).expect("trash"),
            "/home/a/.gemini/.visual-console-trash/tmp/proj123/chats/session-2026-09-16T12-34-abcdef01.jsonl"
        );
    }

    #[test]
    fn restore_path_inverts_the_trash_path() {
        let wsl = wsl();
        let original = "/home/a/.gemini/tmp/proj123/chats/session-2026-09-16T12-34-abcdef01.jsonl";
        let trashed = wsl.trash_path_for(original).expect("trash");
        assert_eq!(wsl.restore_path_for(&trashed).expect("restore"), original);
    }

    #[test]
    fn trash_path_rejects_files_outside_the_tmp_root() {
        let wsl = wsl();
        for outside in [
            "/home/a/.gemini/projects.json",
            "/home/a/.gemini/.visual-console-trash/tmp/x/chats/a.jsonl",
            "/home/a/.gemini/tmp",
            "/etc/passwd",
        ] {
            let error = wsl.trash_path_for(outside).expect_err("必须拒绝");
            assert_eq!(error.to_string(), MESSAGE_TRASH, "输入 {outside}");
        }
    }

    #[test]
    fn restore_path_rejects_files_outside_the_trash() {
        let wsl = wsl();
        for outside in [
            "/home/a/.gemini/tmp/x/chats/a.jsonl",
            "/home/a/.gemini/.visual-console-trash",
            "/home/a/.gemini/projects.json",
        ] {
            let error = wsl.restore_path_for(outside).expect_err("必须拒绝");
            assert_eq!(error.to_string(), MESSAGE_RESTORE);
        }
    }

    #[test]
    fn purge_fence_requires_the_trash_tmp_root() {
        let wsl = wsl();
        assert!(
            wsl.assert_purgeable("/home/a/.gemini/.visual-console-trash/tmp/x/chats/a.jsonl")
                .is_ok()
        );
        let error = wsl
            .assert_purgeable("/home/a/.gemini/tmp/x/chats/a.jsonl")
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), MESSAGE_PURGE);
    }

    #[test]
    fn readable_fence_accepts_both_roots() {
        let wsl = wsl();
        assert!(
            wsl.assert_readable("/home/a/.gemini/tmp/x/chats/a.jsonl")
                .is_ok()
        );
        assert!(
            wsl.assert_readable("/home/a/.gemini/.visual-console-trash/tmp/x/chats/a.jsonl")
                .is_ok()
        );
        assert!(
            wsl.assert_readable("/home/a/.gemini/projects.json")
                .is_err()
        );
    }

    #[test]
    fn derived_paths_stay_in_the_same_chats_directory() {
        let wsl = wsl();
        let source = "/home/a/.gemini/tmp/proj123/chats/session-2026-09-16T12-34-abcdef01.jsonl";
        assert_eq!(
            wsl.derived_path_for(source, "1122334455667788", &stamp())
                .expect("derived"),
            "/home/a/.gemini/tmp/proj123/chats/session-2026-09-16T12-34-11223344.jsonl"
        );
    }

    #[test]
    fn derived_paths_from_the_trash_land_in_the_active_tree() {
        let wsl = wsl();
        let source = "/home/a/.gemini/.visual-console-trash/tmp/proj123/chats/session-2026-09-16T12-34-abcdef01.jsonl";
        assert_eq!(
            wsl.derived_path_for(source, "1122334455667788", &stamp())
                .expect("derived"),
            "/home/a/.gemini/tmp/proj123/chats/session-2026-09-16T12-34-11223344.jsonl"
        );
    }

    #[test]
    fn derived_paths_reject_sources_outside_both_roots() {
        let wsl = wsl();
        let error = wsl
            .derived_path_for("/etc/passwd", "x", &stamp())
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), MESSAGE_BRANCH_SOURCE);
    }

    #[test]
    fn projects_json_is_inverted_into_key_to_cwd() {
        let content = r#"{"projects":{"/home/a/work":"proj123","/home/a/other":"proj456"}}"#;
        let projects = parse_projects(content);
        assert_eq!(
            projects.get("proj123").map(String::as_str),
            Some("/home/a/work")
        );
        assert_eq!(
            projects.get("proj456").map(String::as_str),
            Some("/home/a/other")
        );
    }

    #[test]
    fn a_missing_or_malformed_projects_file_yields_an_empty_map() {
        // 用户可能删过这个文件，但历史会话仍应可读。
        assert!(parse_projects("").is_empty());
        assert!(parse_projects("not json").is_empty());
        assert!(parse_projects("{}").is_empty());
        assert!(parse_projects(r#"{"projects":[]}"#).is_empty());
        // 值不是字符串的条目被跳过。
        assert!(parse_projects(r#"{"projects":{"/a":42}}"#).is_empty());
    }

    #[test]
    fn local_layout_uses_the_host_separator() {
        let local = GeminiLayout::new("C:\\Users\\akim\\.gemini", StoreKind::Local);
        assert!(local.active_root().ends_with("tmp"));
        if cfg!(windows) {
            assert!(
                !local.active_root().contains('/'),
                "{}",
                local.active_root()
            );
        }
    }
}
