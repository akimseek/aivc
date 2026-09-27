//! PTY 会话生命周期与输出批处理。
//!
//! # 输出为什么要批处理
//!
//! 交互式 CLI 的输出极碎：一次重绘可能产生几十个几十字节的小块。逐块送到渲染层会让
//! 一次 IPC 只含几十字节，消息数量却上看千级。因此按 [`FLUSH_INTERVAL_MS`] 合并成一个
//! 批次——约 60 次/秒，与人眼刷新率一致，再快也没有意义。
//!
//! 批处理本身做成纯结构：`push` 累积、`drain` 取走。定时器由调用方驱动，因此合并行为
//! 可以脱离真实 PTY 测试。
//!
//! # 为什么读取用阻塞线程
//!
//! portable-pty 的读取器是阻塞的 `Read`，不是异步 `AsyncRead`。把它包成异步流需要额外
//! 的适配层，而 PTY 输出本来就是「低频、小量、需及时」的流量，一个专用线程足够，且不会
//! 因为适配层的缓冲策略引入额外延迟。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Mutex, mpsc};

use aivc_core::error::{Error, Result, TerminalError};

use crate::command::TerminalCommand;

/// 输出合并窗口（毫秒）。
pub const FLUSH_INTERVAL_MS: u64 = 16;
/// 单个终端默认列数。
///
/// 与现网 `cols || 100` 一致。渲染层通常显式传尺寸，这个默认值只在未传时生效——
/// 但正因为很少走到，写错了不会被发现，所以用测试钉住。
pub const DEFAULT_COLS: u16 = 100;
/// 单个终端默认行数。
///
/// 与现网 `rows || 30` 一致。
pub const DEFAULT_ROWS: u16 = 30;
/// 输出通道容量。
pub const OUTPUT_CHANNEL_CAPACITY: usize = 256;
/// 收养任务查询子进程是否退出的间隔。
///
/// 这个值只影响「进程结束后界面多久显示已退出」。250 毫秒对人眼是「立刻」，而每个终端
/// 每 250 毫秒只做一次非阻塞的 `try_wait()`，代价可以忽略。
pub const REAP_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

/// 输出批处理器。
///
/// 见模块文档：定时由调用方驱动，因此这个结构是纯的。
#[derive(Debug, Default)]
pub struct OutputBatcher {
    pending: Vec<String>,
    bytes: usize,
}

impl OutputBatcher {
    /// 空批处理器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 追加一块输出。
    pub fn push(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        self.bytes += data.len();
        self.pending.push(data.to_owned());
    }

    /// 是否有待发送的内容。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// 待发送的字节数。
    #[must_use]
    pub const fn pending_bytes(&self) -> usize {
        self.bytes
    }

    /// 取走并合并全部待发送内容。
    pub fn drain(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let merged = self.pending.concat();
        self.pending.clear();
        self.bytes = 0;
        Some(merged)
    }
}

/// 一路 PTY 会话。
///
/// # 为什么 writer 与 master 是可选的
///
/// 读取线程阻塞在 `read()` 上，只有**关闭 PTY** 才会让它收到 EOF。因此会话在终止或
/// 进程退出后必须主动释放 master 与 writer；否则读取线程会永久挂起——测试进程不会退出，
/// 生产环境里每关闭一个终端就泄漏一个线程。
pub struct PtySession {
    writer: Option<Box<dyn Write + Send>>,
    master: Option<Box<dyn MasterPty + Send>>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    exited: Arc<AtomicBool>,
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtySession")
            .field("exited", &self.exited.load(Ordering::Relaxed))
            .field("released", &self.master.is_none())
            .finish_non_exhaustive()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        // 会话被丢弃时必须收尾：否则读取线程一直挂着，控制台宿主进程也不退出。
        //
        // 顺序与 `kill()` 一致：kill → wait 回收 → 释放句柄。少了 `wait`，
        // 未回收的进程对象会让 `OpenConsole.exe` 一直活着。
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.release();
    }
}

impl PtySession {
    /// 启动一路会话。
    ///
    /// 返回会话句柄与输出接收端。输出以**原始字节**为单位发送，由调用方按批处理策略
    /// 合并——会话层不假设输出一定是合法 UTF-8（交互式程序会发出光标控制字节）。
    ///
    /// # Errors
    ///
    /// 工作目录建不出来、PTY 分配失败或进程启动失败时返回错误。
    pub fn spawn(
        command: &TerminalCommand,
        cols: u16,
        rows: u16,
    ) -> Result<(Self, mpsc::Receiver<Vec<u8>>)> {
        // **工作目录必须先真的存在。**
        //
        // `portable-pty` 在 Windows 上会把「不是目录」的 `cwd` **悄悄丢掉**，改用
        // `USERPROFILE`（见它 `current_directory()` 里的 `.filter(|path| Path::is_dir())`）。
        // 于是「在某个还没建的目录里开终端」得到的是一个开在用户主目录里的终端，
        // 而且没有任何提示——用户只会觉得「目录没生效」。
        //
        // 现网在启动前 `mkdir -p`，这里补上同一件事；补不上时**明确报错**，不静默降级。
        if !command.cwd.trim().is_empty()
            && !std::path::Path::new(&command.cwd).is_dir()
            && let Err(error) = std::fs::create_dir_all(&command.cwd)
        {
            return Err(spawn_error(&format!(
                "无法创建终端工作目录 {}：{error}",
                command.cwd
            )));
        }
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| spawn_error(&error.to_string()))?;

        let mut builder = CommandBuilder::new(&command.file);
        builder.args(&command.args);
        builder.cwd(&command.cwd);
        // **先删后设**：`CommandBuilder` 默认继承宿主进程的整个环境，而 `env` 只能
        // 覆盖、不能抹掉。Codex 优先读 `CODEX_API_KEY` / `CODEX_ACCESS_TOKEN`，
        // 它们不被删除时网关会被静默绕过——进程正常启动，请求却记在用户自己的额度上。
        // 顺序固定为「删→设」，因此同时出现在两边的变量以「设」为准。
        for key in &command.remove_env {
            builder.env_remove(key);
        }
        for (key, value) in &command.env {
            builder.env(key, value);
        }

        let child = pair
            .slave
            .spawn_command(builder)
            .map_err(|error| spawn_error(&error.to_string()))?;
        // slave 必须在这里释放：不释放时部分平台会一直认为对端还开着，读取端不结束。
        drop(pair.slave);

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| spawn_error(&error.to_string()))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| spawn_error(&error.to_string()))?;

        let (sender, receiver) = mpsc::channel(OUTPUT_CHANNEL_CAPACITY);
        std::thread::Builder::new()
            .name("aivc-pty-reader".to_owned())
            .spawn(move || pump(reader, &sender))
            .map_err(|error| spawn_error(&error.to_string()))?;

        let exited = Arc::new(AtomicBool::new(false));
        Ok((
            Self {
                writer: Some(writer),
                master: Some(pair.master),
                child,
                exited,
            },
            receiver,
        ))
    }

    /// 释放 PTY 句柄，让读取线程收到 EOF。
    ///
    /// 见类型文档：不释放就会泄漏一个永久阻塞的线程。
    fn release(&mut self) {
        // 先放 writer 再放 master。
        self.writer = None;
        self.master = None;
    }

    /// 写入用户输入。
    ///
    /// # Errors
    ///
    /// 写入失败（通常是进程已退出）时返回错误。
    pub fn write(&mut self, data: &str) -> Result<()> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(TerminalError::WriteFailed {
                message: "终端已关闭".to_owned(),
            }
            .into());
        };
        writer
            .write_all(data.as_bytes())
            .and_then(|()| writer.flush())
            .map_err(|error| {
                TerminalError::WriteFailed {
                    message: error.to_string(),
                }
                .into()
            })
    }

    /// 调整窗口尺寸。
    ///
    /// # Errors
    ///
    /// PTY 已关闭时返回错误。
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let Some(master) = self.master.as_ref() else {
            return Err(TerminalError::ResizeFailed {
                message: "终端已关闭".to_owned(),
            }
            .into());
        };
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| {
                TerminalError::ResizeFailed {
                    message: error.to_string(),
                }
                .into()
            })
    }

    /// 终止进程并释放 PTY 句柄。
    ///
    /// 幂等：对已退出的会话再次调用不报错（界面可能重复收到关闭请求）。
    ///
    /// # Errors
    ///
    /// 终止系统调用失败时返回错误。
    pub fn kill(&mut self) -> Result<()> {
        if self.exited.load(Ordering::Relaxed) {
            return Ok(());
        }
        let outcome = self
            .child
            .kill()
            .map_err(|error| TerminalError::KillFailed {
                message: error.to_string(),
            });
        // **必须 wait 回收子进程**。
        //
        // 只 `kill()` 不 `wait()` 会在 Windows 上留下未回收的进程句柄：控制台宿主
        // （`OpenConsole.exe` / `conhost.exe`）在进程对象仍被引用时不会退出。
        // 实测证据：一轮 PTY 测试留下 20 个 conhost + 3 个 OpenConsole，
        // 并把父作业对象的「托管范围为空」判定卡死。
        //
        // `wait` 在这里不会阻塞：`kill()` 已经发出终止，剩下的是回收。
        let reaped = self.child.wait().is_ok();
        if reaped {
            self.exited.store(true, Ordering::Relaxed);
        }
        // 无论终止是否成功都要释放：留一个永久挂起的读取线程比一次失败的系统调用更糟。
        self.release();
        outcome.map_err(Into::into)
    }

    /// 等待进程结束并返回退出码。
    ///
    /// # Errors
    ///
    /// 等待失败时返回错误。
    pub fn wait(&mut self) -> Result<i32> {
        let status = self
            .child
            .wait()
            .map_err(|error| TerminalError::KillFailed {
                message: error.to_string(),
            })?;
        self.exited.store(true, Ordering::Relaxed);
        // 子进程已退出，PTY 不会再有输出；释放句柄让读取线程结束。
        self.release();
        // portable-pty 把退出码包在 ExitStatus 里；非正常退出（信号）没有码，按 1 处理。
        Ok(i32::try_from(status.exit_code()).unwrap_or(1))
    }

    /// 子进程是否已经退出；已退出则返回退出码。
    ///
    /// **非阻塞**。收养任务（[`SessionRegistry::spawn_reaper`]）靠它轮询——那里绝不能
    /// 挂在 `wait()` 上，否则一路会话就把一个任务永久占住了。
    ///
    /// 与 [`Self::wait`] 的关键区别：这里**不动 PTY 句柄**。句柄的释放时机由调用方决定，
    /// 因为「释放就等价于让读取线程收到 EOF、等价于对外宣告终端结束」，那必须是一个
    /// 有意识的动作。
    fn try_wait(&mut self) -> Result<Option<i32>> {
        let status = self
            .child
            .try_wait()
            .map_err(|error| TerminalError::KillFailed {
                message: error.to_string(),
            })?;
        Ok(status.map(|value| i32::try_from(value.exit_code()).unwrap_or(1)))
    }
}

/// 把阻塞读取器的内容泵进通道。
///
/// `sender` 按引用传入：`blocking_send` 只需要 `&self`，按值传入会白白占有一份句柄，
/// 也让「谁持有发送端」这件事变得不清楚。
fn pump(mut reader: Box<dyn Read + Send>, sender: &mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            // 对端关闭。
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if sender.blocking_send(buffer[..count].to_vec()).is_err() {
                    // 接收端已丢弃（会话被移除），继续读只是浪费。
                    break;
                }
            }
        }
    }
}

/// 会话注册表。
#[derive(Default)]
pub struct SessionRegistry {
    sessions: Arc<Mutex<HashMap<String, PtySession>>>,
    /// 已退出会话的退出码，按 id 索引。
    ///
    /// # 为什么退出码要单独走一条路
    ///
    /// **退出码不经过输出通道。** 读取线程只知道「通道关了」，退出码只有子进程句柄知道，
    /// 而句柄归注册表所有。收养任务把码放进来，输出泵在通道关闭之后按 id 取走。
    ///
    /// 顺序是**先放码、再关 PTY**：关 PTY 会立刻让读取线程收到 EOF，泵随即就来取。
    exits: Arc<Mutex<HashMap<String, i32>>>,
}

impl std::fmt::Debug for SessionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRegistry").finish_non_exhaustive()
    }
}

impl SessionRegistry {
    /// 空注册表。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一路会话，并开始盯着它的子进程。
    pub async fn insert(&self, id: String, session: PtySession) {
        self.sessions.lock().await.insert(id.clone(), session);
        self.spawn_reaper(id);
    }

    /// 起一个收养任务，盯着这一路会话的子进程何时退出。
    ///
    /// # 为什么必须有它
    ///
    /// **`Windows` 的 `ConPTY` 下，子进程退出并不会让读取线程收到 EOF**——只有关闭 PTY 主
    /// 句柄才会（这一点由 `conpty_does_not_report_eof_when_the_child_exits_on_its_own`
    /// 实测确认：子进程早已退出、输出也拿到了，15 秒后通道依然开着）。
    ///
    /// 后果曾经是：
    ///
    /// - 用户敲 `exit`、或 CLI 自己崩掉之后，读取线程依然挂在 `read()` 上，
    ///   输出通道不关，`terminal:exit` **永不触发**——界面上的终端标签一直显示运行中；
    /// - 退出码没有任何人拿得到（`PtySession::wait()` 在生产代码里从未被调用过）。
    ///
    /// 这个任务轮询 [`PtySession::try_wait`]（非阻塞）补上这两件事：
    /// 发现已退出 → **先记下退出码** → **再释放 PTY**（让读取线程收到 EOF →
    /// 通道关闭 → 泵发退出事件）。
    ///
    /// 顺序不能反：关 PTY 与泵取码是同一瞬间的事，码必须先落地。
    fn spawn_reaper(&self, id: String) {
        let sessions = Arc::clone(&self.sessions);
        let exits = Arc::clone(&self.exits);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REAP_INTERVAL).await;
                let mut guard = sessions.lock().await;
                // 会话已被 `stop` / `stop_all` 摘走：那一侧已经收过尾了。
                let Some(session) = guard.get_mut(&id) else {
                    return;
                };
                match session.try_wait() {
                    Ok(Some(code)) => {
                        exits.lock().await.insert(id, code);
                        session.release();
                        return;
                    }
                    Ok(None) => {}
                    // 查询失败时收手：再轮询下去只是每 250 毫秒白跑一次。
                    Err(_) => return,
                }
            }
        });
    }

    /// 取走一路会话的退出码（若已被收养任务收割）。
    ///
    /// **取走**而不是读出：调用方是输出泵，每路终端只会问一次，顺手就把表项清掉了。
    /// 返回 `None` 表示这不是「进程自己结束」——例如用户主动关闭了终端，
    /// 那种情况不该报「异常停止」。
    pub async fn take_exit_code(&self, id: &str) -> Option<i32> {
        self.exits.lock().await.remove(id)
    }

    /// 移除并返回一路会话。
    pub async fn remove(&self, id: &str) -> Option<PtySession> {
        self.sessions.lock().await.remove(id)
    }

    /// 终止并移除一路会话。
    ///
    /// # Errors
    ///
    /// 终止失败时返回错误；会话仍会被移除，避免界面卡在一个无法关闭的终端上。
    pub async fn stop(&self, id: &str) -> Result<bool> {
        let Some(mut session) = self.remove(id).await else {
            return Ok(false);
        };
        // **主动关闭不是「异常停止」。** 丢掉可能已落地的退出码：否则当「进程刚好自己
        // 结束」与「用户同时点了关闭」撞在一起时，泵会把一次正常关闭报成崩溃。
        self.exits.lock().await.remove(id);
        let outcome = session.kill();
        // 移除后再终止：即使终止失败也不留下无法关闭的终端。
        outcome.map(|()| true)
    }

    /// 终止全部会话。
    pub async fn stop_all(&self) {
        let mut guard = self.sessions.lock().await;
        for session in guard.values_mut() {
            // 单个会话终止失败不影响其余的会话。
            let _ = session.kill();
        }
        guard.clear();
        self.exits.lock().await.clear();
    }

    /// 当前会话数。
    pub async fn len(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// 是否没有会话。
    pub async fn is_empty(&self) -> bool {
        self.sessions.lock().await.is_empty()
    }

    /// 写入一路会话的输入。
    ///
    /// # Errors
    ///
    /// 会话不存在或写入失败时返回错误。
    pub async fn write(&self, id: &str, data: &str) -> Result<()> {
        let mut guard = self.sessions.lock().await;
        let session = guard
            .get_mut(id)
            .ok_or_else(|| Error::Terminal(TerminalError::NotFound { id: id.to_owned() }))?;
        session.write(data)
    }

    /// 调整一路会话的尺寸。
    ///
    /// # Errors
    ///
    /// 会话不存在或调整失败时返回错误。
    pub async fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<()> {
        let guard = self.sessions.lock().await;
        let session = guard
            .get(id)
            .ok_or_else(|| Error::Terminal(TerminalError::NotFound { id: id.to_owned() }))?;
        session.resize(cols, rows)
    }
}

/// 启动失败。
fn spawn_error(message: &str) -> Error {
    TerminalError::SpawnFailed {
        message: message.to_owned(),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    fn echo_command(text: &str) -> TerminalCommand {
        if cfg!(windows) {
            TerminalCommand {
                file: "cmd.exe".to_owned(),
                args: vec![
                    "/d".to_owned(),
                    "/s".to_owned(),
                    "/c".to_owned(),
                    format!("echo {text}"),
                ],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: BTreeMap::new(),
                remove_env: BTreeSet::new(),
            }
        } else {
            TerminalCommand {
                file: "/bin/sh".to_owned(),
                args: vec!["-c".to_owned(), format!("echo {text}")],
                cwd: std::env::temp_dir().to_string_lossy().into_owned(),
                env: BTreeMap::new(),
                remove_env: BTreeSet::new(),
            }
        }
    }

    #[test]
    fn the_default_terminal_size_matches_the_desktop_reference() {
        // 现网是 `cols || 100` / `rows || 30`（terminal-sessions.ts:153）。
        // 渲染层通常显式传尺寸，因此这个默认值极少走到——正因如此写错也不会被发现。
        assert_eq!(DEFAULT_COLS, 100);
        assert_eq!(DEFAULT_ROWS, 30);
    }

    #[test]
    fn the_flush_interval_matches_the_desktop_reference() {
        // 16 ms ≈ 60 次/秒，与人眼刷新率一致；再快没有意义，再慢会看出卡顿。
        assert_eq!(FLUSH_INTERVAL_MS, 16);
    }

    // --- 输出批处理 ---

    #[test]
    fn empty_chunks_are_ignored() {
        let mut batcher = OutputBatcher::new();
        batcher.push("");
        assert!(batcher.is_empty());
        assert!(batcher.drain().is_none());
    }

    #[test]
    fn chunks_are_merged_in_order() {
        let mut batcher = OutputBatcher::new();
        batcher.push("he");
        batcher.push("llo");
        batcher.push(" world");
        assert!(!batcher.is_empty());
        assert_eq!(batcher.drain().as_deref(), Some("hello world"));
        // 取走后回到空。
        assert!(batcher.is_empty());
        assert_eq!(batcher.pending_bytes(), 0);
    }

    #[test]
    fn draining_twice_only_returns_content_once() {
        let mut batcher = OutputBatcher::new();
        batcher.push("a");
        assert_eq!(batcher.drain().as_deref(), Some("a"));
        assert!(batcher.drain().is_none(), "第二次不该重复发送");
    }

    #[test]
    fn pending_bytes_tracks_the_total() {
        let mut batcher = OutputBatcher::new();
        batcher.push("abc");
        batcher.push("de");
        assert_eq!(batcher.pending_bytes(), 5);
        batcher.drain();
        assert_eq!(batcher.pending_bytes(), 0);
    }

    #[test]
    fn multibyte_output_is_merged_without_corruption() {
        // 合并是按块拼接而不是按字符处理，因此多字节字符不会被切开。
        let mut batcher = OutputBatcher::new();
        batcher.push("你");
        batcher.push("好");
        assert_eq!(batcher.drain().as_deref(), Some("你好"));
    }

    // --- 会话 ---

    #[tokio::test(flavor = "multi_thread")]
    async fn a_session_streams_its_output() {
        let (mut session, mut output) =
            PtySession::spawn(&echo_command("hello"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        let mut collected = Vec::new();
        let deadline = std::time::Duration::from_secs(10);
        while let Ok(Some(chunk)) = tokio::time::timeout(deadline, output.recv()).await {
            collected.extend_from_slice(&chunk);
            // **回答 ConPTY 的光标位置查询（DSR）**。
            //
            // Windows 的 ConPTY 启动后会发 `ESC[6n`，并在拿到应答前不推进会话。
            // 真实环境里由 xterm.js 自动应答；测试里没有终端模拟器，因此必须手动回答，
            // 否则只会收到这一条查询、看不到任何命令输出。
            if collected.windows(4).any(|window| window == b"\x1b[6n") {
                session.write("\x1b[1;1R").expect("应答 DSR");
            }
            if String::from_utf8_lossy(&collected).contains("hello") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&collected);
        assert!(text.contains("hello"), "{text:?}");
        drop(session);
    }

    /// **特征测试：ConPTY 下子进程自然退出，读取线程收不到 EOF。**
    ///
    /// 这条记录的不是「期望行为」，是**平台事实**。钉在这里是为了让收养任务
    /// （[`SessionRegistry::spawn_reaper`]）存在的理由随时可查：
    ///
    /// 只有**关闭 PTY 主句柄**才会让读取线程收到 EOF，而主句柄归 `PtySession` 所有。
    /// 因此子进程跑完、输出也拿到了，通道依然不关。
    ///
    /// **这就是 `terminal:exit` 曾经永不自然触发、终端标签永远显示运行中的根因。**
    /// 修复在 [`SessionRegistry::spawn_reaper`]；端到端回归在 `aivc-ipc` 的
    /// `a_natural_exit_reaches_the_frontend_with_its_code`。
    ///
    /// 这里**故意直接驱动 `PtySession`、不经过 `SessionRegistry`**，所以没有收养任务。
    /// **如果这条测试某天开始失败（收到了 EOF），说明平台行为变了**——那时收养任务
    /// 可以简化，这条注释也就是过时的了。
    #[tokio::test(flavor = "multi_thread")]
    async fn conpty_does_not_report_eof_when_the_child_exits_on_its_own() {
        let (mut session, mut output) = PtySession::spawn(
            &echo_command("aivc-natural-exit"),
            DEFAULT_COLS,
            DEFAULT_ROWS,
        )
        .expect("spawn");

        let mut collected = Vec::new();
        let mut answered = false;
        // 只等 3 秒：这里预期的是**不会**关闭，不需要等满。
        let closed = tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while let Some(chunk) = output.recv().await {
                collected.extend_from_slice(&chunk);
                if !answered && collected.windows(4).any(|w| w == b"\x1b[6n") {
                    answered = true;
                    session.write("\x1b[1;1R").expect("应答 DSR");
                }
            }
        })
        .await;

        // 先证明子进程确实跑完了——否则「通道没关」可能只是因为它还没开始。
        let text = String::from_utf8_lossy(&collected);
        assert!(
            text.contains("aivc-natural-exit"),
            "子进程的输出应当已经到达（证明确实跑完了）：{text:?}"
        );
        assert!(
            closed.is_err(),
            "若这里开始收到 EOF，说明 ConPTY 行为变了，收养任务可以简化。已收到：{text:?}"
        );
        drop(session);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stopping_a_session_closes_its_output() {
        // 终止会话后读取线程应当结束——否则它会一直挂在阻塞读取上。
        let registry = SessionRegistry::new();
        let (session, mut output) =
            PtySession::spawn(&echo_command("bye"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        registry.insert("t1".to_owned(), session).await;
        registry.stop("t1").await.expect("stop");
        // 终止后最多再读到残留内容，随后通道关闭。
        let deadline = std::time::Duration::from_secs(10);
        while matches!(
            tokio::time::timeout(deadline, output.recv()).await,
            Ok(Some(_))
        ) {}
        assert!(registry.is_empty().await);
    }

    /// 在给定时间内持续排空输出，并**像终端模拟器那样应答 DSR**。
    ///
    /// # 为什么必须边等边排空
    ///
    /// `ConPTY` 启动后会发 `ESC[6n`（光标位置查询），并在拿到应答前不推进会话。
    /// 真实环境由 xterm.js 自动应答；测试里没有终端模拟器，因此要手动回答。
    ///
    /// 更重要的是：**阻塞等待进程结束而不排空输出会死锁**——读取线程往有界通道里写，
    /// 通道满了就阻塞，ConPTY 的输出管道随之填满，子进程再也推不动。
    /// 这是生产代码同样必须遵守的约束，不只是测试的事。
    async fn drain_answering_dsr(
        session: &mut PtySession,
        output: &mut mpsc::Receiver<Vec<u8>>,
        limit: std::time::Duration,
    ) -> Vec<u8> {
        let mut collected = Vec::new();
        let step = std::time::Duration::from_millis(50);
        let deadline = std::time::Instant::now() + limit;
        let mut answered = false;
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(step, output.recv()).await {
                Ok(Some(chunk)) => {
                    collected.extend_from_slice(&chunk);
                    if !answered && collected.windows(4).any(|window| window == b"\x1b[6n") {
                        answered = true;
                        let _ = session.write("\x1b[1;1R");
                    }
                }
                // 通道关闭：读取线程已结束。
                Ok(None) => break,
                Err(_) => {}
            }
        }
        collected
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_exit_code_is_reported() {
        let (mut session, mut output) =
            PtySession::spawn(&echo_command("done"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        // **必须先排空输出再 `wait()`**：不排空时 ConPTY 会卡在 DSR 上，`wait()` 永久阻塞。
        drain_answering_dsr(&mut session, &mut output, std::time::Duration::from_secs(5)).await;
        let code = session.wait().expect("wait");
        assert_eq!(code, 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn kill_is_idempotent_after_exit() {
        // 界面可能重复收到关闭请求，重复终止不该报错。
        let (mut session, mut output) =
            PtySession::spawn(&echo_command("x"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        drain_answering_dsr(&mut session, &mut output, std::time::Duration::from_secs(5)).await;
        session.wait().expect("wait");
        session.kill().expect("第一次");
        session.kill().expect("第二次");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resizing_an_open_session_works() {
        let (session, _output) =
            PtySession::spawn(&echo_command("x"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        session.resize(80, 24).expect("resize");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_program_reports_a_spawn_error() {
        let command = TerminalCommand {
            file: "definitely-not-a-real-program-xyz".to_owned(),
            args: Vec::new(),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            remove_env: BTreeSet::new(),
        };
        let error = PtySession::spawn(&command, DEFAULT_COLS, DEFAULT_ROWS).expect_err("必须失败");
        assert!(error.to_string().contains("创建终端失败"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_registry_tracks_and_stops_sessions() {
        let registry = SessionRegistry::new();
        assert!(registry.is_empty().await);
        let (session, _output) =
            PtySession::spawn(&echo_command("x"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
        registry.insert("t1".to_owned(), session).await;
        assert_eq!(registry.len().await, 1);

        assert!(registry.stop("t1").await.expect("stop"));
        assert!(registry.is_empty().await);
        // 再停一次返回 false 而不是报错。
        assert!(!registry.stop("t1").await.expect("stop"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn writing_to_an_unknown_session_is_an_error() {
        let registry = SessionRegistry::new();
        let error = registry.write("missing", "x").await.expect_err("必须失败");
        assert!(error.to_string().contains("终端会话不存在"), "{error}");
        let error = registry
            .resize("missing", 80, 24)
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("终端会话不存在"), "{error}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_all_clears_every_session() {
        let registry = SessionRegistry::new();
        for id in ["a", "b", "c"] {
            let (session, _output) =
                PtySession::spawn(&echo_command("x"), DEFAULT_COLS, DEFAULT_ROWS).expect("spawn");
            registry.insert(id.to_owned(), session).await;
        }
        assert_eq!(registry.len().await, 3);
        registry.stop_all().await;
        assert!(registry.is_empty().await);
    }
}
