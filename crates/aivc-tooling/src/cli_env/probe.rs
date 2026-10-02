//! CLI 环境探测：实际执行外部命令并汇总成环境快照。
//!
//! # 为什么命令执行是一个 trait
//!
//! 探测要启动 `node --version`、`where node` 等外部进程。如果测试也去启动真实进程，
//! 会有三个问题：
//!
//! 1. **结果取决于开发机上装了什么**——同一个测试在不同机器上结论不同；
//! 2. **慢且脆**——进程启动开销让单元测试变成集成测试；
//! 3. **可能挂住**——本工程早先就因为测试留下的 `conhost.exe` 残留卡死过整个会话。
//!
//! 因此 [`CommandRunner`] 把「怎么拿到命令输出」抽出去：测试注入固定输出，
//! 真实实现才去启动进程。探测**逻辑**与**进程启动**由此彻底分离。
//!
//! # 超时必须有
//!
//! 外部命令可能永远不返回（等输入、死锁、网络挂起）。真实实现用
//! 「轮询 + 到期强杀」而不是阻塞等待，并把超时如实记进快照——**超时不是「没装」**，
//! 两者的提示完全不同。
//!
//! # WSL 探测走登录 shell
//!
//! WSL 侧必须用 `bash -lc`：`nvm` / `fnm` 这类版本管理器只在**登录 shell** 里注入
//! `PATH`。用 `bash -c` 会得到一个干净的 `PATH`，于是明明装了的 Node 报「未检测到」。

use std::collections::BTreeMap;
use std::time::Duration;

use aivc_core::target::ProviderId;

use crate::cli_env::core::{
    CommandOutput, EnvironmentFacts, RECOMMENDED_NODE_MAJOR, build_environment_messages,
    cli_version_key, minimum_node_major, node_managed_by_nvm, parse_node_major,
};

/// 一次命令执行的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandRun {
    /// 标准输出。
    pub stdout: String,
    /// 标准错误。
    pub stderr: String,
    /// 是否因超时被强杀。
    pub timed_out: bool,
    /// 退出码；`None` 表示命令根本没启动（不存在）。
    ///
    /// **必须有退出码**：最初我只用「有没有输出」判断成败，结果 stderr 有内容时
    /// 被当成成功——而安装失败的诊断恰恰全在 stderr 上。
    pub exit_code: Option<i32>,
}

impl CommandRun {
    /// 构造一次成功的执行。
    #[must_use]
    pub fn ok(stdout: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: String::new(),
            timed_out: false,
            exit_code: Some(0),
        }
    }

    /// 构造一次非零退出的执行。
    #[must_use]
    pub fn failed(code: i32, stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
            timed_out: false,
            exit_code: Some(code),
        }
    }

    /// 构造一次超时。
    #[must_use]
    pub fn timeout() -> Self {
        Self {
            stdout: String::new(),
            stderr: String::new(),
            timed_out: true,
            exit_code: None,
        }
    }

    /// 输出（两路合并，供解析用）。
    #[must_use]
    pub fn output(&self) -> CommandOutput {
        CommandOutput::new(self.stdout.clone(), self.stderr.clone())
    }
}

/// 命令执行器。
pub trait CommandRunner: Send + Sync {
    /// 执行一条命令，最多等待 `timeout_ms`。
    ///
    /// 实现**不得**返回 `Err`：命令不存在、非零退出、超时都应当表达为
    /// [`CommandRun`] 的字段——探测的语义是「收集事实」而不是「传播错误」。
    fn run(&self, program: &str, args: &[&str], timeout_ms: u64) -> CommandRun;
}

/// 探测范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeScope {
    /// 本机。
    Local,
    /// WSL 发行版。
    Wsl,
}

/// 环境快照：键 → 值。
pub type EnvironmentSnapshot = BTreeMap<String, String>;

/// 探测键。
pub mod keys {
    /// Node 版本。
    pub const NODE_VERSION: &str = "NODE_VERSION";
    /// npm 版本。
    pub const NPM_VERSION: &str = "NPM_VERSION";
    /// nvm 版本。
    pub const NVM_VERSION: &str = "NVM_VERSION";
    /// Node 可执行文件路径。
    pub const NODE_PATH: &str = "NODE_PATH";
    /// npm 可执行文件路径。
    pub const NPM_PATH: &str = "NPM_PATH";
    /// 探测错误。
    pub const DETECTION_ERROR: &str = "DETECTION_ERROR";
}

/// 探测超时（单条命令）。
pub const PROBE_TIMEOUT_MS: u64 = 10_000;

/// 取一条命令的第一行非空输出。
///
/// 版本命令有时会在版本号前后输出额外内容（例如 nvm 的提示），只取第一行非空内容
/// 比整段取回来更稳。
fn first_line(run: &CommandRun) -> String {
    let text = if run.stdout.trim().is_empty() {
        &run.stderr
    } else {
        &run.stdout
    };
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// 按范围执行命令。
fn run_in_scope(
    runner: &dyn CommandRunner,
    scope: ProbeScope,
    program: &str,
    args: &[&str],
) -> CommandRun {
    match scope {
        ProbeScope::Local => runner.run(program, args, PROBE_TIMEOUT_MS),
        ProbeScope::Wsl => {
            // 登录 shell：版本管理器只在登录 shell 里注入 PATH。
            let command = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            runner.run("bash", &["-lc", &command], PROBE_TIMEOUT_MS)
        }
    }
}

/// 找出可执行文件路径。
fn probe_path(runner: &dyn CommandRunner, scope: ProbeScope, name: &str) -> Option<String> {
    // `where` 是 Windows 的、`which` 是 POSIX 的；两个都试，取先有结果的。
    let candidates: &[(&str, &[&str])] = match scope {
        ProbeScope::Local => &[("where", &[name]), ("which", &[name])],
        // WSL 里只会是 POSIX。
        ProbeScope::Wsl => &[("which", &[name])],
    };
    for (program, args) in candidates {
        let run = run_in_scope(runner, scope, program, args);
        if run.timed_out {
            continue;
        }
        let value = first_line(&run);
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

/// 探测环境。
///
/// `provider` 决定要额外探测哪个 CLI 的版本。
#[must_use]
pub fn probe_environment(
    runner: &dyn CommandRunner,
    scope: ProbeScope,
    provider: ProviderId,
) -> EnvironmentSnapshot {
    let mut snapshot = EnvironmentSnapshot::new();

    // 记录超时：超时与「没装」是两回事，必须区分，否则用户会去装一个已经装好的东西。
    let mut timed_out: Vec<String> = Vec::new();

    let record = |snapshot: &mut EnvironmentSnapshot,
                  timed_out: &mut Vec<String>,
                  key: &str,
                  program: &str,
                  args: &[&str]| {
        let run = run_in_scope(runner, scope, program, args);
        if run.timed_out {
            timed_out.push(key.to_owned());
            return;
        }
        let value = first_line(&run);
        if !value.is_empty() {
            snapshot.insert(key.to_owned(), value);
        }
    };

    record(
        &mut snapshot,
        &mut timed_out,
        keys::NODE_VERSION,
        "node",
        &["--version"],
    );
    record(
        &mut snapshot,
        &mut timed_out,
        keys::NPM_VERSION,
        "npm",
        &["--version"],
    );
    record(
        &mut snapshot,
        &mut timed_out,
        keys::NVM_VERSION,
        "nvm",
        &["--version"],
    );
    // 每个平台的 CLI 名字就是平台编号本身。
    record(
        &mut snapshot,
        &mut timed_out,
        cli_version_key(provider),
        provider.as_str(),
        &["--version"],
    );

    if let Some(path) = probe_path(runner, scope, "node") {
        snapshot.insert(keys::NODE_PATH.to_owned(), path);
    }
    if let Some(path) = probe_path(runner, scope, "npm") {
        snapshot.insert(keys::NPM_PATH.to_owned(), path);
    }

    if !timed_out.is_empty() {
        snapshot.insert(
            keys::DETECTION_ERROR.to_owned(),
            format!("命令超时：{}", timed_out.join("、")),
        );
    }
    snapshot
}

/// CLI 环境状态（与现网 `CliEnvironmentStatus` 字段一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
// 这些 bool 是现网 `CliEnvironmentStatus` 的字段，必须逐个保留：
// 前端按字段名读取，合并它们会直接破坏契约。
#[allow(clippy::struct_excessive_bools)]
pub struct CliEnvironmentStatus {
    /// 平台编号。
    pub provider_id: String,
    /// 目标编号。
    pub target_id: String,
    /// `local` 或 `wsl`。
    pub scope: &'static str,
    /// `windows` / `macos` / `linux`。
    pub platform: &'static str,
    /// 该平台要求的最低 Node 主版本。
    pub minimum_node_major: u32,
    /// 推荐的 Node 主版本。
    pub recommended_node_major: u32,
    /// Node 版本原文。
    pub node_version: Option<String>,
    /// npm 版本原文。
    pub npm_version: Option<String>,
    /// nvm 版本原文。
    pub nvm_version: Option<String>,
    /// CLI 版本原文。
    pub cli_version: Option<String>,
    /// Node 路径。
    pub node_path: Option<String>,
    /// npm 路径。
    pub npm_path: Option<String>,
    /// 是否检测到 Node。
    pub has_node: bool,
    /// 是否检测到 npm。
    pub has_npm: bool,
    /// 是否检测到 nvm。
    pub has_nvm: bool,
    /// 是否检测到该平台的 CLI。
    pub has_cli: bool,
    /// Node 是否由 nvm 管理。
    pub node_managed_by_nvm: bool,
    /// Node 是否满足最低版本。
    pub node_meets_minimum: bool,
    /// 是否可以安装 CLI（需要 Node 与 npm）。
    pub can_install_cli: bool,
    /// 是否可以用 nvm 安装 Node。
    pub can_install_node_with_nvm: bool,
    /// 提示文案。
    pub messages: Vec<String>,
}

/// 由快照推导状态。
///
/// 与现网 `checkCliEnvironment` 的判定逐条对应。
/// 名字与现网的 `NVM_VERSION` / `NPM_VERSION` 对应，因此保留这种相近写法。
#[must_use]
#[allow(clippy::similar_names)]
pub fn status_from_snapshot(
    snapshot: &EnvironmentSnapshot,
    provider: ProviderId,
    target_id: &str,
    scope: ProbeScope,
    host_platform: &'static str,
) -> CliEnvironmentStatus {
    let get = |key: &str| {
        snapshot
            .get(key)
            .map_or("", String::as_str)
            .trim()
            .to_owned()
    };
    let node_version = get(keys::NODE_VERSION);
    let npm_version = get(keys::NPM_VERSION);
    let nvm_version = get(keys::NVM_VERSION);
    let cli_version = get(cli_version_key(provider));
    let node_path = get(keys::NODE_PATH);
    let npm_path = get(keys::NPM_PATH);

    let node_major = parse_node_major(&node_version);
    let has_node = !node_version.is_empty();
    let has_npm = !npm_version.is_empty();
    let has_nvm = !nvm_version.is_empty();
    let has_cli = !cli_version.is_empty();
    let managed = node_managed_by_nvm(&node_path);
    let minimum = minimum_node_major(provider);
    let meets = node_major.is_some_and(|major| major >= minimum);
    let can_install_cli = has_node && has_npm;
    let can_install_node_with_nvm = has_nvm;

    let detection_error = get(keys::DETECTION_ERROR);
    let messages = build_environment_messages(&EnvironmentFacts {
        has_node,
        has_npm,
        has_nvm,
        can_install_node_with_nvm,
        detection_error,
    });

    let platform = if scope == ProbeScope::Wsl {
        "linux"
    } else {
        host_platform
    };

    CliEnvironmentStatus {
        provider_id: provider.as_str().to_owned(),
        target_id: target_id.to_owned(),
        scope: if scope == ProbeScope::Wsl {
            "wsl"
        } else {
            "local"
        },
        platform,
        minimum_node_major: minimum,
        recommended_node_major: RECOMMENDED_NODE_MAJOR,
        node_version: none_if_empty(node_version),
        npm_version: none_if_empty(npm_version),
        nvm_version: none_if_empty(nvm_version),
        cli_version: none_if_empty(cli_version),
        node_path: none_if_empty(node_path),
        npm_path: none_if_empty(npm_path),
        has_node,
        has_npm,
        has_nvm,
        has_cli,
        node_managed_by_nvm: managed,
        node_meets_minimum: meets,
        can_install_cli,
        can_install_node_with_nvm,
        messages,
    }
}

/// 空串转 [`None`]。
fn none_if_empty(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

/// 真实命令执行器：轮询等待 + 到期强杀。
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str], timeout_ms: u64) -> CommandRun {
        let mut command = std::process::Command::new(program);
        // 探测是**后台动作**：不给子进程配控制台窗口（Win11 会弹 Windows Terminal 面板）。
        aivc_infra::process::hide_console_window(&mut command);
        command.args(args);
        // 不继承父进程的标准流：探测输出要走管道，而 `null` 避免子进程继承
        // 我们自己的终端句柄（那会让某些平台上的子进程挂住）。
        command.stdin(std::process::Stdio::null());
        command.stdout(std::process::Stdio::piped());
        command.stderr(std::process::Stdio::piped());

        let Ok(mut child) = command.spawn() else {
            // 命令不存在：这不是错误，是「没装」这一事实。
            return CommandRun::ok("");
        };

        let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
        // 循环的返回值就是退出码：命令没起来时为 None，超时时为 None。
        let exit_code: Option<i32>;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exit_code = status.code();
                    break;
                }
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        // 超时必须真的杀掉：留下一个挂住的子进程会一直占着管道。
                        let _ = child.kill();
                        let _ = child.wait();
                        return CommandRun::timeout();
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => {
                    exit_code = None;
                    break;
                }
            }
        }

        // 进程已退出，读取输出。`wait_with_output` 在这里会二次等待，
        // 因此直接读句柄。
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut handle) = child.stdout.take() {
            use std::io::Read as _;
            let _ = handle.read_to_string(&mut stdout);
        }
        if let Some(mut handle) = child.stderr.take() {
            use std::io::Read as _;
            let _ = handle.read_to_string(&mut stderr);
        }
        CommandRun {
            stdout,
            stderr,
            timed_out: false,
            exit_code,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 按命令名返回固定输出的执行器。
    struct Fake {
        /// `(program, args 拼接)` → 输出。
        responses: BTreeMap<String, CommandRun>,
        /// 记录被调用过的命令，供断言「不该执行的命令没有被执行」。
        calls: Mutex<Vec<String>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                responses: BTreeMap::new(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with(mut self, command: &str, stdout: &str) -> Self {
            self.responses
                .insert(command.to_owned(), CommandRun::ok(stdout));
            self
        }

        fn with_timeout(mut self, command: &str) -> Self {
            self.responses
                .insert(command.to_owned(), CommandRun::timeout());
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl CommandRunner for Fake {
        fn run(&self, program: &str, args: &[&str], _timeout_ms: u64) -> CommandRun {
            let key = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.calls.lock().expect("lock").push(key.clone());
            self.responses
                .get(&key)
                .cloned()
                // 未配置的命令视为「不存在」，与真实执行器一致。
                .unwrap_or_else(|| CommandRun::ok(""))
        }
    }

    #[test]
    fn a_complete_environment_is_reported_as_healthy() {
        let runner = Fake::new()
            .with("node --version", "v22.11.0\n")
            .with("npm --version", "10.9.0\n")
            .with("nvm --version", "0.40.1\n")
            .with("codex --version", "0.30.0\n")
            .with("where node", "/usr/bin/node\n")
            .with("where npm", "/usr/bin/npm\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(status.has_node && status.has_npm && status.has_nvm && status.has_cli);
        assert!(status.node_meets_minimum);
        assert!(status.can_install_cli);
        assert!(status.messages.is_empty(), "{:?}", status.messages);
        assert_eq!(status.node_version.as_deref(), Some("v22.11.0"));
        assert_eq!(status.minimum_node_major, 22);
        assert_eq!(status.recommended_node_major, 24);
    }

    #[test]
    fn an_old_node_is_reported_as_not_meeting_the_minimum() {
        let runner = Fake::new()
            .with("node --version", "v18.20.0\n")
            .with("npm --version", "10.0.0\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(status.has_node);
        assert!(!status.node_meets_minimum, "18 不满足 Codex 的 22");
        // 但同一个 Node 对 Claude（要 18）是满足的。
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Claude,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(status.node_meets_minimum);
    }

    #[test]
    fn a_missing_environment_produces_guidance() {
        let runner = Fake::new();
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(!status.has_node && !status.has_npm && !status.can_install_cli);
        assert!(status.messages.contains(&"未检测到 Node.js。".to_owned()));
        assert!(
            status
                .messages
                .iter()
                .any(|message| message.contains("请先安装 nvm"))
        );
    }

    #[test]
    fn nvm_only_reports_can_install_node() {
        let runner = Fake::new().with("nvm --version", "0.40.1\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(status.can_install_node_with_nvm);
        assert!(!status.can_install_cli, "没有 Node 就装不了 CLI");
        assert!(
            status
                .messages
                .iter()
                .any(|message| message.contains("可通过 nvm 安装"))
        );
    }

    #[test]
    fn a_timeout_is_reported_as_a_detection_error_not_as_missing() {
        // 超时与「没装」是两回事：混为一谈会让用户去装一个已经装好的东西。
        let runner = Fake::new()
            .with_timeout("node --version")
            .with("npm --version", "10.0.0\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        let error = snapshot.get(keys::DETECTION_ERROR).expect("应当记录超时");
        assert!(error.contains("命令超时"), "{error}");
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(
            status
                .messages
                .iter()
                .any(|message| message.starts_with("检测失败：")),
            "{:?}",
            status.messages
        );
    }

    #[test]
    fn the_node_path_decides_whether_nvm_manages_it() {
        let runner = Fake::new()
            .with("node --version", "v22.0.0\n")
            .with("where node", "/home/u/.nvm/versions/node/v22/bin/node\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:local",
            ProbeScope::Local,
            "windows",
        );
        assert!(status.node_managed_by_nvm);
    }

    #[test]
    fn wsl_probes_go_through_a_login_shell() {
        // nvm/fnm 只在登录 shell 里注入 PATH；用 `bash -c` 会得到干净的 PATH，
        // 于是明明装了的 Node 报「未检测到」。
        let runner = Fake::new().with("bash -lc node --version", "v22.0.0\n");
        let snapshot = probe_environment(&runner, ProbeScope::Wsl, ProviderId::Codex);
        assert_eq!(
            snapshot.get(keys::NODE_VERSION).map(String::as_str),
            Some("v22.0.0")
        );
        let calls = runner.calls();
        assert!(
            calls.iter().all(|call| call.starts_with("bash -lc ")),
            "WSL 下所有探测都必须走登录 shell：{calls:?}"
        );
    }

    #[test]
    fn wsl_status_reports_linux_platform() {
        let runner = Fake::new();
        let snapshot = probe_environment(&runner, ProbeScope::Wsl, ProviderId::Codex);
        let status = status_from_snapshot(
            &snapshot,
            ProviderId::Codex,
            "codex:wsl:Ubuntu",
            ProbeScope::Wsl,
            "windows",
        );
        // 目标在 WSL 里，平台就必须报 linux，而不是宿主机的 windows——
        // 界面据此决定显示哪个平台的安装指引。
        assert_eq!(status.platform, "linux");
        assert_eq!(status.scope, "wsl");
    }

    #[test]
    fn only_the_requested_provider_cli_is_probed() {
        let runner = Fake::new().with("gemini --version", "0.5.0\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Gemini);
        assert!(snapshot.contains_key("GEMINI_VERSION"));
        assert!(!snapshot.contains_key("CODEX_VERSION"));
    }

    #[test]
    fn the_first_non_empty_line_is_used() {
        // 版本命令有时会在版本号前后输出额外内容。
        let runner = Fake::new().with("node --version", "\n\nv22.0.0\nextra\n");
        let snapshot = probe_environment(&runner, ProbeScope::Local, ProviderId::Codex);
        assert_eq!(
            snapshot.get(keys::NODE_VERSION).map(String::as_str),
            Some("v22.0.0")
        );
    }

    #[test]
    fn stderr_is_used_when_stdout_is_empty() {
        // 有些工具把版本写到 stderr。
        struct StderrOnly;
        impl CommandRunner for StderrOnly {
            fn run(&self, program: &str, _args: &[&str], _timeout_ms: u64) -> CommandRun {
                if program == "node" {
                    CommandRun {
                        stdout: String::new(),
                        stderr: "v20.0.0\n".to_owned(),
                        timed_out: false,
                        exit_code: Some(0),
                    }
                } else {
                    CommandRun::ok("")
                }
            }
        }
        let snapshot = probe_environment(&StderrOnly, ProbeScope::Local, ProviderId::Codex);
        assert_eq!(
            snapshot.get(keys::NODE_VERSION).map(String::as_str),
            Some("v20.0.0")
        );
    }

    #[test]
    fn the_system_runner_reports_a_missing_program_as_empty() {
        // 命令不存在不是错误，是「没装」这一事实。
        let run = SystemRunner.run("aivc-definitely-not-a-real-program", &[], 1_000);
        assert!(!run.timed_out);
        assert!(run.stdout.is_empty());
    }

    #[test]
    fn the_system_runner_captures_output() {
        // 用当前平台一定存在的命令验证管道读取。
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("cmd", &["/c", "echo", "hello"])
        } else {
            ("echo", &["hello"])
        };
        let run = SystemRunner.run(program, args, 5_000);
        assert!(!run.timed_out);
        assert!(run.stdout.contains("hello"), "{:?}", run.stdout);
    }

    #[test]
    fn probe_keys_are_stable() {
        // 这些键是快照的契约，改名会让状态推导静默失效。
        assert_eq!(keys::NODE_VERSION, "NODE_VERSION");
        assert_eq!(keys::NPM_VERSION, "NPM_VERSION");
        assert_eq!(keys::NVM_VERSION, "NVM_VERSION");
        assert_eq!(keys::NODE_PATH, "NODE_PATH");
        assert_eq!(keys::NPM_PATH, "NPM_PATH");
        assert_eq!(keys::DETECTION_ERROR, "DETECTION_ERROR");
        assert_eq!(PROBE_TIMEOUT_MS, 10_000);
    }
}
