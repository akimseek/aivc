//! 组装根：把各层接到一起，并持有应用生命周期。
//!
//! # 为什么需要这一层
//!
//! 前面每一层都只依赖它下面那层：Provider 不知道网关存在，网关不知道命令面存在。
//! 总得有一处把它们**按正确的顺序**建起来——数据库先于仓储、仓储先于上下文、上下文先于
//! 路由器。散在 `main` 或 Tauri 的 `setup` 里会让「哪些东西必须一起存在」变得不可见，
//! 也没法在测试里构造一个完整的应用。
//!
//! 因此这里提供一个 [`App`]：一次 `start` 建好全部组件，`dispatch` 转发命令，
//! `shutdown` 有序收尾。
//!
//! # 启动顺序为什么不能随意调换
//!
//! 1. **目录先于数据库**——SQLite 不会替我们建目录。
//! 2. **数据库先于仓储**——仓储只是句柄，但 `Db::open` 会跑迁移。
//! 3. **上下文先于网关来源**——网关来源要读设置、要拿仓储。
//! 4. **网关来源先于网关状态**——状态构造时要读一次设置快照。
//! 5. **路由器最后**——它要上下文与供应商注册表。
//!
//! # 退出顺序
//!
//! 与启动相反：先停网关（可能还有在途请求），其余组件随 `Drop` 释放。
//! 现网还要求「停 PTY → 停 Gateway → flush 日志」，PTY 部分接入后再补。

pub mod exit;
pub mod harden;
pub mod logging;
pub mod shutdown;
pub mod tauri_app;
pub mod tray;

use std::sync::Arc;

use anyhow::Context as _;

use aivc_core::error::Result;
use aivc_core::target::{Platform, ProviderId, Target, TargetKind};
use aivc_gateway::{GatewayHandle, GatewaySettings, GatewayState, RepositorySource};
use aivc_infra::config::{AppPaths, SettingsStore};
use aivc_infra::db::Db;
use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
use aivc_ipc::{CommandRouter, TargetLookup};
use aivc_provider::{ProviderContext, ProviderRepositories, build_registry};

/// 启动预热只针对这一个平台。
///
/// Codex 是本应用的主平台（`provider_summaries` 把它排在第一位，界面默认也选它）。
/// 其余平台在用户切过去时按需探测——那一次探测本来就要付，提前付只是把它变成
/// **每一个用户都为用不到的平台各付一遍**。
const PRIMARY_WARM_PROVIDER: ProviderId = ProviderId::Codex;

/// 由设置缓存提供目标列表的查找器。
struct RepositoryTargets {
    context: ProviderContext,
}

impl TargetLookup for RepositoryTargets {
    fn resolve(&self, target_id: &str) -> aivc_core::ports::BoxFuture<'_, Option<Target>> {
        let target_id = target_id.to_owned();
        Box::pin(async move {
            self.cached_targets()
                .await
                .into_iter()
                .find(|target| target.id.as_str() == target_id)
        })
    }

    fn list(&self) -> aivc_core::ports::BoxFuture<'_, Vec<Target>> {
        Box::pin(async move { self.cached_targets().await })
    }
}

impl RepositoryTargets {
    /// 读取缓存的目标列表。
    ///
    /// **只读缓存、不主动探测**：界面首屏要立刻出画面。真正的探测由
    /// `codex:list-targets` 的完整实现触发并回写缓存（尚未接线，因此当前列表恒为空）。
    async fn cached_targets(&self) -> Vec<Target> {
        self.context.settings().snapshot().await.cached_targets
    }
}

/// 运行中的应用。
pub struct App {
    router: CommandRouter,
    /// 事件发射器槽位。`App::start` 时装的是 [`aivc_ipc::terminal::NullEmitter`]
    /// （那时窗口还不存在），由 `tauri_app::run` 换成真实发射器。
    emitter: Arc<std::sync::RwLock<Arc<dyn aivc_ipc::terminal::EventEmitter>>>,
    gateway_state: Arc<GatewayState>,
    gateway: Option<GatewayHandle>,
    paths: AppPaths,
}

impl std::fmt::Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("paths", &self.paths.root())
            .field(
                "gateway",
                &self.gateway.as_ref().map(|item| item.address.as_str()),
            )
            .finish_non_exhaustive()
    }
}

impl App {
    /// 建好全部组件。
    ///
    /// `bash` 是用于 WSL 交互的 bash 路径；为 [`None`] 时 WSL 目标不可用（测试与非 WSL
    /// 环境）。
    ///
    /// # Errors
    ///
    /// 目录创建、数据库打开或迁移、设置读取失败时返回错误。这些都属于「不修好就没法用」
    /// 的问题，因此启动阶段必须显式失败，而不是降级运行。
    pub async fn start(
        root: impl Into<std::path::PathBuf>,
        bash: Option<&str>,
    ) -> anyhow::Result<Self> {
        let paths = AppPaths::new(root);
        for directory in paths.required_directories() {
            std::fs::create_dir_all(&directory)
                .with_context(|| format!("创建目录失败：{}", directory.display()))?;
        }

        let db = Db::open(paths.database_path())
            .await
            .context("打开会话数据库失败")?;

        let settings_store = SettingsStore::new(paths.settings_path());
        // **生产必须用 `WslExeLauncher`**。
        //
        // 这里曾经无条件用 `LocalBashLauncher`——它是**测试实现**，文档写着
        // 「忽略发行版参数，仅用于验证协议本身；生产代码不得使用」。
        // 后果是发布版本**永远看不到 WSL**：`is_available()` 为真（Git Bash 在），
        // 但 `list_distros()` 返回空，于是目标列表只有「本机」。
        //
        // 实测证据：`wsl.exe -l -q` 返回 Ubuntu-22.04/24.04，且 Ubuntu-24.04 下
        // `~/.codex/sessions` 确实存在，而应用探测到的 WSL 目标数为 **0**，
        // 且整个探测只用了 0.06 秒（根本没调 `wsl.exe`）。
        //
        // `bash` 参数保留给测试：传了就说明调用方要的是本机 bash 语义。
        let launcher: Arc<dyn aivc_infra::wsl::WslLauncher> = match bash {
            Some(path) => Arc::new(LocalBashLauncher::new(path)),
            None => aivc_infra::wsl::WslExeLauncher::detect().map_or_else(
                || {
                    // **降级必须出声**。静默退回裸文件名曾让「为什么没有 WSL 目标」
                    // 变成一个查不动的矛盾：日志只说「未找到 wsl.exe」，
                    // 而同一台机器上该文件确实存在。
                    tracing::warn!(
                        "未探测到 wsl.exe，回退到裸命令名；若本机装有 WSL，\
                         请检查上一条「找不到 wsl.exe」日志里列出的候选路径"
                    );
                    Arc::new(aivc_infra::wsl::WslExeLauncher::new("wsl.exe"))
                        as Arc<dyn aivc_infra::wsl::WslLauncher>
                },
                |found| Arc::new(found),
            ),
        };
        // **打开常驻 worker**。
        //
        // # 它当初为什么被关掉
        //
        // 2026-09-25 的实机日志显示两类错误：
        //
        // ```text
        // worker 调用失败，回退到一次性执行 distro="Ubuntu-24.04" operation="exists"
        //     error=WSL 持久 worker 协议异常：worker 已退出
        // worker 调用失败，回退到一次性执行 distro="Ubuntu-22.04" operation="exists"
        //     error=WSL 操作超时（60000 ms）：worker 响应
        // ```
        //
        // 一次目标探测因此要 198 秒。当时把它从生产路径上摘了下来——**那不是修好，
        // 是绕开**。
        //
        // # 真正的缺陷
        //
        // 「worker 已退出」= worker 的 stdout 立刻 EOF = `while read` 第一轮就结束。
        // 这只可能是 **stdin 已经是 EOF**：`WslExeLauncher::spawn_bash` 把 stdin 设成了
        // `Stdio::null()`（那是为了修一次性路径上「误读 stdin 导致 30 秒阻塞」而做的），
        // 而 `WslWorker::spawn` 却要求 stdin 是管道。**一个方法承担了两种相反的契约。**
        //
        // 这个缺陷在测试里永远看不见：测试用的是 `LocalBashLauncher`，它当时恰好把
        // stdin 设成了管道。现在两种 launcher 都按同一份契约拆成了
        // `spawn_bash`（stdin = null）与 `spawn_bash_interactive`（stdin = 管道）。
        //
        // # 为什么现在必须开
        //
        // 会话列表的实测数据：WSL2 会在**空闲约 20 秒后自动关闭发行版**，下次调用要
        // 冷启动 **5–7 秒**。一次性路径每次操作都是一个新进程，一旦调用之间有间隔就反复
        // 付这个代价。常驻 worker 让发行版保持活动，从而消掉冷启动。
        let wsl = WslAdapter::new(launcher, WslConfig::default());

        let context = ProviderContext::new(
            paths.clone(),
            settings_store.clone(),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );

        // 网关来源要读设置与仓储，因此必须在上下文之后。
        //
        // 装上记录钩子把「请求已记录」推给界面：工作台靠这个事件实时更新，**不轮询**。
        // 钩子读的是 `emitter` 这个可替换的槽位——`App::start` 时窗口还不存在，
        // 真实发射器由 `tauri_app::run` 稍后装入。
        let emitter_slot = Arc::new(std::sync::RwLock::new(
            Arc::new(aivc_ipc::terminal::NullEmitter) as Arc<dyn aivc_ipc::terminal::EventEmitter>,
        ));
        let hook_slot = Arc::clone(&emitter_slot);
        let record_hook: aivc_gateway::RecordHook = Arc::new(move |record| {
            let payload = serde_json::json!({
                "requestId": record.request_id.as_str(),
                "providerId": record.provider_id.as_str(),
                "vendorId": record.vendor_id.as_str(),
                "outcome": record.outcome.as_str(),
                "upstreamStatus": record.upstream_status,
                "durationMs": record.duration_ms,
                "switched": record.switched,
            });
            // 读锁失败（写入方 panic）时静默跳过：事件送不到不该让转发失败。
            if let Ok(emitter) = hook_slot.read() {
                emitter.emit("gateway:request-recorded", payload);
            }
        });
        let source = Arc::new(
            RepositorySource::new(
                context.vendor().clone(),
                context.failover_rule().clone(),
                context.gateway_request_log().clone(),
                settings_store,
            )
            .await
            .context("读取网关设置失败")?
            .with_record_hook(record_hook),
        );

        let gateway_state = Arc::new(GatewayState::new(source).context("构造网关失败")?);

        // **供应商健康要落库。**
        //
        // 不接这一步时有两个后果：熔断活不过一次重启（内存里的状态从不读回），
        // 而界面上的健康视图恒为空（`gateway:get-vendor-health` 读的那张表没有写入方）。
        //
        // 用钩子而不是让网关直接依赖仓储：与 `set_switch_hook` 同一个模式，
        // 网关仍然不认识数据库，测试替身也不必实现任何东西。
        {
            let health = context.gateway_health().clone();
            let loader = health.clone();
            gateway_state.set_health_hook(aivc_gateway::HealthHook {
                load: Arc::new(move || {
                    let repo = loader.clone();
                    Box::pin(async move { repo.load_all().await.unwrap_or_default() })
                }),
                save: Arc::new(move |entry| {
                    let repo = health.clone();
                    Box::pin(async move {
                        // 落库失败只记日志：健康状态写不进去不该影响已经完成的转发。
                        if let Err(error) = repo.upsert(&entry).await {
                            tracing::warn!(%error, "写入供应商健康状态失败");
                        }
                    })
                }),
            });
        }

        // 路由切换同样推给界面：工作台的「当前供应商」靠它实时更新。
        //
        // 与记录钩子共用同一个发射器槽位——两者都要等窗口建好才有接收方。
        {
            let switch_slot = Arc::clone(&emitter_slot);
            gateway_state.set_switch_hook(Arc::new(move |notice| {
                let payload = serde_json::json!({
                    // 现网把它叫 `terminalId`；路由编号就是终端绑定的标识，
                    // 因此两个键都给，前端用哪个都行。
                    "terminalId": notice.route_id,
                    "routeId": notice.route_id,
                    "vendorId": notice.vendor_id,
                    "reason": notice.reason,
                    "mode": if notice.mode == aivc_gateway::RouteMode::Locked {
                        "locked"
                    } else {
                        "dynamic"
                    },
                });
                if let Ok(emitter) = switch_slot.read() {
                    emitter.emit("gateway:vendor-switched", payload);
                }
            }));
        }

        let router = CommandRouter::new(
            context.clone(),
            Arc::new(build_registry()),
            Arc::new(RepositoryTargets {
                context: context.clone(),
            }),
            // 版本号由这里给出：ivc-ipc 不依赖构建脚本生成的常量。
            env!("CARGO_PKG_VERSION"),
        )
        // 装入网关状态：路由切换命令需要它。
        .with_gateway(Arc::clone(&gateway_state));

        // **预热刻意不在这里启动**：它由入口（`main.rs`）显式 `spawn`。
        // 理由见 [`App::warm_targets`]——它必须由调用方提供运行时，而且不能与
        // 调用方并发地改目标缓存。
        Ok(Self {
            router,
            gateway_state,
            gateway: None,
            paths,
            emitter: emitter_slot,
        })
    }

    /// 启动预热：**只预热主平台**，并优先复用缓存。
    ///
    /// # 为什么只预热主平台
    ///
    /// 这里原来无条件探测**全部 4 个平台 × 2 个发行版**，实测 **6.3 秒**的 `wsl.exe`
    /// 调用（日志：`目标预热完成 count=8 elapsed_ms=6296`），而且和界面自己的首次探测
    /// 抢同一个发行版——反而让首屏更慢。
    ///
    /// 其余平台在用户切过去时按需探测：那一次探测本来就要付，提前付只是把它变成
    /// **每一个用户都为用不到的平台各付一遍**。
    ///
    /// # 预热真正不可替代的作用
    ///
    /// 只有一个：**让发行版热起来**（WSL2 空闲约 20 秒就会关掉它，冷启动 5–7 秒）。
    /// 目标缓存本身由界面的首次探测刷新，不需要预热代劳——用户选中平台后，界面一定会
    /// 调一次 `codex:list-targets`。
    ///
    /// 因此分两步：
    ///
    /// 1. **有缓存**（后续启动）：只按缓存里的可达 WSL 目标温热 worker，一个目标一次
    ///    廉价探测；
    /// 2. **没缓存**（首次启动）：才做完整探测，而且只探主平台。
    ///
    /// # 为什么抽成独立方法，而不是留在 `App::start` 里
    ///
    /// 一是 `App::start` 要表达的是**装配顺序**，预热不属于那里；
    /// 二是预热跑在后台任务里时测试无法确定性地等它结束，抽出来之后测试可以直接 `await`。
    ///
    /// # 而且**必须**由调用方启动，不能由 `App::start` 代劳
    ///
    /// 它曾经由 `App::start` 里的一句 `tokio::spawn` 启动，后果是一处随机的测试失败：
    /// 预热的「首次启动」分支会真实探测目标，而探测结果按平台**整片替换**缓存
    /// （`merge_cached_targets`）。测试注入的合成目标恰好也叫 `codex:local`——
    /// 后台预热一旦跑在注入之后，就把它换成真机上那个，或者真机探测不到时整个清掉。
    /// 症状是「未解析到目标：codex:local」，**每次命中不同的测试**，实测 10 次里 1 次。
    ///
    /// 现在由生产入口（`main.rs`）用 `tauri::async_runtime::spawn` 启动，测试不启动。
    /// **刻意不提供 `App::spawn_warmup` 之类的包装**：同步版本在 `block_on` 之外调用会
    /// 直接 panic（`there is no reactor running`，实测启动即崩）——而单元测试看不见，
    /// 因为 `#[tokio::test]` 自带运行时。让调用方明确地提供运行时，比包装一层更安全。
    ///
    /// **失败只记日志**：预热失败不该影响启动，真正的探测还会照常进行。
    pub async fn warm_targets(context: ProviderContext) {
        let started = std::time::Instant::now();
        let cached: Vec<Target> = context
            .settings()
            .snapshot()
            .await
            .cached_targets
            .into_iter()
            .filter(|target| target.provider == PRIMARY_WARM_PROVIDER)
            .collect();

        if cached.is_empty() {
            // 首次启动：**必须**探一次，否则界面上一个目标都没有。
            let sessions = aivc_ipc::sessions::SessionCommands::new(
                Arc::new(build_registry()),
                Arc::new(RepositoryTargets {
                    context: context.clone(),
                }),
            );
            let only = serde_json::json!({ "providerId": PRIMARY_WARM_PROVIDER.as_str() });
            match sessions.list_targets(&context, &only).await {
                Ok(targets) => tracing::info!(
                    count = targets.len(),
                    elapsed_ms = started.elapsed().as_millis(),
                    "首次启动预热完成（仅主平台）"
                ),
                Err(error) => tracing::debug!(%error, "目标预热失败（不影响启动）"),
            }
            return;
        }

        // 后续启动：缓存已经有了，预热只剩「让发行版热起来」这一件事。
        let mut warmed = 0usize;
        for target in cached
            .iter()
            .filter(|target| target.kind == TargetKind::Wsl && target.available)
        {
            let (Some(distro), Some(home)) =
                (target.distro.as_deref(), target.codex_home.as_deref())
            else {
                continue;
            };
            // 走 worker：这一次调用就会把常驻 worker 建起来，发行版因此在接下来一段
            // 时间里保持活动。（worker 失败会自动回退，不影响结果。）
            let _ = context
                .wsl()
                .exists(distro, &format!("{home}/sessions"))
                .await;
            warmed += 1;
        }
        tracing::info!(
            warmed,
            cached = cached.len(),
            elapsed_ms = started.elapsed().as_millis(),
            "目标预热完成（复用缓存，仅温热 worker）"
        );
    }

    /// 命令路由器。
    #[must_use]
    pub const fn router(&self) -> &CommandRouter {
        &self.router
    }

    /// 运行根。
    #[must_use]
    pub const fn paths(&self) -> &AppPaths {
        &self.paths
    }

    /// 网关共享状态。
    #[must_use]
    pub const fn gateway_state(&self) -> &Arc<GatewayState> {
        &self.gateway_state
    }

    /// 已启动的网关地址；未启动时为 [`None`]。
    #[must_use]
    pub fn gateway_address(&self) -> Option<&str> {
        self.gateway.as_ref().map(|handle| handle.address.as_str())
    }

    /// 装入真实事件发射器。
    ///
    /// 由 `tauri_app::run` 在窗口建好后调用。在此之前装的是
    /// [`aivc_ipc::terminal::NullEmitter`]——启动早期的事件被**丢弃**而不是报错：
    /// 那时本来也没有接收方，而为一个没有接收方的事件让启动失败是不划算的。
    pub fn install_emitter(&self, emitter: Arc<dyn aivc_ipc::terminal::EventEmitter>) {
        if let Ok(mut slot) = self.emitter.write() {
            *slot = Arc::clone(&emitter);
        }
        // **终端处理器也要换。**
        //
        // 它在路由器构造时就拿到了一份发射器（那时窗口还不存在，只能先给 NullEmitter），
        // 而那是**另一份**。只换上面那个槽位的话，终端输出会一直写进虚空：
        // 进程正常启动、不报任何错、界面永远空白。网关事件早就走槽位模式，
        // 终端此前没跟上。
        self.router.install_terminal_emitter(emitter);
    }

    /// 按设置决定是否启动网关。
    ///
    /// 设置里关闭网关时什么都不做——比「启动了再关掉」少一次端口占用窗口。
    ///
    /// # Errors
    ///
    /// 找不到可用端口时返回错误。
    pub async fn start_gateway_if_enabled(&mut self) -> anyhow::Result<Option<String>> {
        let settings: GatewaySettings = self.gateway_state.settings();
        if !settings.enabled {
            return Ok(None);
        }
        // **先把熔断状态读回来再开始服务。**
        //
        // 放在 `serve` 之前：之后的每次请求都要靠它判断哪些供应商已被熔断。
        self.gateway_state.hydrate_health().await;
        let handle = aivc_gateway::serve(Arc::clone(&self.gateway_state))
            .await
            .context("启动本地网关失败")?;
        let address = handle.address.clone();
        self.gateway = Some(handle);
        Ok(Some(address))
    }

    /// 转发一条命令。
    ///
    /// # Errors
    ///
    /// 未知命令、未接线命令、入参非法或业务失败时返回错误。
    pub async fn dispatch(
        &self,
        command: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.router.dispatch(command, args).await
    }

    /// 有序收尾。
    pub async fn shutdown(mut self) {
        if let Some(handle) = self.gateway.take() {
            handle.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 构造一个只依赖临时目录的应用。
    async fn app() -> (App, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = App::start(dir.path().join("runtime"), None)
            .await
            .expect("start");
        (app, dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn starting_creates_the_required_directories() {
        let (app, _dir) = app().await;
        for directory in app.paths().required_directories() {
            assert!(directory.is_dir(), "{} 未创建", directory.display());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn starting_creates_the_database_with_migrations_applied() {
        let (app, _dir) = app().await;
        assert!(app.paths().database_path().is_file());
        // 迁移跑过之后仓储应当可以正常查询，而不是报「表不存在」。
        let vendors = app
            .dispatch("vendor:list", json!({}))
            .await
            .expect("list vendors");
        assert!(vendors.as_array().expect("数组").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commands_dispatch_through_the_assembled_app() {
        let (app, _dir) = app().await;
        let saved = app
            .dispatch(
                "workspace:save-preset",
                json!({ "name": "工作区", "cwd": "/w" }),
            )
            .await
            .expect("save preset");
        assert!(saved.is_string());

        let presets = app
            .dispatch("workspace:list-presets", json!({}))
            .await
            .expect("list presets");
        assert_eq!(presets.as_array().expect("数组").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_gateway_is_not_started_when_disabled_in_settings() {
        let (mut app, _dir) = app().await;
        app.router
            .context()
            .settings()
            .update(|raw| raw.set_gateway_enabled(false))
            .await
            .expect("update");
        app.gateway_state().refresh_settings().await;

        let started = app.start_gateway_if_enabled().await.expect("start");
        assert!(started.is_none(), "设置关闭时不该启动网关");
        assert!(app.gateway_address().is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_gateway_starts_when_enabled_and_reports_its_address() {
        let (mut app, _dir) = app().await;
        // 端口 0 让操作系统分配，测试之间不会互相冲突。
        app.router
            .context()
            .settings()
            .update(|raw| {
                raw.set_gateway_enabled(true);
                raw.set_gateway_port(0);
            })
            .await
            .expect("update");
        app.gateway_state().refresh_settings().await;

        let address = app
            .start_gateway_if_enabled()
            .await
            .expect("start")
            .expect("已启动");
        assert!(address.starts_with("http://127.0.0.1:"), "{address}");
        assert_eq!(app.gateway_address(), Some(address.as_str()));
        app.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_command_still_reports_through_the_app() {
        let (app, _dir) = app().await;
        let error = app
            .dispatch("nope:nope", json!({}))
            .await
            .expect_err("必须失败");
        assert_eq!(error.to_string(), "未知命令：nope:nope");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_command_reports_the_gap_through_the_app() {
        // 命令面的缺口在组装之后依然可见。
        //
        // 用一条**由界面层处理**的命令：组装根这一层的路由器接不住它，因此落到
        // 「尚未接线」的兜底上。断言的价值在**缺口可见**，而不是某个具体命令未实现。
        let (app, _dir) = app().await;
        let error = app
            .dispatch("clipboard:copy-text", json!({}))
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("命令尚未接线"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn target_listing_is_empty_without_a_cached_probe() {
        // 只读缓存：没探测过时列表为空，而不是去碰文件系统。
        let (app, _dir) = app().await;
        let targets = app
            .dispatch("codex:list-cached-targets", json!({}))
            .await
            .expect("targets");
        assert!(targets.as_array().expect("数组").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_without_a_gateway_is_safe() {
        let (app, _dir) = app().await;
        app.shutdown().await;
    }
}
