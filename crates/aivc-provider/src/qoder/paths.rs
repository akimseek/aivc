//! Qoder 存储布局与**多产物**变更路径。
//!
//! # 一份会话由五个产物组成
//!
//! ```text
//! <config_dir>/                          默认 ~/.qoder-cn
//! ├── projects/<项目键>/<会话编号>.jsonl   ← transcript（主产物）
//! ├── projects/<项目键>/<会话编号>/        ← 关联目录
//! ├── tasks/<会话编号>/
//! ├── file-history/<会话编号>/
//! └── logs/sessions/<项目键>/<会话编号>/
//! ```
//!
//! 其余三个平台一份会话就是一个文件；Qoder 不是。删除与恢复必须把这一组**一起**
//! 移动，否则会留下孤儿目录，而且恢复时会因为缺少关联数据而出错。
//!
//! # 移动前必须整体预检
//!
//! [`assert_move_targets_available`] 在动任何东西之前检查全部目标位置。只要有**一个**
//! 目标已存在就整体拒绝——部分移动会让会话处于「transcript 在回收站、关联目录在
//! 活跃目录」的分裂状态，比直接失败更难收拾。
//!
//! # 相对路径的形状校验
//!
//! transcript 在 `projects/` 下必须**恰好两层**（`<项目键>/<文件>`），且会话编号必须
//! 是**单一路径段**。这两条校验缺一不可：
//!
//! - 层数不对 → 说明路径不属于 Qoder 的布局；
//! - 编号含分隔符或 `.`/`..` → 拼接出的关联路径会逃出预期目录（路径穿越）。

use aivc_core::error::{Result, SessionError};
use aivc_core::ports::StoreKind;
use aivc_core::session::SessionView;

use crate::common::{assert_session_file_inside, join_path, relative_to};

/// 配置目录名。
pub const CONFIG_DIR_NAME: &str = ".qoder-cn";
/// 会话目录名。
pub const PROJECTS_DIR_NAME: &str = "projects";
/// 回收站目录名。
pub const TRASH_DIR_NAME: &str = ".visual-console-trash";
/// 会话文件扩展名。
pub const SESSION_FILE_SUFFIX: &str = ".jsonl";
/// 会话文件通配模式。
pub const SESSION_FILE_GLOB: &str = "*.jsonl";
/// 配置目录的环境变量名。
pub const CONFIG_DIR_ENV: &str = "QODERCN_CONFIG_DIR";

/// 通用操作围栏文案（路径不属于任何已知视图）。
pub const MESSAGE_OPERATE: &str = "拒绝操作 Qoder 会话目录之外的文件。";
/// 路径形状不合法时的文案。
pub const MESSAGE_LAYOUT: &str = "拒绝操作 Qoder 会话目录之外的会话文件。";

/// 一份会话的一个产物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QoderStoragePath {
    /// 源路径。
    pub source: String,
    /// 目标路径。
    pub destination: String,
    /// 是否是主产物（transcript）。
    pub primary: bool,
}

/// Qoder 的存储布局。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QoderLayout {
    config_dir: String,
    kind: StoreKind,
}

impl QoderLayout {
    /// 构造。
    #[must_use]
    pub fn new(config_dir: impl Into<String>, kind: StoreKind) -> Self {
        Self {
            config_dir: config_dir.into(),
            kind,
        }
    }

    /// 配置根目录（默认 `~/.qoder-cn`）。
    #[must_use]
    pub fn config_dir(&self) -> &str {
        &self.config_dir
    }

    /// 存储后端类型。
    #[must_use]
    pub const fn kind(&self) -> StoreKind {
        self.kind
    }

    /// 某个视图的存储根：活跃为配置目录，回收站为 `.visual-console-trash`。
    #[must_use]
    pub fn storage_root(&self, view: SessionView) -> String {
        match view {
            SessionView::Active => self.config_dir.clone(),
            SessionView::Trash => join_path(self.kind, &self.config_dir, TRASH_DIR_NAME),
        }
    }

    /// 某个视图的会话根目录：`<storage_root>/projects`。
    #[must_use]
    pub fn projects_root(&self, view: SessionView) -> String {
        join_path(self.kind, &self.storage_root(view), PROJECTS_DIR_NAME)
    }

    /// 断言路径属于指定视图。
    pub fn assert_operable(&self, path: &str, view: SessionView) -> Result<()> {
        assert_session_file_inside(path, &self.projects_root(view), self.kind, MESSAGE_OPERATE)
    }

    /// 判断路径属于哪个视图。
    pub fn view_for_path(&self, path: &str) -> Result<SessionView> {
        for view in [SessionView::Active, SessionView::Trash] {
            if crate::common::is_strictly_inside(path, &self.projects_root(view), self.kind) {
                return Ok(view);
            }
        }
        Err(SessionError::SessionPathNotAllowed {
            message: MESSAGE_OPERATE.to_owned(),
        }
        .into())
    }

    /// 构造一份会话全部产物的「源 → 目标」路径对。
    ///
    /// `to_view` 为 [`None`] 时表示同视图内移动（用于仅重命名/校验）。
    pub fn storage_paths(
        &self,
        transcript_path: &str,
        session_id: &str,
        from_view: SessionView,
        to_view: Option<SessionView>,
    ) -> Result<Vec<QoderStoragePath>> {
        let source_projects = self.projects_root(from_view);
        // 先做形状校验再算相对路径：顺序反了的话，越界输入会得到一个看似合理的
        // 垃圾路径，错误就从「明确拒绝」退化成「静默的数据损坏」。
        let relative = relative_to(transcript_path, &source_projects).ok_or_else(layout_error)?;
        let parts: Vec<&str> = relative.split(['/', '\\']).collect();
        let shape_ok = parts.len() == 2
            && !parts[0].is_empty()
            && parts[1].to_ascii_lowercase().ends_with(SESSION_FILE_SUFFIX)
            && is_single_path_segment(session_id);
        if !shape_ok {
            return Err(layout_error());
        }

        let project_key = parts[0];
        let file_name = parts[1];
        let target_view = to_view.unwrap_or(from_view);
        let source_base = self.storage_root(from_view);
        let destination_base = self.storage_root(target_view);

        // 关联产物：与 transcript 同属一份会话，必须一起移动。
        let related = [
            join_path(
                self.kind,
                PROJECTS_DIR_NAME,
                &join_path(self.kind, project_key, session_id),
            ),
            join_path(self.kind, "tasks", session_id),
            join_path(self.kind, "file-history", session_id),
            join_path(
                self.kind,
                "logs/sessions",
                &join_path(self.kind, project_key, session_id),
            ),
        ];

        let mut paths: Vec<QoderStoragePath> = related
            .into_iter()
            .map(|relative_path| QoderStoragePath {
                source: join_path(self.kind, &source_base, &relative_path),
                destination: join_path(self.kind, &destination_base, &relative_path),
                primary: false,
            })
            .collect();
        let transcript_relative = join_path(
            self.kind,
            PROJECTS_DIR_NAME,
            &join_path(self.kind, project_key, file_name),
        );
        paths.push(QoderStoragePath {
            source: join_path(self.kind, &source_base, &transcript_relative),
            destination: join_path(self.kind, &destination_base, &transcript_relative),
            primary: true,
        });
        Ok(paths)
    }

    /// 会话文件所在的项目键。
    #[must_use]
    pub fn project_key_of(&self, transcript_path: &str, view: SessionView) -> String {
        relative_to(transcript_path, &self.projects_root(view))
            .and_then(|relative| relative.split(['/', '\\']).next().map(str::to_owned))
            .unwrap_or_default()
    }
}

/// 路径形状不合法。
fn layout_error() -> aivc_core::Error {
    SessionError::SessionPathNotAllowed {
        message: MESSAGE_LAYOUT.to_owned(),
    }
    .into()
}

/// 是否是单个路径段（非空、非 `.`/`..`、不含分隔符）。
///
/// 这条校验是**路径穿越**的防线：会话编号若含分隔符，拼接出的关联路径会跑到
/// `projects/` 之外。
#[must_use]
pub fn is_single_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

/// 判断是否是 Qoder 会话文件。
#[must_use]
pub fn is_session_file_name(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(SESSION_FILE_SUFFIX)
}

/// 从文件名提取会话编号（去掉 `.jsonl`）。
#[must_use]
pub fn session_id_from_file_path(file_path: &str) -> String {
    let name = file_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_path)
        .to_owned();
    name.strip_suffix(SESSION_FILE_SUFFIX)
        .unwrap_or(&name)
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wsl() -> QoderLayout {
        QoderLayout::new("/home/a/.qoder-cn", StoreKind::Wsl)
    }

    fn transcript(project: &str, id: &str) -> String {
        format!("/home/a/.qoder-cn/projects/{project}/{id}.jsonl")
    }

    #[test]
    fn layout_roots_follow_the_view() {
        let wsl = wsl();
        assert_eq!(wsl.storage_root(SessionView::Active), "/home/a/.qoder-cn");
        assert_eq!(
            wsl.storage_root(SessionView::Trash),
            "/home/a/.qoder-cn/.visual-console-trash"
        );
        assert_eq!(
            wsl.projects_root(SessionView::Active),
            "/home/a/.qoder-cn/projects"
        );
        assert_eq!(
            wsl.projects_root(SessionView::Trash),
            "/home/a/.qoder-cn/.visual-console-trash/projects"
        );
    }

    #[test]
    fn single_path_segment_rejects_traversal() {
        assert!(is_single_path_segment("abc-123"));
        assert!(!is_single_path_segment(""));
        assert!(!is_single_path_segment("."));
        assert!(!is_single_path_segment(".."));
        // 含分隔符的编号会让关联路径逃出 projects/。
        assert!(!is_single_path_segment("a/b"));
        assert!(!is_single_path_segment("a\\b"));
    }

    #[test]
    fn storage_paths_cover_all_five_artifacts() {
        let wsl = wsl();
        let paths = wsl
            .storage_paths(
                &transcript("proj1", "sess-1"),
                "sess-1",
                SessionView::Active,
                Some(SessionView::Trash),
            )
            .expect("paths");
        assert_eq!(paths.len(), 5);

        let sources: Vec<&str> = paths.iter().map(|item| item.source.as_str()).collect();
        assert!(sources.contains(&"/home/a/.qoder-cn/projects/proj1/sess-1.jsonl"));
        assert!(sources.contains(&"/home/a/.qoder-cn/projects/proj1/sess-1"));
        assert!(sources.contains(&"/home/a/.qoder-cn/tasks/sess-1"));
        assert!(sources.contains(&"/home/a/.qoder-cn/file-history/sess-1"));
        assert!(sources.contains(&"/home/a/.qoder-cn/logs/sessions/proj1/sess-1"));

        // 主产物必须被标记出来。
        let primary: Vec<&QoderStoragePath> = paths.iter().filter(|item| item.primary).collect();
        assert_eq!(primary.len(), 1);
        assert!(primary[0].source.ends_with("sess-1.jsonl"));
    }

    #[test]
    fn storage_paths_redirect_every_artifact_into_the_trash() {
        let wsl = wsl();
        let paths = wsl
            .storage_paths(
                &transcript("proj1", "sess-1"),
                "sess-1",
                SessionView::Active,
                Some(SessionView::Trash),
            )
            .expect("paths");
        for item in &paths {
            assert!(
                item.destination.contains("/.visual-console-trash/"),
                "{} 未指向回收站",
                item.destination
            );
        }
    }

    #[test]
    fn restore_paths_invert_the_trash_paths() {
        let wsl = wsl();
        let original = transcript("proj1", "sess-1");
        let trashed = wsl
            .storage_paths(
                &original,
                "sess-1",
                SessionView::Active,
                Some(SessionView::Trash),
            )
            .expect("to trash");
        // 从回收站路径反推回活跃路径。
        let trashed_transcript = trashed
            .iter()
            .find(|item| item.primary)
            .expect("primary")
            .destination
            .clone();
        let restored = wsl
            .storage_paths(
                &trashed_transcript,
                "sess-1",
                SessionView::Trash,
                Some(SessionView::Active),
            )
            .expect("restore");
        let restored_transcript = restored
            .iter()
            .find(|item| item.primary)
            .expect("primary")
            .destination
            .clone();
        assert_eq!(restored_transcript, original);
    }

    #[test]
    fn storage_paths_reject_a_wrong_depth() {
        let wsl = wsl();
        for bad in [
            "/home/a/.qoder-cn/projects/sess-1.jsonl",     // 只有一层
            "/home/a/.qoder-cn/projects/a/b/sess-1.jsonl", // 三层
            "/home/a/.qoder-cn/projects",                  // 根目录本身
            "/etc/passwd",                                 // 完全在外
        ] {
            let error = wsl
                .storage_paths(bad, "sess-1", SessionView::Active, None)
                .expect_err("必须拒绝");
            assert_eq!(error.to_string(), MESSAGE_LAYOUT, "输入 {bad}");
        }
    }

    #[test]
    fn storage_paths_reject_a_non_jsonl_file() {
        let wsl = wsl();
        let error = wsl
            .storage_paths(
                "/home/a/.qoder-cn/projects/proj1/notes.txt",
                "notes",
                SessionView::Active,
                None,
            )
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), MESSAGE_LAYOUT);
    }

    #[test]
    fn storage_paths_reject_a_traversing_session_id() {
        // 编号含分隔符时，拼接出的关联路径会逃出 projects/。
        let wsl = wsl();
        for bad_id in ["../evil", "a/b", "..", ""] {
            let error = wsl
                .storage_paths(
                    &transcript("proj1", "sess-1"),
                    bad_id,
                    SessionView::Active,
                    Some(SessionView::Trash),
                )
                .expect_err("必须拒绝");
            assert_eq!(error.to_string(), MESSAGE_LAYOUT, "编号 {bad_id}");
        }
    }

    #[test]
    fn view_detection_distinguishes_active_and_trash() {
        let wsl = wsl();
        assert_eq!(
            wsl.view_for_path(&transcript("proj1", "sess-1"))
                .expect("view"),
            SessionView::Active
        );
        assert_eq!(
            wsl.view_for_path("/home/a/.qoder-cn/.visual-console-trash/projects/proj1/s.jsonl")
                .expect("view"),
            SessionView::Trash
        );
        let error = wsl.view_for_path("/etc/passwd").expect_err("必须拒绝");
        assert_eq!(error.to_string(), MESSAGE_OPERATE);
    }

    #[test]
    fn project_key_is_the_first_segment() {
        let wsl = wsl();
        assert_eq!(
            wsl.project_key_of(&transcript("proj1", "sess-1"), SessionView::Active),
            "proj1"
        );
        assert_eq!(wsl.project_key_of("/etc/passwd", SessionView::Active), "");
    }

    #[test]
    fn session_ids_come_from_the_file_name() {
        assert_eq!(
            session_id_from_file_path("/a/projects/p/abc-123.jsonl"),
            "abc-123"
        );
        assert!(is_session_file_name("abc.jsonl"));
        assert!(is_session_file_name("ABC.JSONL"));
        assert!(!is_session_file_name("abc.json"));
    }

    #[test]
    fn local_layout_uses_the_host_separator() {
        let local = QoderLayout::new("C:\\Users\\akim\\.qoder-cn", StoreKind::Local);
        assert!(
            local
                .projects_root(SessionView::Active)
                .ends_with("projects")
        );
        if cfg!(windows) {
            assert!(
                !local.projects_root(SessionView::Active).contains('/'),
                "{}",
                local.projects_root(SessionView::Active)
            );
        }
    }
}
