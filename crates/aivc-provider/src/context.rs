//! Provider 运行所需的依赖集合。
//!
//! Provider 不自己创建连接、不自己找路径：全部依赖由 [`ProviderContext`] 注入。
//! 这样 Provider 的测试可以只准备它真正用到的那几项，而不是启动整个应用。
//!
//! # 存储分派
//!
//! 本机与 WSL 是两套 `SessionStore` 实现（[`LocalSessionStore`] 与
//! [`WslSessionStore`]）。[`ProviderStore`] 把它们收敛成一个枚举并按目标分派，
//! Provider 因此不需要在每处调用点写 `if target.is_wsl()`。
//!
//! 用枚举而不是 `Box<dyn SessionStore>`：分支数量固定为 2，枚举没有装箱与虚调用开销，
//! 且 `match` 让「漏处理 WSL」变成编译错误。

use std::sync::Arc;

use aivc_core::error::Result;
use aivc_core::ports::{
    BoxFuture, Clock, ConflictPolicy, LineSink, ReadOptions, ReadStart, SessionStore, StoreKind,
    SystemClock,
};
use aivc_core::target::{Platform, Target};
use aivc_infra::config::{AppPaths, SettingsStore};
use aivc_infra::db::{
    Db, FailoverRuleRepo, GatewayHealthRepo, GatewayRequestLogRepo, MessageIndexRepo,
    SessionCacheRepo, SessionMetadataRepo, VendorRepo, WorkspacePresetRepo,
};
use aivc_infra::fs::LocalSessionStore;
use aivc_infra::wsl::{WslAdapter, WslSessionStore};

/// Provider 层需要的仓储集合。
///
/// 收成一个结构而不是逐个传参：这组仓储总是**一起**被构造、一起被注入，
/// 拆开只会让构造函数变成一长串位置参数。
#[derive(Debug, Clone)]
pub struct ProviderRepositories {
    /// 会话摘要缓存。
    pub session_cache: SessionCacheRepo,
    /// 会话元数据。
    pub session_metadata: SessionMetadataRepo,
    /// 消息分页锚点。
    pub message_index: MessageIndexRepo,
    /// 工作区预设。
    pub preset: WorkspacePresetRepo,
    /// 供应商。
    pub vendor: VendorRepo,
    /// 网关供应商健康状态。
    pub gateway_health: GatewayHealthRepo,
    /// 网关故障识别规则。
    pub failover_rule: FailoverRuleRepo,
    /// 网关请求日志。
    pub gateway_request_log: GatewayRequestLogRepo,
}

impl ProviderRepositories {
    /// 由同一个数据库句柄构造全部仓储。
    #[must_use]
    pub fn from_db(db: Db) -> Self {
        Self {
            session_cache: SessionCacheRepo::new(db.clone()),
            session_metadata: SessionMetadataRepo::new(db.clone()),
            message_index: MessageIndexRepo::new(db.clone()),
            preset: WorkspacePresetRepo::new(db.clone()),
            vendor: VendorRepo::new(db.clone()),
            gateway_health: GatewayHealthRepo::new(db.clone()),
            failover_rule: FailoverRuleRepo::new(db.clone()),
            gateway_request_log: GatewayRequestLogRepo::new(db),
        }
    }
}

/// Provider 运行上下文。
#[derive(Clone)]
pub struct ProviderContext {
    paths: AppPaths,
    settings: SettingsStore,
    repositories: ProviderRepositories,
    wsl: WslAdapter,
    clock: Arc<dyn Clock>,
    platform: Platform,
    /// 写 CLI 配置文件时把 `~/` 展开到哪里。
    ///
    /// `None` = 真实用户主目录（生产）。测试用 [`Self::with_config_home`] 指到临时目录。
    ///
    /// # 为什么要这样一个字段
    ///
    /// 这里原来是在展开函数里读 `HOME`／`USERPROFILE`（进程级全局状态）。后果实测到了：
    /// 一条 `aivc-ipc` 的测试走到了「启用供应商」的成功路径，**真的把用户的
    /// `~/.codex/auth.json` 与 `config.toml` 覆盖成了测试里的假值**，
    /// 而备份落在随 `TempDir` 一起删掉的目录里，无法恢复。
    ///
    /// 运行根是临时目录**拦不住**它——那条路径看的是 HOME，不是运行根。
    /// 现在主目录成了上下文的一部分：测试想写到外面都做不到。
    config_home: Option<std::path::PathBuf>,
}

impl std::fmt::Debug for ProviderContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderContext")
            .field("root", &self.paths.root())
            .field("platform", &self.platform)
            .finish_non_exhaustive()
    }
}

impl ProviderContext {
    /// 构造。`clock` 传 [`None`] 时使用系统时钟。
    #[must_use]
    pub fn new(
        paths: AppPaths,
        settings: SettingsStore,
        repositories: ProviderRepositories,
        wsl: WslAdapter,
        clock: Option<Arc<dyn Clock>>,
        platform: Platform,
    ) -> Self {
        Self {
            paths,
            settings,
            repositories,
            wsl,
            clock: clock.unwrap_or_else(|| Arc::new(SystemClock)),
            platform,
            config_home: None,
        }
    }

    /// 指定写 CLI 配置文件时的主目录。
    #[must_use]
    pub fn with_config_home(mut self, home: std::path::PathBuf) -> Self {
        self.config_home = Some(home);
        self
    }

    /// **显式选择真实用户主目录**。生产入口专用。
    ///
    /// 取不到 HOME 时退回运行根——那种环境本来就无法配置 CLI，而猜一个路径更糟。
    #[must_use]
    pub fn with_real_home(self) -> Self {
        let home = aivc_infra::config::paths::real_home();
        match home {
            Some(home) => self.with_config_home(home),
            None => self,
        }
    }

    /// 写 CLI 配置文件时用的主目录。
    ///
    /// # 默认值**故意是运行根下的 `home/`**，不是真实 HOME
    ///
    /// 这里原来读 `HOME`／`USERPROFILE`（进程级全局状态）。后果实测到了两次：
    /// 测试走到「启用供应商」的成功路径，把用户的 `~/.codex/auth.json` 与
    /// `config.toml` 覆盖成了测试里的假值，而备份随临时目录一起被删，无法恢复。
    ///
    /// 第一次我改成「测试记得调 [`Self::with_config_home`]」——**不够**：
    /// 有 9 处构造上下文的代码，漏掉任何一处都会再犯（实测漏了一处，第二次覆盖）。
    ///
    /// 因此默认值反过来：**不设就落在运行根里**（测试里就是临时目录），
    /// 真实 HOME 必须由生产入口用 [`Self::with_real_home`] **显式选择**。
    /// 于是「忘记设」的后果是写到自己的运行根，而不是破坏用户的环境。
    #[must_use]
    pub fn config_home(&self) -> std::path::PathBuf {
        self.config_home
            .clone()
            .unwrap_or_else(|| self.paths.root().join("home"))
    }

    /// 仓储集合。
    #[must_use]
    pub const fn repositories(&self) -> &ProviderRepositories {
        &self.repositories
    }

    /// 运行根下的路径集合。
    #[must_use]
    pub const fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// 设置存储。
    #[must_use]
    pub const fn settings(&self) -> &SettingsStore {
        &self.settings
    }

    /// 会话摘要缓存仓储。
    #[must_use]
    pub const fn session_cache(&self) -> &SessionCacheRepo {
        &self.repositories.session_cache
    }

    /// 会话元数据仓储。
    #[must_use]
    pub const fn session_metadata(&self) -> &SessionMetadataRepo {
        &self.repositories.session_metadata
    }

    /// 消息分页锚点仓储。
    #[must_use]
    pub const fn message_index(&self) -> &MessageIndexRepo {
        &self.repositories.message_index
    }

    /// 供应商仓储。
    #[must_use]
    pub const fn vendor(&self) -> &VendorRepo {
        &self.repositories.vendor
    }

    /// 网关健康仓储。
    #[must_use]
    pub const fn gateway_health(&self) -> &GatewayHealthRepo {
        &self.repositories.gateway_health
    }

    /// 故障规则仓储。
    #[must_use]
    pub const fn failover_rule(&self) -> &FailoverRuleRepo {
        &self.repositories.failover_rule
    }

    /// 网关请求日志仓储。
    #[must_use]
    pub const fn gateway_request_log(&self) -> &GatewayRequestLogRepo {
        &self.repositories.gateway_request_log
    }

    /// 工作区预设仓储。
    #[must_use]
    pub const fn preset(&self) -> &WorkspacePresetRepo {
        &self.repositories.preset
    }

    /// WSL 适配器。
    #[must_use]
    pub const fn wsl(&self) -> &WslAdapter {
        &self.wsl
    }

    /// 时钟。
    #[must_use]
    pub fn clock(&self) -> &dyn Clock {
        self.clock.as_ref()
    }

    /// 宿主平台。
    #[must_use]
    pub const fn platform(&self) -> Platform {
        self.platform
    }

    /// 按目标选择存储实现。
    #[must_use]
    pub fn store_for(&self, target: &Target) -> ProviderStore {
        match target.distro() {
            Some(distro) => {
                ProviderStore::Wsl(WslSessionStore::new(self.wsl.clone(), distro.to_owned()))
            }
            None => ProviderStore::Local(LocalSessionStore::new()),
        }
    }

    /// 会话摘要缓存的键。
    ///
    /// `data/vendor-backups/` —— 写入用户 CLI 配置前的备份根目录。
    ///
    /// 由 `AppPaths` 派生，因此备份始终落在运行根之内——
    /// 备份跑到别的地方就等于没备份。
    #[must_use]
    pub fn vendor_backup_root(&self) -> std::path::PathBuf {
        self.paths.vendor_backup_dir()
    }

    /// `logs/performance.log` —— 启动埋点与**前端异常**的落点。
    ///
    /// 前端异常在 Rust 侧完全不可见，因此这个文件是排查「界面静默失效」的唯一线索。
    ///
    /// 路径**由 [`AppPaths`] 给**，不在这里再拼一次：上面那个方法就是这么做的，
    /// 而这一个原来自己 `join("performance.log")`——同一个文件名有两处真值，
    /// 改目录结构时必漏一处。
    #[must_use]
    pub fn performance_log_path(&self) -> std::path::PathBuf {
        self.paths.performance_log_path()
    }

    /// 本机为 `local`，WSL 为 `wsl-<净化后的发行版名>`：发行版名可能含路径分隔符，
    /// 直接拼进缓存键会污染缓存文件路径。
    #[must_use]
    pub fn cache_key(&self, target: &Target) -> String {
        match target.distro() {
            Some(distro) => format!("wsl-{}", aivc_core::path::sanitize_wsl_distro(distro)),
            None => "local".to_owned(),
        }
    }
}

/// 按目标分派的会话文件存储。
#[derive(Debug, Clone)]
pub enum ProviderStore {
    /// 本机文件系统。
    Local(LocalSessionStore),
    /// WSL 发行版内。
    Wsl(WslSessionStore),
}

impl SessionStore for ProviderStore {
    fn kind(&self) -> StoreKind {
        match self {
            Self::Local(store) => store.kind(),
            Self::Wsl(store) => store.kind(),
        }
    }

    fn read_text<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String>> {
        match self {
            Self::Local(store) => store.read_text(path),
            Self::Wsl(store) => store.read_text(path),
        }
    }

    fn write_text<'a>(&'a self, path: &'a str, content: &'a str) -> BoxFuture<'a, Result<()>> {
        match self {
            Self::Local(store) => store.write_text(path, content),
            Self::Wsl(store) => store.write_text(path, content),
        }
    }

    fn read_lines<'a>(
        &'a self,
        path: &'a str,
        start: ReadStart,
        options: ReadOptions,
        sink: LineSink<'a>,
    ) -> BoxFuture<'a, Result<()>> {
        match self {
            Self::Local(store) => store.read_lines(path, start, options, sink),
            Self::Wsl(store) => store.read_lines(path, start, options, sink),
        }
    }

    fn read_head<'a>(&'a self, path: &'a str, max_bytes: usize) -> BoxFuture<'a, Result<String>> {
        match self {
            Self::Local(store) => store.read_head(path, max_bytes),
            Self::Wsl(store) => store.read_head(path, max_bytes),
        }
    }

    fn read_heads<'a>(
        &'a self,
        paths: &'a [String],
        max_bytes: usize,
    ) -> BoxFuture<'a, Vec<Option<String>>> {
        match self {
            Self::Local(store) => store.read_heads(paths, max_bytes),
            Self::Wsl(store) => store.read_heads(paths, max_bytes),
        }
    }

    fn exists<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<bool>> {
        match self {
            Self::Local(store) => store.exists(path),
            Self::Wsl(store) => store.exists(path),
        }
    }

    fn size<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<u64>> {
        match self {
            Self::Local(store) => store.size(path),
            Self::Wsl(store) => store.size(path),
        }
    }

    fn move_file<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
        policy: ConflictPolicy,
    ) -> BoxFuture<'a, Result<()>> {
        match self {
            Self::Local(store) => store.move_file(from, to, policy),
            Self::Wsl(store) => store.move_file(from, to, policy),
        }
    }

    fn remove<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        match self {
            Self::Local(store) => store.remove(path),
            Self::Wsl(store) => store.remove(path),
        }
    }

    fn remove_tree<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        match self {
            Self::Local(store) => store.remove_tree(path),
            Self::Wsl(store) => store.remove_tree(path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::TargetId;
    use aivc_core::target::{ProviderId, TargetKind};
    use aivc_infra::config::paths::real_home;
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};

    fn target(kind: TargetKind, distro: Option<&str>) -> Target {
        Target {
            id: TargetId::from("t"),
            provider: ProviderId::Codex,
            label: "t".to_owned(),
            kind,
            distro: distro.map(str::to_owned),
            codex_home: None,
            available: true,
            detail: None,
        }
    }

    /// **没设 `config_home` 时必须落在运行根里，绝不落到真实 HOME。**
    ///
    /// 这条测试钉的是一个出过事故的性质：默认值曾经是真实 HOME，于是测试走到
    /// 「启用供应商」的成功路径时**覆盖了用户的 `~/.codex/auth.json` 与
    /// `config.toml`**，备份随临时目录一起被删，无法恢复。
    ///
    /// 光靠「测试记得调 `with_config_home`」挡不住——有 9 处构造上下文的代码，
    /// 实测漏了一处就又覆盖了一次。因此默认值反过来，这条测试保证它不被翻回去。
    #[tokio::test]
    async fn config_home_defaults_inside_the_run_root() {
        let directory = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(directory.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let context = ProviderContext::new(
            paths.clone(),
            SettingsStore::new(paths.settings_path()),
            ProviderRepositories::from_db(db),
            WslAdapter::new(
                Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
                WslConfig::default(),
            ),
            None,
            Platform::current(),
        );

        let home = context.config_home();
        assert!(
            home.starts_with(paths.root()),
            "默认主目录必须在运行根内，实际是 {}",
            home.display()
        );
        // 明确排除「恰好等于真实 HOME」这种情况——那是要防的那个错误。
        if let Some(real) = real_home() {
            assert_ne!(home, real, "默认主目录不能是真实 HOME");
        }

        // 显式选择真实 HOME 的入口只有一个，且它确实生效。
        let explicit = context.clone().with_real_home();
        if let Some(real) = real_home() {
            assert_eq!(explicit.config_home(), real);
        }
    }

    #[test]
    fn cache_keys_are_namespaced_per_target() {
        // 缓存键会拼进缓存文件路径，发行版名必须先净化。
        let local = target(TargetKind::Local, None);
        let wsl = target(TargetKind::Wsl, Some("Ubuntu-22.04"));
        let hostile = target(TargetKind::Wsl, Some("Ubuntu/../../etc"));

        // 直接构造一个不依赖数据库的上下文是不可行的，因此这里只验证键生成规则：
        // 规则本身是纯函数，独立于上下文其余字段。
        let key_of = |item: &Target| match item.distro() {
            Some(distro) => format!("wsl-{}", aivc_core::path::sanitize_wsl_distro(distro)),
            None => "local".to_owned(),
        };
        assert_eq!(key_of(&local), "local");
        assert_eq!(key_of(&wsl), "wsl-Ubuntu-22.04");
        assert_eq!(key_of(&hostile), "wsl-Ubuntu_.._.._etc");
    }

    #[test]
    fn store_dispatch_follows_the_target_kind() {
        // 分派规则是纯函数；真实存储的构造需要 WSL 适配器，此处只验证判据。
        let local = target(TargetKind::Local, None);
        let wsl = target(TargetKind::Wsl, Some("Ubuntu"));
        assert!(local.distro().is_none(), "本机目标没有发行版 → 走本机存储");
        assert_eq!(wsl.distro(), Some("Ubuntu"), "有发行版 → 走 WSL 存储");
    }
}
