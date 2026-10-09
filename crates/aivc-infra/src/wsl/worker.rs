//! WSL 持久 worker：长度前缀帧协议与客户端。
//!
//! # 为什么要常驻进程
//!
//! 现网每次 WSL 文件操作都拉起一次 `wsl.exe`，单次固定开销 50–150 ms（进程创建 +
//! WSL 服务往返 + base64 脚本包装）。会话列表要遍历数百个文件时，这个开销线性放大。
//! 常驻 worker 把每次操作降到一次 stdin/stdout 往返（约 1–3 ms）。
//!
//! # 协议
//!
//! 文本行帧，字段用制表符分隔，二进制安全的字段一律 base64：
//!
//! ```text
//! 请求  <id>\t<op>\t<b64-arg1>\t<b64-arg2>...
//! 响应  <id>\t<exit_code>\t<b64-stdout>\t<b64-stderr>
//! ```
//!
//! 选文本行而不是长度前缀二进制帧，是因为 worker 由 **bash** 实现，而 bash 无法方便地
//! 读取二进制长度头。制表符分隔 + base64 让 bash 用一次 `read` 就能解析，同时保持
//! 二进制安全（base64 不含制表符与换行）。
//!
//! # 为什么每个操作在子 shell 中执行
//!
//! worker 是**无状态**的：每个请求都在独立的命令替换子 shell 里执行，不 `cd`、不设
//! 变量，因此一个请求不会污染下一个。这也让 worker 崩溃的影响面收敛为「重连即可」。
//!
//! # 降级
//!
//! 任一环节失败（worker 启动失败、协议超时、进程退出）都由上层回退到一次性
//! `wsl.exe` 执行，并通过 [`WslWorkerPool`] 丢弃坏掉的 worker。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use aivc_core::error::{Result, WslError};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use super::launcher::WslLauncher;

/// 单次 worker 调用的默认超时。
///
/// 与现网一次性执行的兜底超时一致（60 s）。
pub const WORKER_CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// 命令执行结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// 标准输出（已按 UTF-8 解码）。
    pub stdout: String,
    /// 标准错误（已按 UTF-8 解码）。
    pub stderr: String,
    /// 退出码。
    pub exit_code: i32,
}

impl CommandOutput {
    /// 退出码是否为 0。
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.exit_code == 0
    }

    /// 失败时构造错误；成功时返回自身。
    pub fn into_success(self, label: &str) -> Result<Self> {
        if self.is_success() {
            return Ok(self);
        }
        let detail = if self.stderr.trim().is_empty() {
            self.stdout.trim()
        } else {
            self.stderr.trim()
        };
        Err(WslError::CommandFailed {
            message: if detail.is_empty() {
                format!("{label} 失败，退出码 {}", self.exit_code)
            } else {
                detail.to_owned()
            },
        }
        .into())
    }
}

/// worker 支持的操做。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerOp {
    /// 读取整个文件。
    Read {
        /// 文件路径。
        path: String,
    },
    /// 读取文件头部至多 N 字节。
    Head {
        /// 文件路径。
        path: String,
        /// 字节上限。
        max_bytes: u64,
    },
    /// **一次调用**读取多个文件的头部。
    ///
    /// 列表路径的关键操作：每个文件一次调用会让 6 路并发一起去抢 WSL 的冷启动
    /// （见 `WslAdapter::read_heads`）。走 worker 之后它还多一层好处——不再需要为
    /// 每一批启动一个 `wsl.exe` 进程，只是一次管道往返。
    ReadHeads {
        /// 文件路径列表；返回值与它**一一对应**。
        paths: Vec<String>,
        /// 每个文件的字节上限。
        max_bytes: u64,
    },
    /// 写入文件（自动创建父目录）。
    Write {
        /// 文件路径。
        path: String,
        /// 内容。
        content: String,
    },
    /// 读取文件大小。
    Size {
        /// 文件路径。
        path: String,
    },
    /// 判断文件是否存在。
    Exists {
        /// 文件路径。
        path: String,
    },
    /// 用 `find -printf` 列出匹配文件。
    List {
        /// 搜索根目录。
        root: String,
        /// 文件名模式（如 `rollout-*.jsonl`）。
        pattern: String,
    },
    /// 移动文件。
    Move {
        /// 源路径。
        source: String,
        /// 目标路径。
        destination: String,
        /// 目标已存在时是否拒绝。
        fail_if_exists: bool,
    },
    /// 删除文件。
    Remove {
        /// 文件路径。
        path: String,
    },
    /// 递归删除文件或目录树。
    RemoveTree {
        /// 文件或目录路径。
        path: String,
    },
    /// 执行任意命令。
    Exec {
        /// 程序名或绝对路径。
        program: String,
        /// 参数列表。
        args: Vec<String>,
    },
}

impl WorkerOp {
    /// 协议中的操作名。
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Read { .. } => "read",
            Self::Head { .. } => "head",
            Self::ReadHeads { .. } => "read_heads",
            Self::Write { .. } => "write",
            Self::Size { .. } => "size",
            Self::Exists { .. } => "exists",
            Self::List { .. } => "list",
            Self::Move { .. } => "move",
            Self::Remove { .. } => "remove",
            Self::RemoveTree { .. } => "remove_tree",
            Self::Exec { .. } => "exec",
        }
    }

    /// 协议参数（已 base64 编码）。
    fn arguments(&self) -> Vec<String> {
        let encode = |value: &str| BASE64.encode(value.as_bytes());
        match self {
            Self::Head { path, max_bytes } => vec![encode(path), max_bytes.to_string()],
            Self::Write { path, content } => vec![encode(path), encode(content)],
            // 单路径操作共用同一组参数。
            Self::Read { path }
            | Self::Size { path }
            | Self::Exists { path }
            | Self::Remove { path }
            | Self::RemoveTree { path } => vec![encode(path)],
            Self::List { root, pattern } => vec![encode(root), encode(pattern)],
            Self::Move {
                source,
                destination,
                fail_if_exists,
            } => vec![
                encode(source),
                encode(destination),
                if *fail_if_exists { "1" } else { "0" }.to_owned(),
            ],
            Self::Exec { program, args } => {
                vec![encode(program), encode(&nul_join(args))]
            }
            Self::ReadHeads { paths, max_bytes } => {
                vec![encode(&nul_join(paths)), max_bytes.to_string()]
            }
        }
    }
}

/// 用 NUL 连接多个字符串，并**以 NUL 结尾**。
///
/// 结尾的 NUL 是必需的：bash 的 `read -d ''` 在缺少结尾分隔符时会丢弃最后一段。
/// NUL 不可能出现在路径或命令行参数里，因此它是安全的天然分隔符。
fn nul_join(items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("{}\0", items.join("\0"))
    }
}

/// 常驻 worker 脚本。
///
/// 每个请求在命令替换子 shell 中执行，因此 worker 自身无状态。stderr 先落到临时文件
/// 再读取，避免命令替换同时捕获两个流时的顺序问题。
const WORKER_SCRIPT: &str = r#"
set -o pipefail
ERRFILE="${TMPDIR:-/tmp}/aivc-wsl-worker-$$.err"
# `read_heads` 的中间文件；与 ERRFILE 一起在退出时清理。
HEADFILE="${TMPDIR:-/tmp}/aivc-wsl-worker-$$.heads"
trap 'rm -f "$ERRFILE" "$HEADFILE"' EXIT

# BSD/macOS 的 base64 没有 -w0，回退到默认换行输出后由 tr 去掉换行。
b64() { if base64 -w0 </dev/null >/dev/null 2>&1; then base64 -w0; else base64 | tr -d '\n'; fi; }
enc() { printf '%s' "$1" | b64; }
dec() { printf '%s' "$1" | base64 -d; }
emit() { printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4"; }
errout() { if [ -s "$ERRFILE" ]; then b64 < "$ERRFILE"; else printf ''; fi; }

while IFS=$'\t' read -r id op a1 a2 a3; do
  [ -z "$id" ] && continue
  case "$op" in
    read)
      p=$(dec "$a1")
      # `2>` 必须写在 `<` 之前：重定向从左到右生效；若 `<` 先失败，报错会落在
      # worker 自己的 stderr 上而不是 ERRFILE，调用方就看不到失败原因。
      out=$(b64 2>"$ERRFILE" < "$p"); code=$?
      emit "$id" "$code" "$out" "$(errout)"
      ;;
    head)
      p=$(dec "$a1"); n="$a2"
      out=$(head -c "$n" "$p" 2>"$ERRFILE" | b64); code=$?
      emit "$id" "$code" "$out" "$(errout)"
      ;;
    read_heads)
      # 一次调用读多个文件的头部，每个文件输出一行：`ok <base64>` 或 `missing`。
      #
      # **先落到文件再整体编码**：用 shell 变量逐行累加，在 40 个文件、每个至多
      # 64 KB 时会出现二次方级的复制开销。
      #
      # 注意结果**必须装进响应帧**，不能直接写到 stdout——worker 的 stdout 就是
      # 协议通道，多写的每一行都会被客户端当成一个响应帧。
      n="$a2"
      : > "$HEADFILE"
      while IFS= read -r -d '' p; do
        if [ -f "$p" ]; then
          printf 'ok ' >> "$HEADFILE"
          head -c "$n" "$p" 2>>"$ERRFILE" | b64 >> "$HEADFILE"
        else
          printf 'missing' >> "$HEADFILE"
        fi
        printf '\n' >> "$HEADFILE"
      done < <(printf '%s' "$a1" | base64 -d)
      emit "$id" 0 "$(b64 < "$HEADFILE")" "$(errout)"
      ;;
    write)
      p=$(dec "$a1")
      mkdir -p "$(dirname "$p")" 2>"$ERRFILE" && dec "$a2" 2>>"$ERRFILE" > "$p"; code=$?
      emit "$id" "$code" "" "$(errout)"
      ;;
    size)
      p=$(dec "$a1")
      out=$(stat -c '%s' "$p" 2>"$ERRFILE" | b64); code=$?
      emit "$id" "$code" "$out" "$(errout)"
      ;;
    exists)
      p=$(dec "$a1")
      if [ -e "$p" ]; then emit "$id" 0 "$(enc 1)" ""; else emit "$id" 0 "$(enc 0)" ""; fi
      ;;
    list)
      root=$(dec "$a1"); pattern=$(dec "$a2")
      out=$(find "$root" -type f -name "$pattern" -printf '%p\t%T@\t%C@\t%s\n' 2>"$ERRFILE" | b64); code=$?
      emit "$id" "$code" "$out" "$(errout)"
      ;;
    move)
      s=$(dec "$a1"); d=$(dec "$a2"); guard="$a3"
      if [ "$guard" = "1" ] && [ -e "$d" ]; then
        printf '%s' '原位置已存在同名会话文件，无法恢复。' > "$ERRFILE"
        emit "$id" 17 "" "$(errout)"
      else
        mkdir -p "$(dirname "$d")" 2>"$ERRFILE" && mv -- "$s" "$d" 2>>"$ERRFILE"; code=$?
        emit "$id" "$code" "" "$(errout)"
      fi
      ;;
    remove)
      p=$(dec "$a1")
      rm -f -- "$p" 2>"$ERRFILE"; code=$?
      emit "$id" "$code" "" "$(errout)"
      ;;
    remove_tree)
      p=$(dec "$a1")
      rm -rf -- "$p" 2>"$ERRFILE"; code=$?
      emit "$id" "$code" "" "$(errout)"
      ;;
    exec)
      # 参数以 NUL 分隔后整体 base64，但**必须**直接从 `base64 -d` 的进程替换读取：
      # bash 的命令替换会丢弃 NUL 字节（`ignored null byte in input`），
      # 先存进变量会把所有参数粘成一个。blob 以 NUL 结尾，否则 `read -d ''`
      # 会因为缺少结尾分隔符而丢弃最后一段。
      prog=$(dec "$a1")
      args=()
      while IFS= read -r -d '' arg; do args+=("$arg"); done < <(printf '%s' "$a2" | base64 -d)
      out=$("$prog" "${args[@]}" 2>"$ERRFILE" | b64); code=$?
      emit "$id" "$code" "$out" "$(errout)"
      ;;
    *)
      printf '%s' "unknown operation: $op" > "$ERRFILE"
      emit "$id" 2 "" "$(errout)"
      ;;
  esac
done
"#;

/// 一个常驻 worker 连接。
#[derive(Debug)]
pub struct WslWorker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    distro: String,
}

impl WslWorker {
    /// 启动 worker。
    pub fn spawn(launcher: &dyn WslLauncher, distro: &str) -> Result<Self> {
        // **必须是 `spawn_bash_interactive`**。
        //
        // 这里曾经用 `spawn_bash`，而它在生产实现（`WslExeLauncher`）里把 stdin 设成了
        // null——于是 `child.stdin` 是 `None`，worker **永远起不来**，每次调用都退回
        // 一次性执行（比不开 worker 还慢，因为多付一次进程创建）。而测试用的是
        // `LocalBashLauncher`，它当时恰好是 piped，所以这个缺陷在测试里看不见。
        //
        // 现在两个实现都按同一份契约分成两个方法，缺陷不会再有藏身之处。
        let mut child = launcher.spawn_bash_interactive(distro, WORKER_SCRIPT)?;
        let stdin = child.stdin.take().ok_or_else(|| WslError::WorkerProtocol {
            message: format!(
                "worker 的 stdin 不可用（launcher: {}）——\
                 `spawn_bash_interactive` 的契约要求 stdin 是管道",
                launcher.describe()
            ),
        })?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WslError::WorkerProtocol {
                message: "worker 的 stdout 不可用".to_owned(),
            })?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            distro: distro.to_owned(),
        })
    }

    /// 目标发行版。
    #[must_use]
    pub fn distro(&self) -> &str {
        &self.distro
    }

    /// 发起一次调用。
    pub async fn call(&mut self, operation: &WorkerOp) -> Result<CommandOutput> {
        self.call_with_timeout(operation, WORKER_CALL_TIMEOUT).await
    }

    /// 发起一次调用并指定超时。
    pub async fn call_with_timeout(
        &mut self,
        operation: &WorkerOp,
        timeout: Duration,
    ) -> Result<CommandOutput> {
        let id = self.next_id;
        self.next_id += 1;

        let mut frame = format!("{id}\t{}", operation.name());
        for argument in operation.arguments() {
            frame.push('\t');
            frame.push_str(&argument);
        }
        frame.push('\n');

        let write = async {
            self.stdin.write_all(frame.as_bytes()).await?;
            self.stdin.flush().await
        };
        tokio::time::timeout(timeout, write)
            .await
            .map_err(|_| WslError::Timeout {
                timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
                label: format!("worker 写入（{}）", self.distro),
            })?
            .map_err(|error| WslError::WorkerProtocol {
                message: format!("worker 写入失败：{error}"),
            })?;

        let mut line = String::new();
        let read = self.stdout.read_line(&mut line);
        let read = tokio::time::timeout(timeout, read)
            .await
            .map_err(|_| WslError::Timeout {
                timeout_ms: timeout.as_millis().try_into().unwrap_or(u64::MAX),
                label: format!("worker 响应（{}）", self.distro),
            })?
            .map_err(|error| WslError::WorkerProtocol {
                message: format!("worker 读取失败：{error}"),
            })?;

        if read == 0 {
            return Err(WslError::WorkerProtocol {
                message: format!("worker 已退出（{}）", self.distro),
            }
            .into());
        }

        parse_response(&line, id)
    }

    /// 关闭 worker。
    pub async fn shutdown(mut self) {
        // 关掉 stdin 会让 bash 的 `read` 返回非零并退出循环。
        drop(self.stdin);
        let _ = tokio::time::timeout(Duration::from_secs(3), self.child.wait()).await;
        let _ = self.child.start_kill();
    }

    /// 该 worker 的进程是否仍在运行。
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

/// 解析一行响应。
fn parse_response(line: &str, expected_id: u64) -> Result<CommandOutput> {
    let line = line.trim_end_matches(['\n', '\r']);
    let mut columns = line.split('\t');
    let id = columns.next().unwrap_or_default();
    let exit_code = columns.next().unwrap_or_default();
    let stdout = columns.next().unwrap_or_default();
    let stderr = columns.next().unwrap_or_default();

    let parsed_id: u64 = id.parse().map_err(|_| WslError::WorkerProtocol {
        message: format!("响应缺少有效编号：{line}"),
    })?;
    if parsed_id != expected_id {
        // 编号错位说明协议已经不可信，必须丢弃该 worker 而不是继续用。
        return Err(WslError::WorkerProtocol {
            message: format!("响应编号错位：期望 {expected_id}，收到 {parsed_id}"),
        }
        .into());
    }
    let exit_code: i32 = exit_code.parse().map_err(|_| WslError::WorkerProtocol {
        message: format!("响应缺少有效退出码：{line}"),
    })?;

    Ok(CommandOutput {
        stdout: decode_base64(stdout),
        stderr: decode_base64(stderr),
        exit_code,
    })
}

/// 解码 base64 字段；非法内容按空串处理（协议字段损坏不应让调用方 panic）。
fn decode_base64(value: &str) -> String {
    BASE64.decode(value).map_or_else(
        |_| String::new(),
        |bytes| String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// worker 失败后的冷却时间。
///
/// 见 [`WslWorkerPool::should_try`]：一个注定起不来的 worker 不该让**每一次**操作都
/// 先付一次失败的启动。
pub const WORKER_FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

/// 每个发行版一个 worker 的池。
///
/// 访问按发行版串行化：worker 本身是单线程 bash 循环，并发写同一 stdin 会让帧交错。
#[derive(Clone)]
pub struct WslWorkerPool {
    launcher: Arc<dyn WslLauncher>,
    workers: Arc<Mutex<HashMap<String, Arc<Mutex<WslWorker>>>>>,
    /// 各发行版最近一次 worker 失败的时刻（冷却判据）。
    failed_at: Arc<Mutex<HashMap<String, std::time::Instant>>>,
}

impl std::fmt::Debug for WslWorkerPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WslWorkerPool")
            .field("launcher", &self.launcher.describe())
            // workers 在 `Mutex` 之后，`Debug` 里无法同步读取，故不列出。
            .finish_non_exhaustive()
    }
}

impl WslWorkerPool {
    /// 由 launcher 构造。
    #[must_use]
    pub fn new(launcher: Arc<dyn WslLauncher>) -> Self {
        Self {
            launcher,
            workers: Arc::new(Mutex::new(HashMap::new())),
            failed_at: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 该 launcher 是否可用。
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.launcher.is_available()
    }

    /// 发起一次调用；失败时丢弃 worker 并返回错误（由上层决定是否回退）。
    pub async fn call(&self, distro: &str, operation: &WorkerOp) -> Result<CommandOutput> {
        self.call_with_timeout(distro, operation, WORKER_CALL_TIMEOUT)
            .await
    }

    /// 发起一次调用并指定超时。
    pub async fn call_with_timeout(
        &self,
        distro: &str,
        operation: &WorkerOp,
        timeout: Duration,
    ) -> Result<CommandOutput> {
        let worker = match self.worker_for(distro).await {
            Ok(worker) => worker,
            Err(error) => {
                // **连启动都失败也要记下来**：否则每一次操作都会再试一次启动。
                self.mark_failed(distro).await;
                return Err(error);
            }
        };
        let mut guard = worker.lock().await;
        let outcome = guard.call_with_timeout(operation, timeout).await;
        if outcome.is_err() {
            // 协议异常或进程已死：丢弃该 worker，并**记下这次失败**。
            drop(guard);
            self.discard(distro).await;
            self.mark_failed(distro).await;
        } else {
            // 成功一次就说明它可用，清掉失败记录。
            self.clear_failed(distro).await;
        }
        outcome
    }

    /// 该发行版现在**值不值得尝试** worker。
    ///
    /// # 为什么需要它
    ///
    /// worker 只是加速手段。但如果某个发行版的 worker 注定起不来（例如发行版注册在册
    /// 却进不去），每次操作都会先付一次注定失败的启动，再回退到一次性执行——
    /// **同一个操作付两遍进程创建**。实测中一个进不去的发行版让一次目标探测从数秒
    /// 涨到 19 秒。
    ///
    /// 冷却期内直接说「不值得」，让上层**安静地**走一次性路径：没有重复的失败启动，
    /// 也没有每次调用都刷一遍的告警。
    ///
    /// 冷却而不是永久禁用：失败可能是暂时的（发行版正在启动、系统繁忙），
    /// 过一段时间重试一次，成本很低。
    pub async fn should_try(&self, distro: &str) -> bool {
        let failed = self.failed_at.lock().await;
        match failed.get(distro) {
            Some(at) => at.elapsed() >= WORKER_FAILURE_COOLDOWN,
            None => true,
        }
    }

    /// 记下一次 worker 失败。
    async fn mark_failed(&self, distro: &str) {
        self.failed_at
            .lock()
            .await
            .insert(distro.to_owned(), std::time::Instant::now());
    }

    /// 清除失败记录（成功一次就说明它可用）。
    async fn clear_failed(&self, distro: &str) {
        self.failed_at.lock().await.remove(distro);
    }

    /// 取得（或创建）某发行版的 worker。
    async fn worker_for(&self, distro: &str) -> Result<Arc<Mutex<WslWorker>>> {
        let mut workers = self.workers.lock().await;
        if let Some(existing) = workers.get(distro) {
            return Ok(Arc::clone(existing));
        }
        let worker = WslWorker::spawn(self.launcher.as_ref(), distro)?;
        let handle = Arc::new(Mutex::new(worker));
        workers.insert(distro.to_owned(), Arc::clone(&handle));
        Ok(handle)
    }

    /// 丢弃某发行版的 worker。
    pub async fn discard(&self, distro: &str) {
        let removed = self.workers.lock().await.remove(distro);
        if let Some(worker) = removed {
            // 可能仍有并发调用持有锁；等待它释放后关闭。
            if let Ok(worker) = Arc::try_unwrap(worker) {
                worker.into_inner().shutdown().await;
            }
        }
    }

    /// 关闭全部 worker（应用退出时调用）。
    pub async fn shutdown_all(&self) {
        let workers = std::mem::take(&mut *self.workers.lock().await);
        for (_, worker) in workers {
            if let Ok(worker) = Arc::try_unwrap(worker) {
                worker.into_inner().shutdown().await;
            }
        }
    }

    /// 当前活跃的 worker 数量（诊断用）。
    pub async fn worker_count(&self) -> usize {
        self.workers.lock().await.len()
    }
}

/// 供诊断使用：worker 脚本内容。
#[must_use]
pub const fn worker_script() -> &'static str {
    WORKER_SCRIPT
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wsl::launcher::LocalBashLauncher;

    /// 构造一个可用的 worker 池；本机 bash 不可运行时返回 `None`，测试随即跳过。
    ///
    /// 受限沙箱下 MSYS2 bash 无法创建内部 signal pipe，此时**协议本身没有被验证**，
    /// 由 CI（Linux 原生 bash）覆盖。
    fn pool() -> Option<WslWorkerPool> {
        let launcher = LocalBashLauncher::detect().filter(LocalBashLauncher::is_usable)?;
        Some(WslWorkerPool::new(Arc::new(launcher)))
    }

    fn b64(value: &str) -> String {
        BASE64.encode(value.as_bytes())
    }

    #[tokio::test]
    async fn a_failed_worker_is_not_retried_on_every_call() {
        // **回归守门。**
        //
        // worker 失败后曾在下一次调用立刻重建，于是**每一次操作都要先付一次注定失败的
        // 启动**，再回退到一次性执行——同一个操作付两遍进程创建。
        //
        // 实测后果：本机有一个发行版注册在册却进不去，一次目标探测因此从数秒涨到
        // **19 秒**。
        //
        // 这条测试不需要真的 bash：用一个不存在的可执行文件，启动必然失败。
        let pool = WslWorkerPool::new(Arc::new(LocalBashLauncher::new(
            "definitely-not-a-real-bash",
        )));
        assert!(pool.should_try("test").await, "第一次应当尝试 worker");

        let outcome = pool
            .call("test", &WorkerOp::Exists { path: "/x".into() })
            .await;
        assert!(outcome.is_err(), "启动失败必须报错，好让上层回退");

        assert!(
            !pool.should_try("test").await,
            "失败之后必须进入冷却：否则每次操作都会再付一次注定失败的启动"
        );
        // 别的发行版不受影响——冷却按发行版记录。
        assert!(pool.should_try("other").await, "冷却不该波及其他发行版");
    }

    #[test]
    fn operation_names_match_the_protocol() {
        assert_eq!(WorkerOp::Read { path: "x".into() }.name(), "read");
        assert_eq!(
            WorkerOp::Head {
                path: "x".into(),
                max_bytes: 1
            }
            .name(),
            "head"
        );
        assert_eq!(
            WorkerOp::Move {
                source: "a".into(),
                destination: "b".into(),
                fail_if_exists: true
            }
            .name(),
            "move"
        );
        assert_eq!(
            WorkerOp::Exec {
                program: "ls".into(),
                args: vec![]
            }
            .name(),
            "exec"
        );
    }

    #[test]
    fn exec_arguments_are_nul_joined_before_encoding() {
        let operation = WorkerOp::Exec {
            program: "echo".to_owned(),
            args: vec!["a b".to_owned(), "c".to_owned()],
        };
        let arguments = operation.arguments();
        assert_eq!(arguments.len(), 2);
        assert_eq!(decode_base64(&arguments[0]), "echo");
        // 结尾的 NUL 是必需的：bash 的 `read -d '` 缺少结尾分隔符时会丢最后一段。`n        assert_eq!(decode_base64(&arguments[1]), "a b\0c\0");
    }

    #[test]
    fn move_encodes_the_conflict_guard_as_a_flag() {
        let guard = WorkerOp::Move {
            source: "a".to_owned(),
            destination: "b".to_owned(),
            fail_if_exists: true,
        };
        assert_eq!(guard.arguments()[2], "1");
        let open = WorkerOp::Move {
            source: "a".to_owned(),
            destination: "b".to_owned(),
            fail_if_exists: false,
        };
        assert_eq!(open.arguments()[2], "0");
    }

    #[test]
    fn responses_are_parsed_and_ids_are_checked() {
        let output =
            parse_response(&format!("7\t0\t{}\t{}", b64("hello"), b64("")), 7).expect("parse");
        assert_eq!(output.stdout, "hello");
        assert_eq!(output.exit_code, 0);
        assert!(output.is_success());

        // 编号错位必须报错：协议已不可信。
        assert!(parse_response(&format!("8\t0\t{}\t", b64("x")), 7).is_err());
        // 缺少退出码。
        assert!(parse_response("7\t\t\n", 7).is_err());
        // 非数字编号。
        assert!(parse_response("abc\t0\t\t\n", 7).is_err());
    }

    #[test]
    fn corrupt_base64_fields_degrade_to_empty_strings() {
        let output = parse_response("1\t0\t!!!not-base64!!!\t\n", 1).expect("parse");
        assert!(output.stdout.is_empty());
    }

    #[test]
    fn non_zero_exit_codes_become_errors_with_stderr_detail() {
        let output = CommandOutput {
            stdout: String::new(),
            stderr: "  boom  ".to_owned(),
            exit_code: 3,
        };
        let error = output.into_success("读取").expect_err("必须失败");
        assert_eq!(error.to_string(), "boom");

        // WSL 系统错误可能写到 stdout，仍然必须保留。
        let output = CommandOutput {
            stdout: "  HCS/E_ACCESSDENIED  ".to_owned(),
            stderr: String::new(),
            exit_code: -1,
        };
        let error = output.into_success("探测").expect_err("必须失败");
        assert_eq!(error.to_string(), "HCS/E_ACCESSDENIED");

        // 两条输出均为空时给出带退出码的说明。
        let output = CommandOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: 3,
        };
        let error = output.into_success("读取").expect_err("必须失败");
        assert_eq!(error.to_string(), "读取 失败，退出码 3");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_executes_real_commands_through_the_protocol() {
        let Some(pool) = pool() else {
            return;
        };
        let output = pool
            .call(
                "test",
                &WorkerOp::Exec {
                    program: "printf".to_owned(),
                    args: vec!["hello %s".to_owned(), "world".to_owned()],
                },
            )
            .await
            .expect("exec");
        assert_eq!(output.stdout, "hello world");
        assert!(output.is_success());

        // 同一 worker 连续调用：编号递增且互不干扰。
        let output = pool
            .call(
                "test",
                &WorkerOp::Exec {
                    program: "printf".to_owned(),
                    args: vec!["second".to_owned()],
                },
            )
            .await
            .expect("exec again");
        assert_eq!(output.stdout, "second");
        assert_eq!(pool.worker_count().await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_round_trips_files_with_binary_safe_framing() {
        let Some(pool) = pool() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        let path_str = path.to_string_lossy().into_owned();

        // 内容含制表符、换行与中文——这些正是「文本行帧」最容易踩的字符。
        let content = "{\"a\":\"中\t文\"}\nline2\nline3";
        pool.call(
            "test",
            &WorkerOp::Write {
                path: path_str.clone(),
                content: content.to_owned(),
            },
        )
        .await
        .expect("write")
        .into_success("write")
        .expect("write ok");

        let read = pool
            .call(
                "test",
                &WorkerOp::Read {
                    path: path_str.clone(),
                },
            )
            .await
            .expect("read");
        assert_eq!(read.stdout, content, "含制表符与换行的内容必须原样往返");

        let size = pool
            .call(
                "test",
                &WorkerOp::Size {
                    path: path_str.clone(),
                },
            )
            .await
            .expect("size");
        assert_eq!(
            size.stdout.trim().parse::<u64>().expect("size"),
            content.len() as u64
        );

        let exists = pool
            .call(
                "test",
                &WorkerOp::Exists {
                    path: path_str.clone(),
                },
            )
            .await
            .expect("exists");
        assert_eq!(exists.stdout.trim(), "1");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_reads_only_the_requested_head() {
        let Some(pool) = pool() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("big.jsonl");
        let path_str = path.to_string_lossy().into_owned();
        pool.call(
            "test",
            &WorkerOp::Write {
                path: path_str.clone(),
                content: "x".repeat(1_000),
            },
        )
        .await
        .expect("write");

        let head = pool
            .call(
                "test",
                &WorkerOp::Head {
                    path: path_str,
                    max_bytes: 64,
                },
            )
            .await
            .expect("head");
        assert_eq!(head.stdout.len(), 64);
    }

    #[test]
    fn read_heads_arguments_are_nul_joined_and_carry_the_limit() {
        let operation = WorkerOp::ReadHeads {
            paths: vec!["/a".to_owned(), "/b".to_owned()],
            max_bytes: 65_536,
        };
        assert_eq!(operation.name(), "read_heads");
        let args = operation.arguments();
        assert_eq!(args.len(), 2);
        assert_eq!(args[1], "65536");
        // 路径以 NUL 连接并**以 NUL 结尾**：`read -d ''` 在缺少结尾分隔符时会丢弃
        // 最后一段——那正是「少读一个文件」的形状。
        assert_eq!(args[0], b64("/a\0/b\0"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_reads_many_heads_in_one_call() {
        // 列表路径的关键操作：一次调用读一批文件头。
        let Some(pool) = pool() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let first = dir.path().join("a.jsonl");
        let second = dir.path().join("b.jsonl");
        let missing = dir.path().join("nope.jsonl");
        std::fs::write(&first, "alpha").expect("write");
        std::fs::write(&second, "beta").expect("write");

        let paths = vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
            missing.to_string_lossy().into_owned(),
        ];
        let output = pool
            .call(
                "test",
                &WorkerOp::ReadHeads {
                    paths: paths.clone(),
                    max_bytes: 1024,
                },
            )
            .await
            .expect("read_heads");
        assert!(output.is_success(), "stderr={}", output.stderr);

        let heads = crate::wsl::adapter::parse_head_batch(&output.stdout, paths.len());
        assert_eq!(heads.len(), paths.len(), "结果必须与请求一一对应");
        assert_eq!(heads[0].as_deref(), Some("alpha"));
        assert_eq!(heads[1].as_deref(), Some("beta"));
        assert_eq!(heads[2], None, "不存在的文件是 None，而不是空串");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_lists_files_with_the_find_printf_format() {
        let Some(pool) = pool() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("sessions");
        std::fs::create_dir_all(root.join("nested")).expect("mkdir");
        for name in ["rollout-a.jsonl", "rollout-b.jsonl", "ignore.txt"] {
            std::fs::write(root.join(name), "x").expect("write");
        }
        std::fs::write(root.join("nested").join("rollout-c.jsonl"), "y").expect("write");

        let listing = pool
            .call(
                "test",
                &WorkerOp::List {
                    root: root.to_string_lossy().into_owned(),
                    pattern: "rollout-*.jsonl".to_owned(),
                },
            )
            .await
            .expect("list");

        let files = crate::wsl::decode::parse_wsl_session_file_list(&listing.stdout);
        assert_eq!(files.len(), 3, "递归列出全部匹配文件：{files:?}");
        assert!(
            files
                .iter()
                .all(|file| file.file_path.to_ascii_lowercase().ends_with(".jsonl"))
        );
        assert!(files.iter().all(|file| file.size == 1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_move_honours_the_conflict_guard() {
        let Some(pool) = pool() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let source = dir.path().join("source.jsonl");
        let target = dir.path().join("target.jsonl");
        std::fs::write(&source, "source").expect("write");
        std::fs::write(&target, "target").expect("write");

        // 目标已存在 → 拒绝，且两个文件都不动。
        let outcome = pool
            .call(
                "test",
                &WorkerOp::Move {
                    source: source.to_string_lossy().into_owned(),
                    destination: target.to_string_lossy().into_owned(),
                    fail_if_exists: true,
                },
            )
            .await
            .expect("move");
        assert_eq!(outcome.exit_code, 17);
        assert!(outcome.stderr.contains("原位置已存在同名会话文件"));
        assert_eq!(std::fs::read_to_string(&source).expect("source"), "source");

        // 移到不存在的目标 → 成功。
        let free = dir.path().join("free").join("moved.jsonl");
        let outcome = pool
            .call(
                "test",
                &WorkerOp::Move {
                    source: source.to_string_lossy().into_owned(),
                    destination: free.to_string_lossy().into_owned(),
                    fail_if_exists: true,
                },
            )
            .await
            .expect("move");
        assert!(outcome.is_success(), "{outcome:?}");
        assert_eq!(std::fs::read_to_string(&free).expect("moved"), "source");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn worker_reports_failures_for_missing_files() {
        let Some(pool) = pool() else {
            return;
        };
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("missing.jsonl");

        let output = pool
            .call(
                "test",
                &WorkerOp::Read {
                    path: missing.to_string_lossy().into_owned(),
                },
            )
            .await
            .expect("read");
        assert!(!output.is_success());
        assert!(!output.stderr.is_empty());

        let exists = pool
            .call(
                "test",
                &WorkerOp::Exists {
                    path: missing.to_string_lossy().into_owned(),
                },
            )
            .await
            .expect("exists");
        assert_eq!(exists.stdout.trim(), "0");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_operations_are_reported_without_killing_the_worker() {
        let Some(pool) = pool() else {
            return;
        };
        // 直接构造一个未在脚本中实现的操作名，验证兜底分支。
        let worker = pool.worker_for("test").await.expect("worker");
        let mut guard = worker.lock().await;
        let frame = format!("1\tdefinitely-not-an-op\t{}\n", b64("x"));
        guard
            .stdin
            .write_all(frame.as_bytes())
            .await
            .expect("write");
        guard.stdin.flush().await.expect("flush");
        let mut line = String::new();
        guard.stdout.read_line(&mut line).await.expect("read_line");
        drop(guard);

        let output = parse_response(&line, 1).expect("parse");
        assert_eq!(output.exit_code, 2);
        assert!(output.stderr.contains("unknown operation"));

        // worker 必须仍然可用。
        let output = pool
            .call(
                "test",
                &WorkerOp::Exec {
                    program: "printf".to_owned(),
                    args: vec!["alive".to_owned()],
                },
            )
            .await
            .expect("exec");
        assert_eq!(output.stdout, "alive");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn discarding_a_worker_forces_a_fresh_one() {
        let Some(pool) = pool() else {
            return;
        };
        pool.call(
            "test",
            &WorkerOp::Exec {
                program: "true".to_owned(),
                args: Vec::new(),
            },
        )
        .await
        .expect("exec");
        assert_eq!(pool.worker_count().await, 1);

        pool.discard("test").await;
        assert_eq!(pool.worker_count().await, 0);

        // 丢弃后再调用会重建，且仍能正常工作。
        let output = pool
            .call(
                "test",
                &WorkerOp::Exec {
                    program: "printf".to_owned(),
                    args: vec!["rebuilt".to_owned()],
                },
            )
            .await
            .expect("exec");
        assert_eq!(output.stdout, "rebuilt");
        assert_eq!(pool.worker_count().await, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_all_closes_every_worker() {
        let Some(pool) = pool() else {
            return;
        };
        pool.call(
            "a",
            &WorkerOp::Exec {
                program: "true".to_owned(),
                args: Vec::new(),
            },
        )
        .await
        .expect("exec a");
        pool.call(
            "b",
            &WorkerOp::Exec {
                program: "true".to_owned(),
                args: Vec::new(),
            },
        )
        .await
        .expect("exec b");
        assert_eq!(pool.worker_count().await, 2);

        pool.shutdown_all().await;
        assert_eq!(pool.worker_count().await, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_call_timeout_is_reported_and_discards_the_worker() {
        let Some(pool) = pool() else {
            return;
        };
        // `sleep 5` 配 100 ms 超时：必须超时而不是挂住。
        let outcome = pool
            .call_with_timeout(
                "test",
                &WorkerOp::Exec {
                    program: "sleep".to_owned(),
                    args: vec!["5".to_owned()],
                },
                Duration::from_millis(100),
            )
            .await;
        assert!(outcome.is_err());
        assert!(outcome.expect_err("必须超时").to_string().contains("超时"));
        // 超时后 worker 已被丢弃，下次调用会重建。
        assert_eq!(pool.worker_count().await, 0);
    }
}
