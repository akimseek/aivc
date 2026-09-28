//! 终端命令：启动、写入、调整尺寸、停止。
//!
//! # 输出是**推送**的
//!
//! `terminal:start` 只是把会话建起来；真正的输出通过 [`EventEmitter`] 以
//! `terminal:data` 事件推给渲染层。
//!
//! # 必须把渲染层的写回转发给 PTY
//!
//! 这条链路看起来只是「用户敲键盘」，实际上**关系到终端能不能动**：
//!
//! `ConPTY` 启动后会发 `ESC[6n`（光标位置查询），**并在拿到应答前不推进会话**。
//! 真实环境由 xterm.js 自动应答，而应答就是通过 `terminal:write` 回到 PTY 的。
//!
//! 也就是说：**如果这条链路断了，终端会静默卡住**——不是报错，是什么都不发生。
//! 这一点由 `aivc-terminal` 的实测确认过（见验收清单的「PTY 泄漏的根因与修复」）。
//!
//! # 一个会话一个泵
//!
//! 每个会话起一个 tokio 任务，把 PTY 的输出接收端泵到事件里。任务是**独立于命令调用**
//! 的：`terminal:start` 返回后输出仍在到达。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aivc_core::error::{Error, IpcError, Result};
use aivc_core::target::ProviderId;
use aivc_terminal::command::{
    CommandParams, LaunchTarget, TerminalCommand, build_command, extra_args,
};
use aivc_terminal::session::{DEFAULT_COLS, DEFAULT_ROWS, PtySession, SessionRegistry};
use aivc_terminal::shell::{
    ShellKind, auto_kind_for, build_system_shell_command, find_on_path, git_bash_candidates,
    resolve_shell,
};
use aivc_terminal::status::{TerminalStatusDetector, TerminalStatusEvent};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::route::{NullRouteSource, RouteSource};
use crate::validate::require_terminal_id;

/// 一路终端的状态检测器句柄。
///
/// # 为什么必须是共享的
///
/// 检测器要同时看到**用户输入**（[`TerminalStatusDetector::mark_input`]，判定「一轮从哪
/// 一刻开始」）与**输出**（[`TerminalStatusDetector::inspect_output`]）。这两者来自两个
/// 互不相干的任务：`terminal:write`（渲染层写回）与输出泵。
///
/// 此前检测器是泵任务里的一个**局部变量**：`write` 根本够不到它，于是 `mark_input`
/// **在生产代码里从未被调用过**。而 `inspect_output` 的第一件事就是
/// `if !self.turn_active { return None }`——所以整套状态检测**一次都没有跑起来**：
/// 「等待确认」「本轮已完成」「会话异常停止」全部静默。单元测试全绿，因为它们直接调
/// 检测器，绕过了这条从未接上的线。
///
/// 只做输出侧检测也补不回来：没有「用户提交了什么」这个起点，就无法区分「新一轮开始了」
/// 与「上一轮留在屏幕上的旧提示符」——后者会把每一帧重绘都误报成「本轮已完成」。
type DetectorHandle = Arc<Mutex<TerminalStatusDetector>>;

/// 找一个「可能不在 PATH 里」的 shell。
///
/// Git Bash 单独处理：它**默认不在 PATH 里**（Git for Windows 的安装器只把 `cmd`
/// 目录加进去），因此先找常见安装位置，找不到再退回 PATH。
fn locate_shell(kind: ShellKind, program: &str) -> Option<PathBuf> {
    if kind == ShellKind::GitBash {
        let found = git_bash_candidates(
            std::env::var("ProgramFiles").ok().as_deref(),
            std::env::var("LOCALAPPDATA").ok().as_deref(),
        )
        .into_iter()
        .find(|path| path.is_file());
        if found.is_some() {
            return found;
        }
    }
    std::env::var("PATH")
        .ok()
        .and_then(|path_env| find_on_path(program, &path_env))
}

/// 输出事件通道。
pub const CHANNEL_DATA: &str = "terminal:data";
/// 退出事件通道。
pub const CHANNEL_EXIT: &str = "terminal:exit";
/// 状态事件通道：终端进入「等待确认 / 出错」等需要用户注意的状态。
///
/// **与 `terminal:data` 分开是有意的**：数据是高频的原始字节，状态是低频的语义事件。
/// 混在一条通道上，前端就不得不在每个字节块上重跑一遍检测逻辑。
pub const CHANNEL_STATUS: &str = "terminal:status";

/// 事件发射器。
///
/// 抽成 trait 的理由与 [`crate::cli`] 的命令执行器相同：测试要能捕获事件，
/// 而不是依赖真实的 Tauri 窗口。
pub trait EventEmitter: Send + Sync {
    /// 发送一个事件。
    fn emit(&self, channel: &str, payload: Value);
}

/// 丢弃事件的发射器（用于不需要事件的场合）。
pub struct NullEmitter;

impl EventEmitter for NullEmitter {
    fn emit(&self, channel: &str, payload: Value) {
        // **丢弃也要留痕。**
        //
        // 这个发射器曾经是完全静默的，代价是：终端进程正常启动、不报任何错，但输出
        // 全部消失——排查时无从下手。**一次写入虚空**和**真的没有事件**在日志里必须
        // 能区分开，否则「没输出」这件事永远只能靠猜。
        //
        // 正常情况下不该看到这条日志：真实运行时发射器会被换成 `TauriEmitter`。
        // 如果它出现了，说明**发射器没有被换掉**。
        let bytes = payload
            .get("data")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        tracing::debug!(channel, bytes, "事件被 NullEmitter 丢弃");
    }
}

/// 终端命令处理器。
pub struct TerminalCommands {
    registry: Arc<SessionRegistry>,
    /// 发射器**槽位**，不是发射器本身。
    ///
    /// # 为什么必须是槽位
    ///
    /// 处理器在**构造时**就要一份发射器，而那时窗口还不存在（真实发射器装不进去）。
    /// 此前这里存的是 `Arc<dyn EventEmitter>` ——一次捕获、永不更新。后果是
    /// `App::install_emitter` 换掉的是**另一个**持有者，输出泵仍然往 `NullEmitter`
    /// 里写：**进程正常启动、不报任何错、终端永远空白。**
    ///
    /// 改成共享槽位后，`install_emitter` 换的那一份与这里读的是同一个，
    /// 输出立刻生效，不需要重建处理器。
    ///
    /// 网关事件（`aivc-app` 里的两个钩子）早就是这个模式，终端只是没跟上。
    emitter: Arc<std::sync::RwLock<Arc<dyn EventEmitter>>>,
    /// 每路终端的状态检测器，按 `terminalId` 索引。
    ///
    /// 存在的唯一理由是**让写回路径够得到检测器**：见 [`DetectorHandle`]。
    detectors: Arc<Mutex<HashMap<String, DetectorHandle>>>,
    /// 网关路由的来源。
    ///
    /// # 为什么必须注入而不是自己建
    ///
    /// 「创建一条路由」需要网关状态、供应商仓储与 WSL 探测三样东西，而 `aivc-ipc` 的
    /// 终端处理器**不该知道**这些怎么组装（测试更不该为了启动一个终端去建一个网关）。
    /// 这里只依赖 [`RouteSource`] 这个契约，默认实现是 [`NullRouteSource`]。
    routes: Arc<dyn RouteSource>,
}

impl TerminalCommands {
    /// 构造。
    #[must_use]
    pub fn new(registry: Arc<SessionRegistry>, emitter: Arc<dyn EventEmitter>) -> Self {
        Self {
            registry,
            emitter: Arc::new(std::sync::RwLock::new(emitter)),
            detectors: Arc::new(Mutex::new(HashMap::new())),
            routes: Arc::new(NullRouteSource),
        }
    }

    /// 装入真实的路由来源。
    ///
    /// 用 `&mut self`：它只在组装根里被调用一次，而路由来源是**结构的一部分**，
    /// 不像发射器那样需要在窗口就绪后被换掉。
    pub fn install_routes(&mut self, routes: Arc<dyn RouteSource>) {
        self.routes = routes;
    }

    /// 换掉终端事件的发射器。
    ///
    /// 由 `App::install_emitter` 在窗口就绪后调用。**用 `&self`**：处理器是被共享
    /// 持有的，换发射器不该要求独占（那会逼调用方去改所有权结构）。
    pub fn install_emitter(&self, emitter: Arc<dyn EventEmitter>) {
        // 中毒的锁说明有线程在持锁时 panic 了；此时**宁可丢弃事件**也不 unwrap ——
        // 换发射器失败不该让整个应用崩溃。
        if let Ok(mut slot) = self.emitter.write() {
            *slot = emitter;
        }
    }

    /// 供测试断言「换装的是同一个槽位」。
    ///
    /// `#[cfg(test)]`：这不是对外契约，只给测试用——让测试能验证**泵读的槽位与换装的
    /// 槽位是同一个**。那正是曾经出错的地方（换装换的是另一份，泵读的还是旧的）。
    #[cfg(test)]
    pub(crate) fn emitter_slot(&self) -> &Arc<std::sync::RwLock<Arc<dyn EventEmitter>>> {
        &self.emitter
    }

    /// 供测试预置检测器。
    ///
    /// `#[cfg(test)]`：真实路径上检测器由 [`Self::spawn_session`] 登记，测试不该去启动
    /// 一个真的 `codex` 进程才能验证「写回够不够得到检测器」。
    #[cfg(test)]
    pub(crate) fn detector_slot(&self) -> &Arc<Mutex<HashMap<String, DetectorHandle>>> {
        &self.detectors
    }

    /// 启动一路终端。
    ///
    /// # 四个平台走同一条路
    ///
    /// `providerId` 决定拼哪条命令行，`targetId` 决定在本机还是某个 WSL 发行版里跑。
    /// 两个维度都交给 [`build_command`]，因此这里**不再有**「暂不支持」这种分支：
    /// 那个分支曾经让 Claude / Gemini / Qoder 完全无法启动。
    ///
    /// # 路由是按终端创建的
    ///
    /// 启动前先向 [`RouteSource`] 要一条路由。拿到就把上游地址与本地令牌注入命令；
    /// 拿不到（平台不支持网关、网关没启动、没有可用供应商）就**照常启动**——
    /// 直连官方 API 是合法的降级，不是错误。
    ///
    /// 启动失败时必须把它销毁：终端没起来，路由留着就是一条永远不会被请求的凭据。
    ///
    /// # Errors
    ///
    /// 参数非法、路由创建失败（供应商平台不匹配）、命令构造失败或 PTY 启动失败。
    pub async fn start(&self, args: &Value, distro: Option<&str>) -> Result<Value> {
        let composed = self.compose(args, distro).await?;
        match self
            .spawn_session(
                composed.terminal_id,
                &composed.command,
                composed.cols,
                composed.rows,
                composed.provider,
            )
            .await
        {
            // 把**后端选定的供应商**一并返回。
            //
            // 界面不在启动时挑供应商（它还不知道哪家合适），而是绑定后端选出来的那一家，
            // 之后靠 `gateway:vendor-switched` 更新。现网就是这么做的：启动结果里带
            // `vendorId` / `vendorMode`。少了它，状态栏就无从显示「当前由谁服务」。
            Ok(mut value) => {
                if let Some(binding) = composed.binding.as_ref() {
                    value["vendorId"] = json!(binding.vendor_id);
                    value["vendorMode"] = json!(mode_str(binding.mode));
                }
                Ok(value)
            }
            Err(error) => {
                // 终端没起来，这条路由永远不会被请求。留着它只会积累凭据。
                if let Some(binding) = composed.binding.as_ref() {
                    self.routes.destroy(&binding.route_id).await;
                }
                Err(error)
            }
        }
    }

    /// 组装一次启动所需的全部东西，但**不起进程**。
    ///
    /// # 为什么要把「组装」从「起进程」里拆出来
    ///
    /// 因为「路由有没有真的进到命令里」是本条链路上**最容易出错、又最看不见**的一环：
    /// 路由生产者、命令构造器、路由销毁三处都各自有测试，唯独「拿到路由之后有没有
    /// 把它交给命令构造器」这一段只能靠人看代码。而这段恰恰就是此前那个漏洞的形状——
    /// `ResolvedRoute` 有类型、有消费者，就是没有生产者，于是终端只能传 `None`，
    /// **测试全绿而网关完全没生效**。
    ///
    /// 拆出来之后，[`Self::start`] 只剩「组装 → 起进程 → 失败则销毁路由」，而组装结果
    /// 可以被直接断言。
    async fn compose(&self, args: &Value, distro: Option<&str>) -> Result<Composed> {
        let terminal_id = require_terminal_id(args.get("terminalId"))?;
        let provider = require_provider(args)?;
        let params = CommandParams {
            session_id: optional_string(args, "sessionId"),
            cwd: optional_string(args, "cwd"),
            codex_home: optional_string(args, "codexHome"),
            cli_args: optional_string(args, "cliArgs"),
            use_cwd_flag: args
                .get("useCodexCwdFlag")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            home: home_dir(),
        };
        let extra = extra_args(&params)?;

        // `distro` 由调用方从 `targetId` 解出（与 `start_system` 同一个入口）。
        let distro = distro.map(str::trim).filter(|value| !value.is_empty());
        let vendor_id = optional_string(args, "vendorId");
        let binding = self
            .routes
            .create(provider, distro, vendor_id.as_deref(), &terminal_id)
            .await?;
        let route = binding.as_ref().map(|item| &item.route);
        let target = match distro {
            Some(name) => LaunchTarget::wsl(name),
            None => LaunchTarget::local(cfg!(windows)),
        };
        let mut command = build_command(provider, &params, route, &extra, &target);
        if distro.is_none() {
            // Windows：把 npm / nvm 的安装目录补进子进程的 PATH。
            //
            // 从资源管理器启动时，应用继承的是**那一刻**的 PATH；之后再装的 Node / CLI
            // 不在里面，而 `cmd.exe` 只认 PATH——表现为「不是内部或外部命令」或一闪而过。
            // 非 Windows 上是空操作（那边的 shim 是可执行脚本，shell 自己找得到）。
            aivc_terminal::locate::extend_path(&mut command.env);
        }

        let cols = args
            .get("cols")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(DEFAULT_COLS);
        let rows = args
            .get("rows")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(DEFAULT_ROWS);

        Ok(Composed {
            terminal_id,
            provider,
            command,
            binding,
            cols,
            rows,
        })
    }

    /// 起 PTY、登记会话、把输出泵起来。
    ///
    /// 抽出来是因为 Codex 终端与系统终端**只有「拼什么命令」不同**，其余完全一样。
    async fn spawn_session(
        &self,
        terminal_id: String,
        command: &TerminalCommand,
        cols: u16,
        rows: u16,
        provider: ProviderId,
    ) -> Result<Value> {
        let (session, output) = PtySession::spawn(command, cols, rows)?;
        self.registry.insert(terminal_id.clone(), session).await;
        // 检测器在这里建、在这里登记：`write` 之后要靠 `terminalId` 找回同一个实例。
        let detector: DetectorHandle = Arc::new(Mutex::new(TerminalStatusDetector::new(provider)));
        if let Ok(mut map) = self.detectors.lock() {
            map.insert(terminal_id.clone(), Arc::clone(&detector));
        }
        // 泵独立于本次调用：命令返回后输出仍在到达。
        tokio::spawn(pump_output(
            Arc::clone(&self.emitter),
            Arc::clone(&self.registry),
            Arc::clone(&self.detectors),
            Arc::clone(&self.routes),
            terminal_id.clone(),
            detector,
            output,
        ));
        Ok(json!({ "terminalId": terminal_id, "cols": cols, "rows": rows }))
    }

    /// 启动一路**系统终端**。
    ///
    /// 与 [`Self::start`] 的区别：那个跑的是 Codex（带网关路由与配置注入），这个跑的是
    /// **用户本来就在用的那个 shell**——因此它**不注入任何环境变量**。
    ///
    /// `distro` 由调用方从 `targetId` 解出：目标是 WSL 目标、而用户没指定 shell 时，
    /// 应当开那个发行版里的 shell，在本机开一个 PowerShell 是答非所问。
    ///
    /// # Errors
    ///
    /// 参数非法、指定的 shell 在本机找不到、或 PTY 启动失败时返回错误。
    pub async fn start_system(&self, args: &Value, distro: Option<&str>) -> Result<Value> {
        let terminal_id = require_terminal_id(args.get("terminalId"))?;
        let kind = match args.get("kind").and_then(Value::as_str) {
            Some(value) => ShellKind::parse(value)?,
            None => auto_kind_for(distro),
        };
        let choice = resolve_shell(kind, cfg!(windows), distro);
        // 只有「可能不在 PATH 里」的 shell 才需要查找；`cmd.exe` / `wsl.exe` 由系统保证。
        let program = if choice.search_path {
            locate_shell(kind, choice.program)
        } else {
            None
        };
        let home = home_dir();
        // 系统终端开在**用户主目录**，不是应用的工作目录。
        //
        // 这里原本用 `default_work_dir(&home)`（即 `<home>/.akim`），那个目录是给 Codex
        // 用的，**在没跑过 Codex 的机器上根本不存在**——而 PTY 用不存在的目录启动会直接
        // 失败。更糟的是失败被后续的 `resize`（"终端会话不存在"）盖住，界面只剩一句
        // 和原因毫无关系的报错。
        let cwd = optional_string(args, "cwd").unwrap_or_else(|| {
            if home.trim().is_empty() {
                // 连主目录都取不到时用临时目录：它是**必然存在**的。
                std::env::temp_dir().to_string_lossy().into_owned()
            } else {
                home
            }
        });
        let command = build_system_shell_command(kind, program, &cwd, distro, cfg!(windows))?;

        let cols = args
            .get("cols")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(DEFAULT_COLS);
        let rows = args
            .get("rows")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .unwrap_or(DEFAULT_ROWS);
        self.spawn_session(terminal_id, &command, cols, rows, ProviderId::Codex)
            .await
    }

    /// 把输入写入 PTY。
    ///
    /// **这是终端能动的关键一环**：渲染层（xterm.js）对 `ESC[6n` 等查询的应答
    /// 正是通过这里回到 PTY 的。断了这条链路，终端会静默卡住。
    ///
    /// 同时把输入交给状态检测器——**顺序不能反**，理由见 [`Self::mark_input`]。
    ///
    /// # Errors
    ///
    /// 会话不存在或写入失败时返回错误。
    pub async fn write(&self, args: &Value) -> Result<()> {
        let terminal_id = require_string(args, "terminalId")?;
        let data = args
            .get("data")
            .and_then(Value::as_str)
            .ok_or_else(|| invalid("data"))?;
        self.mark_input(&terminal_id, data);
        self.registry.write(&terminal_id, data).await
    }
    /// 把用户输入告诉该终端的检测器，状态有变化就发事件。
    ///
    /// # 为什么必须在写进 PTY **之前**调用
    ///
    /// `mark_input` 干的正是「把检测器置为运行中」这件事，而 `inspect_output` 在非运行态
    /// 直接返回。先写 PTY 的话，程序对这次输入产生的**第一批**输出有可能先被泵取走并检查——
    /// 那一批会被整批丢掉，而它恰恰常常就带着提示符或确认请求。
    ///
    /// 检测器不存在（终端不是经由本处理器启动的）时**静默跳过**：写回本身才是这条命令的
    /// 契约，检测只是搭车。
    fn mark_input(&self, terminal_id: &str, data: &str) {
        let handle = self
            .detectors
            .lock()
            .ok()
            .and_then(|map| map.get(terminal_id).cloned());
        let Some(handle) = handle else {
            return;
        };
        let event = handle
            .lock()
            .ok()
            .and_then(|mut detector| detector.mark_input(data));
        if let Some(event) = event {
            emit(
                &self.emitter,
                CHANNEL_STATUS,
                status_payload(terminal_id, &event),
            );
        }
    }

    /// 调整终端尺寸。
    ///
    /// # Errors
    ///
    /// 会话不存在或调整失败时返回错误。
    pub async fn resize(&self, args: &Value) -> Result<()> {
        let terminal_id = require_string(args, "terminalId")?;
        let cols = require_u16(args, "cols")?;
        let rows = require_u16(args, "rows")?;
        self.registry.resize(&terminal_id, cols, rows).await
    }

    /// 停止一路终端。
    ///
    /// # 顺带销毁网关路由
    ///
    /// 路由是**按终端**创建的，终端没了它就永远不会再被请求——留着只是让网关的路由表
    /// 无限增长，并保留一份活着的本地凭据。销毁失败不影响「停止终端」这件事本身，
    /// 因此这里不传播它的错误。
    ///
    /// # Errors
    ///
    /// 会话不存在时返回错误。
    pub async fn stop(&self, args: &Value) -> Result<Value> {
        let terminal_id = require_string(args, "terminalId")?;
        let stopped = self.registry.stop(&terminal_id).await?;
        // 检测器随之退休：它按会话持有输出缓冲，留着不清理会随终端开关持续增长。
        if let Ok(mut map) = self.detectors.lock() {
            map.remove(&terminal_id);
        }
        self.routes.destroy(&terminal_id).await;
        Ok(json!({ "stopped": stopped }))
    }

    /// 当前会话数（供诊断与测试）。
    pub async fn count(&self) -> usize {
        self.registry.len().await
    }
}

/// 路由模式的线上形状（与 `vendor:route-mode` 的返回值一致）。
fn mode_str(mode: aivc_gateway::RouteMode) -> &'static str {
    if mode == aivc_gateway::RouteMode::Locked {
        "locked"
    } else {
        "dynamic"
    }
}

/// 一路终端启动所需的全部东西（尚未起进程）。
///
/// 拆出来的理由见 [`TerminalCommands::compose`]。
struct Composed {
    terminal_id: String,
    provider: ProviderId,
    command: TerminalCommand,
    /// 创建成功的路由；启动失败时用它销毁。
    binding: Option<crate::route::RouteBinding>,
    cols: u16,
    rows: u16,
}

/// 把 PTY 输出泵到事件里，直到通道关闭。
///
/// 通道关闭意味着读取线程结束（PTY 已关闭），此时发一条退出事件。
///
/// # 顺带做状态检测
///
/// 检测器每路终端一个（它带缓冲，用于识别**被切分到多个数据块里**的提示文本——
/// 逐块独立扫描会漏掉这种最常见的形态）。泵负责**输出侧**（`inspect_output`）；
/// **输入侧**（`mark_input`）由写回路径负责，两边共用同一个检测器实例。
///
/// # 关于退出码
///
/// 泵只能观察到「通道关闭」，**退出码不经过输出通道**——它由收养任务
/// （`SessionRegistry::spawn_reaper`）放进注册表，这里在通道关闭之后取走。
///
/// 取到码就喂 [`TerminalStatusDetector::mark_exit`]：非零退出才报「会话异常停止」。
/// 取不到码说明这不是「进程自己结束」（用户主动关闭了终端），按 0 处理——正常结束，
/// 静默。**这不是拿 0 糊弄**：`None` 在这里有确切含义，而主动关闭本就不该报异常。
async fn pump_output(
    emitter: Arc<std::sync::RwLock<Arc<dyn EventEmitter>>>,
    registry: Arc<SessionRegistry>,
    detectors: Arc<Mutex<HashMap<String, DetectorHandle>>>,
    routes: Arc<dyn RouteSource>,
    terminal_id: String,
    detector: DetectorHandle,
    mut output: mpsc::Receiver<Vec<u8>>,
) {
    while let Some(chunk) = output.recv().await {
        // 检测要用**文本**，因此先解一份出来。用有损解码：交互式输出含光标控制
        // 序列，它们本来就不是合法 UTF-8，严格解码会直接失败。
        let text = String::from_utf8_lossy(&chunk).into_owned();
        // 原始字节照旧原样送出——xterm 的 `write` 接受 `Uint8Array`。
        emit(
            &emitter,
            CHANNEL_DATA,
            json!({ "terminalId": terminal_id, "data": chunk }),
        );
        // 锁只在这一小块里持有：写回路径要能随时插进来标记输入。
        let event = detector
            .lock()
            .ok()
            .and_then(|mut d| d.inspect_output(&text));
        if let Some(event) = event {
            emit(
                &emitter,
                CHANNEL_STATUS,
                status_payload(&terminal_id, &event),
            );
        }
    }

    let exit_code = registry.take_exit_code(&terminal_id).await;
    let event = detector
        .lock()
        .ok()
        .and_then(|mut d| d.mark_exit(exit_code.unwrap_or(0)));
    if let Some(event) = event {
        emit(
            &emitter,
            CHANNEL_STATUS,
            status_payload(&terminal_id, &event),
        );
    }

    if let Ok(mut map) = detectors.lock() {
        map.remove(&terminal_id);
    }
    // **子进程自己退出时也要销毁路由。**
    //
    // 此前只有 `stop`（用户主动关终端）会销毁它。子进程自己退出——`/exit`、
    // 或 CLI 崩溃——走的是这条路，路由于是留在网关路由表里：
    //
    // - 那条路由的**本地令牌仍然有效**，能继续打到那家供应商；
    // - `route_id` 用的就是终端编号，而界面的编号是「时间戳 + 自增」，
    //   正常情况下不会撞上；但**表本身会随终端开关一直增长**。
    //
    // 与 `stop` 的语义一致：路由不存在时静默返回（终端可能压根没走网关）。
    routes.destroy(&terminal_id).await;
    emit(
        &emitter,
        CHANNEL_EXIT,
        json!({ "terminalId": terminal_id, "exitCode": exit_code }),
    );
}

/// 组装 `terminal:status` 的载荷。
///
/// # 为什么由调用方传 `terminal_id`
///
/// [`TerminalStatusEvent::terminal_id`] 由检测器 `publish` 时填成**空串**——检测器不知道
/// 自己属于哪一路终端，它只管文本。此前泵直接把这个空串发出去，前端按 `terminalId`
/// 分发，于是每条状态事件都落到了 `""` 这个不存在的键上：**关注提醒永远不会出现在
/// 任何一个终端上**。这是与「`mark_input` 从未被调用」互相独立的第二个断点——
/// 只修其中一个，功能依然是死的。
///
/// 所以这里显式用调用方手上的真实 id 覆盖，`terminal_id` 字段只作展示用途。
fn status_payload(terminal_id: &str, event: &TerminalStatusEvent) -> Value {
    json!({
        "terminalId": terminal_id,
        "status": event.status.as_str(),
        "attention": event.attention,
    })
}

/// 从**槽位**里取当前发射器并发一条事件。
///
/// **每次发射都重新读**——这正是修好「输出写进 NullEmitter」的关键：发射器在终端
/// 启动之后才被 `install_emitter` 换上，任何在启动时缓存下来的副本都会一直是旧的。
///
/// 锁中毒（持锁线程 panic）时**丢弃事件**而不是 unwrap：丢一条输出远好过让整个
/// 应用崩掉。
fn emit(slot: &Arc<std::sync::RwLock<Arc<dyn EventEmitter>>>, channel: &str, payload: Value) {
    if let Ok(current) = slot.read() {
        current.emit(channel, payload);
    }
}

/// 主目录。
///
/// 取不到时不臆造：返回空串，由 `default_work_dir` 决定退路。
fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_default()
}

/// 构造一个「参数无效」错误。
fn invalid(name: &str) -> Error {
    Error::Ipc(IpcError::InvalidParameter {
        name: name.to_owned(),
    })
}

/// 把字符串错误包成命令面错误。
///
/// `#[cfg(test)]`：生产路径上的失败都来自 `validate`（参数非法）或 `PTY`（启动失败），
/// 各自带着更准确的信息。这个助手留给测试里的**替身**——它们要能凭空造一个失败。
#[cfg(test)]
fn failed(message: String) -> Error {
    Error::Ipc(IpcError::CommandFailed { message })
}

/// 必填字符串参数。
fn require_string(args: &Value, name: &str) -> Result<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid(name))
}

/// 可选字符串参数（空白视为未提供）。
fn optional_string(args: &Value, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// 必填 u16 参数。
fn require_u16(args: &Value, name: &str) -> Result<u16> {
    args.get(name)
        .and_then(Value::as_u64)
        .and_then(|value| u16::try_from(value).ok())
        .ok_or_else(|| invalid(name))
}

/// 必填平台参数。
fn require_provider(args: &Value) -> Result<ProviderId> {
    let raw = require_string(args, "providerId")?;
    match raw.as_str() {
        "codex" => Ok(ProviderId::Codex),
        "gemini" => Ok(ProviderId::Gemini),
        "claude" => Ok(ProviderId::Claude),
        "qoder" => Ok(ProviderId::Qoder),
        _ => Err(invalid("providerId")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use aivc_terminal::command::ResolvedRoute;

    /// 记录事件的发射器。
    #[derive(Default)]
    struct Recording {
        events: Mutex<Vec<(String, Value)>>,
    }

    impl Recording {
        fn channels(&self) -> Vec<String> {
            self.events
                .lock()
                .expect("lock")
                .iter()
                .map(|(channel, _)| channel.clone())
                .collect()
        }

        fn payloads(&self, channel: &str) -> Vec<Value> {
            self.events
                .lock()
                .expect("lock")
                .iter()
                .filter(|(name, _)| name == channel)
                .map(|(_, payload)| payload.clone())
                .collect()
        }
    }

    impl EventEmitter for Recording {
        fn emit(&self, channel: &str, payload: Value) {
            self.events
                .lock()
                .expect("lock")
                .push((channel.to_owned(), payload));
        }
    }

    fn commands(emitter: Arc<Recording>) -> TerminalCommands {
        TerminalCommands::new(Arc::new(SessionRegistry::new()), emitter)
    }

    /// 一次 `create` 调用留下的记录：平台、发行版、供应商、终端编号。
    type CreateCall = (ProviderId, Option<String>, Option<String>, String);

    /// 记录调用的路由来源。
    ///
    /// 存在的理由是**让「路由有没有接上终端」可以被观测**：这条线此前根本不存在，
    /// 因此也不可能有任何测试覆盖它。
    #[derive(Default)]
    struct FakeRoutes {
        created: Mutex<Vec<CreateCall>>,
        destroyed: Mutex<Vec<String>>,
        /// `Some` 时 `create` 返回它——用来测「路由创建失败必须传播」。
        fail: Option<String>,
        /// 创建成功时发给终端的路由。
        route: Option<ResolvedRoute>,
    }

    /// 命令对外的全部可见内容：参数与环境变量。
    ///
    /// 路由的注入方式按平台不同（本机 Windows 走 `env`，POSIX 与 WSL 走命令行），
    /// 因此断言时把两处摊平了一起看。
    fn command_surface(command: &TerminalCommand) -> String {
        let mut parts = vec![command.args.join(" ")];
        parts.extend(
            command
                .env
                .iter()
                .map(|(key, value)| format!("{key}={value}")),
        );
        parts.join(" ")
    }

    impl FakeRoutes {
        fn returning(route: ResolvedRoute) -> Self {
            Self {
                route: Some(route),
                ..Self::default()
            }
        }

        fn destroys(&self) -> Vec<String> {
            self.destroyed.lock().expect("lock").clone()
        }
    }

    impl RouteSource for FakeRoutes {
        fn create<'a>(
            &'a self,
            provider: ProviderId,
            distro: Option<&'a str>,
            vendor_id: Option<&'a str>,
            terminal_id: &'a str,
        ) -> aivc_core::ports::BoxFuture<'a, Result<Option<crate::route::RouteBinding>>> {
            Box::pin(async move {
                self.created.lock().expect("lock").push((
                    provider,
                    distro.map(str::to_owned),
                    vendor_id.map(str::to_owned),
                    terminal_id.to_owned(),
                ));
                if let Some(message) = self.fail.as_ref() {
                    return Err(failed(message.clone()));
                }
                Ok(self.route.clone().map(|route| crate::route::RouteBinding {
                    route_id: terminal_id.to_owned(),
                    route,
                    vendor_id: "v1".to_owned(),
                    mode: aivc_gateway::RouteMode::Dynamic,
                }))
            })
        }

        fn destroy<'a>(&'a self, route_id: &'a str) -> aivc_core::ports::BoxFuture<'a, ()> {
            Box::pin(async move {
                self.destroyed
                    .lock()
                    .expect("lock")
                    .push(route_id.to_owned());
            })
        }
    }

    /// 一个**必然建不出来**的工作目录：它的父级是一个文件。
    ///
    /// 用它制造「路由已创建、PTY 启动失败」这条**确定性**路径。这比去 mock 一个 PTY
    /// 简单得多，而且走的是真实失败分支：`PtySession::spawn` 会先尝试建目录，
    /// 父级是文件时 `create_dir_all` 必然失败。
    ///
    /// 返回值里的 `TempDir` 必须**活着**——它一被丢弃，父文件就没了，失败也就不成立。
    fn uncreatable_cwd() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("临时目录");
        let blocker = dir.path().join("not-a-directory");
        std::fs::write(&blocker, b"x").expect("写文件");
        let cwd = blocker.join("sub").to_string_lossy().into_owned();
        (dir, cwd)
    }

    #[tokio::test]
    async fn the_created_route_is_actually_written_into_the_command() {
        // **这是本轮最重要的一条测试。**
        //
        // 此前那个漏洞的形状是：`ResolvedRoute` 有类型、有三个消费者，**就是没有生产者**，
        // 于是终端启动时只能传 `None`——网关完全没生效，而所有单元测试都是绿的。
        //
        // 只测「路由被创建了」还不够：创建了却不交给命令构造器，症状与完全没有生产者
        // 一模一样。因此这里直接断言**令牌与地址出现在最终命令里**。
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes::returning(ResolvedRoute {
            base_url: "http://127.0.0.1:8787/gateway/claude/term-7".to_owned(),
            local_token: "local-token-abc".to_owned(),
        }));
        let mut commands = commands(recording);
        commands.install_routes(routes);

        let composed = commands
            .compose(
                &json!({ "terminalId": "term-7", "providerId": "claude" }),
                None,
            )
            .await
            .expect("组装");

        // 环境变量的传递方式按平台不同（本机 Windows 走 `env`，POSIX 走命令行），
        // 因此把两处摊平了一起看。
        let surface = command_surface(&composed.command);
        assert!(
            surface.contains("local-token-abc"),
            "本地令牌必须进入命令，否则网关会拒绝这次请求：{surface}"
        );
        assert!(
            surface.contains("/gateway/claude/term-7"),
            "路由地址必须进入命令，否则终端直连官方 API：{surface}"
        );
    }

    #[tokio::test]
    async fn without_a_route_the_command_carries_no_gateway_configuration() {
        // 反面对照：拿不到路由时命令里**不该**出现任何网关痕迹——否则用户会以为
        // 自己配的供应商生效了。
        let recording = Arc::new(Recording::default());
        let mut commands = commands(recording);
        commands.install_routes(Arc::new(FakeRoutes::default()));

        let composed = commands
            .compose(
                &json!({ "terminalId": "term-8", "providerId": "claude" }),
                None,
            )
            .await
            .expect("组装");

        let surface = command_surface(&composed.command);
        assert!(!surface.contains("/gateway/"), "{surface}");
        assert!(composed.binding.is_none());
    }

    #[tokio::test]
    async fn the_start_result_carries_the_vendor_the_backend_chose() {
        // 界面**不在启动时挑供应商**（它还不知道哪家合适），而是绑定后端选出来的那一家，
        // 之后靠 `gateway:vendor-switched` 更新。因此启动结果必须带上它——
        // 少了它，状态栏无从显示「当前由谁服务」。
        let recording = Arc::new(Recording::default());
        let mut commands = commands(recording);
        commands.install_routes(Arc::new(FakeRoutes::returning(ResolvedRoute {
            base_url: "http://127.0.0.1:8787/gateway/claude/term-9".to_owned(),
            local_token: "tok".to_owned(),
        })));
        let dir = tempfile::tempdir().expect("临时目录");

        let value = commands
            .start(
                &json!({
                    "terminalId": "term-9",
                    "providerId": "claude",
                    "cwd": dir.path().to_string_lossy(),
                }),
                None,
            )
            .await
            .expect("启动");

        assert_eq!(value["vendorId"], "v1");
        assert_eq!(value["vendorMode"], "dynamic");
        commands
            .stop(&json!({ "terminalId": "term-9" }))
            .await
            .expect("停止");
    }

    #[tokio::test]
    async fn without_a_route_the_start_result_has_no_vendor() {
        // 直连官方 API 时没有供应商可言。返回一个假的名字会让界面显示一个不存在的绑定。
        let recording = Arc::new(Recording::default());
        let mut commands = commands(recording);
        commands.install_routes(Arc::new(FakeRoutes::default()));
        let dir = tempfile::tempdir().expect("临时目录");

        let value = commands
            .start(
                &json!({
                    "terminalId": "term-10",
                    "providerId": "codex",
                    "cwd": dir.path().to_string_lossy(),
                }),
                None,
            )
            .await
            .expect("启动");

        assert!(value.get("vendorId").is_none(), "{value}");
        assert!(value.get("vendorMode").is_none(), "{value}");
        commands
            .stop(&json!({ "terminalId": "term-10" }))
            .await
            .expect("停止");
    }

    #[tokio::test]
    async fn the_route_reaches_a_non_codex_terminal() {
        // **回归测试**：终端此前只接了 Codex，其余平台一律「暂不支持启动 X 终端」。
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes::default());
        let mut commands = commands(recording);
        commands.install_routes(routes.clone());
        let (_keep, cwd) = uncreatable_cwd();

        let error = commands
            .start(
                &json!({
                    "terminalId": "term-1",
                    "providerId": "gemini",
                    "cwd": cwd,
                }),
                None,
            )
            .await
            .expect_err("工作目录建不出来，必须失败");
        assert!(
            !error.to_string().contains("暂不支持"),
            "非 Codex 平台不该再被拒绝：{error}"
        );
        assert_eq!(
            routes.created.lock().expect("lock").as_slice(),
            &[(ProviderId::Gemini, None, None, "term-1".to_owned())],
            "必须为 Gemini 创建路由，并把终端编号作为路由编号"
        );
    }

    #[tokio::test]
    async fn a_failed_launch_destroys_the_route_it_created() {
        // 终端没起来，那条路由永远不会被请求。留着它就是一份活着的凭据。
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes::returning(ResolvedRoute {
            base_url: "http://127.0.0.1:8787/gateway/claude/term-2".to_owned(),
            local_token: "tok".to_owned(),
        }));
        let mut commands = commands(recording);
        commands.install_routes(routes.clone());
        let (_keep, cwd) = uncreatable_cwd();

        commands
            .start(
                &json!({
                    "terminalId": "term-2",
                    "providerId": "claude",
                    "cwd": cwd,
                }),
                None,
            )
            .await
            .expect_err("工作目录建不出来，必须失败");

        assert_eq!(routes.destroys(), vec!["term-2".to_owned()]);
    }

    #[tokio::test]
    async fn stopping_a_terminal_destroys_its_route() {
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes::default());
        let mut commands = commands(recording);
        commands.install_routes(routes.clone());

        // 会话不存在也要销毁路由：路由是按终端编号建的，终端编号是路由的唯一线索。
        commands
            .stop(&json!({ "terminalId": "term-3" }))
            .await
            .expect("停止未知会话是幂等的");
        assert_eq!(routes.destroys(), vec!["term-3".to_owned()]);
    }

    #[tokio::test]
    async fn a_route_failure_stops_the_launch_before_the_pty() {
        // 显式指定的供应商属于别的平台时必须失败，而不是悄悄换一个供应商。
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes {
            fail: Some("供应商协议与终端类型不匹配。".to_owned()),
            ..FakeRoutes::default()
        });
        let mut commands = commands(recording);
        commands.install_routes(routes.clone());

        let error = commands
            .start(
                &json!({
                    "terminalId": "term-4",
                    "providerId": "claude",
                    "vendorId": "codex-vendor",
                }),
                None,
            )
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("不匹配"), "{error}");
        assert_eq!(commands.count().await, 0, "失败时不该留下会话");
        assert!(routes.destroys().is_empty(), "没有创建成功就不该销毁");
    }

    #[tokio::test]
    async fn a_terminal_id_that_cannot_be_a_route_is_refused() {
        // 含 `/` 的编号会让网关按一个不存在的路由去鉴权——终端能起来、请求全失败。
        // 因此必须在入口挡住，而不是静默替换字符。
        let recording = Arc::new(Recording::default());
        let mut commands = commands(recording);
        commands.install_routes(Arc::new(FakeRoutes::default()));

        let error = commands
            .start(&json!({ "terminalId": "a/b", "providerId": "codex" }), None)
            .await
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("终端编号"), "{error}");
    }

    #[tokio::test]
    async fn the_distro_is_handed_to_the_route_source() {
        // WSL 目标的上游地址必须换成发行版内可达的宿主地址，因此发行版必须在路由
        // 创建**之前**就传下去。
        //
        // 这里**不看启动结果**：WSL 形态的 `cwd` 是宿主主目录（真正的目录处理在发行版
        // 脚本里），因此 PTY 一定起得来，而发行版本身存不存在取决于本机。
        // 本测试要钉的是「发行版有没有传到路由那一层」，与启动成败无关。
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes::default());
        let mut commands = commands(recording);
        commands.install_routes(routes.clone());
        let dir = tempfile::tempdir().expect("临时目录");

        let _ = commands
            .start(
                &json!({
                    "terminalId": "term-5",
                    "providerId": "codex",
                    "cwd": dir.path().to_string_lossy(),
                }),
                Some("Ubuntu-24.04"),
            )
            .await;

        assert_eq!(
            routes.created.lock().expect("lock")[0].1.as_deref(),
            Some("Ubuntu-24.04")
        );
        let _ = commands.stop(&json!({ "terminalId": "term-5" })).await;
    }

    #[tokio::test]
    async fn a_blank_distro_is_treated_as_this_machine() {
        // 空发行版名会拼出 `wsl.exe -d ""`，那是必然失败的命令。
        let recording = Arc::new(Recording::default());
        let routes = Arc::new(FakeRoutes::default());
        let mut commands = commands(recording);
        commands.install_routes(routes.clone());
        let (_keep, cwd) = uncreatable_cwd();

        let _ = commands
            .start(
                &json!({
                    "terminalId": "term-6",
                    "providerId": "codex",
                    "cwd": cwd,
                }),
                Some("   "),
            )
            .await;

        assert_eq!(routes.created.lock().expect("lock")[0].1, None);
    }

    #[test]
    fn a_shared_emitter_can_be_recorded() {
        // 保证 trait 对象与具体类型的关系正确。
        let recording = Arc::new(Recording::default());
        let as_trait: Arc<dyn EventEmitter> = recording.clone();
        as_trait.emit("x", json!(1));
        assert_eq!(recording.channels(), vec!["x".to_owned()]);
    }

    /// **这条测试本该早就在。**
    ///
    /// 真实应用是这样装配的：处理器在**构造时**先拿 `NullEmitter`（那时窗口还不存在），
    /// 窗口就绪后再 `install_emitter` 换上真正的发射器。
    ///
    /// 而处理器此前把发射器**捕获成自己的一份**，换装换的却是另一份——于是输出泵一直
    /// 往 `NullEmitter` 里写：终端进程正常启动、不报任何错、界面永远空白。
    /// 单元测试全绿，功能完全不可用。
    ///
    /// 这里断言的是那个决定性的事实：**换装改的就是泵读的同一个槽位**。
    #[test]
    fn install_emitter_replaces_the_slot_the_pump_reads() {
        let recording = Arc::new(Recording::default());
        // 构造时先给 `NullEmitter`——真实应用就是这样（那时窗口还不存在）。
        let commands =
            TerminalCommands::new(Arc::new(SessionRegistry::new()), Arc::new(NullEmitter));

        let slot = Arc::clone(commands.emitter_slot());
        let as_trait: Arc<dyn EventEmitter> = recording.clone();
        commands.install_emitter(as_trait);

        // 原地替换：还是同一个 Arc，不是换了一个。
        assert!(Arc::ptr_eq(&slot, commands.emitter_slot()));

        // 槽位里现在装的必须是新发射器，而且发出去的事件真的到得了它。
        let current = slot.read().expect("槽位未中毒").clone();
        current.emit("terminal:data", json!({ "terminalId": "t1", "data": [1] }));
        assert_eq!(
            recording.payloads("terminal:data").len(),
            1,
            "换装之后事件必须到达新发射器"
        );
    }

    /// 锁中毒（持锁线程 panic）时**丢弃事件**，不 panic、不 unwrap。
    /// 丢一条输出远好过让整个应用崩掉。
    #[test]
    fn a_poisoned_slot_drops_events_instead_of_panicking() {
        let slot: Arc<std::sync::RwLock<Arc<dyn EventEmitter>>> =
            Arc::new(std::sync::RwLock::new(Arc::new(NullEmitter)));

        let poisoned = Arc::clone(&slot);
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.write().expect("首次加锁成功");
            panic!("故意毒化锁");
        })
        .join();

        // 不 panic 即通过。
        emit(
            &slot,
            "terminal:data",
            json!({ "terminalId": "t1", "data": [] }),
        );
    }

    #[test]
    fn missing_arguments_are_reported_by_name() {
        // 参数错误要指名道姓，否则调用方无从下手。
        assert!(require_string(&json!({}), "terminalId").is_err());
        assert!(require_string(&json!({ "terminalId": "  " }), "terminalId").is_err());
        assert!(require_u16(&json!({ "cols": -1 }), "cols").is_err());
        assert!(require_u16(&json!({ "cols": 70000 }), "cols").is_err());
        assert!(require_u16(&json!({ "cols": 80 }), "cols").is_ok());
    }

    #[test]
    fn the_provider_is_parsed_strictly() {
        // 与其他命令面一致：**不能**用 `from_target_id`，它会把未知输入兜底成 Codex。
        assert_eq!(
            require_provider(&json!({ "providerId": "claude" })).expect("claude"),
            ProviderId::Claude
        );
        assert!(require_provider(&json!({ "providerId": "codex-cli" })).is_err());
        assert!(require_provider(&json!({ "providerId": "openai" })).is_err());
    }

    #[test]
    fn optional_arguments_treat_blank_as_absent() {
        assert_eq!(optional_string(&json!({}), "cwd"), None);
        assert_eq!(optional_string(&json!({ "cwd": "   " }), "cwd"), None);
        assert_eq!(
            optional_string(&json!({ "cwd": " /tmp " }), "cwd"),
            Some("/tmp".to_owned())
        );
    }

    /// **回归测试：整条状态检测链的两个断点。**
    ///
    /// 这两个断点互相独立，只修任何一个，关注提醒都依然是死的：
    ///
    /// 1. `mark_input` **在生产代码里从未被调用**——检测器是泵任务里的局部变量，写回路径
    ///    够不到它。而 `inspect_output` 第一件事就是 `if !self.turn_active { return None }`，
    ///    于是输出侧检测每次都在第一行返回。
    /// 2. `terminal:status` 事件上的 `terminalId` 是**空串**——检测器 `publish` 时填的
    ///    `String::new()`，前端按 `terminalId` 分发，状态永远落到一个不存在的键上。
    ///
    /// 这里用一个**不存在的会话**：注册表写回必然失败，但 `mark_input` 发生在写回**之前**，
    /// 所以它照样得跑完。这既证明接线存在，也顺带把顺序钉住——顺序反了会丢掉程序对本次
    /// 输入产生的第一批输出，而那一批常常正好带着确认请求。
    #[tokio::test]
    async fn writing_input_marks_the_shared_detector_and_labels_the_event() {
        let recording = Arc::new(Recording::default());
        let commands = commands(Arc::clone(&recording));
        let detector: DetectorHandle =
            Arc::new(Mutex::new(TerminalStatusDetector::new(ProviderId::Codex)));
        commands
            .detector_slot()
            .lock()
            .expect("未中毒")
            .insert("t9".to_owned(), Arc::clone(&detector));

        let error = commands
            .write(&json!({ "terminalId": "t9", "data": "hello\r" }))
            .await
            .expect_err("会话不存在，写回必须失败");
        assert!(error.to_string().contains("终端会话不存在"), "{error}");

        assert!(
            detector.lock().expect("未中毒").is_turn_active(),
            "写回必须把检测器置为运行中，否则输出侧检测永远不会开始"
        );

        let statuses = recording.payloads(CHANNEL_STATUS);
        assert_eq!(statuses.len(), 1, "应当恰好发出一条状态事件：{statuses:?}");
        assert_eq!(statuses[0]["status"], "running");
        assert_eq!(
            statuses[0]["terminalId"], "t9",
            "事件必须带上真实的 terminalId，否则前端无法把它归到任何一路终端"
        );
    }

    #[tokio::test]
    async fn stopping_an_unknown_session_reports_not_stopped() {
        // **幂等而不是报错**：界面可能重复收到关闭请求（标签关闭 + 窗口关闭），
        // 第二次停止不该弹错误。这与 `SessionRegistry` 的契约一致。
        let recording = Arc::new(Recording::default());
        let commands = commands(recording);
        let value = commands
            .stop(&json!({ "terminalId": "missing" }))
            .await
            .expect("不该报错");
        assert_eq!(value["stopped"], false);
    }

    #[tokio::test]
    async fn writing_to_an_unknown_session_is_rejected() {
        let recording = Arc::new(Recording::default());
        let commands = commands(recording);
        let error = commands
            .write(&json!({ "terminalId": "missing", "data": "x" }))
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("终端会话不存在"), "{error}");
    }

    #[tokio::test]
    async fn the_pump_forwards_output_and_reports_exit() {
        // 直接驱动泵，而不是通过 `start`——后者会真的启动 `codex` CLI：
        // 依赖机器上装了什么、且是交互式的，不适合作为单元测试。
        //
        // 用一条确定的命令（`echo`）验证「PTY 输出 → 事件」与「通道关闭 → 退出事件」。
        let recording = Arc::new(Recording::default());
        let emitter: Arc<dyn EventEmitter> = recording.clone();

        let command = if cfg!(windows) {
            TerminalCommand {
                file: "cmd.exe".to_owned(),
                args: vec![
                    "/d".to_owned(),
                    "/s".to_owned(),
                    "/c".to_owned(),
                    "echo aivc-pump-probe".to_owned(),
                ],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: std::collections::BTreeMap::new(),
                remove_env: std::collections::BTreeSet::new(),
            }
        } else {
            TerminalCommand {
                file: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), "echo aivc-pump-probe".to_owned()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: std::collections::BTreeMap::new(),
                remove_env: std::collections::BTreeSet::new(),
            }
        };
        let (mut session, output) =
            PtySession::spawn(&command, DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        let pump = tokio::spawn(pump_output(
            // 泵现在收的是**槽位**；测试里放一个已经装好发射器的槽位即可。
            Arc::new(std::sync::RwLock::new(Arc::clone(&emitter))),
            Arc::new(SessionRegistry::new()),
            Arc::new(Mutex::new(HashMap::new())),
            "t1".to_owned(),
            Arc::new(Mutex::new(TerminalStatusDetector::new(ProviderId::Codex))),
            output,
        ));

        // 边等边应答 DSR：ConPTY 不拿到应答就不推进会话。
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        let mut answered = false;
        while std::time::Instant::now() < deadline {
            let mut collected = Vec::new();
            for payload in recording.payloads(CHANNEL_DATA) {
                if let Some(bytes) = payload.get("data").and_then(Value::as_array) {
                    collected.extend(bytes.iter().filter_map(Value::as_u64).map(|v| v as u8));
                }
            }
            if !answered && collected.windows(4).any(|w| w == b"\x1b[6n") {
                answered = true;
                session.write("\u{1b}[1;1R").expect("应答 DSR");
            }
            if String::from_utf8_lossy(&collected).contains("aivc-pump-probe") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        // 会话结束后通道关闭，泵发退出事件。
        drop(session);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), pump).await;

        let mut collected = Vec::new();
        for payload in recording.payloads(CHANNEL_DATA) {
            if let Some(bytes) = payload.get("data").and_then(Value::as_array) {
                collected.extend(bytes.iter().filter_map(Value::as_u64).map(|v| v as u8));
            }
        }
        assert!(
            String::from_utf8_lossy(&collected).contains("aivc-pump-probe"),
            "输出应当被推成事件：{:?}",
            String::from_utf8_lossy(&collected)
        );
        // 每条数据事件都必须带会话编号，否则前端无法路由到对应标签。
        for payload in recording.payloads(CHANNEL_DATA) {
            assert_eq!(payload["terminalId"], "t1");
        }
    }
    /// **回归测试：子进程自然退出必须一路走到前端。**
    ///
    /// 此前**不会**。`ConPTY` 下子进程退出**不会**让读取线程收到 EOF——只有关闭 PTY 主句柄
    /// 才会，而主句柄在注册表手里。于是用户敲 `exit`、或 CLI 自己崩掉之后，输出通道一直
    /// 不关，`terminal:exit` 永不触发：**界面上的终端标签一直显示运行中**，退出码也没有
    /// 任何人拿得到。
    ///
    /// 这里**不 drop、不 stop、不 kill**，全指望收养任务发现子进程已退出、先记下退出码、
    /// 再释放 PTY。这是「自然退出」这条路径唯一的自动化证据。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_natural_exit_reaches_the_frontend_with_its_code() {
        let recording = Arc::new(Recording::default());
        let emitter: Arc<dyn EventEmitter> = recording.clone();
        let registry = Arc::new(SessionRegistry::new());

        let command = if cfg!(windows) {
            TerminalCommand {
                file: "cmd.exe".to_owned(),
                args: vec![
                    "/d".to_owned(),
                    "/s".to_owned(),
                    "/c".to_owned(),
                    "echo aivc-reap-probe".to_owned(),
                ],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: std::collections::BTreeMap::new(),
                remove_env: std::collections::BTreeSet::new(),
            }
        } else {
            TerminalCommand {
                file: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), "echo aivc-reap-probe".to_owned()],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: std::collections::BTreeMap::new(),
                remove_env: std::collections::BTreeSet::new(),
            }
        };
        let (session, output) =
            PtySession::spawn(&command, DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        registry.insert("t1".to_owned(), session).await;

        // 没有终端模拟器，得有人替 xterm.js 回答 ConPTY 的光标位置查询（DSR）——
        // 不回答的话 ConPTY 不推进会话，子进程也不会结束。
        let answering = {
            let registry = Arc::clone(&registry);
            let recording = Arc::clone(&recording);
            tokio::spawn(async move {
                for _ in 0..100 {
                    let seen = recording.payloads(CHANNEL_DATA).iter().any(|payload| {
                        payload
                            .get("data")
                            .and_then(Value::as_array)
                            .is_some_and(|bytes| {
                                let raw: Vec<u8> = bytes
                                    .iter()
                                    .filter_map(Value::as_u64)
                                    .map(|v| v as u8)
                                    .collect();
                                raw.windows(4).any(|w| w == b"\x1b[6n")
                            })
                    });
                    if seen {
                        let _ = registry.write("t1", "\u{1b}[1;1R").await;
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            })
        };

        let pump = tokio::spawn(pump_output(
            Arc::new(std::sync::RwLock::new(Arc::clone(&emitter))),
            Arc::clone(&registry),
            Arc::new(Mutex::new(HashMap::new())),
            "t1".to_owned(),
            Arc::new(Mutex::new(TerminalStatusDetector::new(ProviderId::Codex))),
            output,
        ));

        let settled = tokio::time::timeout(std::time::Duration::from_secs(20), pump).await;
        answering.abort();
        assert!(
            settled.is_ok(),
            "自然退出必须在超时前让泵结束；否则标签会永远显示运行中。已收到的输出：{:?}",
            recording.payloads(CHANNEL_DATA)
        );

        let exits = recording.payloads(CHANNEL_EXIT);
        assert_eq!(exits.len(), 1, "应当恰好一条退出事件：{exits:?}");
        assert_eq!(exits[0]["terminalId"], "t1");
        assert_eq!(
            exits[0]["exitCode"], 0,
            "`cmd /c echo` 正常结束，退出码应当是 0：{exits:?}"
        );
    }

    #[tokio::test]
    async fn resizing_an_unknown_session_is_rejected() {
        let recording = Arc::new(Recording::default());
        let commands = commands(recording);
        let error = commands
            .resize(&json!({ "terminalId": "missing", "cols": 80, "rows": 24 }))
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("终端会话不存在"), "{error}");
    }
}
