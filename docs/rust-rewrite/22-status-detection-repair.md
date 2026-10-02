# 22 · 状态检测链路的修复

承接 [21](21-session-handoff.md) 第 3 节记录的「已确认：状态检测整条链路是死的」。
本轮把它修完了，并且在修的过程中发现**第二处独立断点**——只修第一个，功能依然是死的。

## 1. 两个断点

关注中心条要在界面上出现，需要一整条链都通。两个断点互相独立：

| # | 断点 | 后果 |
| --- | --- | --- |
| 1 | `mark_input` **在生产代码里从未被调用** | `turn_active` 永远 `false`，`inspect_output` 每次都在第一行返回 |
| 2 | `terminal:status` 事件的 `terminalId` 是**空串** | 前端按 `terminalId` 分发，状态永远落到 `""` 这个不存在的键上 |

### 断点 1：检测器接口只接了三分之一

`inspect_output` 在 `crates/aivc-terminal/src/status.rs` 的第一件事就是：

```rust
if !self.turn_active || data.is_empty() {
    return None;
}
```

而 `turn_active` 只由 `mark_input` 置位。生产代码里 `mark_input` 的调用点是**零**——
检测器是输出泵任务（`crates/aivc-ipc/src/terminal.rs` 的 `pump_output`）里的一个
**局部变量**，写回路径（`terminal:write`）根本够不到它。

### 断点 2：事件带着空编号出门

`TerminalStatusDetector::publish` 构造事件时填的是：

```rust
terminal_id: String::new(),
```

检测器**不知道**自己属于哪一路终端——它只管文本。泵当时直接把 `event.terminal_id`
发出去，于是每条状态事件的 `terminalId` 都是 `""`。

前端的接收侧（`ui/src/app/use-terminals.ts`）是按 `terminalId` 建索引的：

```ts
next[payload.terminalId] = payload.attention;
```

于是提醒被写进 `""` 这个键。而关注中心条是这样算出来的：

```ts
terminals.terminals.filter((item) => terminals.attention[item.id]);
```

`""` 匹配不上任何终端的 `id`，所以**即使断点 1 修好了，条也永远不出现**。

**这两个断点为什么难发现**：单元测试直接调检测器，绕过了「谁来调」和「谁来贴编号」
这两件事；而门禁只证明类型对、逻辑对。缺的全是**装配**。

## 2. 修法

### 共享检测器

新增类型别名与一处字段（`DetectorHandle` 的文档注释里写了为什么必须共享）：

```rust
type DetectorHandle = Arc<Mutex<TerminalStatusDetector>>;
// TerminalCommands 上：
detectors: Arc<Mutex<HashMap<String, DetectorHandle>>>,
```

- `spawn_session` 建检测器、登记进表、把同一个 `Arc` 交给泵
- `write` 按 `terminalId` 找回同一个实例，喂 `mark_input`
- `stop` 与泵退出时从表里摘掉——它按会话持有输出缓冲，不清理会持续增长

**为什么选共享状态而不是「第二条通道」**：第二条通道会让泵看到的顺序与真实顺序脱钩。
写回是立刻写进 PTY 的，而通道消息要排队；程序对这次输入产生的**第一批**输出完全可能
先被泵取走。共享状态让顺序由锁保证，没有这个竞态。

### 顺序：先标记，后写 PTY

```rust
self.mark_input(&terminal_id, data);      // 先
self.registry.write(&terminal_id, data).await   // 后
```

**不能反**。`mark_input` 干的就是「把检测器置为运行中」，反过来的话，程序对这次输入的
第一批输出有机会先被泵检查——而 `inspect_output` 在非运行态直接丢，那一批就被整批
丢掉。而它恰恰常常正好带着提示符或确认请求。

### 编号在发射处贴上

新增 `status_payload(terminal_id, event)`，用调用方手上的**真实 id** 覆盖，并把这个
陷阱写进它的文档注释。泵与 `mark_input` 两处都走它——只有一处走，另一处迟早会退回去。

## 3. 验证

### 单元测试（含假绿检查）

`writing_input_marks_the_shared_detector_and_labels_the_event`：故意用一个**不存在的
会话**——注册表写回必然失败，但 `mark_input` 发生在写回**之前**，所以它照样得跑完。
这既证明接线存在，也顺带把顺序钉住。

**两个断点各自都做了假绿检查**（临时改回旧行为，确认测试真的会红）：

| 回退哪一处 | 结果 |
| --- | --- |
| `status_payload` 改回 `event.terminal_id` | FAILED：`left: String("")` ← 正是生产上的那个空串 |
| 去掉 `self.mark_input(...)` 调用 | FAILED：`is_turn_active()` 断言 |

### 端到端（真实应用 + 真实 PTY）

触发点选得很讲究：确认模式的匹配要求提示文本落在缓冲区**末尾**，而交互式 shell 的提示符
永远跟在输出后面。所以用一条**打印后阻塞**的命令：

```
Read-Host "Do you want to proceed"
```

它会停在 `Do you want to proceed: ` 上等输入，缓冲区尾部正好落在确认模式上。

```
$env:CDP_PROBE = '...点 + Shell，等 xterm 出现...'
$env:CDP_TYPE  = 'Read-Host "Do you want to proceed"'
$env:CDP_READ  = '...读 .attention-bar / .xterm-rows...'
./scripts/ui-check.ps1 -Name 22-关注提醒 -Wait 12
```

实测结果（截图见 `shots/22-关注提醒.png`）：

```json
{"bar":"1 个终端在等你确认。\n\n等待确认",
 "active":"xterm-helper-textarea",
 "tail":"PS C:\\Users\\akim> Read-Host \"Do you want to proceed\"\nDo you want to proceed:  "}
```

**关注中心条真的出现了**，且与终端尾部的文本对得上。

## 4. 当时仍未接线：`mark_exit`（✅ 已在下一轮修完）

> **本节提出的疑问已由 [23](23-natural-exit-and-exit-code.md) 查清并修完。**
> 答案是：**自然退出时读取线程确实收不到 EOF** —— 也就是说 `terminal:exit` 事件
> 本身从不自然触发，问题比 `mark_exit` 大得多。修法是给注册表加一个轮询
> `try_wait()` 的收养任务。以下保留当时的记录。
>
> 一处需要更正的推断：本节曾说「泵在通道关闭时可以把会话从注册表取出来、`wait()`
> 拿真码」。23 采用的是**注册表把码放进一张表、泵事后取走**，因为 `wait()` 在
> `try_wait()` 已经收割过之后的行为没有把握——**不猜**。

`mark_exit` 的生产调用点依然是**零**。本轮**故意不接**，因为拿不到真实的退出码：

- 泵只能观察到「输出通道关闭」，而**退出码只有 `PtySession::wait()` 知道**
- 会话归 `SessionRegistry` 所有，泵没有句柄
- 用 `0` 去调它是个**假动作**——`mark_exit(0)` 本来就返回 `None`，等于把「没接线」
  伪装成「接好了」。这正是本项目反复出现的那类病，不能再添一个

它的唯一用户可见效果是「会话异常停止」（退出码非 0 时的错误提醒）。
`turn_active = false` 那一半无所谓：通道关了就不会再有输出。

**修它之前要先查清一件事**（不要猜）：**子进程自然退出时，读取线程到底会不会收到
EOF？** 现有测试 `the_pump_forwards_output_and_reports_exit` 是显式 `drop(session)`
之后才等到通道关闭的——它证明不了自然退出。若自然退出收不到 EOF，那 `terminal:exit`
事件本身也从不自然触发，问题比 `mark_exit` 更大。

## 5. 门禁

| 项 | 结果 |
| --- | --- |
| `cargo test -p aivc-ipc` | 131 通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| `cargo run -p xtask -- check-layers` | 通过（8 crate / 5 层） |
| `cargo build -p aivc-app` | 通过 |
| 端到端关注提醒 | 出现 |
| 旧平台 `src/` 改动 | 0 |

> **踩坑记录**：`cargo test` 经 PowerShell 的 `2>&1 | Select-Object` 管道时，
> pwsh 会因 cargo 写 stderr 报 `NativeCommandError`，把退出码变成 1。
> **这不是测试失败。** 看 `$LASTEXITCODE` 或输出的 `test result` 行。

> **踩坑记录**：`cargo xtask` 在本仓库不可用（没有安装 cargo-xtask），
> 要用 `cargo run -p xtask -- check-layers`。

> **踩坑记录**：子 `pwsh` 在提权沙箱里**不在 PATH 上**，`& pwsh -File ...` 会报
> `CommandNotFoundException`。当前已经在 pwsh 里，直接 `& .\scripts\ui-check.ps1`。

> **文档位置**：`docs/rust-rewrite/` 在**工作区根**（`D:\code\ai\docs\`），
> 不在 `aivc-rs/` 里面。
