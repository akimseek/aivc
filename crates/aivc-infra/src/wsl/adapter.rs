//! WSL 适配器：一次性执行、持久 worker 与端口实现。
//!
//! # 两条路径的分工
//!
//! | 操作 | 走哪条 | 理由 |
//! | --- | --- | --- |
//! | `size` / `exists` / `read_head` / `list_session_files` | **worker** | 会话列表热路径，每次操作省下 50–150 ms 进程创建开销；输出有界 |
//! | `read_text` / `write_text` / `move` / `remove` | **worker** | 输出有界（读取前先用 `size` 校验上限） |
//! | `read_lines` | **一次性流式** | 帧协议一次性返回整个文件，大 JSONL 会整体进内存；流式读取才能保持内存有界 |
//! | `list_distros` / `run_shell` / 探测类 | **一次性** | 不经发行版内的文件系统，或本身就是任意脚本 |
//!
//! worker 任一步失败都会**回退到一次性执行**，并丢弃坏掉的 worker。降级是刻意的：
//! worker 只是加速手段，不能成为可用性的单点。

use std::sync::Arc;
use std::time::Duration;

use aivc_core::error::{Result, WslError};
use aivc_core::path::sanitize_wsl_distro;
use aivc_core::ports::{
    BoxFuture, ConflictPolicy, LineControl, LineSink, ReadOptions, ReadStart, SessionStore,
    StoreKind,
};
use aivc_core::session::SessionFile;
use aivc_core::shell::posix_shell_quote;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

use super::decode::{decode_wsl_output, parse_wsl_session_file_list};
use super::launcher::WslLauncher;
use super::worker::{CommandOutput, WorkerOp, WslWorkerPool};

/// Docker Desktop 注册到 WSL 的内部发行版，不是用户可用的 CLI 目标。
pub const INTERNAL_WSL_DISTROS: [&str; 2] = ["docker-desktop", "docker-desktop-data"];

/// 适配器配置。默认值与现网逐条对齐。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WslConfig {
    /// 一次性执行的兜底超时（现网 60 s）。
    pub command_timeout: Duration,
    /// shell 命令输出上限（现网 16 MB）。
    pub shell_max_output_bytes: usize,
    /// 读取文件的输出上限（现网 32 MB）。
    pub read_file_max_output_bytes: usize,
    /// stderr 上限（现网 1 MB）。
    pub error_max_output_bytes: usize,
    /// 是否启用持久 worker。排障时可关掉以强制走一次性路径。
    pub use_worker: bool,
}

impl Default for WslConfig {
    fn default() -> Self {
        Self {
            command_timeout: Duration::from_secs(60),
            shell_max_output_bytes: 16 * 1024 * 1024,
            read_file_max_output_bytes: 32 * 1024 * 1024,
            error_max_output_bytes: 1024 * 1024,
            use_worker: true,
        }
    }
}

/// WSL 适配器。
#[derive(Clone)]
pub struct WslAdapter {
    launcher: Arc<dyn WslLauncher>,
    pool: WslWorkerPool,
    config: WslConfig,
}

impl std::fmt::Debug for WslAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WslAdapter")
            .field("launcher", &self.launcher.describe())
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl WslAdapter {
    /// 由 launcher 与配置构造。
    #[must_use]
    pub fn new(launcher: Arc<dyn WslLauncher>, config: WslConfig) -> Self {
        Self {
            pool: WslWorkerPool::new(Arc::clone(&launcher)),
            launcher,
            config,
        }
    }

    /// 使用默认配置构造。
    #[must_use]
    pub fn with_launcher(launcher: Arc<dyn WslLauncher>) -> Self {
        Self::new(launcher, WslConfig::default())
    }

    /// 配置。
    #[must_use]
    pub const fn config(&self) -> WslConfig {
        self.config
    }

    /// launcher 是否可用。
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.launcher.is_available()
    }

    /// 关闭全部 worker（应用退出时调用）。
    pub async fn shutdown(&self) {
        self.pool.shutdown_all().await;
    }

    /// 在发行版内启动任意程序（stdin/stdout/stderr 均接管）。
    ///
    /// 供 `codex app-server` 这类需要长期双向通信的进程使用。
    pub fn spawn_program(
        &self,
        distro: &str,
        program: &str,
        args: &[String],
    ) -> Result<tokio::process::Child> {
        self.launcher.spawn_program(distro, program, args)
    }

    // -----------------------------------------------------------------------
    // 发行版与探测
    // -----------------------------------------------------------------------

    /// 列出可用的 WSL 发行版（已排除 Docker Desktop 内部发行版）。
    ///
    /// 必须**直接**调用 `wsl.exe -l -q`：它列出的是发行版本身，不能在某个发行版内部
    /// 执行。输出是 UTF-16LE 且每行带一个 NUL，因此要拿**原始字节**去解码，
    /// 不能先转成 UTF-8 字符串。
    pub async fn list_distros(&self) -> Vec<String> {
        let child = match self.launcher.spawn_wsl_raw(&["-l", "-q"]) {
            Ok(child) => child,
            Err(error) => {
                tracing::debug!(%error, "无法启动 wsl.exe 列出发行版");
                return Vec::new();
            }
        };
        let Ok(raw) = self
            .collect_child(
                child,
                self.config.command_timeout,
                self.config.shell_max_output_bytes,
                "列出发行版",
            )
            .await
        else {
            // `wsl.exe -l -q` 失败时**必须留下原因**。
            //
            // 实测踩过：在受限令牌下 `wsl.exe` 返回
            // `Wsl/EnumerateDistros/Service/E_ACCESSDENIED`，而这里只是静默返回空列表
            // ——上层看到的是「没有 WSL 目标」，完全看不出是被拒绝访问。
            // 这类静默失败最容易浪费排查时间。
            tracing::warn!("wsl.exe -l -q 执行失败，WSL 目标将为空（可能是权限不足或 WSL 未启动）");
            return Vec::new();
        };
        // **退出码非零时绝不能解析输出**。
        //
        // 实测踩过：受限令牌下 `wsl.exe -l -q` 失败，把
        // `拒绝访问。` / `错误代码: Wsl/EnumerateDistros/Service/E_ACCESSDENIED`
        // 写到了输出里，而这里照常解析——于是上层拿到两个**名为错误信息的「发行版」**。
        //
        // 后果不只是列表难看：每个假发行版都会触发后续的 `probe_codex_home` 与
        // `exists` 调用，白白付出进程启动开销（实测每次数秒）。
        if raw.exit_code != 0 {
            let detail = String::from_utf8_lossy(&raw.stderr);
            let detail = detail.trim();
            tracing::warn!(
                exit_code = raw.exit_code,
                detail = if detail.is_empty() {
                    "（无 stderr）"
                } else {
                    detail
                },
                "wsl.exe -l -q 返回非零退出码，忽略其输出"
            );
            return Vec::new();
        }
        parse_distro_list(&decode_wsl_output(&raw.stdout))
    }

    /// 在发行版内执行一段 bash 脚本（一次性）。
    ///
    /// 脚本经 **base64 透传**：`wsl.exe` 转发参数时会对引号与反斜杠再解析，
    /// 含双引号与 `$(...)` 的脚本必须绕开这一层。
    pub async fn run_shell(&self, distro: &str, script: &str) -> Result<String> {
        let output = self.run_shell_raw(distro, script).await?;
        output
            .into_success(&format!("WSL 执行（{distro}）"))
            .map(|o| o.stdout)
    }

    /// 在发行版内执行 bash 脚本并返回原始输出（含退出码）。
    pub async fn run_shell_raw(&self, distro: &str, script: &str) -> Result<CommandOutput> {
        let wrapped = wrap_script_for_transport(script);
        let child = self.launcher.spawn_bash(distro, &wrapped)?;
        let raw = self
            .collect_child(
                child,
                self.config.command_timeout,
                self.config.shell_max_output_bytes,
                &format!("WSL 执行（{distro}）"),
            )
            .await?;
        Ok(raw.into_command_output())
    }

    /// 在与 CLI 终端一致的交互 shell 中执行探测，加载 `.bashrc` 中的 nvm 等配置。
    pub async fn run_cli_shell(&self, distro: &str, script: &str) -> Result<CommandOutput> {
        let wrapped = super::script_transfer::wrap_script_preserving_stdin(script);
        self.run_shell_raw(distro, &format!("bash -ic {}", posix_shell_quote(&wrapped)))
            .await
    }

    /// 判断发行版内是否存在某命令，使用与终端启动一致的 shell 环境。
    pub async fn command_exists(&self, distro: &str, command: &str) -> bool {
        let script = format!("command -v {} >/dev/null 2>&1", posix_shell_quote(command));
        self.run_cli_shell(distro, &script)
            .await
            .is_ok_and(|output| output.is_success())
    }

    /// 读取发行版内的环境变量。读取失败视为目标环境异常，向上抛错。
    pub async fn get_env(&self, distro: &str, name: &str) -> Result<String> {
        let script = format!("printenv {}", posix_shell_quote(name));
        self.run_shell(distro, &script)
            .await
            .map(|value| value.trim().to_owned())
    }

    /// 执行命令并取修剪后的 stdout；失败返回空串（探测性调用）。
    pub async fn get_text(&self, distro: &str, command: &str, args: &[String]) -> String {
        let mut script = posix_shell_quote(command);
        for argument in args {
            script.push(' ');
            script.push_str(&posix_shell_quote(argument));
        }
        self.run_shell(distro, &script)
            .await
            .map_or_else(|_| String::new(), |value| value.trim().to_owned())
    }

    /// 探测可达宿主网关的网络地址。
    ///
    /// 顺序与现网一致：mirrored/localhost 转发直连 → 默认路由网关 → `resolv.conf`
    /// nameserver → 回退 `127.0.0.1`。NAT 模式下 `127.0.0.1` 指向 WSL 自身，
    /// 必须改用默认路由网关地址。
    ///
    /// # 回退时要出声
    ///
    /// 探测失败与「mirrored 模式下 127.0.0.1 本来就对」**返回的是同一个值**。
    /// 不区分的话，NAT 模式下探测失败会表现为：终端正常启动、每次请求都连不上网关，
    /// 而日志里那句地址看起来完全正常——**只有一种目标会坏，且没有任何线索**。
    ///
    /// 实测确认过 mirrored 的情况：宿主开一个监听端口，发行版内
    /// `exec 3<>/dev/tcp/127.0.0.1/<port>` 直接连通，因此那条路径返回 `127.0.0.1` 是**对的**。
    pub async fn detect_gateway_host(&self, distro: &str, port: u16) -> String {
        const FALLBACK: &str = "127.0.0.1";
        let script = gateway_host_probe_script(port);
        // 探测本身有 5 s 上限，避免 WSL 网络异常时长时间阻塞。
        let outcome =
            tokio::time::timeout(Duration::from_secs(5), self.run_shell(distro, &script)).await;
        match outcome {
            Ok(Ok(text)) => {
                let found = text
                    .lines()
                    .rev()
                    .map(str::trim)
                    .find(|line| !line.is_empty());
                if let Some(host) = found {
                    return host.to_owned();
                }
                tracing::warn!(
                    distro,
                    port,
                    "宿主地址探测没有输出，回退到 127.0.0.1；\
                     若发行版是 NAT 网络，终端将连不上网关"
                );
                FALLBACK.to_owned()
            }
            // 探测失败仍然回退，不阻断终端启动——但必须让它可见。
            outcome => {
                tracing::warn!(
                    distro,
                    port,
                    failed = outcome.is_err() || outcome.is_ok_and(|inner| inner.is_err()),
                    "宿主地址探测失败，回退到 127.0.0.1；\
                     若发行版是 NAT 网络，终端将连不上网关"
                );
                FALLBACK.to_owned()
            }
        }
    }

    /// 探测发行版内的 Codex 配置目录。
    ///
    /// 顺序：`CODEX_HOME` → `$HOME/.codex` → `/home/*/.codex`。优先返回**含 `sessions`
    /// 子目录**的候选，其次返回存在的目录；都找不到返回空串。
    ///
    /// # 为什么是 `Result` 而不是 `Option`/空串
    ///
    /// 「发行版可达、但没有 Codex 目录」与「发行版**根本进不去**」是两件不同的事，
    /// 调用方必须能区分。
    ///
    /// 这里**曾经**有一个 `resolve_codex_home() -> String` 的宽松版本，它把执行失败折叠成
    /// 空串，于是两者不可分辨，界面就会写出「可进入 WSL，未找到 Codex 目录」——
    /// **把一个进不去的发行版描述成可以进入**，用户点下去只会收到失败。实测中确实存在
    /// 这种发行版：注册在册，`wsl.exe -d <distro>` 却返回
    /// `Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED`。
    ///
    /// 那个方法是**隐患本身**，已删除且不再保留——只要它还在，就会有人用它。
    ///
    /// # Errors
    ///
    /// 发行版不可进入时返回一句人能读的原因（已截断——完整错误里会带上整段脚本）。
    pub async fn probe_codex_home(&self, distro: &str) -> Result<String, String> {
        self.run_shell(distro, &codex_home_probe_script())
            .await
            .map(|value| value.trim().to_owned())
            .map_err(|error| aivc_core::text::clamp_text(&error.to_string(), 160))
    }

    // -----------------------------------------------------------------------
    // 文件操作（worker 优先，失败回退一次性）
    // -----------------------------------------------------------------------

    /// 读取整个文件。
    pub async fn read_text(&self, distro: &str, path: &str) -> Result<String> {
        let size = self.size(distro, path).await?;
        if size > self.config.read_file_max_output_bytes as u64 {
            return Err(WslError::FileTooLarge {
                limit: self.config.read_file_max_output_bytes,
                path: path.to_owned(),
            }
            .into());
        }
        self.call(
            distro,
            &WorkerOp::Read {
                path: path.to_owned(),
            },
        )
        .await?
        .into_success(&format!("读取 {path}"))
        .map(|output| output.stdout)
    }

    /// 读取文件头部至多 `max_bytes` 字节。
    pub async fn read_head(&self, distro: &str, path: &str, max_bytes: usize) -> Result<String> {
        self.call(
            distro,
            &WorkerOp::Head {
                path: path.to_owned(),
                max_bytes: max_bytes as u64,
            },
        )
        .await?
        .into_success(&format!("读取 {path} 头部"))
        .map(|output| output.stdout)
    }

    /// 一次性读取多个文件的头部。
    ///
    /// 这是列表路径的关键优化，理由见
    /// [`aivc_core::ports::SessionStore::read_heads`]：每个文件一次 `wsl.exe`，而列表是
    /// 6 路并发拉起的，那 6 个进程会**同时抢同一次 WSL 冷启动**（实测冷启动 5–7 秒，
    /// 争抢时单次可达 30 秒）。
    ///
    /// 这里按固定大小**分批**、**串行**发起：一次调用覆盖一批文件，既避免并发争抢，
    /// 又不会让单次输出撑爆 `shell_max_output_bytes`。
    ///
    /// 路径本身以 base64 放进引号 heredoc，**完全不经过 shell 解析**——含空格、引号、
    /// `$`、换行的路径都不会破坏脚本。
    ///
    /// 返回值与 `paths` 一一对应；读不到的位置是 [`None`]，向量长度绝不缩短。
    pub async fn read_heads(
        &self,
        distro: &str,
        paths: &[String],
        max_bytes: usize,
    ) -> Vec<Option<String>> {
        let mut heads: Vec<Option<String>> = Vec::with_capacity(paths.len());
        for chunk in paths.chunks(HEAD_BATCH_SIZE) {
            // 走 `call`：worker 优先、失败自动回退到一次性执行。
            //
            // 走 worker 之后，这一批不再需要启动一个 `wsl.exe` 进程，只是一次管道
            // 往返（实测从数百毫秒降到个位数毫秒）。
            let operation = WorkerOp::ReadHeads {
                paths: chunk.to_vec(),
                max_bytes: max_bytes as u64,
            };
            let failure = match self.call(distro, &operation).await {
                Ok(output) if output.is_success() => {
                    heads.extend(parse_head_batch(&output.stdout, chunk.len()));
                    continue;
                }
                Ok(output) => output.stderr.trim().to_owned(),
                Err(error) => error.to_string(),
            };
            tracing::warn!(count = chunk.len(), reason = %failure, "批量读取文件头失败");
            // **补齐长度**，否则调用方按下标对齐时会整体错位。
            heads.extend(std::iter::repeat_n(None, chunk.len()));
        }
        heads
    }

    /// 文件是否存在。
    pub async fn exists(&self, distro: &str, path: &str) -> bool {
        let Ok(output) = self
            .call(
                distro,
                &WorkerOp::Exists {
                    path: path.to_owned(),
                },
            )
            .await
        else {
            return false;
        };
        output.is_success() && output.stdout.trim() == "1"
    }

    /// 文件大小（字节）。
    pub async fn size(&self, distro: &str, path: &str) -> Result<u64> {
        let output = self
            .call(
                distro,
                &WorkerOp::Size {
                    path: path.to_owned(),
                },
            )
            .await?
            .into_success(&format!("读取 {path} 大小"))?;
        output.stdout.trim().parse::<u64>().map_err(|_| {
            WslError::CommandFailed {
                message: format!("无法解析文件大小：{path}"),
            }
            .into()
        })
    }

    /// 写入文件（自动创建父目录）。
    pub async fn write_text(&self, distro: &str, path: &str, content: &str) -> Result<()> {
        self.call(
            distro,
            &WorkerOp::Write {
                path: path.to_owned(),
                content: content.to_owned(),
            },
        )
        .await?
        .into_success(&format!("写入 {path}"))?;
        Ok(())
    }

    /// 移动文件。
    pub async fn move_file(
        &self,
        distro: &str,
        source: &str,
        destination: &str,
        fail_if_exists: bool,
    ) -> Result<()> {
        self.call(
            distro,
            &WorkerOp::Move {
                source: source.to_owned(),
                destination: destination.to_owned(),
                fail_if_exists,
            },
        )
        .await?
        .into_success(&format!("移动 {source}"))?;
        Ok(())
    }

    /// 删除文件。
    pub async fn remove(&self, distro: &str, path: &str) -> Result<()> {
        self.call(
            distro,
            &WorkerOp::Remove {
                path: path.to_owned(),
            },
        )
        .await?
        .into_success(&format!("删除 {path}"))?;
        Ok(())
    }

    /// 递归删除文件或目录树。
    ///
    /// `rm -f` 对目录无效，因此单独提供这一操作：Qoder 的一份会话含目录产物。
    pub async fn remove_tree(&self, distro: &str, path: &str) -> Result<()> {
        self.call(
            distro,
            &WorkerOp::RemoveTree {
                path: path.to_owned(),
            },
        )
        .await?
        .into_success(&format!("删除 {path}"))?;
        Ok(())
    }

    /// 递归列出会话文件（`find -printf` 格式）。
    pub async fn list_session_files(
        &self,
        distro: &str,
        root: &str,
        pattern: &str,
    ) -> Result<Vec<SessionFile>> {
        if !self.exists(distro, root).await {
            return Ok(Vec::new());
        }
        let output = self
            .call(
                distro,
                &WorkerOp::List {
                    root: root.to_owned(),
                    pattern: pattern.to_owned(),
                },
            )
            .await?
            .into_success(&format!("列出 {root}"))?;
        Ok(parse_wsl_session_file_list(&output.stdout))
    }

    /// 按固定深度列出会话文件。
    ///
    /// 某些平台的布局是固定的两层（`projects/<项目目录>/<文件>`），递归下去没有意义，
    /// 只会误收用户放在项目目录里的备份或附件。
    pub async fn list_session_files_at_depth(
        &self,
        distro: &str,
        root: &str,
        pattern: &str,
        min_depth: u32,
        max_depth: u32,
    ) -> Result<Vec<SessionFile>> {
        if !self.exists(distro, root).await {
            return Ok(Vec::new());
        }
        let script = format!(
            "find {} -mindepth {} -maxdepth {} -type f -name {} -printf '%p\\t%T@\\t%C@\\t%s\\n'",
            posix_shell_quote(root),
            min_depth,
            max_depth,
            posix_shell_quote(pattern)
        );
        let output = self.run_shell(distro, &script).await?;
        Ok(parse_wsl_session_file_list(&output))
    }

    /// 按**路径模式**列出会话文件。
    ///
    /// 与 [`WslAdapter::list_session_files_at_depth`] 的区别：深度固定但中间目录名不固定
    /// 时（如 `tmp/<项目标识>/chats/`），用路径模式比用深度更准确——项目标识是工作目录
    /// 的哈希，无法用 `-name` 表达。
    pub async fn list_session_files_matching(
        &self,
        distro: &str,
        root: &str,
        path_pattern: &str,
    ) -> Result<Vec<SessionFile>> {
        if !self.exists(distro, root).await {
            return Ok(Vec::new());
        }
        let script = format!(
            "find {} -path {} -type f -printf '%p\\t%T@\\t%C@\\t%s\\n'",
            posix_shell_quote(root),
            posix_shell_quote(path_pattern)
        );
        let output = self.run_shell(distro, &script).await?;
        Ok(parse_wsl_session_file_list(&output))
    }

    /// 流式逐行读取。
    ///
    /// **刻意不走 worker**：帧协议一次性返回整个文件，大 JSONL 会整体进内存。
    /// 这里用一次性进程流式读取 stdout，配合回调提前停止，内存占用与读取行数成正比
    /// 而不是与文件大小成正比。
    pub async fn read_lines(
        &self,
        distro: &str,
        path: &str,
        start: ReadStart,
        options: ReadOptions,
        sink: &mut (dyn FnMut(&str, u64, u64) -> LineControl + Send),
    ) -> Result<()> {
        // 字节偏移在远程无法 seek：只能退化为按行号读取。
        let script = match start.byte_offset() {
            Some(_) | None => {
                let first_line = start.line_number().max(1);
                if first_line > 1 {
                    format!("tail -n +{} -- {}", first_line, posix_shell_quote(path))
                } else {
                    format!("cat -- {}", posix_shell_quote(path))
                }
            }
        };
        let timeout = options
            .timeout_ms
            .map_or(self.config.command_timeout, Duration::from_millis);

        let mut child = self.launcher.spawn_bash(distro, &script)?;
        let Some(stdout) = child.stdout.take() else {
            return Err(WslError::WorkerProtocol {
                message: "无法接管子进程 stdout".to_owned(),
            }
            .into());
        };
        let mut lines = BufReader::new(stdout).lines();
        let mut line_number = start.line_number().max(1);
        let mut stopped_early = false;

        let read_all = async {
            while let Some(line) =
                lines
                    .next_line()
                    .await
                    .map_err(|error| WslError::CommandFailed {
                        message: format!("读取 {path} 失败：{error}"),
                    })?
            {
                // 字节偏移传 0（未知）：WSL 侧只能按行号读取（`tail -n +N`），拿不到
                // 绝对偏移。写入 0 让上层知道这个锚点不可用于 seek，只能按行号定位。
                if sink(&line, line_number, 0) == LineControl::Stop {
                    stopped_early = true;
                    break;
                }
                line_number += 1;
            }
            Ok::<(), aivc_core::Error>(())
        };

        if let Err(_elapsed) = tokio::time::timeout(timeout, read_all).await {
            let _ = child.start_kill();
            return Err(WslError::Timeout {
                timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
                label: format!("读取 {path}"),
            }
            .into());
        }

        if stopped_early {
            // 提前停止是预期行为，非零退出码不应被误报为读取失败。
            let _ = child.start_kill();
            return Ok(());
        }

        let status = child
            .wait()
            .await
            .map_err(|error| WslError::CommandFailed {
                message: format!("等待读取进程失败：{error}"),
            })?;
        if status.success() {
            return Ok(());
        }
        // 非零退出：把 stderr 作为诊断返回。
        let mut stderr = String::new();
        if let Some(pipe) = child.stderr.take() {
            let mut buffer = Vec::new();
            let _ = pipe
                .take(self.config.error_max_output_bytes as u64)
                .read_to_end(&mut buffer)
                .await;
            stderr = String::from_utf8_lossy(&buffer).trim().to_owned();
        }
        Err(WslError::CommandFailed {
            message: if stderr.is_empty() {
                format!("读取 {path} 失败，退出码 {}", status.code().unwrap_or(-1))
            } else {
                stderr
            },
        }
        .into())
    }

    // -----------------------------------------------------------------------
    // 内部：worker 调用与一次性回退
    // -----------------------------------------------------------------------

    /// 调用 worker；失败时回退到一次性执行。
    ///
    /// worker 只是加速手段：协议异常、进程退出、超时都必须降级，而不是让功能不可用。
    ///
    /// `should_try` 是**安静降级**的关键：某个发行版的 worker 刚失败过时直接走一次性
    /// 路径，既不重复付一次注定失败的启动，也不每次调用都刷一条告警。
    async fn call(&self, distro: &str, operation: &WorkerOp) -> Result<CommandOutput> {
        if self.config.use_worker && self.pool.should_try(distro).await {
            match self.pool.call(distro, operation).await {
                Ok(output) => return Ok(output),
                Err(error) => {
                    tracing::warn!(
                        distro,
                        operation = operation.name(),
                        %error,
                        "WSL worker 调用失败，回退到一次性执行"
                    );
                }
            }
        }
        self.run_once_fallback(distro, operation).await
    }

    /// 用一次性执行完成 worker 操作。
    async fn run_once_fallback(&self, distro: &str, operation: &WorkerOp) -> Result<CommandOutput> {
        self.run_shell_raw(distro, &fallback_script(operation))
            .await
    }

    /// 收集子进程输出，带超时与体积上限。
    ///
    /// 返回**原始字节**：`wsl -l -q` 的输出是 UTF-16LE，先转成 UTF-8 字符串会破坏它，
    /// 因此解码必须由调用方在拿到字节后进行。
    async fn collect_child(
        &self,
        mut child: tokio::process::Child,
        timeout: Duration,
        max_output: usize,
        label: &str,
    ) -> Result<RawOutput> {
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let error_limit = self.config.error_max_output_bytes;

        let collect = async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            if let Some(pipe) = stdout {
                let _ = pipe.take(max_output as u64).read_to_end(&mut out).await;
            }
            if let Some(pipe) = stderr {
                let _ = pipe.take(error_limit as u64).read_to_end(&mut err).await;
            }
            let status = child.wait().await;
            (out, err, status)
        };

        let (out, err, status) =
            tokio::time::timeout(timeout, collect)
                .await
                .map_err(|_| WslError::Timeout {
                    timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
                    label: label.to_owned(),
                })?;

        let status = status.map_err(|error| WslError::CommandFailed {
            message: format!("等待 WSL 进程失败：{error}"),
        })?;

        Ok(RawOutput {
            stdout: out,
            stderr: err,
            exit_code: status.code().unwrap_or(-1),
        })
    }
}

/// 子进程的原始输出。
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawOutput {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: i32,
}

impl RawOutput {
    /// 发行版输出为 UTF-8，WSL 系统错误可能为 UTF-16LE。
    fn into_command_output(self) -> CommandOutput {
        CommandOutput {
            stdout: decode_wsl_output(&self.stdout),
            stderr: decode_wsl_output(&self.stderr),
            exit_code: self.exit_code,
        }
    }
}

/// 把脚本包装成经 base64 透传的形式。
///
/// `wsl.exe` 转发参数时会对引号与反斜杠再解析，含双引号与 `$(...)` 的脚本必须绕开
/// 这一层：把脚本 base64 后由 `base64 -d` 解码再交给 bash。
#[must_use]
pub fn wrap_script_for_transport(script: &str) -> String {
    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(script.as_bytes());
    format!(
        "printf %s {} | base64 -d | bash",
        posix_shell_quote(&encoded)
    )
}

/// 解析 `wsl -l -q` 的输出为发行版列表。
#[must_use]
pub fn parse_distro_list(decoded: &str) -> Vec<String> {
    decoded
        .replace('\0', "")
        .split(['\n', '\r'])
        .map(|line| line.trim().trim_start_matches('*').trim().to_owned())
        .filter(|line| {
            !line.is_empty()
                && !INTERNAL_WSL_DISTROS
                    .iter()
                    .any(|internal| internal.eq_ignore_ascii_case(line))
        })
        .collect()
}

/// 宿主网关地址探测脚本。
#[must_use]
pub fn gateway_host_probe_script(port: u16) -> String {
    [
        "set -e".to_owned(),
        format!("port={port}"),
        // 1. mirrored 模式或 localhost 转发：127.0.0.1 直连。
        "if (exec 3<>/dev/tcp/127.0.0.1/$port) 2>/dev/null; then exec 3>&- 3<&-; echo 127.0.0.1; exit 0; fi"
            .to_owned(),
        // 2. NAT 模式：默认路由网关通常即宿主。
        "host=$(ip route show default 2>/dev/null | awk '{print $3; exit}')".to_owned(),
        "if [ -n \"$host\" ]; then echo \"$host\"; exit 0; fi".to_owned(),
        // 3. 退路：resolv.conf 的 nameserver。
        "awk '/^nameserver/{print $2; exit}' /etc/resolv.conf 2>/dev/null || echo 127.0.0.1".to_owned(),
    ]
    .join("\n")
}

/// Codex 配置目录探测脚本。
///
/// 优先返回含 `sessions` 子目录的候选，其次返回存在的目录。
#[must_use]
pub fn codex_home_probe_script() -> String {
    [
        "candidates=\"\"".to_owned(),
        "add() { [ -n \"$1\" ] && candidates=\"$candidates$1\n\"; }".to_owned(),
        "add \"$CODEX_HOME\"".to_owned(),
        "add \"$HOME/.codex\"".to_owned(),
        "for d in /home/*/.codex; do [ -d \"$d\" ] && add \"$d\"; done".to_owned(),
        "while IFS= read -r c; do [ -n \"$c\" ] && [ -d \"$c/sessions\" ] && { echo \"$c\"; exit 0; }; done <<EOF"
            .to_owned(),
        "$candidates".to_owned(),
        "EOF".to_owned(),
        "while IFS= read -r c; do [ -n \"$c\" ] && [ -d \"$c\" ] && { echo \"$c\"; exit 0; }; done <<EOF"
            .to_owned(),
        "$candidates".to_owned(),
        "EOF".to_owned(),
        "exit 0".to_owned(),
    ]
    .join("\n")
}

/// 一次批量读头调用覆盖的文件数。
///
/// 上限由 `shell_max_output_bytes` 反推：每个文件至多 `LIST_HEAD_BYTES`（64 KB），
/// base64 后约 1.37 倍，40 个文件约 3.5 MB，离默认 16 MB 有足够余量。
pub const HEAD_BATCH_SIZE: usize = 40;

/// 批量读头的脚本。
///
/// 输出每个文件一行：`ok <base64>` 或 `missing`。用前缀而不是「空行即失败」，
/// 是为了把**存在但为空**的文件（`ok ` 后为空串）与**不存在**区分开。
///
/// 路径以 base64 逐行写入引号 heredoc（`<<'AIVC_PATHS'`），因此路径里的任何字符都
/// 不会进入 shell 的解析——这正是这个项目里 WSL 脚本一贯的做法。
#[must_use]
pub fn read_heads_script(paths: &[String], max_bytes: usize) -> String {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut script = String::from("while IFS= read -r p; do\n");
    script.push_str("  [ -z \"$p\" ] && continue\n");
    script.push_str("  f=$(printf %s \"$p\" | base64 -d)\n");
    script.push_str("  if [ -f \"$f\" ]; then\n");
    script.push_str("    printf 'ok '; head -c ");
    script.push_str(&max_bytes.to_string());
    script.push_str(" -- \"$f\" | base64 | tr -d '\\n'\n");
    script.push_str("  else\n");
    script.push_str("    printf 'missing'\n");
    script.push_str("  fi\n");
    script.push_str("  printf '\\n'\n");
    script.push_str("done <<'AIVC_PATHS'\n");
    for path in paths {
        script.push_str(&engine.encode(path.as_bytes()));
        script.push('\n');
    }
    script.push_str("AIVC_PATHS\n");
    script
}

/// 解析批量读头的输出。
///
/// `expected` 是这一批请求的文件数。输出行数不足时**补 [`None`]**，多出的行忽略——
/// 补齐而不是截断，是为了让结果始终与请求一一对应。错位比缺失更危险：调用方会拿
/// A 文件的内容去配 B 文件的 mtime。
#[must_use]
pub fn parse_head_batch(stdout: &str, expected: usize) -> Vec<Option<String>> {
    use base64::Engine;
    let engine = base64::engine::general_purpose::STANDARD;
    let mut heads = Vec::with_capacity(expected);
    let mut lines = stdout.split('\n');
    for _ in 0..expected {
        let line = lines.next().unwrap_or_default().trim_end_matches('\r');
        // 只认 `ok ` 前缀：它同时说明「文件存在」与「后面是 base64」。
        let decoded = line.strip_prefix("ok ").and_then(|payload| {
            engine
                .decode(payload)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        });
        heads.push(decoded);
    }
    heads
}

/// 用一次性 shell 命令完成 worker 操作（回退路径）。
fn fallback_script(operation: &WorkerOp) -> String {
    match operation {
        WorkerOp::Read { path } => format!("cat -- {}", posix_shell_quote(path)),
        WorkerOp::Head { path, max_bytes } => {
            format!("head -c {max_bytes} -- {}", posix_shell_quote(path))
        }
        // 与 worker 分支**共用同一个脚本**：回退路径与 worker 路径必须产出同一种输出，
        // 否则「同一批文件走哪条路」会给出不同的结果，那是最难查的一类差异。
        WorkerOp::ReadHeads { paths, max_bytes } => {
            read_heads_script(paths, usize::try_from(*max_bytes).unwrap_or(usize::MAX))
        }
        WorkerOp::Write { path, content } => {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(content.as_bytes());
            format!(
                "mkdir -p {} && printf %s {} | base64 -d > {}",
                posix_shell_quote(&posix_dirname(path)),
                posix_shell_quote(&encoded),
                posix_shell_quote(path)
            )
        }
        WorkerOp::Size { path } => format!("stat -c '%s' -- {}", posix_shell_quote(path)),
        WorkerOp::Exists { path } => format!(
            "if [ -e {} ]; then echo 1; else echo 0; fi",
            posix_shell_quote(path)
        ),
        WorkerOp::List { root, pattern } => format!(
            "find {} -type f -name {} -printf '%p\\t%T@\\t%C@\\t%s\\n'",
            posix_shell_quote(root),
            posix_shell_quote(pattern)
        ),
        WorkerOp::Move {
            source,
            destination,
            fail_if_exists,
        } => {
            let guard = if *fail_if_exists {
                format!(
                    "if [ -e {} ]; then echo {} >&2; exit 17; fi; ",
                    posix_shell_quote(destination),
                    posix_shell_quote("原位置已存在同名会话文件，无法恢复。")
                )
            } else {
                String::new()
            };
            format!(
                "{guard}mkdir -p {} && mv -- {} {}",
                posix_shell_quote(&posix_dirname(destination)),
                posix_shell_quote(source),
                posix_shell_quote(destination)
            )
        }
        WorkerOp::Remove { path } => format!("rm -f -- {}", posix_shell_quote(path)),
        WorkerOp::RemoveTree { path } => format!("rm -rf -- {}", posix_shell_quote(path)),
        WorkerOp::Exec { program, args } => {
            let mut parts = vec![posix_shell_quote(program)];
            parts.extend(args.iter().map(|argument| posix_shell_quote(argument)));
            parts.join(" ")
        }
    }
}

/// POSIX 目录名（回退脚本里需要）。
fn posix_dirname(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_owned(),
        Some(index) => path[..index].to_owned(),
        None => ".".to_owned(),
    }
}

/// 面向 Provider 的 WSL 会话文件存储。
///
/// 只实现端口；目录布局、文件格式与分支规则仍由各 Provider 决定。
#[derive(Debug, Clone)]
pub struct WslSessionStore {
    adapter: WslAdapter,
    distro: String,
}

impl WslSessionStore {
    /// 构造。
    #[must_use]
    pub fn new(adapter: WslAdapter, distro: impl Into<String>) -> Self {
        Self {
            adapter,
            distro: distro.into(),
        }
    }

    /// 目标发行版。
    #[must_use]
    pub fn distro(&self) -> &str {
        &self.distro
    }

    /// 用于缓存文件名的安全发行版名。
    #[must_use]
    pub fn safe_distro(&self) -> String {
        sanitize_wsl_distro(&self.distro)
    }
}

impl SessionStore for WslSessionStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Wsl
    }

    fn read_text<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move { self.adapter.read_text(&self.distro, path).await })
    }

    fn write_text<'a>(&'a self, path: &'a str, content: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.adapter.write_text(&self.distro, path, content).await })
    }

    fn read_lines<'a>(
        &'a self,
        path: &'a str,
        start: ReadStart,
        options: ReadOptions,
        mut sink: LineSink<'a>,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.adapter
                .read_lines(&self.distro, path, start, options, sink.as_mut())
                .await
        })
    }

    fn read_head<'a>(&'a self, path: &'a str, max_bytes: usize) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move { self.adapter.read_head(&self.distro, path, max_bytes).await })
    }

    /// **覆盖默认实现**：默认实现会为每个文件启动一个 `wsl.exe`，而列表是 6 路并发
    /// 拉起的——那 6 个进程会同时抢 WSL 的冷启动（实测单次 5–7 秒，争抢时可达 30 秒）。
    /// 这里改成一次调用读一批。
    fn read_heads<'a>(
        &'a self,
        paths: &'a [String],
        max_bytes: usize,
    ) -> BoxFuture<'a, Vec<Option<String>>> {
        Box::pin(async move {
            self.adapter
                .read_heads(&self.distro, paths, max_bytes)
                .await
        })
    }

    fn exists<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move { Ok(self.adapter.exists(&self.distro, path).await) })
    }

    fn size<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<u64>> {
        Box::pin(async move { self.adapter.size(&self.distro, path).await })
    }

    fn move_file<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
        policy: ConflictPolicy,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.adapter
                .move_file(
                    &self.distro,
                    from,
                    to,
                    policy == ConflictPolicy::FailIfExists,
                )
                .await
        })
    }

    fn remove<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.adapter.remove(&self.distro, path).await })
    }

    fn remove_tree<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move { self.adapter.remove_tree(&self.distro, path).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsl::launcher::LocalBashLauncher;

    #[test]
    fn wsl_system_errors_are_decoded_and_preserved() {
        let message = "拒绝访问：Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED";
        let raw = RawOutput {
            stdout: message.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            stderr: Vec::new(),
            exit_code: -1,
        };
        let error = raw
            .into_command_output()
            .into_success("探测")
            .expect_err("系统错误");
        assert_eq!(error.to_string(), message);
    }

    #[test]
    fn default_config_matches_the_existing_limits() {
        let config = WslConfig::default();
        assert_eq!(config.command_timeout, Duration::from_secs(60));
        assert_eq!(config.shell_max_output_bytes, 16 * 1024 * 1024);
        assert_eq!(config.read_file_max_output_bytes, 32 * 1024 * 1024);
        assert_eq!(config.error_max_output_bytes, 1024 * 1024);
        assert!(config.use_worker);
    }

    #[test]
    fn distro_list_parsing_strips_noise_and_internal_distros() {
        // `wsl -l -q` 的输出：每行带 NUL，默认发行版带 `*` 前缀。
        let decoded = "Ubuntu\0\r\n*Debian\0\r\ndocker-desktop\0\r\nDOCKER-DESKTOP-DATA\0\r\n\r\n";
        assert_eq!(parse_distro_list(decoded), vec!["Ubuntu", "Debian"]);
        assert!(parse_distro_list("").is_empty());
        assert!(parse_distro_list("\0\r\n").is_empty());
    }

    #[test]
    fn head_batch_script_never_embeds_raw_paths() {
        use base64::Engine;
        // **注入安全**：路径只以 base64 出现在引号 heredoc 里，任何字符都不会进入
        // shell 解析。含引号、`$`、反引号、换行的路径都不能出现在脚本正文里。
        let nasty = "a'b\"c$HOME`id`\nd/e.jsonl";
        let script = read_heads_script(&[nasty.to_owned()], 4096);
        assert!(
            !script.contains("$HOME") && !script.contains("c$"),
            "原始路径泄漏进了脚本：{script}"
        );
        assert!(!script.contains("a'b"), "引号没有被 base64 吞掉：{script}");
        assert!(script.starts_with("while IFS= read -r p; do"));
        assert!(script.contains("<<'AIVC_PATHS'"));
        assert!(script.trim_end().ends_with("AIVC_PATHS"));
        // 路径本身可还原。
        let encoded = base64::engine::general_purpose::STANDARD.encode(nasty);
        assert!(script.contains(&encoded), "路径必须以 base64 形式出现");
    }

    #[test]
    fn head_batch_parsing_keeps_one_slot_per_requested_file() {
        use base64::Engine;
        let engine = base64::engine::general_purpose::STANDARD;
        let first = engine.encode(b"{\"type\":\"session_meta\"}");
        // 存在但为空的文件：`ok ` 后面是空的 base64，必须解析成 `Some("")`，
        // 而不是与「不存在」混为一谈。
        let stdout = format!("ok {first}\nok \nmissing\n");
        let heads = parse_head_batch(&stdout, 3);
        assert_eq!(heads.len(), 3);
        assert_eq!(heads[0].as_deref(), Some("{\"type\":\"session_meta\"}"));
        assert_eq!(heads[1].as_deref(), Some(""));
        assert_eq!(heads[2], None);
    }

    #[test]
    fn head_batch_parsing_pads_instead_of_truncating() {
        // 输出比请求少（例如脚本被中途截断）时必须**补齐**：调用方按下标把内容与
        // 文件元数据对齐，短一个就会整体错位——那比缺失危险得多。
        let heads = parse_head_batch("missing\n", 3);
        assert_eq!(heads, vec![None, None, None]);
        // 输出比请求多则忽略多余的。
        let heads = parse_head_batch("missing\nmissing\nmissing\nmissing\n", 2);
        assert_eq!(heads.len(), 2);
    }

    #[test]
    fn script_transport_wrapping_is_base64_passthrough() {
        let wrapped = wrap_script_for_transport("echo \"hi\" $(whoami)");
        assert!(wrapped.starts_with("printf %s '"));
        assert!(wrapped.ends_with("| base64 -d | bash"));
        // 脚本本体不得以明文出现：这正是绕开 wsl.exe 再解析的关键。
        assert!(!wrapped.contains("whoami"));
    }

    #[test]
    fn gateway_probe_script_keeps_the_existing_fallback_order() {
        let script = gateway_host_probe_script(18_080);
        let dev_tcp = script
            .find("/dev/tcp/127.0.0.1/$port")
            .expect("dev/tcp 探测");
        let route = script.find("ip route show default").expect("默认路由探测");
        let resolv = script.find("/etc/resolv.conf").expect("resolv.conf 回退");
        // 顺序必须是「直连 → 默认路由 → resolv.conf」。
        assert!(dev_tcp < route && route < resolv);
        assert!(script.contains("port=18080"));
        // 最终兜底必须是 127.0.0.1。
        assert!(script.contains("|| echo 127.0.0.1"));
    }

    #[test]
    fn fallback_scripts_quote_every_path() {
        // 含空格与单引号的路径必须被正确引用，否则回退路径会注入命令。
        let script = fallback_script(&WorkerOp::Remove {
            path: "/tmp/it's a file.jsonl".to_owned(),
        });
        assert!(script.contains("'/tmp/it'\\''s a file.jsonl'"), "{script}");

        let script = fallback_script(&WorkerOp::List {
            root: "/a b".to_owned(),
            pattern: "*.jsonl".to_owned(),
        });
        assert!(script.contains("'/a b'"));
        assert!(script.contains("'*.jsonl'"));
    }

    #[test]
    fn fallback_move_script_includes_the_conflict_guard() {
        let guarded = fallback_script(&WorkerOp::Move {
            source: "/a".to_owned(),
            destination: "/b".to_owned(),
            fail_if_exists: true,
        });
        assert!(guarded.contains("exit 17"));
        assert!(guarded.contains("原位置已存在同名会话文件"));

        let open = fallback_script(&WorkerOp::Move {
            source: "/a".to_owned(),
            destination: "/b".to_owned(),
            fail_if_exists: false,
        });
        assert!(!open.contains("exit 17"));
    }

    #[test]
    fn posix_dirname_handles_root_and_bare_names() {
        assert_eq!(posix_dirname("/a/b/c.jsonl"), "/a/b");
        assert_eq!(posix_dirname("/a"), "/");
        assert_eq!(posix_dirname("bare.jsonl"), ".");
    }

    #[test]
    fn store_reports_wsl_kind_and_safe_distro_name() {
        let launcher = LocalBashLauncher::new("bash");
        let store =
            WslSessionStore::new(WslAdapter::with_launcher(Arc::new(launcher)), "Ubuntu/../x");
        assert_eq!(store.kind(), StoreKind::Wsl);
        assert!(store.kind().is_wsl());
        assert_eq!(store.distro(), "Ubuntu/../x");
        // 发行版名进入缓存文件名前必须净化。
        assert_eq!(store.safe_distro(), "Ubuntu_.._x");
    }

    // 以下用例需要能运行的 bash；受限沙箱下自动跳过。

    fn adapter() -> Option<WslAdapter> {
        let launcher = LocalBashLauncher::detect().filter(LocalBashLauncher::is_usable)?;
        Some(WslAdapter::with_launcher(Arc::new(launcher)))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_round_trips_files_through_the_worker() {
        let Some(adapter) = adapter() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.jsonl");
        let path_str = path.to_string_lossy().into_owned();

        adapter
            .write_text("test", &path_str, "one\ntwo\n")
            .await
            .expect("write");
        assert!(adapter.exists("test", &path_str).await);
        assert_eq!(adapter.size("test", &path_str).await.expect("size"), 8);
        assert_eq!(
            adapter.read_text("test", &path_str).await.expect("read"),
            "one\ntwo\n"
        );
        assert_eq!(
            adapter.read_head("test", &path_str, 3).await.expect("head"),
            "one"
        );

        adapter.remove("test", &path_str).await.expect("remove");
        assert!(!adapter.exists("test", &path_str).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_reads_lines_streaming_with_absolute_numbers() {
        let Some(adapter) = adapter() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        adapter
            .write_text("test", &path_str, "one\ntwo\nthree\n")
            .await
            .expect("write");

        let mut seen = Vec::new();
        adapter
            .read_lines(
                "test",
                &path_str,
                ReadStart::Line(2),
                ReadOptions::none(),
                &mut |line: &str, number: u64, _offset: u64| {
                    seen.push((number, line.to_owned()));
                    LineControl::Continue
                },
            )
            .await
            .expect("read lines");
        assert_eq!(seen, vec![(2, "two".to_owned()), (3, "three".to_owned())]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_read_lines_can_stop_early_without_reporting_an_error() {
        let Some(adapter) = adapter() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        adapter
            .write_text("test", &path_str, "one\ntwo\nthree\n")
            .await
            .expect("write");

        let mut count = 0u64;
        adapter
            .read_lines(
                "test",
                &path_str,
                ReadStart::BEGINNING,
                ReadOptions::none(),
                &mut |_: &str, _: u64, _: u64| {
                    count += 1;
                    LineControl::Stop
                },
            )
            .await
            .expect("提前停止不得报错");
        assert_eq!(count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_reports_missing_files_with_stderr_detail() {
        let Some(adapter) = adapter() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir
            .path()
            .join("missing.jsonl")
            .to_string_lossy()
            .into_owned();

        let outcome = adapter.read_text("test", &missing).await;
        assert!(outcome.is_err());
        assert!(!adapter.exists("test", &missing).await);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_shell_and_probes_work() {
        let Some(adapter) = adapter() else {
            return;
        };
        assert_eq!(
            adapter
                .run_shell("test", "printf 'ok'")
                .await
                .expect("shell"),
            "ok"
        );
        // 含双引号与命令替换的脚本必须能正确透传。
        assert_eq!(
            adapter
                .run_shell("test", "printf \"%s\" \"$(printf nested)\"")
                .await
                .expect("nested"),
            "nested"
        );
        assert!(adapter.command_exists("test", "printf").await);
        assert!(
            !adapter
                .command_exists("test", "definitely-not-a-command")
                .await
        );
        assert!(
            !adapter
                .get_env("test", "HOME")
                .await
                .expect("env")
                .is_empty()
        );
        assert_eq!(
            adapter.get_text("test", "printf", &["hi".to_owned()]).await,
            "hi"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_falls_back_to_one_shot_when_the_worker_is_disabled() {
        // 关掉 worker 后全部操作都必须仍然可用——这正是降级路径的意义。
        let Some(launcher) = LocalBashLauncher::detect().filter(LocalBashLauncher::is_usable)
        else {
            return;
        };
        let config = WslConfig {
            use_worker: false,
            ..WslConfig::default()
        };
        let adapter = WslAdapter::new(Arc::new(launcher), config);

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("a.jsonl");
        let path_str = path.to_string_lossy().into_owned();

        adapter
            .write_text("test", &path_str, "content")
            .await
            .expect("write");
        assert!(adapter.exists("test", &path_str).await);
        assert_eq!(adapter.size("test", &path_str).await.expect("size"), 7);
        assert_eq!(
            adapter.read_text("test", &path_str).await.expect("read"),
            "content"
        );

        // 移动的冲突保护在回退路径上同样生效。
        let target = dir.path().join("b.jsonl");
        let target_str = target.to_string_lossy().into_owned();
        adapter
            .write_text("test", &target_str, "other")
            .await
            .expect("write target");
        let outcome = adapter
            .move_file("test", &path_str, &target_str, true)
            .await;
        assert!(outcome.is_err());
        assert!(
            outcome
                .expect_err("必须拒绝")
                .to_string()
                .contains("无法恢复")
        );

        assert_eq!(
            adapter
                .list_session_files("test", dir.path().to_str().expect("utf8"), "*.jsonl")
                .await
                .expect("list")
                .len(),
            2
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn adapter_gateway_detection_returns_a_non_empty_host() {
        let Some(adapter) = adapter() else {
            return;
        };
        // 本机 bash 没有 /dev/tcp 与 ip 命令时也必须回退到 127.0.0.1，而不是空串。
        let host = adapter.detect_gateway_host("test", 18_080).await;
        assert!(!host.trim().is_empty(), "网关地址不得为空");
    }
}
