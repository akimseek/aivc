# 21 · 会话收口与接手清单

> 本文记录**当前真实状态**与下一步的起点。前序文档：[15](15-new-frontend-in-aivc-rs.md)（新前端）、
> [17](17-components-and-terminal.md)（组件层与终端）、[19](19-ui-verification-and-two-bugs.md)（界面验证流程）、
> [20](20-message-count-migration.md)（条数语义）。

---

## 1. 核验结果（本轮实测）

| 项 | 结果 |
| --- | --- |
| 七项门禁（`scripts/verify.ps1`） | **全部通过** |
| 前端 `tsc --noEmit` | 通过 |
| 前端测试 `vitest run` | **27 项通过**（3 个文件） |
| 旧工程 `D:\code\ai\src\` | **未改动**（`git status --porcelain -- src/` 为空） |
| `aivc-rs` 运行期残留 | 无（`data/`、`logs/` 每轮清理） |
| `PENDING` | **2 条**：`app:command`、`app:exit-choice` |

## 2. 本轮完成的功能

| 项 | 验证方式 |
| --- | --- |
| 界面验证流程（截图 + DOM 探测 + 点击） | 全程在用 |
| ① 标题线：512 KB 补读 + 取首行去标记 + 不显示裸 UUID | 截图验证，标题从 UUID 变为真实提示词 |
| ② 列表不再声称已知总数 | 截图验证，显示「—」 |
| 跨平台搜索范围筛选 | **CDP 点击验证** |
| 终端关注中心汇总 | ⚠️ 无回归已验证；**条本身未触发过** |

## 3. 未验证 / 未完成

### 未验证

- **终端关注中心条**：显示条件是「有终端处于等待确认」。需要真实的 Codex 确认提示才会
  触发，普通 shell 输出不会命中检测器。**代码路径、无回归、条件为假时不渲染**这三点已验，
  但条长什么样、点下去是否真能切过去，**没有验证过**。
- **原生对话框相关路径**（导出「另存到…」、技能导入的目录选择）：CDP 拍不到操作系统的
  真窗口，只能手动验。
- **终端输入链路**：`term.onData → terminal:write`（xterm 对 `ESC[6n` 的应答）从未手动验证。
  这条断了的话，终端显示正常但完全没反应。

### 未完成

#### ✅ 已修复：终端输出曾是全部丢弃（**这条线已完成**）

> **结论：内嵌终端现在端到端可用。** 下面保留完整的过程记录——它是这个项目里
> 「层与层脱节」这一类 bug 最有代表性的例子，值得留下来。
>
> 四个环节现在都验证过了：命令路由 ✓、进程启动 ✓、输出显示 ✓、输入回传 ✓。
> 验证方式：`scripts/ui-check.ps1` + `cdp-shot.cjs`（可截图、可求值、**可派发真实键盘事件**）。

**曾经的症状**：内嵌终端（**系统终端与 Codex 终端都一样**）**从来没有显示过任何输出**。

**证据链**（每一环都实测过）：

| 环节 | 证据 |
| --- | --- |
| 路由已接上 | 终端标签出现、界面无错误 |
| PTY 是真的 | `aivc-terminal` 的 `default = ["pty"]`，用真实 `portable-pty` |
| shell 真的启动了 | 点击 `+ Shell` 后 `powershell.exe` 1→2 个，新进程带 conhost |
| 数据真的产生了 | 日志 `事件被 NullEmitter 丢弃 channel="terminal:data" bytes=4` |
| **但被丢弃了** | 同一条日志 |

**根因**：发射器有**两个互不相干的持有者**。

```rust
router.rs:115   TerminalCommands::new(registry, Arc::new(NullEmitter))  // 构造时捕获一份
terminal.rs:88  emitter: Arc<dyn EventEmitter>,                         // 存成自己的一份
lib.rs:88       emitter: Arc<RwLock<Arc<dyn EventEmitter>>>,            // App 层另一个
tauri_app.rs:216 .install_emitter(Arc::new(TauriEmitter(...)))          // 只换 App 层那个
```

**修法**（两选一）：

- **A**：`TerminalCommands.emitter` 改成 `Arc<RwLock<Arc<dyn EventEmitter>>>`，App 与路由器**共用同一个 Arc**
- **B**（改动面更小）：构造时传入一个**转发代理**，它每次 `emit` 去读那个可替换的槽位；
  给 `CommandRouter::new` 加一个发射器参数，`install_emitter` 指向同一个槽位

**动手前必须先读清**：`App` 如何持有 `CommandRouter`（`Arc`？可变借用？在 `start` 的哪个阶段安装？）。
emit 路径上引入 `RwLock` 时要留意嵌套调用导致的死锁。

**配套测试**：`install_emitter_replaces_the_slot_the_pump_reads`（已加）——
断言换装改的**就是泵读的同一个槽位**（`Arc::ptr_eq`），且换装后事件真的到达新发射器。
另有 `a_poisoned_slot_drops_events_instead_of_panicking` 覆盖失败安全。

> **一个值得记住的观察：这个 bug 在旧设计下根本没法测。**
>
> 旧字段是 `emitter: Arc<dyn EventEmitter>` —— 一个**值**。处理器构造时把它捕获一份，
> 测试能观察的只有「传进去的那个」，而 bug 恰恰是「事后换装影响不到它」：
> **测试里没有可以插进去观察的位置。**
>
> 改成槽位之后缝才出现，测试也才写得出来。**修好结构，测试才可能写出来** ——
> 反过来靠加测试去抓这类问题往往是徒劳的。
>
> 同样地，本会话另有三个 bug 也属于「结构上不可见」：外层路由漏接（测试查命令表、
> 不查 dispatch 的 arm）、`resize` 掩盖错误（测试断言返回值，不断言界面显示了什么）、
> 早期输出被丢弃（丢弃发生在事件与接收端之间，两侧各自都「正确」）。
> **把它们照出来的是打开应用看一眼，不是加测试。**

#### 已留下的一处诊断改进

`NullEmitter::emit` 现在会在 **debug 级**打印被丢弃的通道与字节数。
**正常情况下不该看到这条日志**；看到就说明发射器没被换掉。
以前它完全静默，导致「写入虚空」与「真的没有事件」在日志里无法区分。

### 其他未完成

| 项 | 说明 |
| --- | --- |
| `app:exit-choice` | 退出确认。**改窗口生命周期**，见下 |
| `app:command` | 窗口控制（最小化/最大化/关闭） |
| `Option<u64>` 条数语义 | 彻底区分「空会话」与「未知」。方案见 [20](20-message-count-migration.md) |
| `skill:import` 选单个文件 | 后端 `import_from_path` 本就支持文件与目录，缺的是前端的**文件**选择框 |
| 关注中心条本身 | ✅ **已修复**，见 [22](22-status-detection-repair.md) |

#### ✅ 已修复：状态检测整条链路是死的

> **本节记录的两个断点已在后续轮次修完**，完整记录见 [22](22-status-detection-repair.md)。
> 修的过程中还发现了**第二处独立断点**——`terminal:status` 事件的 `terminalId` 是空串，
> 只修 `mark_input` 的话功能依然是死的。以下保留当时的原始证据与推理，
> 作为「门禁全绿但功能从未生效」这一类问题的样本。
>
> **仍未接线**：无。`mark_exit` 与「自然退出无人察觉」已在 [23](23-natural-exit-and-exit-code.md) 修完。

**直接证据**（`crates/aivc-terminal/src/status.rs:190`）：

```rust
pub fn inspect_output(&mut self, data: &str) -> Option<TerminalStatusEvent> {
    if !self.turn_active || data.is_empty() {
        return None;        // ← 第一行就返回，什么都不检测
    }
```

而 `turn_active` **只由 `mark_input` 置位**；输出泵（`aivc-ipc/src/terminal.rs` 的
`pump_output`）**只调用 `inspect_output`，从不调用 `mark_input`**。

→ `turn_active` 永远是 `false` → **每次都在第一行返回**。

**后果**：`terminal:status` 事件从不产生。依赖它的全部失效——关注中心条、桌面通知
（`terminal:attention-notification`）。

**实测确认**：我在系统终端里 `echo "Permission required?"`，终端尾部确实出现了
`status.rs:369` 所匹配的文本，但 `.attention-bar` 数量为 0。

**更彻底的证据——检测器三个方法各自的调用点（已逐个查过）**：

| 方法 | 生产调用点 |
| --- | --- |
| `inspect_output` | `crates/aivc-ipc/src/terminal.rs:344`（输出泵）—— **唯一一处** |
| `mark_input` | **零**——只出现在 `status.rs` 的测试里 |
| `mark_exit` | **零**——只出现在测试里 |

也就是说：检测器的接口**只接了三分之一**。`turn_active` 永远为 false，
`inspect_output` 每次都在第一行返回；退出时的错误识别（`mark_exit`）同样从不发生。

**这不是「等级不够的缺陷」，是整个功能从未生效过。**

**修法**：让 `terminal:write` 的输入也喂给检测器。检测器现在是泵任务里的**局部变量**，
`write` 够不到它。两个方向：

- 泵与写回共享一个 `Arc<Mutex<HashMap<String, TerminalStatusDetector>>>`
- 或让写回通过**第二条通道**把「用户输入了」这一事件发给泵任务

**同一类形状**：能力只接了一半（`inspect_output` 接了，`mark_input` 没接），
另一半静默缺失，而门禁全绿。这与终端输出写进 `NullEmitter`、外层路由漏接是同一个病。
| `platform_command` 与 `HANDLED_BY_PLATFORM` 的双向一致性 | 目前只有单向 |

## 4. 下一步为什么是 `app:exit-choice`，以及它的风险

它会拦截窗口关闭请求 → 发事件问前端 → 按回答决定「退出 / 最小化 / 取消」。

**风险是具体的**：改到一半会让应用**关不掉**。这不是测试能兜住的 —— 测试不会真的点
关闭按钮，而 CDP 会话在窗口关闭时也会一起断掉。因此它需要：

1. **失败安全**：问了前端但超时没答复时，**照常退出**，不能把用户锁在应用里
2. 一次连续的、能跑完「构建 → 手动关闭窗口 → 看行为」的窗口
3. 做完后 `PENDING` 只剩 `app:command`

## 5. 工具与约定（接手时先读这段）

### 界面验证

```
./scripts/ui-check.ps1 -Name 13-xxx            # 截图
$env:CDP_PROBE = '...'; ./scripts/ui-check.ps1 -Name 13-xxx   # 截图 + 求值
```

- **表达式必须走环境变量 `CDP_PROBE`**：PowerShell 会吃掉命令行里的双引号，
  `querySelectorAll(".row")` 会变成 `querySelectorAll(.row)` 并报 `SyntaxError`
- 脚本会**拒绝在 `ui/dist` 比可执行文件新时运行** —— Tauri 在编译时嵌入前端，
  只跑 `vite build` 看到的是旧界面。这是踩过一次之后加的
- 脚本会打印求值异常（否则抛错与「结果为空」长得一样，也会白费轮次）

### 构建顺序（重要）

```
改前端 → vite build → **cargo build -p aivc-app** → ui-check
```

漏掉第三步就会验证旧资源，而且进程、日志、探测**全都不报错**。

### 沙箱

`vite` / `vitest` / `cargo` 联网 / `wsl.exe` / 应用调试端口都需要提权。
`cargo` 默认 `--offline`，但**联网是可用的**（早期把 `SEC_E_NO_CREDENTIALS` 误判成
「机器没网」，实际是沙箱拿不到 Windows 凭据）。

## 6. 截图索引

| 文件 | 说明 |
| --- | --- |
| `01-启动.png` | 首次看到界面（标题全是 UUID） |
| `02-基线.png` | 问题复现基线 |
| `05-标题修复后` | 标题变为「目录 · 时间」 |
| `06-标题补读后` | 标题变为真实提示词 |
| `07-标题粒度.png` | 标题去掉 markdown 标记 |
| `09-条数改为未知` | 会话头显示「—」 |
| `11-搜索范围.png` | 搜索范围筛选（CDP 点击后） |
| `12-关注中心.png` | 关注中心改动后的无回归确认 |
