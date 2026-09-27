//! 运行根下的路径派生。
//!
//! 所有应用自有产物都从 [`AppPaths::root`] 派生，集中在此以便审计
//! 「应用绝不写入系统用户目录」这一约束。
//!
//! ```text
//! <root>/
//! ├── data/
//! │   ├── aivc.db                  SQLite（全新库，不读旧工程 app.db）
//! │   ├── aivc-settings.json       设置（全新文件，不读旧工程 settings.json）
//! │   ├── cache/                   会话摘要缓存目录
//! │   ├── webview/                WebView 的数据目录（缓存 / localStorage）
//! │   └── vendor-backups/          写入用户 CLI 配置前的备份
//! └── logs/
//!     ├── gateway.log              网关文本日志（含轮转 gateway.log.1）
//!     └── performance.log          启动阶段性能埋点
//! ```
//!
//! # 与旧工程的关系：**完全区分**
//!
//! 这是「全新库、不迁移旧数据」这条决策的延伸，而且**不只是文件名不同**：
//!
//! 1. **数据库**是 `aivc.db` 而非旧工程的 `app.db`；
//! 2. **设置**是 `aivc-settings.json` 而非旧工程的 `settings.json`；
//! 3. 旧工程把设置放在 `data/user-data/`（Electron 的 `userData`）下，本工程**不再
//!    沿用那个目录结构**——它是 Electron 的形态，与本工程无关。
//!
//! 曾经的做法是设置沿用旧文件名与旧目录。后果实测到了：两个应用**写同一个
//! `settings.json`**，其中的 `cachedTargets` 会互相覆盖，界面上的标签取决于谁最后写
//! （实测文件里同时出现了 `WSL · Ubuntu-24.04` 与旧格式的 `Gemini：WSL：Ubuntu-24.04`）。
//! 既然不迁移旧数据，就没有任何理由共用这个文件。
//!
//! 旧工程的 `session-metadata.json` **没有任何调用方**，也已删除——留着它只会给
//! 「顺手读一下旧数据」留一个入口。

use std::path::{Path, PathBuf};

/// 运行根下的路径集合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    root: PathBuf,
}

impl AppPaths {
    /// 由运行根构造。
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 运行根。
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `data/` —— 应用数据目录。
    #[must_use]
    pub fn data_dir(&self) -> PathBuf {
        self.root.join("data")
    }

    /// `logs/` —— 日志目录。
    #[must_use]
    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    /// `data/aivc.db` —— SQLite 数据库。
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.data_dir().join("aivc.db")
    }

    /// `data/aivc-settings.json` —— 设置文件。
    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.data_dir().join("aivc-settings.json")
    }

    /// `data/webview/` —— `WebView` 的数据目录。
    ///
    /// # 为什么必须显式指定
    ///
    /// 不指定的话，Tauri 2 默认把它放在 `%LOCALAPPDATA%\<identifier>\EBWebView`，
    /// 于是本模块开头那条「应用绝不写入系统用户目录」的约束**并不成立**——而且
    /// `localStorage`（「上次用的平台与目标」就存在那里）也跟着跑到系统目录，
    /// 应用不再便携。
    ///
    /// 这里曾经有一个 `user_data_dir()` 返回 `data/user-data/`，那是 Electron 的
    /// `userData` 形态；它**只被用来拼设置路径**，从未真正接到 `WebView` 上。
    /// 现已删除，取而代之的是这个真正接上的目录，且名字不带旧工程的痕迹。
    ///
    /// 平台差异：只有 Windows（`WebView2`）支持自定义数据目录，因此
    /// `tauri_app` 只在 Windows 上设置它。见 [`AppPaths::webview_data_dir`] 的调用点。
    #[must_use]
    pub fn webview_data_dir(&self) -> PathBuf {
        self.data_dir().join("webview")
    }

    /// `data/exports/` —— 导出会话的默认落点。
    ///
    /// 为什么需要一个默认落点：原生「保存到…」对话框需要 Tauri 的 dialog 插件
    /// （离线环境装不上），而导出**不该因此不可用**。因此由后端挑一个确定的位置，
    /// 再把实际路径回给界面显示——用户至少知道东西去哪了，并能用系统文件管理器打开。
    #[must_use]
    pub fn export_dir(&self) -> PathBuf {
        self.data_dir().join("exports")
    }

    /// `data/cache/` —— 会话摘要缓存根目录。
    #[must_use]
    pub fn cache_dir(&self) -> PathBuf {
        self.data_dir().join("cache")
    }

    /// `data/vendor-backups/` —— 写入用户 CLI 配置前的备份根目录。
    #[must_use]
    pub fn vendor_backup_dir(&self) -> PathBuf {
        self.data_dir().join("vendor-backups")
    }

    /// `logs/performance.log` —— 启动阶段性能埋点。
    #[must_use]
    pub fn performance_log_path(&self) -> PathBuf {
        self.log_dir().join("performance.log")
    }

    /// `logs/gateway.log` —— 网关文本日志。
    #[must_use]
    pub fn gateway_log_path(&self) -> PathBuf {
        self.log_dir().join("gateway.log")
    }

    /// 需要预先创建的目录（按创建顺序）。
    ///
    /// `webview/` 必须在窗口创建**之前**就存在：它是 `WebView2` 的数据目录。
    #[must_use]
    pub fn required_directories(&self) -> Vec<PathBuf> {
        vec![
            self.data_dir(),
            self.cache_dir(),
            self.webview_data_dir(),
            self.export_dir(),
            self.log_dir(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> AppPaths {
        AppPaths::new("D:/code/ai/aivc-rs")
    }

    #[test]
    fn derives_every_path_from_the_runtime_root() {
        let paths = paths();
        assert_eq!(paths.data_dir(), PathBuf::from("D:/code/ai/aivc-rs/data"));
        assert_eq!(paths.log_dir(), PathBuf::from("D:/code/ai/aivc-rs/logs"));
        assert_eq!(
            paths.database_path(),
            PathBuf::from("D:/code/ai/aivc-rs/data/aivc.db")
        );
        assert_eq!(
            paths.settings_path(),
            PathBuf::from("D:/code/ai/aivc-rs/data/aivc-settings.json")
        );
        assert_eq!(
            paths.webview_data_dir(),
            PathBuf::from("D:/code/ai/aivc-rs/data/webview")
        );
        assert_eq!(
            paths.cache_dir(),
            PathBuf::from("D:/code/ai/aivc-rs/data/cache")
        );
        assert_eq!(
            paths.export_dir(),
            PathBuf::from("D:/code/ai/aivc-rs/data/exports")
        );
        assert_eq!(
            paths.vendor_backup_dir(),
            PathBuf::from("D:/code/ai/aivc-rs/data/vendor-backups")
        );
        assert_eq!(
            paths.performance_log_path(),
            PathBuf::from("D:/code/ai/aivc-rs/logs/performance.log")
        );
        assert_eq!(
            paths.gateway_log_path(),
            PathBuf::from("D:/code/ai/aivc-rs/logs/gateway.log")
        );
    }

    #[test]
    fn database_file_name_differs_from_the_legacy_project() {
        // 全新库决策：不得复用旧工程的 app.db。
        let path = paths().database_path();
        assert_eq!(path.file_name().and_then(|v| v.to_str()), Some("aivc.db"));
        assert!(!path.to_string_lossy().contains("app.db"));
    }

    #[test]
    fn settings_file_differs_from_the_legacy_project_in_name_and_place() {
        // **回归守门**：设置曾经叫 `data/user-data/settings.json`——与旧工程**同名同址**。
        // 后果是两个应用写同一个文件、`cachedTargets` 互相覆盖，界面上的目标标签取决于
        // 谁最后写。既然不迁移旧数据，就没有理由共用。
        let path = paths().settings_path();
        assert_eq!(
            path.file_name().and_then(|v| v.to_str()),
            Some("aivc-settings.json"),
            "设置文件名必须与旧工程区分"
        );
        assert!(
            !path.to_string_lossy().contains("user-data"),
            "不得沿用 Electron 的 userData 目录结构：{path:?}"
        );
    }

    #[test]
    fn every_derived_path_stays_under_the_runtime_root() {
        // 审计「应用不写系统用户目录」这一约束：所有路径都必须位于运行根之下。
        let paths = paths();
        let root = paths.root();
        for path in [
            paths.data_dir(),
            paths.log_dir(),
            paths.database_path(),
            paths.settings_path(),
            paths.webview_data_dir(),
            paths.cache_dir(),
            paths.vendor_backup_dir(),
            paths.performance_log_path(),
            paths.gateway_log_path(),
        ] {
            assert!(
                path.starts_with(root),
                "{} 逃出了运行根 {}",
                path.display(),
                root.display()
            );
        }
    }

    #[test]
    fn required_directories_are_deduplicated_and_ordered() {
        let dirs = paths().required_directories();
        assert_eq!(dirs.len(), 5);
        assert_eq!(dirs[0], paths().data_dir());
        // 其余目录都位于 data 之下，必须先建 data。
        for dir in &dirs[1..dirs.len() - 1] {
            assert!(
                dir.starts_with(paths().data_dir()),
                "{dir:?} 不在 data 之下"
            );
        }
        assert!(
            dirs.contains(&paths().webview_data_dir()),
            "WebView 数据目录必须在窗口创建前就建好"
        );
        let unique: std::collections::BTreeSet<_> = dirs.iter().collect();
        assert_eq!(unique.len(), dirs.len());
    }
}
