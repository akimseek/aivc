//! Codex 官方 `app-server` 的 JSON-RPC 客户端。
//!
//! # 协议
//!
//! 换行分隔的 JSON-RPC 2.0，走子进程的 stdin/stdout：
//!
//! ```text
//! 请求    {"jsonrpc":"2.0","id":1,"method":"thread/fork","params":{...}}\n
//! 通知    {"jsonrpc":"2.0","method":"initialized"}\n
//! 响应    {"id":1,"result":{...}}\n  或  {"id":1,"error":{"message":"..."}}\n
//! ```
//!
//! 服务端会主动推送**没有数字 id** 的通知（进度、日志）。客户端必须跳过它们而不是
//! 当成响应——否则一次无关推送就会让请求配对错位，后续所有请求都拿到别人的结果。
//!
//! # 启动方式
//!
//! | 目标 | 启动命令 |
//! | --- | --- |
//! | 本机（Windows） | `cmd.exe /d /s /c "<codex.cmd 路径> app-server"` |
//! | 本机（其他） | `codex app-server` |
//! | WSL | `wsl.exe -d <发行版> -- bash -ic "export CODEX_HOME=…; exec codex app-server"` |
//!
//! Windows 必须经 `cmd.exe`：npm 安装的 `codex` 是 `.cmd` 批处理包装器，
//! 直接 `CreateProcess` 无法执行它。
//!
//! # 为什么失败时要带上 stderr
//!
//! app-server 的失败几乎都发生在启动阶段（未安装、版本过旧、Node 版本不符），
//! 而这些信息**只在 stderr 上**。丢掉它会让用户看到「已退出（退出码 1）」这种
//! 无从下手的提示。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

use aivc_core::error::{Result, WslError};
use aivc_infra::wsl::WslAdapter;

/// app-server 调用超时（现网 60 s）。
pub const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(60);

/// 构造一次调用超时的错误。
fn timeout_error(timeout: Duration, method: &str) -> aivc_core::Error {
    WslError::Timeout {
        timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
        label: format!("调用 Codex 官方分支服务（{method}）"),
    }
    .into()
}

/// 客户端身份，与服务端 `initialize` 握手时声明。
pub const CLIENT_NAME: &str = "AI Visual Console";
/// 客户端版本。
pub const CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// app-server 的启动方式。
#[derive(Clone)]
pub enum AppServerLauncher {
    /// 本机直接启动。
    Local {
        /// 可执行文件（Windows 上是 `cmd.exe`）。
        program: String,
        /// 参数。
        args: Vec<String>,
        /// 传给子进程的 `CODEX_HOME`。
        codex_home: Option<String>,
    },
    /// 在 WSL 发行版内启动。
    Wsl {
        /// 适配器（内部持有 launcher）。
        adapter: WslAdapter,
        /// 发行版名。
        distro: String,
        /// 传给子进程的 `CODEX_HOME`。
        codex_home: Option<String>,
    },
}

impl std::fmt::Debug for AppServerLauncher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Local { program, .. } => f
                .debug_struct("AppServerLauncher::Local")
                .field("program", program)
                .finish_non_exhaustive(),
            Self::Wsl { distro, .. } => f
                .debug_struct("AppServerLauncher::Wsl")
                .field("distro", distro)
                .finish_non_exhaustive(),
        }
    }
}

impl AppServerLauncher {
    /// 本机启动方式。
    #[must_use]
    pub fn local(codex_home: Option<String>) -> Self {
        let (program, args) = local_command();
        Self::Local {
            program,
            args,
            codex_home,
        }
    }

    /// WSL 启动方式。
    ///
    /// 用 `bash -ic`（**交互式**）：`codex` 常由 `nvm` / `fnm` 按 profile 注入 PATH，
    /// 非交互式 shell 看不到它。
    #[must_use]
    pub fn wsl(adapter: WslAdapter, distro: impl Into<String>, codex_home: Option<String>) -> Self {
        Self::Wsl {
            adapter,
            distro: distro.into(),
            codex_home,
        }
    }

    /// 启动进程。
    pub fn spawn(&self) -> Result<Child> {
        match self {
            Self::Local {
                program,
                args,
                codex_home,
            } => {
                let mut command = tokio::process::Command::new(program);
                command.args(args);
                if let Some(home) = codex_home {
                    command.env("CODEX_HOME", home);
                }
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| {
                        WslError::CommandFailed {
                            message: format!("无法启动 Codex App Server：{error}"),
                        }
                        .into()
                    })
            }
            Self::Wsl {
                adapter,
                distro,
                codex_home,
            } => {
                let home = codex_home
                    .as_ref()
                    .map(|value| {
                        format!(
                            "export CODEX_HOME={}; ",
                            aivc_core::shell::posix_shell_quote(value)
                        )
                    })
                    .unwrap_or_default();
                let script = format!("{home}exec codex app-server");
                adapter.spawn_program(distro, "bash", &["-ic".to_owned(), script])
            }
        }
    }

    /// 诊断描述。
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Local { program, args, .. } => format!("{program} {}", args.join(" ")),
            Self::Wsl { distro, .. } => format!("wsl -d {distro} -- bash -ic 'codex app-server'"),
        }
    }
}

/// 本机 `codex app-server` 的启动命令。
fn local_command() -> (String, Vec<String>) {
    if cfg!(windows) {
        let executable = resolve_windows_codex_command();
        (
            "cmd.exe".to_owned(),
            vec![
                "/d".to_owned(),
                "/s".to_owned(),
                "/c".to_owned(),
                format!("{} app-server", quote_windows_cmd_argument(&executable)),
            ],
        )
    } else {
        ("codex".to_owned(), vec!["app-server".to_owned()])
    }
}

/// 解析 Windows 上 `codex` 的安装位置。
///
/// 与现网一致地搜索 npm 全局目录、`nvm` 符号链接与 Node 安装目录；都找不到时回退到
/// 裸命令名，让 PATH 决定。
#[must_use]
pub fn resolve_windows_codex_command() -> String {
    let mut directories: Vec<PathBuf> = Vec::new();
    for variable in ["APPDATA", "LOCALAPPDATA"] {
        if let Ok(value) = std::env::var(variable) {
            directories.push(Path::new(&value).join("npm"));
        }
    }
    if let Ok(value) = std::env::var("NVM_SYMLINK") {
        directories.push(PathBuf::from(value));
    }
    for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(value) = std::env::var(variable) {
            directories.push(Path::new(&value).join("nodejs"));
        }
    }
    for directory in directories {
        for name in ["codex.cmd", "codex.exe"] {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate.to_string_lossy().into_owned();
            }
        }
    }
    "codex.cmd".to_owned()
}

/// 按 `cmd.exe` 的规则引用参数：内层双引号写两遍。
#[must_use]
pub fn quote_windows_cmd_argument(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// 一个已连接的 app-server 会话。
pub struct AppServerClient {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
    next_id: u64,
    label: String,
}

impl std::fmt::Debug for AppServerClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppServerClient")
            .field("label", &self.label)
            .field("next_id", &self.next_id)
            .finish_non_exhaustive()
    }
}

impl AppServerClient {
    /// 启动并连接。
    pub fn connect(launcher: &AppServerLauncher) -> Result<Self> {
        let mut child = launcher.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| WslError::CommandFailed {
            message: "App Server 的 stdin 不可用".to_owned(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| WslError::CommandFailed {
            message: "App Server 的 stdout 不可用".to_owned(),
        })?;
        let stderr_pipe = child.stderr.take();

        let stderr = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = stderr_pipe {
            let sink = Arc::clone(&stderr);
            // 后台持续读取：管道写满会让 app-server 阻塞在写 stderr 上，
            // 表现为「请求永远没有响应」。
            tokio::spawn(async move {
                let mut lines = BufReader::new(pipe).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(mut sink) = sink.lock() {
                        sink.push_str(&line);
                        sink.push('\n');
                    }
                }
            });
        }

        Ok(Self {
            child,
            stdin,
            reader: BufReader::new(stdout),
            stderr,
            next_id: 1,
            label: launcher.describe(),
        })
    }

    /// 完成 `initialize` 握手。
    pub async fn initialize(&mut self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": CLIENT_NAME,
                    "title": CLIENT_NAME,
                    "version": CLIENT_VERSION,
                },
                "capabilities": {
                    "experimentalApi": false,
                    "requestAttestation": false,
                },
            }),
            APP_SERVER_TIMEOUT,
        )
        .await?;
        self.notify("initialized", APP_SERVER_TIMEOUT).await
    }

    /// 发起一次请求并等待配对响应。
    pub async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        let payload = format!(
            "{}\n",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        );
        self.write(&payload, timeout, method).await?;

        // 读取直到拿到本请求的响应；服务端通知（没有数字 id）一律跳过。
        let read = async {
            loop {
                let mut line = String::new();
                let read = self.reader.read_line(&mut line).await.map_err(|error| {
                    WslError::CommandFailed {
                        message: format!("读取 App Server 响应失败：{error}"),
                    }
                })?;
                if read == 0 {
                    return Err(self.prefer_stderr(self.exited_error()));
                }
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                if message.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = message.get("error") {
                    let detail = error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("Codex App Server 请求失败。");
                    return Err(WslError::CommandFailed {
                        message: detail.to_owned(),
                    }
                    .into());
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
        };

        tokio::time::timeout(timeout, read).await.map_err(|_| {
            aivc_core::Error::from(WslError::Timeout {
                timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
                label: format!("调用 Codex 官方分支服务（{method}）"),
            })
        })?
    }

    /// 发送通知（无需响应）。
    pub async fn notify(&mut self, method: &str, timeout: Duration) -> Result<()> {
        let payload = format!("{}\n", json!({ "jsonrpc": "2.0", "method": method }));
        self.write(&payload, timeout, method).await
    }

    /// 关闭连接。
    pub async fn shutdown(mut self) {
        let _ = self.stdin.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        let _ = self.child.start_kill();
    }

    /// 进程是否仍在运行。
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// 已收集到的 stderr。
    #[must_use]
    pub fn stderr(&self) -> String {
        self.stderr
            .lock()
            .map(|sink| sink.trim().to_owned())
            .unwrap_or_default()
    }

    async fn write(&mut self, payload: &str, timeout: Duration, method: &str) -> Result<()> {
        let write = async {
            self.stdin
                .write_all(payload.as_bytes())
                .await
                .map_err(|error| WslError::CommandFailed {
                    message: format!("写入 App Server 请求失败：{error}"),
                })?;
            self.stdin
                .flush()
                .await
                .map_err(|error| WslError::CommandFailed {
                    message: format!("写入 App Server 请求失败：{error}"),
                })?;
            // 显式标注返回类型：否则块类型会被推成 `Result<(), WslError>`，
            // 与函数签名不一致。
            Ok::<(), aivc_core::Error>(())
        };
        // 显式两步：`timeout` 返回嵌套 `Result`，分开写比连缀 `?` 更清楚。
        let outcome = tokio::time::timeout(timeout, write)
            .await
            .map_err(|_| timeout_error(timeout, method))?;
        outcome.map_err(|error| self.prefer_stderr(error))
    }

    /// 若已收集到 stderr，用它替换原始错误。
    ///
    /// 子进程启动失败时，第一次写入会拿到「管道正在被关闭」这类无信息量的 OS 错误，
    /// 而真正原因（未安装、Node 版本不符、`CODEX_HOME` 不可读）**只在 stderr 上**。
    /// 丢掉它会让用户看到无从下手的提示。
    fn prefer_stderr(&self, fallback: aivc_core::Error) -> aivc_core::Error {
        let detail = self.stderr();
        if detail.is_empty() {
            fallback
        } else {
            WslError::CommandFailed { message: detail }.into()
        }
    }

    /// 进程已退出时的兜底错误（stderr 为空时使用）。
    fn exited_error(&self) -> aivc_core::Error {
        WslError::CommandFailed {
            message: format!("Codex App Server 已退出（{}）。", self.label),
        }
        .into()
    }
}

/// 启动、初始化、执行动作、关闭。
///
/// 无论动作成功与否都会关闭子进程——否则失败的 app-server 会一直挂在后台。
pub async fn with_app_server<T, F>(launcher: &AppServerLauncher, action: F) -> Result<T>
where
    F: AsyncFnOnce(&mut AppServerClient) -> Result<T>,
{
    let mut client = AppServerClient::connect(launcher)?;
    let outcome = async {
        client.initialize().await?;
        action(&mut client).await
    }
    .await;
    client.shutdown().await;
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_command_quoting_doubles_inner_quotes() {
        assert_eq!(
            quote_windows_cmd_argument("C:\\Program Files\\codex.cmd"),
            "\"C:\\Program Files\\codex.cmd\""
        );
        // cmd.exe 的转义规则是「内层双引号写两遍」，不是反斜杠转义。
        assert_eq!(quote_windows_cmd_argument("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn local_command_matches_the_platform() {
        let (program, args) = local_command();
        if cfg!(windows) {
            assert_eq!(program, "cmd.exe");
            assert_eq!(args[..3], ["/d", "/s", "/c"]);
            assert!(args[3].ends_with("app-server"));
        } else {
            assert_eq!(program, "codex");
            assert_eq!(args, vec!["app-server"]);
        }
    }

    #[test]
    fn windows_codex_resolution_returns_something_usable() {
        let resolved = resolve_windows_codex_command();
        assert!(!resolved.is_empty());
        // 找不到时回退到裸命令名，交给 PATH 解析。
        if !Path::new(&resolved).is_file() {
            assert_eq!(resolved, "codex.cmd");
        }
    }

    #[test]
    fn launcher_describes_itself_for_diagnostics() {
        let local = AppServerLauncher::local(Some("/tmp/home".to_owned()));
        assert!(local.describe().contains("app-server"));
        assert!(format!("{local:?}").contains("Local"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connecting_to_a_missing_program_reports_an_error() {
        let launcher = AppServerLauncher::Local {
            program: "definitely-not-a-real-program".to_owned(),
            args: Vec::new(),
            codex_home: None,
        };
        let error = AppServerClient::connect(&launcher).expect_err("必须失败");
        assert!(error.to_string().contains("无法启动 Codex App Server"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dead_process_surfaces_its_stderr() {
        // 用一个立刻退出并写 stderr 的进程模拟 app-server 启动失败。
        let launcher = if cfg!(windows) {
            AppServerLauncher::Local {
                program: "cmd.exe".to_owned(),
                args: vec![
                    "/d".to_owned(),
                    "/s".to_owned(),
                    "/c".to_owned(),
                    "echo boom 1>&2 & exit /b 1".to_owned(),
                ],
                codex_home: None,
            }
        } else {
            AppServerLauncher::Local {
                program: "sh".to_owned(),
                args: vec!["-c".to_owned(), "echo boom >&2; exit 1".to_owned()],
                codex_home: None,
            }
        };
        let mut client = AppServerClient::connect(&launcher).expect("spawn");
        // 等进程退出并让后台读取任务把 stderr 收进来。
        tokio::time::sleep(Duration::from_millis(300)).await;
        let outcome = client
            .request("initialize", json!({}), Duration::from_secs(5))
            .await;
        let error = outcome.expect_err("必须失败");
        assert!(
            error.to_string().contains("boom"),
            "stderr 内容必须出现在错误里，实际：{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn requests_time_out_instead_of_hanging() {
        // 一个只读 stdin、从不回应的进程。
        let launcher = if cfg!(windows) {
            AppServerLauncher::Local {
                program: "cmd.exe".to_owned(),
                args: vec![
                    "/d".to_owned(),
                    "/s".to_owned(),
                    "/c".to_owned(),
                    // 既不读 stdin 也不立刻退出：`pause` 之类的命令会因 stdin 立刻 EOF
                    // 而退出，达不到「挂住」的效果。次数取小值，避免测试结束后仍留下
                    // 长时间存活的外部进程拖慢整个套件。
                    "ping -n 3 127.0.0.1 > nul".to_owned(),
                ],
                codex_home: None,
            }
        } else {
            AppServerLauncher::Local {
                program: "sleep".to_owned(),
                args: vec!["3".to_owned()],
                codex_home: None,
            }
        };
        let mut client = AppServerClient::connect(&launcher).expect("spawn");
        let outcome = client
            .request("initialize", json!({}), Duration::from_millis(200))
            .await;
        let error = outcome.expect_err("必须超时");
        assert!(error.to_string().contains("超时"), "实际：{error}");
        // 显式关闭，确保子进程被回收而不是留到测试进程结束。
        client.shutdown().await;
    }
}
