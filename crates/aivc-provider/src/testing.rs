//! 测试支撑：在临时目录上搭一个可用的 [`ProviderContext`]。
//!
//! 只在单元测试中使用。真实的 `ProviderContext` 需要运行根、设置文件、数据库与 WSL
//! 适配器；这里把它们全部指向临时目录，因此测试互不干扰、也不碰用户的真实数据。
//!
//! WSL 适配器默认指向一个**不可用**的本机 bash（发行版参数被忽略），因此本机路径的
//! 测试不受 WSL 是否存在影响；需要真实 bash 的测试自行用
//! [`crate::testing::usable_bash`] 探测。

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aivc_core::id::TargetId;
use aivc_core::ports::StoreKind;
use aivc_core::target::{Platform, ProviderId, Target, TargetKind};
use aivc_infra::config::{AppPaths, SettingsStore};
use aivc_infra::db::Db;
use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig, WslLauncher};

use crate::context::{ProviderContext, ProviderRepositories};

/// 一个隔离的测试上下文。
pub struct TestContext {
    /// 已构造好的上下文。
    pub context: ProviderContext,
    /// 充当 `CODEX_HOME` 的目录。
    pub codex_home: PathBuf,
    directory: tempfile::TempDir,
}

impl TestContext {
    /// 在临时目录上构造。
    pub async fn new() -> Self {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = directory.path().to_path_buf();
        let codex_home = root.join("codex-home");

        let paths = AppPaths::new(root.join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("create runtime dirs");
        }
        let db = Db::open(paths.database_path())
            .await
            .expect("open database");
        let settings = SettingsStore::new(paths.settings_path());

        let launcher: Arc<dyn WslLauncher> =
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash"));
        let wsl = WslAdapter::new(
            launcher,
            WslConfig {
                // 测试里不需要 worker：它只会引入与协议无关的失败面。
                use_worker: false,
                ..WslConfig::default()
            },
        );

        let context = ProviderContext::new(
            paths,
            settings,
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );

        Self {
            context,
            codex_home,
            directory,
        }
    }

    /// 临时目录根。
    #[must_use]
    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    /// 构造一个本机 Codex 目标。
    #[must_use]
    pub fn local_target(&self) -> Target {
        Target {
            id: TargetId::from("codex:local"),
            provider: ProviderId::Codex,
            label: "本机 Codex".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some(self.codex_home.to_string_lossy().into_owned()),
            available: true,
            detail: None,
        }
    }

    /// 在 `codex_home` 下写一个会话文件并返回其路径。
    ///
    /// `relative` 形如 `sessions/2026/09/16/rollout-...-<uuid>.jsonl`。
    pub fn write_session(&self, relative: &str, content: &str) -> PathBuf {
        let path = self
            .codex_home
            .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create session dir");
        }
        std::fs::write(&path, content).expect("write session");
        path
    }

    /// 本机存储的缓存键。
    #[must_use]
    pub fn cache_key(&self, target: &Target) -> String {
        self.context.cache_key(target)
    }
}

/// 判断本机 bash 是否真的能跑（受限沙箱下 MSYS2 无法创建 signal pipe）。
#[must_use]
pub fn usable_bash() -> Option<LocalBashLauncher> {
    LocalBashLauncher::detect().filter(LocalBashLauncher::is_usable)
}

/// 本机存储类型（测试断言用）。
#[must_use]
pub const fn local_kind() -> StoreKind {
    StoreKind::Local
}

/// 生成一个合法的会话文件名（含 uuid）。
#[must_use]
pub fn session_file_name(stamp: &str, uuid: &str) -> String {
    format!("rollout-{stamp}-{uuid}.jsonl")
}

/// 一段最小可解析的会话内容。
#[must_use]
pub fn session_content(uuid: &str, title: &str) -> String {
    [
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:00.000Z","type":"session_meta","payload":{{"id":"{uuid}","timestamp":"2026-09-16T12:00:00.000Z","cwd":"/work","model":"gpt-5"}}}}"#
        ),
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{{"type":"user_message","message":"{title}"}}}}"#
        ),
    ]
    .join("\n")
}
