//! 命令路由：把 `(命令名, 参数)` 分发到对应处理函数。
//!
//! # 为什么先做路由、后接 Tauri
//!
//! Tauri 的 `#[tauri::command]` 只是把一次 IPC 调用转成一次 Rust 函数调用。真正有内容
//! 的是**分发逻辑**：81 条命令名映射到哪些处理函数、返回什么形状、未知命令与未实现命令
//! 分别怎么报错。把这部分做成纯函数之后：
//!
//! - 分发可以脱离 Tauri 穷举测试；
//! - Tauri 接入退化成约十行胶水（一个 `invoke(command, args)` 转调 [`CommandRouter::dispatch`]）；
//! - 依赖注入的失败面从「几百个 crate 的构建」收敛到「一个函数是否正确」。
//!
//! # 未实现的命令必须显式报错
//!
//! [`PENDING`] 列出已注册但尚未接线的命令。它们返回明确的「未实现」错误，而不是静默
//! 返回空值——静默成功会让渲染层以为操作成功了。有一条测试断言
//! `已实现 + PENDING == 81`，因此这个缺口是可追踪、只在缩小而不会悄悄变大的。

use std::sync::Arc;

use serde_json::Value;

use aivc_core::error::{Error, IpcError, Result};
use aivc_provider::ProviderContext;

use crate::commands::{COMMAND_COUNT, CommandDomain, domain_of, is_known};
use crate::gateway::{GatewayHealth, GatewayLogs, GatewayRules, GatewaySettingsCommands};
use crate::sessions::{SessionCommands, TargetLookup};
use crate::settings::{SessionSettings, WorkspaceCommands};
use crate::validate::{optional_string, require_boolean, require_positive_integer, require_string};
use crate::vendor::VendorCommands;

/// 由**界面层**处理的命令：`aivc-app` 的 `platform_command` 在转给路由器之前拦下它们。
///
/// # 为什么不在这里实现
///
/// 这几条要用 Tauri 的平台能力（剪贴板、原生对话框、系统通知），而 `aivc-ipc`
/// **刻意不依赖 tauri**——命令层保持与界面框架无关，才能在测试里直接驱动。
///
/// # 为什么必须显式列出来
///
/// 对路由器来说它们「不归我管」，看起来就像未接线。这个跨层契约**如果只存在于人的
/// 记忆里**，就会出现「命令表说已实现、路由器接不住、界面层其实处理了」这种三方各说
/// 各话的状态——而单元测试仍然是全绿的。
///
/// `every_implemented_command_is_actually_routed` 拿这张清单区分「故意不在这一层」
/// 与「漏接了」。界面层新增这类命令时必须同步加到这里。
pub const HANDLED_BY_PLATFORM: [&str; 7] = [
    "app:command",
    "app:exit-choice",
    "clipboard:copy-text",
    "clipboard:read-text",
    "dialog:choose-directory",
    "dialog:choose-file",
    "terminal:attention-notification",
];

/// 已注册但尚未接线的命令。
///
/// **当前为空**——命令面已经全部接线。这个常量与配套的不变式仍然保留：它是一条
/// 「缺口必须可见」的机制，而不是一张待办清单；下一个未接线的域在这里登记，
/// 测试会保证它不会悄悄变长。
///
/// 历史上它曾经列出 `vendor:refresh-balance` 等发起 HTTP 的命令，以及
/// `app:command` / `app:exit-choice`（窗口控制与退出确认）。
pub const PENDING: [&str; 0] = [];

/// 命令路由器。
pub struct CommandRouter {
    context: ProviderContext,
    sessions: SessionCommands,
    skills: crate::skills::SkillCommands,
    cli: crate::cli::CliCommands,
    terminal: crate::terminal::TerminalCommands,
    shell: crate::shell::ShellCommands,
    /// 目标查找器。
    ///
    /// 命令层各自持有一份（`sessions` 与 `skills`），这里也留一份：系统终端要根据
    /// `targetId` 判断「该开本机 shell 还是那个发行版的 shell」。
    targets: Arc<dyn TargetLookup>,
    /// 网关状态。
    ///
    /// **可选**：路由切换需要它，而多数命令不需要。测试构造路由器时不必先建一个网关。
    gateway: Option<Arc<aivc_gateway::GatewayState>>,
    /// 应用版本号。由组装根注入——ivc-ipc 不该依赖构建脚本生成的常量。
    app_version: String,
}

impl CommandRouter {
    /// 构造。
    pub fn new(
        context: ProviderContext,
        registry: Arc<aivc_provider::ProviderRegistry>,
        targets: Arc<dyn TargetLookup>,
        app_version: impl Into<String>,
    ) -> Self {
        Self {
            context,
            // 两个命令层共享同一个注册表与查找器：`Arc::clone` 之后
            // `registry` 不再被消耗，但下面的 `targets` 被移动了，
            // 因此这里**必须**克隆而不是移动。
            sessions: SessionCommands::new(registry, Arc::clone(&targets)),
            skills: crate::skills::SkillCommands::new(Arc::clone(&targets)),
            targets,
            cli: crate::cli::CliCommands::new(),
            shell: crate::shell::ShellCommands::new(Arc::new(crate::shell::SystemOpener)),
            gateway: None,
            terminal: crate::terminal::TerminalCommands::new(
                Arc::new(aivc_terminal::session::SessionRegistry::new()),
                Arc::new(crate::terminal::NullEmitter),
            ),
            app_version: app_version.into(),
        }
    }

    /// `models:list` 的实现。
    ///
    /// 与 `vendor:list-models` 是两件事：那个查供应商的 HTTP 接口，
    /// 这个查 **Qoder CLI 自身**的清单。
    async fn dispatch_models(&self, args: &Value) -> Result<Value> {
        let target_id = require_string(args.get("targetId"), "targetId")?;
        // **非 Qoder 目标必须报错**（现网语义），不是返回空列表——否则界面上
        // 「不支持」与「一个模型都没有」长得一模一样。
        if !target_id.starts_with("qoder:") {
            return Err(Self::failed_message(
                "当前目标不支持 CLI 模型列表。".to_owned(),
            ));
        }
        let (_, _, distro) = crate::cli::scope_for_target(&target_id);
        // 命令执行是阻塞的，且最长等 20 秒——必须让出执行器线程。
        let models = tokio::task::spawn_blocking(move || {
            aivc_tooling::qoder_models::list_models(
                &aivc_tooling::cli_env::probe::SystemRunner,
                distro.as_deref(),
                cfg!(windows),
            )
        })
        .await
        .map_err(|error| Self::failed_message(format!("模型清单任务失败：{error}")))?
        .map_err(Self::failed_message)?;
        Ok(Value::Array(
            models
                .into_iter()
                .map(|model| serde_json::json!({ "id": model.id }))
                .collect(),
        ))
    }

    /// 供应商路由动作族的实现。
    ///
    /// 手动切换只改路由的**当前供应商**；自动故障转移不改它——那只影响本次请求。
    async fn dispatch_route(&self, command: &str, args: &Value) -> Result<Value> {
        let route_id = require_string(args.get("routeId"), "routeId")?;
        match command {
            "vendor:route-switch" => {
                let vendor_id = require_string(args.get("vendorId"), "vendorId")?;
                match self
                    .require_gateway()?
                    .switch_route_vendor(&route_id, &vendor_id, None)
                    .await
                {
                    Some(route) => Ok(serde_json::json!({
                        "routeId": route.route_id,
                        "vendorId": route.vendor_id,
                    })),
                    None => Err(Self::invalid_parameter("routeId")),
                }
            }
            "vendor:route-mode" => {
                let raw = require_string(args.get("mode"), "mode")?;
                let mode = match raw.as_str() {
                    "dynamic" => aivc_gateway::RouteMode::Dynamic,
                    "locked" => aivc_gateway::RouteMode::Locked,
                    _ => return Err(Self::invalid_parameter("mode")),
                };
                match self
                    .require_gateway()?
                    .set_route_mode(&route_id, mode)
                    .await
                {
                    Some(route) => Ok(serde_json::json!({
                        "routeId": route.route_id,
                        "mode": if route.mode == aivc_gateway::RouteMode::Locked {
                            "locked"
                        } else {
                            "dynamic"
                        },
                    })),
                    None => Err(Self::invalid_parameter("routeId")),
                }
            }
            other => Err(unknown(other)),
        }
    }

    /// 终端动作族的实现。
    ///
    /// 抽出来是因为 `dispatch` 的分支已经很多；终端是一个自洽的动作族，单独成组更易读。
    async fn dispatch_terminal(&self, command: &str, args: &Value) -> Result<Value> {
        match command {
            // 目标编号同时决定两件事：命令是「本机形态」还是「WSL 形态」，
            // 以及路由里的上游地址要不要换成发行版内可达的宿主地址。
            "terminal:start" => {
                let distro = self.target_distro(args).await;
                self.terminal.start(args, distro.as_deref()).await
            }
            // 系统终端：跑用户本来就在用的那个 shell，而不是 Codex。
            "terminal:start-system" => {
                let distro = self.target_distro(args).await;
                self.terminal.start_system(args, distro.as_deref()).await
            }
            // 写入与调整尺寸都只回 `null`：前端不消费返回值。
            "terminal:write" => self.terminal.write(args).await.map(|()| Value::Null),
            "terminal:resize" => self.terminal.resize(args).await.map(|()| Value::Null),
            "terminal:stop" => self.terminal.stop(args).await,
            other => Err(unknown(other)),
        }
    }

    /// `gateway:reset-vendor-health` 的实现：**数据库与内存两份都要清**。
    ///
    /// 转发路径读的是内存里的 `HealthTracker`，而 `GatewayHealth::reset_vendor_health`
    /// 清的是数据库那一份。只做后者时，界面上按「重置健康」之后熔断依然生效——
    /// 按钮看起来没反应。网关未装入时内存里本就没有状态，跳过即可。
    async fn reset_vendor_health(&self, ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let removed = GatewayHealth::reset_vendor_health(ctx, args).await?;
        if let Some(gateway) = self.gateway.as_ref() {
            let vendor_id = args
                .get("vendorId")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(aivc_core::id::VendorId::from);
            gateway.reset_health(vendor_id.as_ref()).await;
        }
        to_value(removed)
    }

    /// 取参数里 `targetId` 对应的 WSL 发行版。
    ///
    /// 解析不出来时返回 `None` **而不是报错**：目标可能已经被删掉了，而此时在本机开
    /// 一个系统终端仍然是有用的——为了一个陈旧的 `targetId` 让整个操作失败没有道理。
    async fn target_distro(&self, args: &Value) -> Option<String> {
        let id = args.get("targetId").and_then(Value::as_str)?;
        if id.is_empty() {
            return None;
        }
        self.targets
            .resolve(id)
            .await
            .and_then(|target| target.distro)
    }

    /// Skill 动作族的实现。
    ///
    /// `skill:open-folder` 交给系统文件管理器，`skill:import` 的**源路径由界面**用原生
    /// 目录选择框给出（`aivc-ipc` 不依赖 tauri，取不到平台对话框）。因此这里全是纯粹的
    /// 文件操作，能被测试直接驱动。
    async fn dispatch_skill(&self, command: &str, args: &Value) -> Result<Value> {
        let target_id = require_string(args.get("targetId"), "targetId")?;
        // 这四条需要 skill 名；前两条只需要目标。
        let skill_name = || require_string(args.get("skillName"), "skillName");
        match command {
            // 与 `shell:open-session-folder` 同构：先解析路径，再交给系统打开。
            "skill:open-folder" => {
                let folder = self.skills.skill_root(&target_id).await?;
                self.shell
                    .open_path(&serde_json::json!({ "path": folder }))?;
                Ok(Value::Null)
            }
            "skill:list" => to_value(self.skills.list(&target_id).await?),
            "skill:list-trash" => to_value(self.skills.list_trash(&target_id).await?),
            "skill:set-enabled" => Ok(self
                .skills
                .set_enabled(
                    &target_id,
                    &skill_name()?,
                    require_boolean(args.get("enabled"), "enabled")?,
                )
                .await?),
            "skill:delete" => Ok(self.skills.delete(&target_id, &skill_name()?).await?),
            "skill:restore" => Ok(self.skills.restore(&target_id, &skill_name()?).await?),
            "skill:purge" => Ok(self.skills.purge(&target_id, &skill_name()?).await?),
            // 源路径由**界面**用原生目录选择框给出（`dialog:choose-directory`）。
            //
            // 选择框放在前端是有意的：`aivc-ipc` 不依赖 tauri，取不到平台对话框。
            // 这样这条命令也就成了纯粹的文件操作，能被测试直接驱动。
            "skill:import" => {
                let source = require_string(args.get("source"), "source")?;
                // `overwrite` **默认为 false**：覆盖别人的技能是不可逆的，
                // 宁可让界面多问一次，也不默认替用户决定。
                let overwrite = args
                    .get("overwrite")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Ok(self
                    .skills
                    .import_from_path(&target_id, &source, overwrite)
                    .await?)
            }
            other => Err(unknown(other)),
        }
    }

    /// 构造一个「命令失败」错误。
    fn failed_message(message: String) -> Error {
        Error::Ipc(IpcError::CommandFailed { message })
    }

    /// 构造一个「参数无效」错误。
    fn invalid_parameter(name: &str) -> Error {
        Error::Ipc(IpcError::InvalidParameter {
            name: name.to_owned(),
        })
    }

    /// CLI 环境与安装的实现。
    ///
    /// 安装是破坏性操作，因此这里**先构造计划再执行**：命令拼错会在计划阶段暴露，
    /// 而不是等真的装了一半才发现。
    async fn dispatch_cli(&self, command: &str, args: &Value) -> Result<Value> {
        let provider_id = require_string(args.get("providerId"), "providerId")?;
        let target_id = optional_string(args.get("targetId")).unwrap_or_default();
        match command {
            "cli:check-environment" => self.cli.check_environment(&provider_id, &target_id).await,
            "cli:install" => {
                // 两个可选参数：缺省时分别表示「不装 Node」与「用推荐版本」。
                let ensure_node = match args.get("ensureNode") {
                    Some(value) => require_boolean(Some(value), "ensureNode")?,
                    None => false,
                };
                let node_major = match args.get("nodeMajor") {
                    Some(value) => Some(
                        u32::try_from(require_positive_integer(Some(value), "nodeMajor")?)
                            .map_err(|_| Self::invalid_parameter("nodeMajor"))?,
                    ),
                    None => None,
                };
                self.cli
                    .install(&provider_id, &target_id, ensure_node, node_major)
                    .await
            }
            other => Err(unknown(other)),
        }
    }

    /// 装入网关状态。
    ///
    /// 由组装根调用；不调用时路由切换命令会明确报「网关未运行」。
    ///
    /// # 顺带装上终端的路由来源
    ///
    /// 「为终端创建一条网关路由」需要网关状态与供应商仓储**同时在手**，而这里正是两者
    /// 第一次碰面的地方。不装网关时终端拿到的仍是 [`crate::route::NullRouteSource`]：
    /// 直接连官方 API，这是合法的降级而不是缺陷。
    #[must_use]
    pub fn with_gateway(mut self, gateway: Arc<aivc_gateway::GatewayState>) -> Self {
        self.terminal
            .install_routes(Arc::new(crate::route::GatewayRouteSource::new(
                Arc::clone(&gateway),
                self.context.clone(),
            )));
        self.gateway = Some(gateway);
        self
    }

    /// 取网关；未装入时报错。
    fn require_gateway(&self) -> Result<&Arc<aivc_gateway::GatewayState>> {
        self.gateway
            .as_ref()
            .ok_or_else(|| Self::failed_message("网关未运行。".to_owned()))
    }

    /// 上下文。
    #[must_use]
    pub const fn context(&self) -> &ProviderContext {
        &self.context
    }

    /// 分发一次调用。
    ///
    /// 返回值已经是**线上形状**（`serde_json::Value`），可以直接交给 Tauri 序列化。
    ///
    /// # Errors
    ///
    /// 换掉**终端事件**的发射器。
    ///
    /// 必须转发到终端处理器：它在构造时就拿到一份发射器（那时窗口还不存在，真实发射器
    /// 装不进去），因此**必须能被事后替换**。此前没人替换它，输出泵就一直往
    /// `NullEmitter` 里写——终端进程正常启动、不报任何错、界面永远空白。
    pub fn install_terminal_emitter(&self, emitter: Arc<dyn crate::terminal::EventEmitter>) {
        self.terminal.install_emitter(emitter);
    }

    /// - 未知命令名 → [`IpcError::CommandFailed`]；
    /// - 已注册但未实现 → 「未实现」错误；
    /// - 入参非法或业务失败 → 对应领域错误。
    pub async fn dispatch(&self, command: &str, args: Value) -> Result<Value> {
        // 调试级：排查「前端点了没反应」时，先确认命令有没有到达这一层。
        tracing::debug!(command, "收到 IPC 命令");
        if !is_known(command) {
            return Err(unknown(command));
        }
        let ctx = &self.context;
        match command {
            // --- 应用 ---
            // 前端启动时就要拿版本号；未接线会让启动流程在这一步中断。
            "app:get-version" => Ok(Value::String(self.app_version.clone())),
            // 平台下拉框的数据源。
            "ai:list-providers" => Ok(Value::Array(self.sessions.provider_summaries())),

            // --- Skill ---
            // 抽到单独方法里：`dispatch` 的分支已经很多，继续内联会超过行数上限，
            // 而 Skill 是一个自洽的动作族，单独成组也更易读。
            // 整个 `skill:` 前缀都交给技能动作族：未列出的会在里面得到明确的
            // 「未知命令」错误，而不是掉进下面某个不相干的 arm。
            other if other.starts_with("skill:") => self.dispatch_skill(other, &args).await,
            "models:list" => self.dispatch_models(&args).await,

            // --- 供应商路由 ---
            "vendor:route-switch" | "vendor:route-mode" => {
                self.dispatch_route(command, &args).await
            }
            // --- 埋点与前端异常 ---
            //
            // 这条命令的主要用途不是性能数据，而是**前端异常的唯一去处**：
            // 渲染层抛错在 Rust 侧完全不可见，没有它「界面静默失效」就是黑盒。
            "performance:log" => {
                let label = require_string(args.get("label"), "label")?;
                let duration = args.get("durationMs").and_then(Value::as_i64).unwrap_or(0);
                let status = optional_string(args.get("status")).unwrap_or_else(|| "ok".to_owned());
                let entry = aivc_tooling::performance::PerformanceEntry {
                    at: aivc_core::time::Timestamp::now().as_str().to_owned(),
                    label,
                    duration_ms: duration,
                    status,
                };
                // 写入失败只记日志：埋点不该影响它所在的业务路径。
                if let Err(error) = aivc_tooling::performance::append_line(
                    &ctx.performance_log_path(),
                    &entry.to_line(),
                ) {
                    tracing::warn!(%error, "写入埋点日志失败");
                }
                Ok(Value::Null)
            }
            // --- Shell ---
            // 打开动作走系统默认程序；不存在的路径在调用它之前就被拒绝。
            "shell:path-exists" => self.shell.path_exists(&args),
            "shell:open-path" => self.shell.open_path(&args),
            // `open-session-folder` 先查出目录再交给系统打开：目录由各 Provider 决定，
            // 命令层不猜路径。
            "shell:open-session-folder" => {
                let folder = self.sessions.get_session_folder_path(ctx, &args).await?;
                self.shell
                    .open_path(&serde_json::json!({ "path": folder }))?;
                Ok(Value::Null)
            }
            // --- 终端 ---
            "terminal:start"
            | "terminal:start-system"
            | "terminal:write"
            | "terminal:resize"
            | "terminal:stop" => self.dispatch_terminal(command, &args).await,
            // --- 供应商查询 ---
            "vendor:list-models" => Ok(VendorCommands::list_models(ctx, &args).await?),
            "vendor:read-configs" => Ok(VendorCommands::read_configs(&args)?),
            "vendor:refresh-balance" => Ok(VendorCommands::refresh_balance(ctx, &args).await?),
            "vendor:refresh-balances" => Ok(VendorCommands::refresh_balances(ctx).await?),
            // --- CLI 环境与安装 ---
            "cli:check-environment" | "cli:install" => self.dispatch_cli(command, &args).await,
            // --- 会话：目标与列表 ---
            "codex:list-targets" => to_value(self.sessions.list_targets(ctx, &args).await?),
            // 缓存目标不探测文件系统，因此与上面的实现不同。
            "codex:list-cached-targets" => {
                to_value(self.sessions.list_cached_targets(ctx, &args).await?)
            }
            "codex:list-sessions" => to_value(self.sessions.list_sessions(ctx, &args).await?),
            "codex:list-trash-sessions" => {
                to_value(self.sessions.list_trash_sessions(ctx, &args).await?)
            }
            "codex:list-cached-sessions" => {
                to_value(self.sessions.list_cached_sessions(ctx, &args).await?)
            }
            "codex:search-sessions" => to_value(self.sessions.search_sessions(ctx, &args).await?),

            // --- 会话：详情与分页 ---
            "codex:get-session" => to_value(self.sessions.get_session(ctx, &args).await?),
            "codex:get-session-summary" => {
                to_value(self.sessions.get_session_summary(ctx, &args).await?)
            }
            "codex:get-session-messages-page" => {
                to_value(self.sessions.get_session_messages_page(ctx, &args).await?)
            }
            "codex:list-session-children" => {
                to_value(self.sessions.list_session_children(ctx, &args).await?)
            }

            // --- 会话：变更 ---
            "codex:delete-session" => to_value(self.sessions.delete_session(ctx, &args).await?),
            "codex:delete-sessions" => to_value(self.sessions.delete_sessions(ctx, &args).await?),
            "codex:purge-session" => to_value(self.sessions.purge_session(ctx, &args).await?),
            "codex:purge-sessions" => to_value(self.sessions.purge_sessions(ctx, &args).await?),
            "codex:restore-session" => to_value(self.sessions.restore_session(ctx, &args).await?),

            // --- 会话：分支 / 复制 / 导出 ---
            "codex:branch-session" => to_value(self.sessions.branch_session(ctx, &args).await?),
            // 快捷访问只读缓存，不触发全量探测——它要立即显示。
            "codex:list-session-quick-access" => self.sessions.list_quick_access(ctx, &args).await,
            "codex:duplicate-session" => {
                to_value(self.sessions.duplicate_session(ctx, &args).await?)
            }
            "codex:export-session" => to_value(self.sessions.export_session(ctx, &args).await?),

            // --- 会话设置 ---
            "codex:set-session-custom-title" => {
                to_value(SessionSettings::set_custom_title(ctx, &args).await?)
            }
            "codex:set-session-favorite" => {
                to_value(SessionSettings::set_favorite(ctx, &args).await?)
            }
            "codex:mark-session-opened" => {
                to_value(SessionSettings::mark_opened(ctx, &args).await?)
            }
            // WSL 的 Codex 配置目录人工覆盖。
            "codex:set-wsl-codex-home" => SessionSettings::set_wsl_codex_home(ctx, &args).await,
            "codex:clear-wsl-codex-home" => SessionSettings::clear_wsl_codex_home(ctx, &args).await,

            // --- 供应商 ---
            "vendor:list" => to_value(VendorCommands::list(ctx, &args).await?),
            "vendor:save" => to_value(VendorCommands::save(ctx, &args).await?),
            "vendor:delete" => to_value(VendorCommands::delete(ctx, &args).await?),
            "vendor:set-enabled" => to_value(VendorCommands::set_enabled(ctx, &args).await?),
            "vendor:enable" => Ok(VendorCommands::enable(ctx, &args).await?),

            // --- 工作区预设 ---
            "workspace:list-presets" => to_value(WorkspaceCommands::list_presets(ctx).await?),
            "workspace:save-preset" => to_value(WorkspaceCommands::save_preset(ctx, &args).await?),
            "workspace:delete-preset" => {
                to_value(WorkspaceCommands::delete_preset(ctx, &args).await?)
            }

            // --- 网关：健康 ---
            "gateway:get-vendor-health" => to_value(GatewayHealth::get_vendor_health(ctx).await?),
            "gateway:reset-vendor-health" => self.reset_vendor_health(ctx, &args).await,

            // --- 网关：故障规则 ---
            "gateway:list-failover-rules" => {
                to_value(GatewayRules::list_failover_rules(ctx).await?)
            }
            "gateway:save-failover-rule" => {
                to_value(GatewayRules::save_failover_rule(ctx, &args).await?)
            }
            "gateway:delete-failover-rule" => {
                to_value(GatewayRules::delete_failover_rule(ctx, &args).await?)
            }
            "gateway:set-failover-rule-enabled" => {
                to_value(GatewayRules::set_failover_rule_enabled(ctx, &args).await?)
            }

            // --- 网关：用量与日志 ---
            "gateway:get-usage-summary" => {
                to_value(GatewayLogs::get_usage_summary(ctx, &args).await?)
            }
            "gateway:get-usage-report" => {
                to_value(GatewayLogs::get_usage_report(ctx, &args).await?)
            }
            "gateway:get-recent-failures" => GatewayLogs::get_recent_failures(ctx, &args).await,
            "gateway:get-failure-diagnostics" => {
                to_value(GatewayLogs::get_failure_diagnostics(ctx, &args).await?)
            }
            "gateway:query-logs" => GatewayLogs::query_logs(ctx, &args).await,
            "gateway:delete-log-entries" => {
                to_value(GatewayLogs::delete_log_entries(ctx, &args).await?)
            }
            "gateway:clear-logs" => to_value(GatewayLogs::clear_logs(ctx, &args).await?),

            // --- 网关：设置 ---
            "gateway:get-port" => GatewaySettingsCommands::get_port(ctx).await,
            "gateway:set-port" => GatewaySettingsCommands::set_port(ctx, &args).await,
            "gateway:set-enabled" => GatewaySettingsCommands::set_enabled(ctx, &args).await,
            "gateway:get-external-api-status" => {
                GatewaySettingsCommands::get_external_api_status(ctx).await
            }
            "gateway:set-external-api-enabled" => {
                GatewaySettingsCommands::set_external_api_enabled(ctx, &args).await
            }
            "gateway:rotate-external-api-token" => {
                GatewaySettingsCommands::rotate_external_api_token(ctx).await
            }

            other => Err(pending(other)),
        }
    }

    /// 已接线的命令数。
    #[must_use]
    pub const fn implemented_count() -> usize {
        COMMAND_COUNT - PENDING.len()
    }
}

/// 序列化为线上形状。
fn to_value<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|error| {
        Error::Ipc(IpcError::CommandFailed {
            message: format!("序列化返回值失败：{error}"),
        })
    })
}

/// 未知命令。
#[must_use]
pub fn unknown(command: &str) -> Error {
    Error::Ipc(IpcError::CommandFailed {
        message: format!("未知命令：{command}"),
    })
}

/// 已注册但未实现。
#[must_use]
pub fn pending(command: &str) -> Error {
    let domain = domain_of(command).map_or("未知", CommandDomain::prefix);
    Error::Ipc(IpcError::CommandFailed {
        message: format!("命令尚未接线：{command}（域：{domain}）"),
    })
}

/// 命令是否已接线。
#[must_use]
pub fn is_implemented(command: &str) -> bool {
    is_known(command) && !PENDING.contains(&command)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::COMMANDS;
    use aivc_core::ports::BoxFuture;
    use aivc_core::target::{Platform, Target};
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
    use aivc_provider::{ProviderRepositories, build_registry};
    use serde_json::json;
    use std::collections::BTreeSet;

    struct NoTargets;

    impl TargetLookup for NoTargets {
        fn resolve(&self, _target_id: &str) -> BoxFuture<'_, Option<Target>> {
            Box::pin(async move { None })
        }

        fn list(&self) -> BoxFuture<'_, Vec<Target>> {
            Box::pin(async move { Vec::<Target>::new() })
        }
    }

    async fn router() -> (CommandRouter, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let wsl = WslAdapter::new(
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
            WslConfig {
                use_worker: false,
                ..WslConfig::default()
            },
        );
        let ctx = ProviderContext::new(
            paths,
            SettingsStore::new(dir.path().join("runtime/settings.json")),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );
        let router = CommandRouter::new(
            ctx,
            Arc::new(build_registry()),
            Arc::new(NoTargets),
            "0.0.0-test",
        );
        (router, dir)
    }

    /// **端到端：终端启动 → 网关真的多了一条路由 → 终端关闭 → 路由消失。**
    ///
    /// # 为什么用 `vendor:route-switch` 做观测
    ///
    /// 「路由器有没有把真实路由源装上」这件事没有直接的命令可查。而
    /// `vendor:route-switch` 恰好需要一个**存在于网关里的路由编号**：路由不在，它返回
    /// `routeId` 参数错误。于是它变成了一个现成的探针——不需要为测试新增任何生产 API。
    ///
    /// 这条测试补的是本轮修复的**最后一环**：路由生产者、命令构造器、路由销毁各自都有
    /// 测试，唯独「组装根有没有把它们接起来」只能靠人看代码。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_started_terminal_registers_a_real_route_on_the_gateway() {
        let (router, _dir) = router().await;
        let ctx = router.context().clone();
        let settings = ctx.settings().clone();
        settings
            .update(|raw| {
                raw.set_gateway_enabled(true);
                // 0 = 由操作系统分配端口，避免和真实应用撞车。
                raw.set_gateway_port(0);
            })
            .await
            .expect("写设置");

        // 同平台两个供应商，用来做切换。
        let switch_target = save_vendor(&ctx, "claude", "vendor-b", 2).await;

        let source = Arc::new(
            aivc_gateway::RepositorySource::new(
                ctx.vendor().clone(),
                ctx.failover_rule().clone(),
                ctx.gateway_request_log().clone(),
                settings,
            )
            .await
            .expect("网关来源"),
        );
        let state = Arc::new(aivc_gateway::GatewayState::new(source).expect("网关状态"));
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");
        let router = router.with_gateway(Arc::clone(&state));

        save_vendor(&ctx, "claude", "vendor-a", 1).await;

        // 起点：这台终端的路由还不存在。
        assert!(
            router
                .dispatch(
                    "vendor:route-switch",
                    json!({ "routeId": "term-wire", "vendorId": switch_target })
                )
                .await
                .is_err(),
            "启动之前不该有这条路由"
        );

        let work = tempfile::tempdir().expect("临时目录");
        router
            .dispatch(
                "terminal:start",
                json!({
                    "terminalId": "term-wire",
                    "providerId": "claude",
                    "cwd": work.path().to_string_lossy(),
                }),
            )
            .await
            .expect("启动终端");

        // 路由存在，所以切换找得到它。
        let switched = router
            .dispatch(
                "vendor:route-switch",
                json!({ "routeId": "term-wire", "vendorId": switch_target }),
            )
            .await
            .expect("路由必须已经在网关里，否则这条命令会报 routeId 参数错误");
        assert_eq!(switched["routeId"], "term-wire");
        assert_eq!(switched["vendorId"], switch_target);

        router
            .dispatch("terminal:stop", json!({ "terminalId": "term-wire" }))
            .await
            .expect("停止终端");

        assert!(
            router
                .dispatch(
                    "vendor:route-switch",
                    json!({ "routeId": "term-wire", "vendorId": switch_target })
                )
                .await
                .is_err(),
            "终端关闭后路由必须消失——否则网关的路由表会随开关终端无限增长"
        );

        handle.shutdown().await;
    }

    /// **「重置健康」必须对真正生效的那份状态生效。**
    ///
    /// 转发路径读的是内存里的 `HealthTracker`，而这条命令清的是数据库那一份——
    /// 两者过去完全脱节，于是按钮按下去行为一点不变（数据库那张表本来就没有写入方）。
    ///
    /// 这条测试**走真实命令面**、并断言**内存里的**快照变了：
    /// 先用一台打不通的供应商把熔断打开，再调命令，然后看快照里还有没有开着的熔断。
    #[tokio::test(flavor = "multi_thread")]
    async fn resetting_health_also_clears_the_live_circuit_state() {
        let (router, _dir) = router().await;
        let ctx = router.context().clone();
        let settings = ctx.settings().clone();
        settings
            .update(|raw| {
                raw.set_gateway_enabled(true);
                raw.set_gateway_port(0);
                // 一次失败就熔断，且不重试到别的候选（只有一家）。
                raw.set_gateway_circuit_failure_threshold(1);
                raw.set_gateway_failure_threshold(1);
            })
            .await
            .expect("写设置");

        // 一台**打不通**的供应商：不会有服务在 1 号端口上听。
        let vendor_id = save_vendor(&ctx, "codex", "坏供应商", 1).await;
        let source = Arc::new(
            aivc_gateway::RepositorySource::new(
                ctx.vendor().clone(),
                ctx.failover_rule().clone(),
                ctx.gateway_request_log().clone(),
                settings,
            )
            .await
            .expect("网关来源"),
        );
        let state = Arc::new(aivc_gateway::GatewayState::new(source).expect("网关状态"));
        state
            .register_route(aivc_gateway::VendorRoute {
                route_id: "r-reset".to_owned(),
                provider_id: aivc_core::target::ProviderId::Codex,
                vendor_id: vendor_id.clone(),
                mode: aivc_gateway::RouteMode::Dynamic,
                local_token: "tok".to_owned(),
                base_url: "http://127.0.0.1:1".to_owned(),
                external: false,
            })
            .await;
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");
        let router = router.with_gateway(Arc::clone(&state));

        // 打一次，把熔断打开。
        let _ = reqwest::Client::new()
            .post(format!(
                "{}/gateway/codex/r-reset/v1/messages",
                handle.address
            ))
            .bearer_auth("tok")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("请求");
        let open_before = state
            .health_snapshot()
            .await
            .into_iter()
            .any(|item| item.status == aivc_core::gateway::GatewayVendorHealthStatus::Open);
        assert!(open_before, "一次失败 + 阈值 1 应当把熔断打开");

        // 走真实命令面重置。
        router
            .dispatch(
                "gateway:reset-vendor-health",
                json!({ "vendorId": vendor_id }),
            )
            .await
            .expect("重置健康");

        let still_open = state
            .health_snapshot()
            .await
            .into_iter()
            .any(|item| item.status == aivc_core::gateway::GatewayVendorHealthStatus::Open);
        assert!(
            !still_open,
            "重置之后内存里的熔断必须被清掉，否则按钮看起来没反应"
        );

        handle.shutdown().await;
    }

    /// 存一条可用的供应商，返回它的编号。
    async fn save_vendor(ctx: &ProviderContext, provider: &str, name: &str, sort: i64) -> String {
        let input: aivc_core::vendor::ApiVendorInput = serde_json::from_value(json!({
            "providerId": provider,
            "name": name,
            "apiKey": "k",
            "apiBaseUrl": "http://127.0.0.1:1",
            "enabled": true,
            "sort": sort,
        }))
        .expect("供应商输入");
        ctx.vendor()
            .save(&input)
            .await
            .expect("保存供应商")
            .id
            .as_str()
            .to_owned()
    }

    /// 关键不变式：已实现 + 未实现 == 全部命令，且两者不相交。
    #[test]
    fn implemented_and_pending_partition_every_command() {
        let pending: BTreeSet<&str> = PENDING.iter().copied().collect();
        assert_eq!(pending.len(), PENDING.len(), "PENDING 里有重复项");

        let mut covered: BTreeSet<&str> = BTreeSet::new();
        for (name, _) in COMMANDS {
            assert!(is_known(name), "{name} 不在命令表里");
            covered.insert(name);
        }
        // PENDING 里不能有命令表之外的名字。
        for name in &pending {
            assert!(is_known(name), "PENDING 含未注册命令：{name}");
        }
        // 两边合起来必须正好覆盖全部命令。
        let implemented: BTreeSet<&str> = COMMANDS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| !pending.contains(name))
            .collect();
        assert_eq!(implemented.len() + pending.len(), COMMAND_COUNT);
        assert_eq!(CommandRouter::implemented_count(), implemented.len());

        // 两者不相交。
        for name in &implemented {
            assert!(!pending.contains(name), "{name} 同时被列为已实现与未实现");
        }
    }

    /// **每条「已实现」的命令都必须真的有 arm 接住。**
    ///
    /// 上面那条不变式只覆盖了「命令表 vs PENDING」这一层，而 `dispatch` 还有一层
    /// **外层路由**（一张字面量清单）。两者可以脱节：
    ///
    /// `terminal:start-system` 曾经同时满足「在命令表里」与「不在 PENDING 里」，
    /// 却没有任何 arm 接住它——于是落到外层 match 的兜底 `Err(pending(other))`，
    /// 报「命令尚未接线」。**测试全绿，功能完全不可用**。
    ///
    /// 这条测试专门堵那个缺口：以**空参数**调用每条已实现的命令，只断言它没有落到
    /// 「尚未接线」的兜底。参数校验失败、目标不存在、探测不到——都是正常结果。
    #[tokio::test(flavor = "multi_thread")]
    async fn every_implemented_command_is_actually_routed() {
        let (router, _dir) = router().await;
        let pending: BTreeSet<&str> = PENDING.iter().copied().collect();
        let platform: BTreeSet<&str> = HANDLED_BY_PLATFORM.iter().copied().collect();

        // 先检查这张清单本身是自洽的——它是上一轮引进的**新单点风险**：
        // 写错一个名字、或与 PENDING 矛盾，都不会有人发现。
        assert_eq!(
            platform.len(),
            HANDLED_BY_PLATFORM.len(),
            "HANDLED_BY_PLATFORM 里有重复项"
        );
        for name in HANDLED_BY_PLATFORM {
            assert!(
                is_known(name),
                "HANDLED_BY_PLATFORM 含未注册命令：{name}（拼错了吗？）"
            );
            assert!(
                !pending.contains(name),
                "{name} 同时被列为「界面层处理」与「尚未接线」——两者矛盾"
            );
        }

        let mut unrouted: Vec<&str> = Vec::new();

        for (name, _) in COMMANDS {
            if pending.contains(name) || platform.contains(name) {
                continue;
            }
            if let Err(error) = router.dispatch(name, json!({})).await
                && error.to_string().contains("尚未接线")
            {
                unrouted.push(name);
            }
        }

        assert!(
            unrouted.is_empty(),
            "这些命令登记为「已实现」，却没有任何 arm 接住它们：{unrouted:?}"
        );
    }

    #[test]
    fn is_implemented_agrees_with_the_pending_list() {
        assert!(is_implemented("codex:list-sessions"));
        assert!(is_implemented("workspace:save-preset"));
        assert!(is_implemented("gateway:get-port"));
        assert!(is_implemented("gateway:save-failover-rule"));
        assert!(is_implemented("gateway:rotate-external-api-token"));
        assert!(is_implemented("vendor:list"));
        assert!(is_implemented("codex:set-wsl-codex-home"));
        assert!(is_implemented("terminal:start"));
        // 系统终端：跑用户本来的 shell，与 Codex 终端是两条独立路径。
        assert!(is_implemented("terminal:start-system"));
        assert!(is_implemented("skill:list"));
        // 需要发起 HTTP 的供应商命令仍未接线。
        assert!(is_implemented("vendor:refresh-balance"));
        // 不在命令表里的名字不算「已接线」。
        assert!(!is_implemented("not:a-command"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_command_names_itself_in_the_error() {
        let (router, _dir) = router().await;
        let error = router
            .dispatch("codex:no-such-command", json!({}))
            .await
            .expect_err("必须失败");
        assert_eq!(error.to_string(), "未知命令：codex:no-such-command");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_command_reports_the_gap_instead_of_succeeding() {
        // 静默成功会让渲染层以为操作成功了，因此未实现必须显式报错。
        //
        // 用一条**由界面层处理**的命令：路由器这一层接不住它，因此落到「尚未接线」
        // 的兜底上。断言的价值在**报错形状**，而不是某个具体命令尚未实现。
        let (router, _dir) = router().await;
        let error = router
            .dispatch("clipboard:copy-text", json!({}))
            .await
            .expect_err("必须失败");
        let message = error.to_string();
        assert!(message.contains("命令尚未接线"), "{message}");
        assert!(message.contains("clipboard:copy-text"), "{message}");
        // 报错里带上所属域，便于定位是哪一批没接。
        assert!(message.contains("域：clipboard"), "{message}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatches_the_workspace_preset_commands() {
        let (router, _dir) = router().await;
        let saved = router
            .dispatch(
                "workspace:save-preset",
                json!({ "name": "工作区", "cwd": "/w" }),
            )
            .await
            .expect("save");
        assert!(saved.is_string(), "编号应当序列化为字符串：{saved}");

        let list = router
            .dispatch("workspace:list-presets", json!({}))
            .await
            .expect("list");
        let items = list.as_array().expect("数组");
        assert_eq!(items.len(), 1);
        // 线上形状是 camelCase。
        assert_eq!(items[0]["name"], "工作区");
        assert_eq!(items[0]["cwd"], "/w");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatches_the_session_settings_commands() {
        let (router, _dir) = router().await;
        let saved = router
            .dispatch(
                "codex:set-session-custom-title",
                json!({ "targetId": "codex:local", "sessionId": "s1", "title": "标题" }),
            )
            .await
            .expect("title");
        assert_eq!(saved["customTitle"], "标题", "线上字段是 camelCase");

        let opened = router
            .dispatch(
                "codex:mark-session-opened",
                json!({ "targetId": "codex:local", "sessionId": "s1" }),
            )
            .await
            .expect("opened");
        assert!(opened["lastOpenedAt"].is_string());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatches_target_listing() {
        let (router, _dir) = router().await;
        // 探测会返回本机真实装有的 CLI 对应的目标（数量取决于环境），因此只断言形状，
        // 以及**探测结果确实回写进了缓存**——后者才是这条命令的关键职责。
        let targets = router
            .dispatch("codex:list-targets", json!({}))
            .await
            .expect("targets");
        let probed = targets.as_array().expect("数组").len();
        // 断言设置缓存而不是 `list-cached-targets`：后者走注入的桩查找器。
        let cached = router.context().settings().snapshot().await.cached_targets;
        assert_eq!(cached.len(), probed, "探测结果必须回写设置缓存");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_pending_command_is_reported_before_its_arguments_are_checked() {
        // 「未接线」必须优先于「参数无效」——否则调用方会以为只是参数写错了。
        //
        // 用一条**由界面层处理**的命令（见上一条测试的说明）：这条断言的价值在**顺序**。
        let (router, _dir) = router().await;
        let error = router
            .dispatch("clipboard:copy-text", json!({}))
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("命令尚未接线"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatches_the_vendor_commands() {
        let (router, _dir) = router().await;
        let saved = router
            .dispatch(
                "vendor:save",
                json!({
                    "providerId": "codex",
                    "name": "主供应商",
                    "apiKey": "sk-test",
                    "apiBaseUrl": "https://api.example.com"
                }),
            )
            .await
            .expect("save");
        let id = saved["id"].as_str().expect("编号").to_owned();

        let list = router
            .dispatch("vendor:list", json!({ "providerId": "codex" }))
            .await
            .expect("list");
        assert_eq!(list.as_array().expect("数组").len(), 1);

        // 严格解析：裸 `claude` 必须被识别成 Claude 而不是兜底成 Codex。
        let claude = router
            .dispatch("vendor:list", json!({ "providerId": "claude" }))
            .await
            .expect("list");
        assert!(claude.as_array().expect("数组").is_empty());

        // 非法平台标识要报错，而不是静默兜底。
        let error = router
            .dispatch("vendor:list", json!({ "providerId": "openai" }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：providerId");

        // 启用要求至少一个启用的配置文件（它要写配置文件）。
        // 这里只验证**校验**生效：成功路径会写真实的用户配置目录，测试里不跑。
        let error = router
            .dispatch("vendor:enable", json!({ "id": id }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "请至少启用一个配置文件。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rotating_the_external_token_returns_it_exactly_once() {
        let (router, _dir) = router().await;
        let rotated = router
            .dispatch("gateway:rotate-external-api-token", json!({}))
            .await
            .expect("rotate");
        let token = rotated["token"].as_str().expect("令牌");
        assert!(!token.is_empty());

        // 之后的状态查询只能看到「已设置」，取不回令牌。
        let status = router
            .dispatch("gateway:get-external-api-status", json!({}))
            .await
            .expect("status");
        assert_eq!(status["hasToken"], true);
        assert!(status.get("token").is_none());

        // 两次轮换必须得到不同的令牌。
        let again = router
            .dispatch("gateway:rotate-external-api-token", json!({}))
            .await
            .expect("rotate");
        assert_ne!(again["token"].as_str().expect("令牌"), token);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wsl_codex_home_overrides_round_trip() {
        let (router, _dir) = router().await;
        let set = router
            .dispatch(
                "codex:set-wsl-codex-home",
                json!({ "distro": "Ubuntu", "codexHome": "/home/u/.codex" }),
            )
            .await
            .expect("set");
        assert_eq!(set["distro"], "Ubuntu");

        // 缺少发行版名要报错。
        let error = router
            .dispatch("codex:set-wsl-codex-home", json!({ "codexHome": "/x" }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：distro");

        router
            .dispatch("codex:clear-wsl-codex-home", json!({ "distro": "Ubuntu" }))
            .await
            .expect("clear");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatches_the_gateway_settings_commands() {
        let (router, _dir) = router().await;
        // 端口 0 表示自动分配。
        let saved = router
            .dispatch("gateway:set-port", json!({ "port": 0 }))
            .await
            .expect("set port");
        assert_eq!(saved["port"], 0);

        let current = router
            .dispatch("gateway:get-port", json!({}))
            .await
            .expect("get port");
        assert_eq!(current["port"], 0);

        // 外部 API 状态只暴露「有没有令牌」，绝不返回令牌本身。
        let status = router
            .dispatch("gateway:get-external-api-status", json!({}))
            .await
            .expect("status");
        assert!(status.get("token").is_none());
        assert_eq!(status["hasToken"], false);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatches_the_gateway_health_and_log_commands() {
        let (router, _dir) = router().await;
        let health = router
            .dispatch("gateway:get-vendor-health", json!({}))
            .await
            .expect("health");
        assert!(health.as_array().expect("数组").is_empty());

        let rules = router
            .dispatch("gateway:list-failover-rules", json!({}))
            .await
            .expect("rules");
        assert!(rules.as_array().expect("数组").is_empty());

        // 用量查询要求显式区间——缺省会让聚合扫全表。
        let error = router
            .dispatch("gateway:get-usage-summary", json!({}))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：periodStart");

        let summary = router
            .dispatch(
                "gateway:get-usage-summary",
                json!({
                    "periodStart": "2026-01-01T00:00:00.000Z",
                    "periodEnd": "2026-12-31T23:59:59.999Z"
                }),
            )
            .await
            .expect("summary");
        assert_eq!(summary["requestCount"], 0);
    }
}
