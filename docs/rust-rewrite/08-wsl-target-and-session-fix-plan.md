# WSL 目标与会话的复核与改进方案

> 触发：实机运行后两个现象 ——
> ① 目标列表只有 `WSL · Ubuntu-24.04`，本机目标不见了；
> ② 点历史会话报错，`listSessions` **30 秒超时**（退出码 -1）。

## 一、已确认的事实（有日志）

```
14:57:09  WSL 发行版列表 distros=["Ubuntu-22.04", "Ubuntu-24.04"]
14:57:11  解析不出 Codex 主目录，跳过该发行版 distro=Ubuntu-22.04
14:57:26  收到 IPC 命令 codex:list-targets
14:57:26  WSL 发行版列表 distros=["Ubuntu-22.04", "Ubuntu-24.04"]
14:57:29  收到 IPC 命令 codex:list-sessions   ×3
14:57:59  ui.listSessions [30155ms] error: 列出 /home/akim/.codex/sessions 失败，退出码 -1
```

**现象 ② 的成因明确**：`列出 /home/akim/.codex/sessions` 花了 30 秒后以 `-1` 失败。
30 秒是**超时**，`-1` 是**进程被强杀后拿不到退出码**。
而我实测「进入 Ubuntu-24.04」只需 3.3 秒（冷）/ 0.1 秒（热）——
**所以问题不在 WSL 慢，在我的调用方式。**

**现象 ① 的成因未确认**：本机 `C:\Users\akim\.codex\sessions` **确实存在**（实测 1 个条目），
`local_target()` 的判据就是它存在。因此理论上本机目标应当出现。
**日志里没有任何关于本机目标的记录**——这是需要补的观测点，不是可以猜的地方。

## 二、复核老平台：它怎么做 WSL 命令

`src/electron/providers/wsl.ts:102`：

```js
const encodedScript = Buffer.from(script, "utf8").toString("base64");
const shellScript = `printf %s ${shellQuote(encodedScript)} | base64 -d | bash`;
spawn(wslExe, ["-d", distro, "--", "bash", "-lc", shellScript], {
  windowsHide: true,
  stdio: ["pipe", "pipe", "pipe"]
});
```

### 关键设计：**脚本先 base64，再作为单行参数传入**

为什么必须这样：从 Windows 到发行版内的 bash，参数要穿过**至少三层解析**——

```text
Rust/Node 构造命令行
  → Windows 命令行解析（引号、反斜杠、`^`、`%` 都有特殊含义）
  → wsl.exe 转发
  → 发行版内 bash 再解析一次
```

任意一层把脚本里的引号、`$`、换行、反引号吃掉或改写，结果从「命令失败」到
**「bash 在等 stdin 而永远不返回」** 都可能。而 base64 之后，
**载荷对每一层都是不含特殊字符的字母数字串**，三层解析都无从破坏它。

### 附带的一点

老平台 `stdio: ["pipe", "pipe", "pipe"]` 且**不写 stdin、不显式关闭**——
它能工作是因为 base64 那层让 bash 不再有理由去读 stdin。
**真正的保障是 base64，不是 stdin 的处理方式。**

## 三、我的实现与它的差异

我的 `WslExeLauncher::spawn_bash`：

```rust
Command::new(&self.executable)
    .arg("-d").arg(distro).arg("--").arg("bash").arg("-lc").arg(script)
    .stdin(Stdio::piped())      // ← 管道开着却不写、不关
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
```

**没有 base64 包装**，脚本原样穿三层解析。**这与 30 秒超时的现象一致**：
如果 bash 因解析破坏而落进「读 stdin」的分支，而 stdin 是个永不关闭的管道，
它就会一直等到超时。

**另一个可疑点**：`.stdin(Stdio::piped())` 但从不写入也不 drop 句柄 ——
即使没有解析问题，这也是一个「可能永久阻塞」的形状。
**应当直接用 `Stdio::null()`**：这些脚本不需要 stdin。

## 四、改进方案

### 第一优先：按老平台的方式传脚本（修现象 ②）

```rust
fn spawn_bash(&self, distro: &str, script: &str) -> Result<Child> {
    // 脚本先 base64：三层解析都无从破坏一个纯字母数字载荷。
    let encoded = BASE64.encode(script);
    let wrapper = format!("printf %s {} | base64 -d | bash", shell_quote(&encoded));
    Command::new(&self.executable)
        .arg("-d").arg(distro).arg("--").arg("bash").arg("-lc").arg(wrapper)
        .stdin(Stdio::null())   // 脚本不需要 stdin；开着它只增加「永久阻塞」的可能
        ...
}
```

**要点**：
1. base64 编码脚本（引入 `base64` 依赖；若离线不可得，用内联实现——
   编码是纯计算，二十行以内，且**可完整测试**）
2. `shell_quote` 仍要保留：base64 字母表含 `+` `/` `=`，某些 shell 下仍需引号
3. `stdin` 改为 `Stdio::null()`
4. 新增测试：**含引号、`$`、换行、反引号的脚本必须能正确往返**
   （用 `LocalBashLauncher` 在测试里跑真脚本，无需 WSL）

### 第二优先：把「本机目标为什么没出现」变成可观测（修现象 ①）

`local_target()` 目前**完全静默**——返回 `None` 时没有任何记录。
这与本工程已修的多处「静默失败」是同一形态。改为：

```rust
fn local_target() -> Option<Target> {
    let Some(home) = local_codex_home() else {
        tracing::debug!("解析不出本机 Codex 主目录，跳过本机目标");
        return None;
    };
    let sessions = home.join("sessions");
    if !sessions.is_dir() {
        tracing::debug!(home = %home.display(), "本机 sessions 目录不存在，跳过本机目标");
        return None;
    }
    ...
}
```

**然后重跑一次，日志会直接说明是哪一步返回了 `None`。**
在此之前，任何关于「本机目标为什么消失」的解释都是猜测。

### 第三优先：给 WSL 调用加「分层超时」

现在 `command_timeout` 是全局 60 秒，而会话列表用了 30 秒。
**目标探测与会话列表对延迟的容忍度不同**，应当按用途给不同的上限：
探测可以等久一点（用户在看下拉框），会话列表应当更快失败并给出可操作的提示。

## 五、为什么我没有直接动手改

**现象 ② 的修法我有把握**（老平台的 base64 做法是明确依据，且可测试）。
**现象 ① 我没有任何证据**——本机目录存在、代码判据也应当通过、日志无记录。
**在这种情况下改代码等于猜。**

因此本方案的执行顺序是：**先加观测 → 重跑 → 拿到事实 → 再改。**
