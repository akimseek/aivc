//! 启动 WSL 侧进程的方式。
//!
//! # 为什么要抽成可注入的接口
//!
//! 生产路径是 `wsl.exe -d <distro> -- bash -lc <script>`，但**协议本身的正确性**与
//! 是否装了 WSL 无关。把「如何进入目标环境的 bash」抽出来之后，就可以用本机 bash
//! （Windows 上通常是 Git Bash）真实地跑一遍持久 worker 协议，而不是只写一个无法
//! 执行的 mock。
//!
//! 生产实现 [`WslExeLauncher`] 忽略本机平台差异，测试实现 [`LocalBashLauncher`]
//! 忽略发行版参数。

use std::path::PathBuf;
use std::process::Stdio;

use aivc_core::error::{Result, WslError};
use tokio::process::{Child, Command};

/// 启动 WSL 侧 bash 进程的接口。
pub trait WslLauncher: Send + Sync {
    /// 在目标发行版内启动一个执行 `script` 的**一次性** bash 进程。
    ///
    /// 实现必须把 stdout/stderr 设为 `piped`，并把 **stdin 设为 null**。
    ///
    /// stdin 必须是 null，这一条有实测依据：脚本已经由 `base64 -d | bash` 提供，不需要
    /// stdin；而一个**开着却不写、也不关**的管道会让任何误读 stdin 的命令**永久阻塞**
    /// ——那正是「30 秒超时」的形状。
    fn spawn_bash(&self, distro: &str, script: &str) -> Result<Child>;

    /// 在目标发行版内启动一个执行 `script` 的**常驻** bash 进程。
    ///
    /// 与 [`WslLauncher::spawn_bash`] 的唯一差别是 **stdin 必须是管道**：它是帧协议的
    /// 写入通道，而这条通道正是常驻 worker 存在的全部意义。
    ///
    /// # 为什么必须是独立的方法
    ///
    /// 这两件事曾经共用一个方法，而那个方法在生产实现 [`WslExeLauncher`] 里把 stdin
    /// 设成了 null。后果是**生产环境的 worker 永远起不来**：`WslWorker::spawn` 拿不到
    /// `child.stdin`，只能报「worker 的 stdin 不可用」，于是每次调用都退回一次性执行
    /// ——每次都多付一次 `wsl.exe` 进程创建，比不开 worker 还慢。
    ///
    /// 更糟的是这个缺陷**在测试里永远看不见**：测试用的是 [`LocalBashLauncher`]，
    /// 它恰好把 stdin 设成了管道。**一个方法承担两种相反的契约，缺陷就会藏在「另一个
    /// 实现」里**，而测试恰好用的是那个实现。
    fn spawn_bash_interactive(&self, distro: &str, script: &str) -> Result<Child>;

    /// 直接启动 `wsl.exe`（**不**经发行版内的 bash）。
    ///
    /// 仅用于 `wsl -l -q` 这类必须在宿主侧执行的调用——它列出的是发行版本身，
    /// 不能在某个发行版内部执行。
    fn spawn_wsl_raw(&self, args: &[&str]) -> Result<Child>;

    /// 在目标发行版内启动任意程序（stdin/stdout/stderr 均接管）。
    ///
    /// 与 [`WslLauncher::spawn_bash`] 的区别：这里不经过 `bash -lc`，因此调用方自己
    /// 决定要不要 shell。`codex app-server` 这类需要长期双向通信的进程用它——
    /// 多套一层 shell 只会让信号与退出码的传递变复杂。
    fn spawn_program(&self, distro: &str, program: &str, args: &[String]) -> Result<Child>;

    /// 该 launcher 是否可用（例如 `wsl.exe` 是否存在）。
    fn is_available(&self) -> bool;

    /// 诊断用的描述。
    fn describe(&self) -> String;
}

/// 启动 `wsl.exe -d <distro> -- bash -lc <wrapped>`。
///
/// `stdin` 与**包装形状**都由调用方决定，因为一次性执行与常驻 worker 对这两者的要求
/// 正好相反（见 trait 上的两个方法）。
fn spawn_wsl_bash(
    executable: &PathBuf,
    distro: &str,
    script: &str,
    stdin: Stdio,
    interactive: bool,
) -> Result<Child> {
    // **脚本必须先 base64 再传**。
    //
    // 实测踩过：`列出 /home/akim/.codex/sessions` 花了 **30 秒**后以退出码 `-1`
    // 失败，而同一个 `wsl.exe` 调用单独跑只要 0.1 秒——卡住的就是参数解析。
    //
    // 脚本从进程到发行版内的 bash 要穿三层解析（Windows 命令行 → wsl.exe →
    // bash），任意一层吃掉引号/`$`/换行，bash 就可能落进「读 stdin」。
    // 编码后载荷是纯 base64 字符，三层都无从破坏。详见
    // [`super::script_transfer`]。
    //
    // 记录每次调用与脚本首行：**「WSL 慢」几乎总是「调用次数多」**，
    // 而单次调用在 shell 里实测只要 0.1 秒（冷启动 5–7 秒）。
    // 不记这个，就分不清「一次很慢」与「很多次加起来慢」。
    let first_line = script.lines().next().unwrap_or("").trim();
    tracing::debug!(
        distro,
        script_bytes = script.len(),
        interactive,
        first_line = %first_line.chars().take(80).collect::<String>(),
        "执行 WSL 脚本"
    );
    // **常驻进程必须用「不占 stdin」的形状**：管道形状会把 bash 的 stdin 吃掉，
    // 而 worker 的 stdin 就是协议通道，于是它第一轮 `read` 就 EOF 并退出。
    let wrapped = if interactive {
        super::script_transfer::wrap_script_preserving_stdin(script)
    } else {
        super::script_transfer::wrap_script(script)
    };
    Command::new(executable)
        .arg("-d")
        .arg(distro)
        .arg("--")
        .arg("bash")
        .arg("-lc")
        .arg(&wrapped)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| WslError::CommandFailed {
            message: format!("无法启动 wsl.exe：{error}"),
        })
        .map_err(Into::into)
}

/// 启动本机 bash 执行 `script`；`stdin` 由调用方决定（契约同上）。
fn spawn_local_bash(executable: &PathBuf, script: &str, stdin: Stdio) -> Result<Child> {
    Command::new(executable)
        .arg("-lc")
        .arg(script)
        .stdin(stdin)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| WslError::CommandFailed {
            message: format!("无法启动本机 bash：{error}"),
        })
        .map_err(Into::into)
}

/// 生产实现：通过 `wsl.exe` 进入发行版。
#[derive(Debug, Clone)]
pub struct WslExeLauncher {
    executable: PathBuf,
}

impl WslExeLauncher {
    /// 由 `wsl.exe` 路径构造。
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// 探测 `wsl.exe` 的常见位置。
    ///
    /// 与现网一致：优先系统目录，其次 PATH 中的裸命令名。
    #[must_use]
    pub fn detect() -> Option<Self> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        for variable in ["SystemRoot", "windir"] {
            if let Ok(root) = std::env::var(variable) {
                candidates.push(PathBuf::from(root).join("System32").join("wsl.exe"));
            }
        }
        candidates.push(PathBuf::from("C:\\Windows\\System32\\wsl.exe"));
        // WSL 从 32 位进程调用时的重定向路径。
        candidates.push(PathBuf::from("C:\\Windows\\Sysnative\\wsl.exe"));
        candidates.push(PathBuf::from("/mnt/c/Windows/System32/wsl.exe"));

        let found = candidates
            .iter()
            .find(|candidate| candidate.is_file())
            .cloned();
        if found.is_none() {
            // **找不到时必须说清检查过哪些路径**。
            //
            // 实测踩过：应用日志里只有一句「未找到 wsl.exe」，而同一台机器上
            // `C:\Windows\System32\wsl.exe` 明明存在、PowerShell 里 `wsl -l -q` 也正常。
            // 只报「没找到」而不报「查过哪里」，等于把矛盾留给排查者。
            let tried: Vec<String> = candidates
                .iter()
                .map(|candidate| {
                    format!(
                        "{}（{}）",
                        candidate.display(),
                        describe_candidate(candidate)
                    )
                })
                .collect();
            tracing::warn!(tried = ?tried, "找不到 wsl.exe，WSL 目标将为空");
        }
        found.map(Self::new)
    }
}

/// 说明一个候选路径的状态，供诊断。
fn describe_candidate(candidate: &std::path::Path) -> &'static str {
    if candidate.is_file() {
        "是文件"
    } else if candidate.exists() {
        "存在但不是文件"
    } else {
        "不存在"
    }
}

impl WslLauncher for WslExeLauncher {
    fn spawn_bash(&self, distro: &str, script: &str) -> Result<Child> {
        // 一次性执行：**不给 stdin**，脚本走管道喂给 bash。
        spawn_wsl_bash(&self.executable, distro, script, Stdio::null(), false)
    }

    fn spawn_bash_interactive(&self, distro: &str, script: &str) -> Result<Child> {
        // 常驻 worker：stdin 是帧协议通道，必须接管；脚本因此改走**参数**形状，
        // 否则 bash 会把 stdin 读干当脚本，worker 立刻退出。
        spawn_wsl_bash(&self.executable, distro, script, Stdio::piped(), true)
    }

    fn spawn_wsl_raw(&self, args: &[&str]) -> Result<Child> {
        Command::new(&self.executable)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| WslError::CommandFailed {
                message: format!("无法启动 wsl.exe：{error}"),
            })
            .map_err(Into::into)
    }

    fn spawn_program(&self, distro: &str, program: &str, args: &[String]) -> Result<Child> {
        Command::new(&self.executable)
            .arg("-d")
            .arg(distro)
            .arg("--")
            .arg(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| WslError::CommandFailed {
                message: format!("无法启动 wsl.exe：{error}"),
            })
            .map_err(Into::into)
    }

    fn is_available(&self) -> bool {
        self.executable.is_file()
    }

    fn describe(&self) -> String {
        format!("wsl.exe（{}）", self.executable.display())
    }
}

/// 测试实现：直接在本机 bash 中执行脚本，忽略发行版参数。
///
/// 仅用于验证协议本身；生产代码不得使用。
#[derive(Debug, Clone)]
pub struct LocalBashLauncher {
    executable: PathBuf,
}

impl LocalBashLauncher {
    /// 由 bash 可执行文件路径构造。
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// 探测本机 bash。
    ///
    /// 覆盖 Windows 上常见的 Git Bash 安装位置与 PATH 中的 `bash`。
    #[must_use]
    pub fn detect() -> Option<Self> {
        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from("D:\\tools\\git\\Git\\bin\\bash.exe"),
            PathBuf::from("C:\\Program Files\\Git\\bin\\bash.exe"),
            PathBuf::from("C:\\Program Files\\Git\\usr\\bin\\bash.exe"),
            PathBuf::from("C:\\Program Files (x86)\\Git\\bin\\bash.exe"),
            PathBuf::from("/bin/bash"),
            PathBuf::from("/usr/bin/bash"),
        ];
        // PATH 中的 bash（`which` 等价物由 `std::env::split_paths` 手工实现，避免额外依赖）。
        if let Some(path) = std::env::var_os("PATH") {
            for directory in std::env::split_paths(&path) {
                candidates.push(directory.join("bash.exe"));
                candidates.push(directory.join("bash"));
            }
        }
        candidates
            .into_iter()
            .find(|candidate| candidate.is_file())
            .map(Self::new)
    }
}

impl LocalBashLauncher {
    /// 实际运行一次以确认 bash **能跑起来**。
    ///
    /// 仅检查文件存在是不够的：受限沙箱下 MSYS2 bash 会因为无法创建内部 signal pipe
    /// 而直接崩溃（`couldn't create signal pipe, Win32 error 5`）。测试据此跳过，
    /// 而不是把环境限制误报成代码缺陷；CI（Linux）上原生 bash 没有这个问题。
    #[must_use]
    pub fn is_usable(&self) -> bool {
        std::process::Command::new(&self.executable)
            .arg("-c")
            .arg("exit 0")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

impl WslLauncher for LocalBashLauncher {
    fn spawn_bash(&self, _distro: &str, script: &str) -> Result<Child> {
        // **与生产实现保持一致：一次性执行的 stdin 是 null**。
        //
        // 这里曾经是 `piped`，而 `WslExeLauncher` 是 `null`——正是这个不一致让
        // 「生产环境 worker 永远起不来」的缺陷在测试里完全看不见。
        // 测试替身与生产实现必须在**契约**上一致，否则测试证明的是替身的行为。
        spawn_local_bash(&self.executable, script, Stdio::null())
    }

    fn spawn_bash_interactive(&self, _distro: &str, script: &str) -> Result<Child> {
        spawn_local_bash(&self.executable, script, Stdio::piped())
    }

    fn spawn_wsl_raw(&self, _args: &[&str]) -> Result<Child> {
        // 本机 bash 没有「列出发行版」的等价物：测试只覆盖 `parse_distro_list`，
        // 端到端由 CI（真实 WSL 环境）或手工验证。
        Err(WslError::ExecutableNotFound.into())
    }

    fn spawn_program(&self, _distro: &str, program: &str, args: &[String]) -> Result<Child> {
        // 测试路径：直接在本机执行，忽略发行版参数。
        Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| WslError::CommandFailed {
                message: format!("无法启动 {program}：{error}"),
            })
            .map_err(Into::into)
    }

    fn is_available(&self) -> bool {
        self.executable.is_file()
    }

    fn describe(&self) -> String {
        format!("本机 bash（{}）", self.executable.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection_returns_something_consistent_with_availability() {
        // 探测成功时 is_available 必须为真，否则 launcher 会在首次调用时才失败。
        if let Some(launcher) = LocalBashLauncher::detect() {
            assert!(launcher.is_available());
            assert!(launcher.describe().contains("bash"));
        }
        if let Some(launcher) = WslExeLauncher::detect() {
            assert!(launcher.is_available());
            assert!(launcher.describe().contains("wsl.exe"));
        }
    }

    #[test]
    fn missing_executable_is_reported_as_unavailable() {
        let launcher = LocalBashLauncher::new("definitely-not-a-real-bash");
        assert!(!launcher.is_available());
    }

    #[tokio::test]
    async fn spawning_a_missing_executable_reports_an_error() {
        let launcher = LocalBashLauncher::new("definitely-not-a-real-bash");
        let outcome = launcher.spawn_bash("Ubuntu", "echo hi");
        assert!(outcome.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn local_bash_launcher_actually_runs_scripts() {
        // 没有 bash、或沙箱不允许 bash 创建内部 signal pipe 时跳过。
        let Some(launcher) = LocalBashLauncher::detect().filter(LocalBashLauncher::is_usable)
        else {
            return;
        };
        let child = launcher
            .spawn_bash("ignored-distro", "printf 'hello'")
            .expect("spawn");
        let output = child.wait_with_output().await.expect("wait");
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout), "hello");
    }
}
