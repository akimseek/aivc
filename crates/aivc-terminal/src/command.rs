//! 终端命令构造。
//!
//! # 这个模块唯一的难点
//!
//! 每个平台的 CLI 都支持「走本地网关」：把它的上游地址指向网关，凭据换成网关的路由令牌。
//! 各平台的约定完全不同，而且**写错了不会报错**——CLI 会安静地直连官方 API，用户看到的是
//! 「走网关没生效，且在花自己的额度」。因此这里的每一项都有测试钉死：
//!
//! | 平台 | 上游地址变量 | 凭据变量 | 传参方式 |
//! | --- | --- | --- | --- |
//! | Codex | `-c model_providers.…base_url` | `OPENAI_API_KEY` | 命令行配置 |
//! | Gemini | `GOOGLE_GEMINI_BASE_URL` | `GEMINI_API_KEY` | 环境变量 |
//! | Claude | `ANTHROPIC_BASE_URL` | `ANTHROPIC_AUTH_TOKEN` | 环境变量 |
//! | Qoder | —— | —— | 不支持网关 |
//!
//! # Codex 为什么要额外清掉两个变量
//!
//! Codex 会优先读 `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`。它们存在时，即使
//! `OPENAI_API_KEY` 已指向网关，Codex 仍会拿旧凭据直连官方 API。因此启用网关时
//! 必须**先 unset** 再 export。
//!
//! 「先 unset」在本机路径上不是一句可以省略的礼貌话：[`TerminalCommand`] 的
//! `env` 是**追加**语义，PTY 默认继承宿主进程的整个环境。只往 `env` 里塞
//! `OPENAI_API_KEY` 而不管被遮蔽的那两个变量，结果是**网关完全没生效、请求却记在
//! 用户自己的额度上**——没有任何报错。[`TerminalCommand::remove_env`] 就是为这件事
//! 存在的。
//!
//! # 唯一的入口是 [`build_command`]
//!
//! 三个 `build_*_command` 函数只负责「本机怎么拼」。**哪个平台 × 哪种目标用哪一个**
//! 收敛在 [`build_command`] 一处：这样新增一个平台时，编译器会在那一个 `match` 上报错，
//! 而不是让新平台静默套用别人的命令行。

use std::collections::{BTreeMap, BTreeSet};

use aivc_core::error::Result;
use aivc_core::shell::{build_posix_shell_command, parse_cli_args, posix_shell_quote};
use aivc_core::target::ProviderId;

/// Codex 走网关时注册的供应商名。
///
/// 名字固定为 `akim_gateway`：Codex 的 `-c` 配置里多处引用它，改名要同步改六处。
pub const CODEX_ROUTE_PROVIDER: &str = "akim_gateway";

/// 没有指定工作目录时的默认目录名。
pub const DEFAULT_WORK_DIR_NAME: &str = ".akim";

/// Codex 会**优先读取**、从而遮蔽 `OPENAI_API_KEY` 的旧凭据变量。
///
/// 见模块文档：只要它们还在环境里，Codex 就会拿旧凭据直连官方 API，
/// 而 `OPENAI_API_KEY` 指向的网关被完全绕过。
pub const CODEX_SHADOWED_CREDENTIALS: [&str; 2] = ["CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];

/// 一条可执行的终端命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalCommand {
    /// 可执行文件。
    pub file: String,
    /// 参数。
    pub args: Vec<String>,
    /// 工作目录。
    pub cwd: String,
    /// 追加的环境变量（按名称排序，便于测试断言）。
    pub env: BTreeMap<String, String>,
    /// 必须从子进程环境里**删除**的变量。
    ///
    /// [# 为什么「设置」不够]
    ///
    /// PTY 默认继承宿主进程的环境，而 `env` 只能覆盖、不能抹掉。Codex 优先读
    /// `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`，因此只设置 `OPENAI_API_KEY` 时，
    /// 宿主机上一旦存在那两者，**网关就被静默绕过**。
    pub remove_env: BTreeSet<String>,
}

impl TerminalCommand {
    /// 构造一条命令（无环境变量改动）。
    #[must_use]
    fn plain(file: impl Into<String>, args: Vec<String>, cwd: String) -> Self {
        Self {
            file: file.into(),
            args,
            cwd,
            env: BTreeMap::new(),
            remove_env: BTreeSet::new(),
        }
    }
}

/// 已解析好上游地址的路由。
///
/// WSL 目标下 `127.0.0.1` 指向 WSL 自身而非宿主，因此地址必须由调用方先探测再传入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRoute {
    /// 网关路由地址。
    pub base_url: String,
    /// 本地路由令牌。
    pub local_token: String,
}

/// 一次终端启动的**目标形态**：在本机跑，还是在某个 WSL 发行版里跑。
///
/// 这两条路径的差别不只是可执行文件名（`codex` vs `wsl.exe`），还包括
/// **环境变量的传递方式**：本机走 PTY 的 `env`，WSL 走 shell 脚本里的
/// `export`——`wsl.exe` 不会把宿主的环境变量带进发行版。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchTarget {
    /// 宿主是否是 Windows（决定本机路径用 `cmd.exe` 还是 POSIX 引号规则）。
    pub windows: bool,
    /// 目标发行版；`None` 或空白表示本机。
    pub distro: Option<String>,
}

impl LaunchTarget {
    /// 本机目标。
    #[must_use]
    pub const fn local(windows: bool) -> Self {
        Self {
            windows,
            distro: None,
        }
    }

    /// WSL 目标。
    #[must_use]
    pub fn wsl(distro: impl Into<String>) -> Self {
        Self {
            windows: true,
            distro: Some(distro.into()),
        }
    }

    /// 目标发行版（已去掉空白；空串视为「本机」）。
    #[must_use]
    pub fn distro(&self) -> Option<&str> {
        self.distro
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

/// 各平台 CLI 的可执行文件名。
///
/// Qoder 是 `qodercn`（国内发行版的命令名），不是 `qoder`——写错的表现是
/// 「终端一闪而过」，因为 shell 找不到这个命令。
#[must_use]
pub const fn cli_program(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "codex",
        ProviderId::Gemini => "gemini",
        ProviderId::Claude => "claude",
        ProviderId::Qoder => "qodercn",
    }
}

/// 构造参数（已完成校验）。
#[derive(Debug, Clone, Default)]
pub struct CommandParams {
    /// 已有会话编号；提供时走「恢复会话」而不是开新会话。
    pub session_id: Option<String>,
    /// 工作目录。
    pub cwd: Option<String>,
    /// Codex 配置目录覆盖。
    pub codex_home: Option<String>,
    /// 附加 CLI 参数（原始字符串，需要解析）。
    pub cli_args: Option<String>,
    /// 是否用 CLI 自己的 `-C` 参数指定目录（而不是先 cd）。
    pub use_cwd_flag: bool,
    /// 主目录（用于默认工作目录）。
    pub home: String,
}

/// 解析附加参数。
///
/// # Errors
///
/// 引号不配对时返回错误。
pub fn extra_args(params: &CommandParams) -> Result<Vec<String>> {
    parse_cli_args(params.cli_args.as_deref().unwrap_or_default())
}

/// 默认工作目录。
#[must_use]
pub fn default_work_dir(home: &str) -> String {
    join_path(home, DEFAULT_WORK_DIR_NAME)
}

/// 拼接路径（用 `/`，与各平台 CLI 的约定一致）。
#[must_use]
pub fn join_path(base: &str, name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), name)
}

/// 本机终端的工作目录。
///
/// Windows 上先把 WSL 挂载路径换算成 Windows 路径，理由见 [`windows_local_cwd`]。
#[must_use]
fn local_cwd(params: &CommandParams, windows: bool) -> String {
    let cwd = params
        .cwd
        .clone()
        .unwrap_or_else(|| default_work_dir(&params.home));
    if windows {
        windows_local_cwd(&cwd, &params.home)
    } else {
        cwd
    }
}

/// Windows 本机终端的**工作目录**：把 WSL 挂载路径换成 Windows 路径。
///
/// # 为什么必须换
///
/// 界面把「当前会话的目录」直接当作终端工作目录传下来
/// （`openTerminal(selectedTarget, selectedSession?.cwd)`）。而一个 **WSL 会话**的目录是
/// `/mnt/d/code/ai` 这样的 POSIX 路径。如果用户此时选中的是**本机目标**，本机终端就会
/// 拿到这个路径，然后发生两件事：
///
/// 1. [`aivc_terminal::session::PtySession::spawn`] 启动前会先建目录，于是在当前盘符下
///    **凭空多出一个** `D:\mnt\d\code\ai`；
/// 2. 终端开在那里，而用户要的是自己的项目目录。
///
/// 换算结果不是 Windows 绝对路径时**退回主目录**——不猜。这与现网的 `toWindowsShellCwd`
/// 一致（那边是退回 `os.homedir()`）。
///
/// # 与现网的一处有意差别
///
/// 现网只对 Codex 与 Qoder 做了这一步，Gemini / Claude 没有。那两处是**遗漏**而不是
/// 有意的差别：同样的输入在它们那里会照样创建那个多余的目录树。因此这里对本机
/// Windows 的三个构造器**统一**处理。
#[must_use]
pub fn windows_local_cwd(cwd: &str, home: &str) -> String {
    let converted = aivc_core::path::wsl_mount_path_to_windows_path(cwd);
    if is_windows_absolute(&converted) {
        converted
    } else {
        home.to_owned()
    }
}

/// 是否是 Windows 绝对路径：`C:\…`、`C:/…` 或 UNC 的 `\\server\share`。
#[must_use]
fn is_windows_absolute(path: &str) -> bool {
    match path.as_bytes() {
        [drive, b':', separator, ..] => {
            drive.is_ascii_alphabetic() && matches!(separator, b'\\' | b'/')
        }
        [b'\\', b'\\', ..] => true,
        _ => false,
    }
}

/// Windows 本机命令：`cmd.exe /d /s /c <程序> <参数…>`。
///
/// # 为什么不自己拼一条 `cmd.exe` 命令行
///
/// 这里曾经用 `build_cmd_command` 把整条命令行拼成一个字符串，再把它当作**一个参数**
/// 交给 `portable_pty`。那是错的，而且是**静默**错的：本机 Windows + 走网关的 Codex
/// 因此从来没起来过，报 `unexpected argument '"model_provider=^"akim_gateway""'`。
///
/// 原因是那条链上有**三个**解析器，而它们的引号规则互不兼容：
///
/// | 层 | 规则 |
/// | --- | --- |
/// | 我们（`cmd_quote`） | `"` `^` `&` `\|` `<` `>` `%` 前加 `^` |
/// | `portable_pty` 的 `append_quoted` | MSVC/CRT：含空格或 `"` 就整体加 `"`、内部 `"` 变 `\"` |
/// | `cmd.exe` | 不把 `\` 当转义 |
///
/// 更要紧的是第四层：`codex.cmd` / `qodercn.cmd` 是 npm shim，会把 `%*` **原样转发**给
/// `node`，由 **node 的 CRT** 再解析一次。也就是说参数最终由 **CRT 规则**决定，
/// 而那正是 `portable_pty` 已经替我们做的事——我们不该在它之前再插一层。
///
/// 因此正确做法是：**把程序与每个参数原样分开交给 `portable_pty`**，让它按 CRT 规则
/// 引号化。实验见 `aivc-ipc/tests/cmd_quoting_probe.rs`（它测的就是「子进程实际收到
/// 什么」，而不是我们对 cmd 行为的推测）。
fn windows_command(
    program: &str,
    args: Vec<String>,
    cwd: String,
    env: BTreeMap<String, String>,
    remove_env: BTreeSet<String>,
) -> TerminalCommand {
    let mut full = Vec::with_capacity(args.len() + 4);
    full.push("/d".to_owned());
    full.push("/s".to_owned());
    full.push("/c".to_owned());
    // 这一项没有空格、也没有引号，因此 `portable_pty` 会原样放过它。
    full.push(program.to_owned());
    full.extend(args);
    TerminalCommand {
        file: "cmd.exe".to_owned(),
        args: full,
        cwd,
        env,
        remove_env,
    }
}

/// 恢复会话的参数。
#[must_use]
pub fn resume_args(session_id: Option<&str>) -> Vec<String> {
    session_id.map_or_else(Vec::new, |id| vec!["--resume".to_owned(), id.to_owned()])
}

/// Codex 走网关时注入的配置参数。
///
/// 六项缺一不可：`wire_api` 决定请求体形状，`requires_openai_auth` 与 `env_key` 决定
/// Codex 去哪儿取凭据，`base_url` 才是真正的上游地址。
///
/// # `requires_openai_auth` 是**布尔**，不能引号化
///
/// 它和另外五项不一样：那些是字符串，这一项是布尔。写成 `"true"`（带引号）时 Codex
/// 会**直接拒绝启动**：
///
/// ```text
/// Error: invalid type: string "true", expected a boolean
/// in `model_providers.akim_gateway.requires_openai_auth`
/// ```
///
/// 这一条实测踩到过：移植时看着「值一律引号化」整齐，就把六项统一引号化了，
/// **而当时的单元测试还断言了那个错误形状**——测试全绿，Codex 一次都没起来过。
/// 它是靠真实运行一个 WSL 终端才发现的（终端里打出了上面那句 Codex 自己的报错）。
#[must_use]
pub fn codex_route_config_args(route: Option<&ResolvedRoute>) -> Vec<String> {
    let Some(route) = route else {
        return Vec::new();
    };
    let quoted =
        |value: &str| serde_json::to_string(value).unwrap_or_else(|_| format!("\"{value}\""));
    let mut args = Vec::with_capacity(12);
    // 第三项 `false` = 这个值**不是字符串**，要原样写出去。
    for (key, value, is_text) in [
        ("model_provider", CODEX_ROUTE_PROVIDER.to_owned(), true),
        (
            &format!("model_providers.{CODEX_ROUTE_PROVIDER}.name"),
            CODEX_ROUTE_PROVIDER.to_owned(),
            true,
        ),
        (
            &format!("model_providers.{CODEX_ROUTE_PROVIDER}.wire_api"),
            "responses".to_owned(),
            true,
        ),
        (
            &format!("model_providers.{CODEX_ROUTE_PROVIDER}.requires_openai_auth"),
            "true".to_owned(),
            false,
        ),
        (
            &format!("model_providers.{CODEX_ROUTE_PROVIDER}.env_key"),
            "OPENAI_API_KEY".to_owned(),
            true,
        ),
        (
            &format!("model_providers.{CODEX_ROUTE_PROVIDER}.base_url"),
            route.base_url.clone(),
            true,
        ),
    ] {
        args.push("-c".to_owned());
        // 字符串一律 JSON 引号化：地址里可能有 `:` 与 `/`，Codex 的 TOML 风格解析器
        // 会把未引号化的值截断。布尔则必须原样。
        args.push(if is_text {
            format!("{key}={}", quoted(&value))
        } else {
            format!("{key}={value}")
        });
    }
    args
}

/// Codex 的完整参数（路由配置在前，用户参数在后）。
#[must_use]
pub fn codex_args(args: &[String], route: Option<&ResolvedRoute>) -> Vec<String> {
    let mut all = codex_route_config_args(route);
    all.extend_from_slice(args);
    all
}

/// Codex 的环境变量。
///
/// 见模块文档：走网关时必须清掉两个旧凭据变量。
#[must_use]
pub fn codex_environment(
    route: Option<&ResolvedRoute>,
    codex_home: Option<&str>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(route) = route {
        env.insert("OPENAI_API_KEY".to_owned(), route.local_token.clone());
    }
    if let Some(home) = codex_home.map(str::trim).filter(|value| !value.is_empty()) {
        env.insert("CODEX_HOME".to_owned(), home.to_owned());
    }
    env
}

/// Codex 命令的 POSIX 形式。
#[must_use]
pub fn codex_invocation(args: &[String], route: Option<&ResolvedRoute>) -> String {
    build_posix_shell_command("codex", &codex_args(args, route))
}

/// Codex 走网关时**必须删除**的旧凭据变量。
///
/// 与 [`codex_environment`] 分开是因为语义不同：那是「设置」，这是「删除」。
/// 两者不能互相替代——只设置 `OPENAI_API_KEY` 时，宿主机上存在的
/// `CODEX_API_KEY` 依然优先，网关被静默绕过。见模块文档。
///
/// 没有路由时返回空集：此时应当**保留**用户自己的登录凭据。
#[must_use]
pub fn codex_env_removals(route: Option<&ResolvedRoute>) -> BTreeSet<String> {
    if route.is_none() {
        return BTreeSet::new();
    }
    CODEX_SHADOWED_CREDENTIALS
        .iter()
        .map(|name| (*name).to_owned())
        .collect()
}

/// 构造一条带环境变量的 POSIX 调用。
///
/// 有路由时用 `env KEY=VALUE … command args` 而不是 `KEY=VALUE command`：后者在
/// `exec` 之后依然有效，但写成 `env` 前缀能让整条命令在任何 shell 里都能独立执行。
#[must_use]
pub fn gateway_invocation(
    command: &str,
    args: &[String],
    route: Option<&ResolvedRoute>,
    environment: &BTreeMap<String, String>,
) -> String {
    if route.is_none() {
        return build_posix_shell_command(command, args);
    }
    let mut prefixed: Vec<String> = environment
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect();
    prefixed.push(command.to_owned());
    prefixed.extend_from_slice(args);
    build_posix_shell_command("env", &prefixed)
}

/// 各平台走网关时的环境变量。
#[must_use]
pub fn provider_environment(
    provider: ProviderId,
    route: Option<&ResolvedRoute>,
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    let Some(route) = route else {
        return env;
    };
    let (base_key, token_key) = match provider {
        ProviderId::Gemini => ("GOOGLE_GEMINI_BASE_URL", "GEMINI_API_KEY"),
        ProviderId::Claude => ("ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"),
        // Codex 走命令行配置而不是环境变量，见 `codex_route_config_args`。
        // Qoder 不支持网关。
        ProviderId::Codex | ProviderId::Qoder => return env,
    };
    env.insert(base_key.to_owned(), route.base_url.clone());
    env.insert(token_key.to_owned(), route.local_token.clone());
    env
}

/// 平台是否支持走本地网关。
#[must_use]
pub fn supports_gateway(provider: ProviderId) -> bool {
    matches!(
        provider,
        ProviderId::Codex | ProviderId::Gemini | ProviderId::Claude
    )
}

/// 本地（非 WSL）Codex 命令。
///
/// `windows` 为真时用 `cmd.exe` 的参数引号规则，否则用 POSIX 规则。
#[must_use]
pub fn build_codex_command(
    params: &CommandParams,
    route: Option<&ResolvedRoute>,
    extra: &[String],
    windows: bool,
) -> TerminalCommand {
    let cwd = local_cwd(params, windows);
    let args = codex_args_for(params, extra, &cwd);
    let executable = if windows { "codex.cmd" } else { "codex" };
    let full_args = codex_args(&args, route);
    let env = codex_environment(route, params.codex_home.as_deref());
    // 见模块文档：不删这两个变量，网关就会被静默绕过。
    let remove_env = codex_env_removals(route);
    if windows {
        return windows_command(executable, full_args, cwd, env, remove_env);
    }
    TerminalCommand {
        file: executable.to_owned(),
        args: full_args,
        cwd,
        env,
        remove_env,
    }
}

/// 本地（非 WSL）Gemini / Claude 命令。
#[must_use]
pub fn build_gateway_cli_command(
    program: &str,
    provider: ProviderId,
    params: &CommandParams,
    route: Option<&ResolvedRoute>,
    extra: &[String],
    windows: bool,
) -> TerminalCommand {
    let cwd = local_cwd(params, windows);
    let mut args = resume_args(params.session_id.as_deref());
    args.extend_from_slice(extra);
    let env = provider_environment(provider, route);
    if windows {
        // Windows 走 `cmd.exe /c`，环境变量由 `env` 字段传而不是塞进命令行。
        return windows_command(program, args, cwd, env, BTreeSet::new());
    }
    TerminalCommand {
        file: std::env::var("SHELL").unwrap_or_else(|_| "bash".to_owned()),
        args: vec![
            "-ic".to_owned(),
            format!("exec {}", gateway_invocation(program, &args, route, &env)),
        ],
        cwd,
        // 环境变量已经写进命令行（`env K=V …`），不再重复传一遍。
        env: BTreeMap::new(),
        remove_env: BTreeSet::new(),
    }
}

/// 本地（非 WSL）Qoder 命令。
///
/// Qoder 不支持网关，因此永远不带路由参数。
#[must_use]
pub fn build_qoder_command(
    params: &CommandParams,
    extra: &[String],
    windows: bool,
) -> TerminalCommand {
    let cwd = local_cwd(params, windows);
    let args = qoder_args(params, extra);
    if windows {
        return windows_command(
            cli_program(ProviderId::Qoder),
            args,
            cwd,
            BTreeMap::new(),
            BTreeSet::new(),
        );
    }
    TerminalCommand {
        file: std::env::var("SHELL").unwrap_or_else(|_| "bash".to_owned()),
        args: vec![
            "-ic".to_owned(),
            format!(
                "exec {}",
                build_posix_shell_command(cli_program(ProviderId::Qoder), &args)
            ),
        ],
        cwd,
        env: BTreeMap::new(),
        remove_env: BTreeSet::new(),
    }
}

/// Qoder 的参数（恢复会话 → `--cwd` → 用户参数）。
fn qoder_args(params: &CommandParams, extra: &[String]) -> Vec<String> {
    let mut args = resume_args(params.session_id.as_deref());
    if let Some(dir) = params.cwd.as_deref().filter(|_| params.use_cwd_flag) {
        args.push("--cwd".to_owned());
        args.push(dir.to_owned());
    }
    args.extend_from_slice(extra);
    args
}

/// Codex 的参数：内嵌终端使用 inline 模式，保留 xterm 回滚历史。
fn codex_args_for(params: &CommandParams, extra: &[String], cwd: &str) -> Vec<String> {
    let mut args = match params.session_id.as_deref() {
        // 「恢复会话」固定带 `--all`：它让 Codex 在全部历史里找会话，而不只是当前目录。
        Some(id) => vec!["resume".to_owned(), "--all".to_owned(), id.to_owned()],
        None if params.use_cwd_flag && params.cwd.is_some() => {
            let mut args = vec!["-C".to_owned(), cwd.to_owned()];
            args.extend_from_slice(extra);
            args
        }
        None => extra.to_vec(),
    };
    // Alternate screen has no terminal scrollback. Both local and WSL
    // launches use this helper, including resumed sessions.
    if !args.iter().any(|arg| arg == "--no-alt-screen") {
        args.insert(0, "--no-alt-screen".to_owned());
    }
    args
}

/// WSL 下「先建目录再进入」的前缀。
#[must_use]
pub fn wsl_cwd_prefix(cwd: Option<&str>) -> String {
    match cwd {
        Some(dir) => format!(
            "mkdir -p {quoted} && cd {quoted} && ",
            quoted = posix_shell_quote(dir)
        ),
        // 没指定目录时回到固定的默认目录，而不是留在 WSL 的当前目录。
        None => format!(
            "mkdir -p {quoted} && cd {quoted} && ",
            quoted = format!("$HOME/{DEFAULT_WORK_DIR_NAME}")
        ),
    }
}

/// WSL 脚本前缀：先清掉被遮蔽的旧凭据，再把环境变量导出到发行版里。
///
/// # 为什么不能像本机那样走 PTY 的 `env`
///
/// `wsl.exe` **不会**把宿主进程的环境变量带进发行版——它只转发脚本本身。因此
/// 「设置」与「删除」两条语义都必须写进脚本：`export` 负责设置，
/// `unset` 负责删除。少了 `unset`，发行版里已有的 `CODEX_API_KEY` 依然会让
/// Codex 绕过网关。
fn wsl_env_prefix(unset: &[&str], env: &BTreeMap<String, String>) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !unset.is_empty() {
        parts.push(format!("unset {}", unset.join(" ")));
    }
    if !env.is_empty() {
        let assignments: Vec<String> = env
            .iter()
            .map(|(key, value)| format!("{key}={}", posix_shell_quote(value)))
            .collect();
        parts.push(format!("export {}", assignments.join(" ")));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!("{}; ", parts.join("; "))
}

/// 把一个 shell 脚本包成 `wsl.exe -d <发行版> -- bash -ic <脚本>`。
fn wsl_command(distro: &str, script: String, home: &str) -> TerminalCommand {
    TerminalCommand::plain(
        "wsl.exe",
        vec![
            "-d".to_owned(),
            distro.to_owned(),
            "--".to_owned(),
            "bash".to_owned(),
            "-ic".to_owned(),
            script,
        ],
        home.to_owned(),
    )
}

/// WSL 下的 Codex 命令。
///
/// 目录处理沿用现网的四种形态，其中**恢复会话不建目录**：会话已经在磁盘上，
/// 它所属的目录必然存在，`mkdir -p` 只会掩盖「目录被删掉了」这件事。
#[must_use]
pub fn build_wsl_codex_command(
    params: &CommandParams,
    route: Option<&ResolvedRoute>,
    extra: &[String],
    distro: &str,
) -> TerminalCommand {
    let cwd = params
        .cwd
        .clone()
        .unwrap_or_else(|| default_work_dir(&params.home));
    let args = codex_args_for(params, extra, &cwd);
    let invocation = build_posix_shell_command("codex", &codex_args(&args, route));
    let script = if params.session_id.is_some() {
        match params.cwd.as_deref() {
            Some(dir) => format!("cd {} && exec {invocation}", posix_shell_quote(dir)),
            None => format!("exec {invocation}"),
        }
    } else if params.use_cwd_flag && params.cwd.is_some() {
        // 目录已由 `-C` 指定，不需要再 cd。
        format!("exec {invocation}")
    } else {
        let prefix = wsl_cwd_prefix(if params.cwd.is_some() {
            Some(cwd.as_str())
        } else {
            None
        });
        format!("{prefix}exec {invocation}")
    };
    let env = codex_environment(route, params.codex_home.as_deref());
    // 只有走网关时才 unset：不走网关时用户自己的登录凭据必须留着。
    let unset: &[&str] = if route.is_some() {
        &CODEX_SHADOWED_CREDENTIALS
    } else {
        &[]
    };
    let prefix = wsl_env_prefix(unset, &env);
    wsl_command(distro, format!("{prefix}{script}"), &params.home)
}

/// WSL 下的 Gemini / Claude 命令。
#[must_use]
pub fn build_wsl_gateway_cli_command(
    program: &str,
    provider: ProviderId,
    params: &CommandParams,
    route: Option<&ResolvedRoute>,
    extra: &[String],
    distro: &str,
) -> TerminalCommand {
    let cwd = params
        .cwd
        .clone()
        .unwrap_or_else(|| default_work_dir(&params.home));
    let mut args = resume_args(params.session_id.as_deref());
    args.extend_from_slice(extra);
    let env = provider_environment(provider, route);
    let prefix = wsl_env_prefix(&[], &env);
    let invocation = build_posix_shell_command(program, &args);
    let dir = wsl_cwd_prefix(if params.cwd.is_some() {
        Some(cwd.as_str())
    } else {
        None
    });
    wsl_command(
        distro,
        format!("{prefix}{dir}exec {invocation}"),
        &params.home,
    )
}

/// WSL 下的 Qoder 命令。
#[must_use]
pub fn build_wsl_qoder_command(
    params: &CommandParams,
    extra: &[String],
    distro: &str,
) -> TerminalCommand {
    let cwd = params
        .cwd
        .clone()
        .unwrap_or_else(|| default_work_dir(&params.home));
    let args = qoder_args(params, extra);
    let invocation = build_posix_shell_command(cli_program(ProviderId::Qoder), &args);
    // `--cwd` 已经指定了目录时不再 cd（现网语义）。
    let script = if params.use_cwd_flag && params.cwd.is_some() {
        format!("exec {invocation}")
    } else {
        let dir = wsl_cwd_prefix(if params.cwd.is_some() {
            Some(cwd.as_str())
        } else {
            None
        });
        format!("{dir}exec {invocation}")
    };
    wsl_command(distro, script, &params.home)
}

/// Claude 的路由设置：用 `--settings` 注入，而**不是只靠环境变量**。
///
/// # 为什么不只靠环境变量（实测踩到）
///
/// Claude Code **自己设置文件里的 `env` 块优先于进程环境**。用户只要在
/// `~/.claude/settings.json` 里写过 `ANTHROPIC_BASE_URL`（本机实测就是这样），
/// 我们注入的同名变量就被**完全覆盖**：
///
/// ```text
/// terminalTail: "Hi! How can I help you today?"   ← Claude 正常回答
/// requestCount: 0                                  ← 网关一条请求都没收到
/// ```
///
/// 请求去了用户配置的那个地址、用的是他自己的密钥。用户看不到任何异常，
/// **只能从账单上发现**。
///
/// `--settings` 是**命令行级别**的配置，压得住用户的设置文件
/// （`claude --help` 里写明 "managed settings and --settings still apply"）。
/// Codex 那边一直是这个道理——`-c` 命令行压得住 `config.toml`，所以它从来没有这个问题。
///
/// 环境变量**仍然照常注入**：老版本或未读 `--settings` 的场合，那条路还在。
///
/// # 代价：令牌会出现在命令行里
///
/// 这是内联 JSON 的代价——本机同用户可以看到进程参数。老平台把**同一个令牌**放在环境
/// 变量里，对同用户的可见性相当。要更严就得写临时文件并管理其生命周期，那是另一件事。
#[must_use]
pub fn claude_route_settings(route: Option<&ResolvedRoute>) -> Vec<String> {
    let Some(route) = route else {
        return Vec::new();
    };
    let settings = serde_json::json!({
        "env": {
            "ANTHROPIC_BASE_URL": route.base_url,
            "ANTHROPIC_AUTH_TOKEN": route.local_token,
        }
    });
    vec!["--settings".to_owned(), settings.to_string()]
}

/// 构造一路终端命令——**这是唯一的入口**。
///
/// # 为什么要收敛到一个函数
///
/// 「哪个平台 × 哪种目标用哪个构造器」曾经散落在调用方：结果是终端只能启动 Codex，
/// 其余平台直接报「暂不支持」；而且新增平台时编译器不会提醒任何遗漏。
///
/// 现在两个维度都在这一处穷举（`match` 没有通配分支），新增一个平台**必须**
/// 同时给出本机与 WSL 两种形态，否则编译不过。
///
/// `route` 对不支持网关的平台（Qoder）会被丢弃：调用方不需要自己判断。
#[must_use]
pub fn build_command(
    provider: ProviderId,
    params: &CommandParams,
    route: Option<&ResolvedRoute>,
    extra: &[String],
    target: &LaunchTarget,
) -> TerminalCommand {
    // Qoder 不支持网关；其余平台的支持情况见 `supports_gateway`。
    let route = route.filter(|_| supports_gateway(provider));
    // **Claude 的路由必须走命令行参数。**
    //
    // 只靠环境变量会被用户 `~/.claude/settings.json` 里的 `env` 块覆盖，
    // 而那种失败**没有任何迹象**（Claude 照常回答，只是没走网关）。
    // 放在用户参数**之前**：与 Codex 的 `-c` 同一位置约定。
    let with_route_args: Vec<String>;
    let extra: &[String] = if provider == ProviderId::Claude {
        let mut combined = claude_route_settings(route);
        combined.extend_from_slice(extra);
        with_route_args = combined;
        &with_route_args
    } else {
        extra
    };
    if let Some(distro) = target.distro() {
        return match provider {
            ProviderId::Codex => build_wsl_codex_command(params, route, extra, distro),
            ProviderId::Gemini | ProviderId::Claude => build_wsl_gateway_cli_command(
                cli_program(provider),
                provider,
                params,
                route,
                extra,
                distro,
            ),
            ProviderId::Qoder => build_wsl_qoder_command(params, extra, distro),
        };
    }
    match provider {
        ProviderId::Codex => build_codex_command(params, route, extra, target.windows),
        ProviderId::Gemini | ProviderId::Claude => build_gateway_cli_command(
            cli_program(provider),
            provider,
            params,
            route,
            extra,
            target.windows,
        ),
        ProviderId::Qoder => build_qoder_command(params, extra, target.windows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::target::ProviderId;

    fn route() -> ResolvedRoute {
        ResolvedRoute {
            base_url: "http://127.0.0.1:8787/gateway/codex/r1".to_owned(),
            local_token: "tok".to_owned(),
        }
    }

    fn params() -> CommandParams {
        CommandParams {
            home: "/home/u".to_owned(),
            ..CommandParams::default()
        }
    }

    #[test]
    fn route_config_injects_all_six_entries() {
        let args = codex_route_config_args(Some(&route()));
        assert_eq!(args.len(), 12, "六项配置，每项两个参数");
        // **字符串**值必须 JSON 引号化。
        assert!(args.contains(&"model_provider=\"akim_gateway\"".to_owned()));
        assert!(args.contains(&"model_providers.akim_gateway.wire_api=\"responses\"".to_owned()));
        assert!(
            args.contains(
                &"model_providers.akim_gateway.base_url=\"http://127.0.0.1:8787/gateway/codex/r1\""
                    .to_owned()
            )
        );
        assert!(
            args.contains(&"model_providers.akim_gateway.env_key=\"OPENAI_API_KEY\"".to_owned())
        );
    }

    /// **回归测试：`requires_openai_auth` 必须是裸布尔。**
    ///
    /// 它曾经和另外五项一起被引号化，于是 Codex 拒绝启动：
    ///
    /// ```text
    /// Error: invalid type: string "true", expected a boolean
    /// in `model_providers.akim_gateway.requires_openai_auth`
    /// ```
    ///
    /// 更糟的是**当时那条 `route_config_injects_all_six_entries` 还断言了引号形状**，
    /// 等于把缺陷锁死在测试里。因此这里单独钉一条，并把「不能有引号」写成断言。
    #[test]
    fn requires_openai_auth_is_a_bare_boolean() {
        let args = codex_route_config_args(Some(&route()));
        assert!(
            args.contains(&"model_providers.akim_gateway.requires_openai_auth=true".to_owned()),
            "必须是裸 true：{args:?}"
        );
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("requires_openai_auth=\"")),
            "引号化会让 Codex 直接拒绝启动：{args:?}"
        );
    }

    #[test]
    fn without_a_route_no_config_is_injected() {
        assert!(codex_route_config_args(None).is_empty());
        assert_eq!(
            codex_args(&["--version".to_owned()], None),
            vec!["--version"]
        );
    }

    #[test]
    fn the_route_config_comes_before_user_arguments() {
        // Codex 的 `-c` 必须在子命令与用户参数之前。
        let args = codex_args(&["resume".to_owned(), "abc".to_owned()], Some(&route()));
        let position = args
            .iter()
            .position(|item| item == "resume")
            .expect("子命令");
        assert!(position >= 12, "路由配置必须排在前面");
    }

    #[test]
    fn codex_environment_sets_the_token_and_home() {
        let env = codex_environment(Some(&route()), Some("/custom/.codex"));
        assert_eq!(env.get("OPENAI_API_KEY").map(String::as_str), Some("tok"));
        assert_eq!(
            env.get("CODEX_HOME").map(String::as_str),
            Some("/custom/.codex")
        );
        // 没有路由时不写令牌。
        let env = codex_environment(None, None);
        assert!(env.is_empty());
    }

    #[test]
    fn a_blank_codex_home_is_ignored() {
        // 设置里可能有空白串，不能把它当成一个真实目录。
        let env = codex_environment(None, Some("   "));
        assert!(!env.contains_key("CODEX_HOME"));
    }

    #[test]
    fn the_codex_invocation_uses_the_posix_shell() {
        // `posix_shell_quote` 会给以 `-` 开头的值加引号。这是有意的：文本上就明确
        // 「这是数据，不是选项」，避免值里出现特殊字符时被 shell 解释。
        let text = codex_invocation(&["--version".to_owned()], None);
        assert_eq!(text, "codex '--version'");
        let text = codex_invocation(&[], Some(&route()));
        // 所有参数都会被引号包住（含 `-c` 本身）——引号不影响 shell 语义，
        // 但让「值里出现特殊字符」不会突然改变命令结构。
        assert!(text.starts_with("codex "), "{text}");
        assert!(text.contains("model_provider="), "{text}");
        assert!(text.contains("akim_gateway"), "{text}");
    }

    #[test]
    fn gateway_invocation_prefixes_with_env() {
        let mut env = BTreeMap::new();
        env.insert("A".to_owned(), "1".to_owned());
        env.insert("B".to_owned(), "two words".to_owned());
        let text = gateway_invocation("gemini", &["--resume".to_owned()], Some(&route()), &env);
        assert!(text.starts_with("env "), "{text}");
        assert!(text.contains("A=1"), "{text}");
        // 含空格的值必须整体被引号包住，否则 env 会收到两个参数。
        assert!(text.contains("'B=two words'"), "{text}");
        assert!(text.contains("gemini"), "{text}");
        assert!(text.contains("--resume"), "{text}");
        // 环境变量按名称排序：A 在 B 之前。
        let a = text.find("A=1").expect("A");
        let b = text.find("B=").expect("B");
        assert!(a < b, "环境变量应按名称排序：{text}");
    }

    #[test]
    fn gateway_invocation_without_a_route_is_plain() {
        let text = gateway_invocation("gemini", &[], None, &BTreeMap::new());
        assert_eq!(text, "gemini");
    }

    #[test]
    fn each_provider_uses_its_own_environment_variables() {
        let route = route();
        let gemini = provider_environment(ProviderId::Gemini, Some(&route));
        assert_eq!(
            gemini.get("GOOGLE_GEMINI_BASE_URL").map(String::as_str),
            Some("http://127.0.0.1:8787/gateway/codex/r1")
        );
        assert_eq!(
            gemini.get("GEMINI_API_KEY").map(String::as_str),
            Some("tok")
        );

        let claude = provider_environment(ProviderId::Claude, Some(&route));
        assert_eq!(
            claude.get("ANTHROPIC_BASE_URL").map(String::as_str),
            Some("http://127.0.0.1:8787/gateway/codex/r1")
        );
        assert_eq!(
            claude.get("ANTHROPIC_AUTH_TOKEN").map(String::as_str),
            Some("tok")
        );
    }

    #[test]
    fn codex_and_qoder_do_not_use_environment_variables_for_routing() {
        // Codex 走命令行配置；Qoder 不支持网关。
        assert!(provider_environment(ProviderId::Codex, Some(&route())).is_empty());
        assert!(provider_environment(ProviderId::Qoder, Some(&route())).is_empty());
    }

    #[test]
    fn gateway_support_is_declared_per_provider() {
        assert!(supports_gateway(ProviderId::Codex));
        assert!(supports_gateway(ProviderId::Gemini));
        assert!(supports_gateway(ProviderId::Claude));
        assert!(!supports_gateway(ProviderId::Qoder));
    }

    #[test]
    fn resuming_a_session_uses_the_all_flag() {
        // 「恢复会话」固定带 `--all`：它让 Codex 在全部历史里找会话，而不只是当前目录。
        let mut with_session = params();
        with_session.session_id = Some("s1".to_owned());
        let command = build_codex_command(&with_session, Some(&route()), &[], false);
        let tail = &command.args[command.args.len() - 3..];
        assert_eq!(tail, ["resume", "--all", "s1"]);
    }

    #[test]
    fn codex_uses_the_cwd_flag_when_asked() {
        let mut with_cwd = params();
        with_cwd.cwd = Some("/work".to_owned());
        with_cwd.use_cwd_flag = true;
        let command = build_codex_command(&with_cwd, None, &["--flag".to_owned()], false);
        assert_eq!(command.args, ["--no-alt-screen", "-C", "/work", "--flag"]);
        assert_eq!(command.cwd, "/work");
    }

    #[test]
    fn the_default_working_directory_is_used_when_none_is_given() {
        let command = build_codex_command(&params(), None, &[], false);
        assert_eq!(command.cwd, "/home/u/.akim");
    }

    /// **回归测试：Claude 的路由必须走 `--settings`，不能只靠环境变量。**
    ///
    /// Claude Code 自己设置文件里的 `env` 块优先于进程环境，因此只注入环境变量时，
    /// 用户只要在 `~/.claude/settings.json` 里写过 `ANTHROPIC_BASE_URL`，
    /// 我们那一份就被完全覆盖——而**没有任何迹象**。
    ///
    /// 这里断言三件事：`--settings` 在；它的 JSON 能被解析回我们要的两个值；
    /// 它排在用户参数**之前**。
    #[test]
    fn claude_gets_the_route_through_settings_not_only_the_environment() {
        let extra = vec!["-p".to_owned(), "hi".to_owned()];
        // 用 **Windows 形态**取参数：那边程序与参数是独立元素，JSON 可以直接解析出来。
        // POSIX / WSL 形态把整条命令嵌在一个 shell 字符串里，见下一条测试。
        let command = build_command(
            ProviderId::Claude,
            &params(),
            Some(&route()),
            &extra,
            &LaunchTarget::local(true),
        );

        let position = command
            .args
            .iter()
            .position(|argument| argument == "--settings")
            .expect("必须带 --settings");
        let payload = &command.args[position + 1];
        let parsed: serde_json::Value = serde_json::from_str(payload).expect("应当是 JSON");
        assert_eq!(parsed["env"]["ANTHROPIC_BASE_URL"], route().base_url);
        assert_eq!(parsed["env"]["ANTHROPIC_AUTH_TOKEN"], route().local_token);

        // 必须排在用户参数之前（与 Codex 的 `-c` 同一约定）。
        let user = command
            .args
            .iter()
            .position(|argument| argument == "-p")
            .expect("用户参数");
        assert!(position < user, "{:?}", command.args);

        // 它是**一个**参数，不能被空格拆开——JSON 里没有空格才谈得上安全。
        assert!(!payload.contains(' '), "内联 JSON 不该带空格：{payload}");
    }

    #[test]
    fn claude_gets_the_settings_payload_in_a_posix_shell_too() {
        // POSIX 形态把整条命令嵌进 `-ic` 的脚本串，因此只能断言它确实在里面。
        let command = build_command(
            ProviderId::Claude,
            &params(),
            Some(&route()),
            &["-p".to_owned(), "hi".to_owned()],
            &LaunchTarget::local(false),
        );
        let script = command.args.last().expect("脚本");
        assert!(script.contains("--settings"), "{script}");
        assert!(script.contains("ANTHROPIC_BASE_URL"), "{script}");
        // 单引号引号化之后，整个 JSON 是**一个** shell 参数（`'{"env":…}'`），
        // 因此里面的双引号不会破坏 shell，也不会被空格拆开。
        assert!(
            script.contains("'{\"env\":{"),
            "设置值必须被单引号整体包住：{script}"
        );
    }

    #[test]
    fn only_claude_gets_the_settings_payload() {
        for provider in [ProviderId::Codex, ProviderId::Gemini, ProviderId::Qoder] {
            let command = build_command(
                provider,
                &params(),
                Some(&route()),
                &[],
                &LaunchTarget::local(false),
            );
            assert!(
                !command.args.iter().any(|argument| argument == "--settings"),
                "{provider:?} 不该带 --settings：{:?}",
                command.args
            );
        }
    }

    #[test]
    fn claude_gets_the_settings_payload_in_wsl_too() {
        let command = build_command(
            ProviderId::Claude,
            &params(),
            Some(&route()),
            &[],
            &LaunchTarget::wsl("Ubuntu-24.04"),
        );
        // WSL 形态是 `wsl.exe -d … -- bash -ic <脚本>`，因此 `--settings` 在脚本里。
        let script = command.args.last().expect("脚本");
        assert!(script.contains("--settings"), "{script}");
        assert!(script.contains("ANTHROPIC_BASE_URL"), "{script}");
    }

    #[test]
    fn a_claude_terminal_without_a_route_gets_no_settings() {
        let command = build_command(
            ProviderId::Claude,
            &params(),
            None,
            &[],
            &LaunchTarget::local(false),
        );
        assert!(
            !command.args.iter().any(|argument| argument == "--settings"),
            "没有路由时不该注入任何设置：{:?}",
            command.args
        );
    }

    #[test]
    fn the_windows_command_goes_through_cmd() {
        let command = build_codex_command(&params(), None, &[], true);
        assert_eq!(command.file, "cmd.exe");
        assert_eq!(&command.args[..3], ["/d", "/s", "/c"]);
        assert_eq!(command.args[3], "codex.cmd");
    }

    /// **回归测试：Windows 上程序与参数必须**分开**交给 PTY，不能自己拼一条 cmd 命令行。**
    ///
    /// 这里曾经把 `build_cmd_command` 拼好的整条命令行当作**一个参数**交给
    /// `portable_pty`。于是它又按 MSVC 规则引号化了一遍：`"` 变 `\"`、而我们那层的 `^`
    /// 原样留下。`cmd.exe` 不把 `\` 当转义，Codex 最终收到
    /// `"model_provider=^"akim_gateway""` 并**拒绝启动**——本机 Windows + 走网关的 Codex
    /// 因此从来没起来过。
    ///
    /// 正确的层是最后那个解析器：`codex.cmd` 会把 `%*` 原样转给 `node`，由 node 的 CRT
    /// 解析，而那正是 `portable_pty` 已经替我们做的事。
    #[test]
    fn a_windows_terminal_leaves_quoting_to_the_pty() {
        const BASE: &str =
            "model_providers.akim_gateway.base_url=\"http://127.0.0.1:8787/gateway/codex/r1\"";

        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            let command = build_command(
                provider,
                &params(),
                Some(&route()),
                &[],
                &LaunchTarget::local(true),
            );
            assert_eq!(command.file, "cmd.exe", "{provider:?}");
            assert_eq!(&command.args[..3], ["/d", "/s", "/c"], "{provider:?}");
            // 第 4 项是可执行文件名，其后才是参数。
            assert!(!command.args[3].contains(' '), "{:?}", command.args);
            // 我们自己那一层的 `^` 转义必须**不再出现**。
            assert!(
                !command.args.iter().any(|argument| argument.contains('^')),
                "{provider:?} 的参数里出现了 cmd 层转义：{:?}",
                command.args
            );
            if provider == ProviderId::Codex {
                // 值里带着未转义的 `"`，由 PTY 按 CRT 规则处理。
                assert!(
                    command
                        .args
                        .contains(&"model_provider=\"akim_gateway\"".to_owned()),
                    "{:?}",
                    command.args
                );
                assert!(
                    command.args.contains(&BASE.to_owned()),
                    "{:?}",
                    command.args
                );
            }
        }
    }

    #[test]
    fn qoder_never_receives_gateway_configuration() {
        // Qoder 不支持网关：给它注入地址只会让它报一个难懂的错误。
        let command = build_qoder_command(&params(), &[], false);
        assert!(!command.args.join(" ").contains("GATEWAY"));
        assert!(command.env.is_empty());
    }

    #[test]
    fn qoder_uses_the_cwd_flag_only_when_asked() {
        let mut with_cwd = params();
        with_cwd.cwd = Some("/work".to_owned());
        let command = build_qoder_command(&with_cwd, &[], false);
        assert!(!command.args.join(" ").contains("--cwd"));

        with_cwd.use_cwd_flag = true;
        let command = build_qoder_command(&with_cwd, &[], false);
        let joined = command.args.join(" ");
        // 值与选项都会被引号包住，因此分别断言。
        assert!(joined.contains("--cwd"), "{joined}");
        assert!(joined.contains("/work"), "{joined}");
    }

    #[test]
    fn gemini_and_claude_share_the_gateway_shape() {
        for (program, provider, key) in [
            ("gemini", ProviderId::Gemini, "GOOGLE_GEMINI_BASE_URL"),
            ("claude", ProviderId::Claude, "ANTHROPIC_BASE_URL"),
        ] {
            let command =
                build_gateway_cli_command(program, provider, &params(), Some(&route()), &[], false);
            let joined = command.args.join(" ");
            assert!(joined.contains(key), "{program} 缺少 {key}：{joined}");
            assert!(joined.contains(program), "{program} 未出现在命令里");
        }
    }

    #[test]
    fn the_wsl_prefix_creates_then_enters_the_directory() {
        let prefix = wsl_cwd_prefix(Some("/work dir"));
        assert!(prefix.starts_with("mkdir -p "), "{prefix}");
        assert!(prefix.contains("&& cd "), "{prefix}");
        // 含空格的路径必须被引号包住，否则 cd 会收到两个参数。
        assert!(
            prefix.contains("'/work dir'") || prefix.contains("\"/work dir\""),
            "{prefix}"
        );
    }

    #[test]
    fn the_wsl_prefix_falls_back_to_the_default_directory() {
        let prefix = wsl_cwd_prefix(None);
        assert!(prefix.contains("$HOME/.akim"), "{prefix}");
    }

    #[test]
    fn extra_arguments_are_parsed_with_shell_rules() {
        let mut with_args = params();
        with_args.cli_args = Some("--model gpt-5 --verbose".to_owned());
        assert_eq!(
            extra_args(&with_args).expect("parse"),
            ["--model", "gpt-5", "--verbose"]
        );
    }

    #[test]
    fn a_single_path_argument_stays_one_argument() {
        // 未加引号的路径里的空格会被 shell 拆开——这正是解析要处理的情况。
        let mut with_args = params();
        with_args.cli_args = Some("--dir \"/my work\"".to_owned());
        assert_eq!(
            extra_args(&with_args).expect("parse"),
            ["--dir", "/my work"]
        );
    }

    // -----------------------------------------------------------------------
    // 环境变量删除
    // -----------------------------------------------------------------------

    #[test]
    fn a_codex_route_deletes_the_credentials_that_would_shadow_it() {
        // 这是「网关没生效、而且在花自己的额度」的**根因**：Codex 优先读
        // `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`，只设置 `OPENAI_API_KEY` 不够。
        let command = build_codex_command(&params(), Some(&route()), &[], false);
        assert_eq!(
            command.remove_env.iter().cloned().collect::<Vec<_>>(),
            vec!["CODEX_ACCESS_TOKEN".to_owned(), "CODEX_API_KEY".to_owned()]
        );
        assert_eq!(
            command.env.get("OPENAI_API_KEY").map(String::as_str),
            Some("tok")
        );
    }

    #[test]
    fn without_a_route_the_users_own_credentials_are_left_alone() {
        // 不走网关时删掉它们会让用户**无法登录**——那是比不生效更糟的结果。
        let command = build_codex_command(&params(), None, &[], false);
        assert!(command.remove_env.is_empty(), "{:?}", command.remove_env);
        assert!(!command.env.contains_key("OPENAI_API_KEY"));
    }

    #[test]
    fn gemini_and_claude_never_delete_anything() {
        // 它们的路由变量（`GEMINI_API_KEY` / `ANTHROPIC_AUTH_TOKEN`）没有被别的变量遮蔽。
        for provider in [ProviderId::Gemini, ProviderId::Claude] {
            let command = build_gateway_cli_command(
                cli_program(provider),
                provider,
                &params(),
                Some(&route()),
                &[],
                true,
            );
            assert!(command.remove_env.is_empty(), "{provider:?}");
        }
    }

    // -----------------------------------------------------------------------
    // WSL 形态
    // -----------------------------------------------------------------------

    #[test]
    fn a_wsl_codex_terminal_unzips_the_route_into_the_distro() {
        let command = build_wsl_codex_command(&params(), Some(&route()), &[], "Ubuntu");
        assert_eq!(command.file, "wsl.exe");
        assert_eq!(&command.args[..5], ["-d", "Ubuntu", "--", "bash", "-ic"]);
        let script = &command.args[5];
        // `wsl.exe` 不转发宿主环境，因此 unset 与 export 都必须写进脚本。
        assert!(
            script.contains("unset CODEX_API_KEY CODEX_ACCESS_TOKEN"),
            "{script}"
        );
        assert!(script.contains("export OPENAI_API_KEY="), "{script}");
        assert!(
            script.contains("model_providers.akim_gateway.base_url"),
            "{script}"
        );
        assert!(script.contains("exec codex"), "{script}");
        // 环境变量已经进了脚本，不该再走 PTY 的 env（那条路在 WSL 下是断的）。
        assert!(command.env.is_empty());
    }

    #[test]
    fn a_wsl_codex_terminal_without_a_route_keeps_the_login() {
        let command = build_wsl_codex_command(&params(), None, &[], "Ubuntu");
        let script = &command.args[5];
        assert!(!script.contains("unset"), "{script}");
        assert!(!script.contains("export"), "{script}");
        assert!(script.contains("exec codex"), "{script}");
    }

    #[test]
    fn a_wsl_gateway_cli_terminal_exports_its_own_variable_names() {
        for (program, provider, key) in [
            ("gemini", ProviderId::Gemini, "GOOGLE_GEMINI_BASE_URL"),
            ("claude", ProviderId::Claude, "ANTHROPIC_BASE_URL"),
        ] {
            let command = build_wsl_gateway_cli_command(
                program,
                provider,
                &params(),
                Some(&route()),
                &[],
                "Ubuntu",
            );
            assert_eq!(command.file, "wsl.exe");
            let script = &command.args[5];
            // 变量按名称排序导出，因此只能断言「赋值存在」，不能断言它紧跟 `export`。
            assert!(script.contains("export "), "{script}");
            assert!(script.contains(&format!("{key}=")), "{script}");
            assert!(script.contains("exec"), "{script}");
        }
    }

    #[test]
    fn a_wsl_qoder_terminal_carries_no_route_variables() {
        let command = build_wsl_qoder_command(&params(), &[], "Ubuntu");
        let script = &command.args[5];
        assert!(!script.contains("export"), "{script}");
        assert!(!script.contains("unset"), "{script}");
        assert!(script.contains("qodercn"), "{script}");
    }

    // -----------------------------------------------------------------------
    // 唯一分派入口
    // -----------------------------------------------------------------------

    #[test]
    fn every_provider_can_be_launched_on_this_machine() {
        // 这条测试之前不可能通过：终端只接了 Codex，其余平台一律报「暂不支持」。
        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            let command = build_command(
                provider,
                &params(),
                Some(&route()),
                &[],
                &LaunchTarget::local(false),
            );
            let joined = command.args.join(" ");
            assert!(
                joined.contains(cli_program(provider)),
                "{provider:?} 的命令里没有 {}：{joined}",
                cli_program(provider)
            );
        }
    }

    #[test]
    fn a_wsl_target_switches_the_executable_to_wsl() {
        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            let command = build_command(
                provider,
                &params(),
                Some(&route()),
                &[],
                &LaunchTarget::wsl("Ubuntu-24.04"),
            );
            assert_eq!(command.file, "wsl.exe", "{provider:?}");
            assert_eq!(command.args[1], "Ubuntu-24.04", "{provider:?}");
        }
    }

    #[test]
    fn a_blank_distro_is_treated_as_this_machine() {
        // 空发行版名会让 `wsl.exe -d ""` 直接报错，因此必须归一化成「本机」。
        let target = LaunchTarget::wsl("   ");
        assert_eq!(target.distro(), None);
        let command = build_command(
            ProviderId::Codex,
            &params(),
            None,
            &[],
            &LaunchTarget {
                windows: true,
                distro: Some("   ".to_owned()),
            },
        );
        assert_eq!(command.file, "cmd.exe");
    }

    // -----------------------------------------------------------------------
    // 本机 Windows 终端的工作目录
    // -----------------------------------------------------------------------

    #[test]
    fn a_wsl_mount_cwd_is_converted_for_a_local_windows_terminal() {
        // **回归测试。** 界面把「当前会话的目录」直接当终端工作目录传下来，而 WSL 会话的
        // 目录是 `/mnt/d/code/ai`。本机终端拿到它会：先被 `PtySession::spawn`
        // `create_dir_all`，于是在当前盘符下**凭空多出** `D:\mnt\d\code\ai`；
        // 然后终端开在那儿，而用户要的是自己的项目目录。
        let mut with_cwd = params();
        with_cwd.home = "C:/Users/akim".to_owned();
        with_cwd.cwd = Some("/mnt/d/code/ai".to_owned());

        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            let command = build_command(
                provider,
                &with_cwd,
                Some(&route()),
                &[],
                &LaunchTarget::local(true),
            );
            assert_eq!(
                command.cwd, "D:\\code\\ai",
                "{provider:?} 必须把挂载路径换成 Windows 路径"
            );
        }
    }

    #[test]
    fn a_posix_terminal_keeps_its_mount_path() {
        // 非 Windows 上 `/mnt/...` 是完全合法的路径，不该动它。
        let mut with_cwd = params();
        with_cwd.cwd = Some("/mnt/d/code/ai".to_owned());
        let command = build_command(
            ProviderId::Codex,
            &with_cwd,
            None,
            &[],
            &LaunchTarget::local(false),
        );
        assert_eq!(command.cwd, "/mnt/d/code/ai");
    }

    #[test]
    fn a_cwd_that_cannot_be_made_absolute_falls_back_to_home() {
        // 换不出绝对路径时**退回主目录**而不是硬跑：现网 `toWindowsShellCwd` 也是退回
        // `os.homedir()`。猜一个路径会让终端开在某个不可预期的位置。
        assert_eq!(
            windows_local_cwd("relative/dir", "C:/Users/akim"),
            "C:/Users/akim"
        );
        assert_eq!(windows_local_cwd("", "C:/Users/akim"), "C:/Users/akim");
    }

    #[test]
    fn an_already_windows_cwd_is_left_alone() {
        assert_eq!(
            windows_local_cwd("D:\\code\\ai", "C:/Users/akim"),
            "D:\\code\\ai"
        );
        assert_eq!(
            windows_local_cwd("D:/code/ai", "C:/Users/akim"),
            "D:/code/ai"
        );
        // UNC 路径也是绝对路径。
        assert_eq!(
            windows_local_cwd("\\\\server\\share", "C:/Users/akim"),
            "\\\\server\\share"
        );
    }

    #[test]
    fn qoder_drops_the_route_even_if_the_caller_passes_one() {
        // 分派入口自己过滤，调用方不需要各自记得「Qoder 不支持网关」。
        let command = build_command(
            ProviderId::Qoder,
            &params(),
            Some(&route()),
            &[],
            &LaunchTarget::local(false),
        );
        assert!(!command.args.join(" ").contains("akim_gateway"));
        assert!(command.env.is_empty());
        assert!(command.remove_env.is_empty());
    }
}
