# 26 · 清空 PENDING：退出确认与应用命令

`PENDING` 归零。命令面**全部接线**。

## 1. 这两条为什么落不到命令路由器里

`app:exit-choice` 要拦窗口关闭、`app:command` 的 `quit` 要弹确认框——都需要 Tauri 的
窗口句柄，而 `aivc-ipc` 里没有 tauri 依赖。因此与剪贴板、原生对话框一样，归
`aivc-app` 的 `platform_command` 处理，并登记进 `HANDLED_BY_PLATFORM`（5 → 7）。

`PENDING` 变成 `[]`，但**机制保留**：它是一条「缺口必须可见」的约定，不是待办清单。
下一个未接线的域在这里登记，测试会保证它不会悄悄变长。

## 2. 退出确认：拦得住，而且关得掉

关闭窗口**不能**直接关掉——应用里跑着终端与本地网关。流程是「拦住关闭 → 问前端 →
按回答决定」，方向由**后端发起**：只有后端拿得到「可以阻止关闭」的钩子。

三条回答：`minimize`（隐藏到托盘）/ `quit`（跑完清理再退出）/ `cancel`（什么都不做）。

### 失败安全：窗口**永远**关得掉

这是本模块最要紧的一条。拦住关闭之后如果前端不响应（页面崩了、卡死、事件没接上），
用户就被**锁在应用里**了。两条放行规则：

| 规则 | 实现 |
| --- | --- |
| 已经在问了，用户**又点了一次**关闭 | `ExitFlow::close_requested` 第二次返回 `Allow` |
| 前端**不可达**（页面已经没了） | `frontend_reachable()` 用 `window.eval` 探测，失败就放行 |

### 为什么**不是**「超时自动退出」

超时和「用户正在读对话框」是**同一个信号**。对话框弹出来之后用户可能花三十秒才决定，
而一个「三十秒没回答就退出」的定时器会在用户还在看的时候**杀掉应用和所有终端**——
拿数据丢失的风险去换一个不存在的锁定风险。

上面两条规则给出同样的「关得掉」保证，而且**不可能在用户正常操作时误触发**。

## 3. 顺带修掉的两处

| 问题 | 说明 |
| --- | --- |
| 托盘「退出」跳过全部清理 | 它是直接 `app.exit(0)`，而清理挂在 `RunEvent::ExitRequested` 的 `code: None` 分支上——带码的请求被显式放行。现在三条退出入口共用 `exit_after_cleanup` |
| 「最小化到托盘」可能让应用彻底够不着 | 窗口 `hide()` 之后**只能**从托盘找回。托盘安装失败时那条路径会变成黑洞。现在 `tray::is_installed()` 不可用时退化成普通最小化（还在任务栏里） |

托盘模块文档里那条「A4 尚未实现」的缺口说明也随之删掉了——它现在实现了。

## 4. 状态机与测试

`ExitFlow` 是**纯逻辑**（新建的 `crates/aivc-app/src/exit.rs`），Tauri 那边只做
「拿到状态 → 照做」。9 条单元测试按行为命名：

- 第一次关闭 → `Prompt`
- **已在询问时再关一次 → `Allow`**（失败安全）
- 取消之后**能再次被问到**（否则用户取消一次就再也关不掉）
- 最小化不进「正在退出」
- 选了退出之后，后续关闭一律放行（收尾途中会再关一次窗口，别拦自己）
- 托盘重复点「退出」只问一次
- 托盘「退出」与窗口关闭**共用同一份状态**（各记一份会弹两个框）
- 锁中毒时**恢复数据**而不是 panic（panic 只会让窗口更关不掉）

## 5. 端到端

`app:command {command:"quit"}` → 对话框出现：

```json
{"dialog":true,"buttons":["取消","最小化到托盘","直接退出"],
 "copy":"最小化后应用会继续在后台运行，终端与本地 Gateway 都不受影响； 直接退出…"}
```

点「取消」→ 对话框消失；**再发一次请求 → 重新弹出**；应用仍然活着：

```json
{"dialogBefore":true,"goneAfterCancel":true,"rePromptedAfterCancel":true,"appAlive":true}
```

截图见 `shots/26-退出确认.png`。

## 6. ✅ 已查清：窗口关闭按钮那条路径是好的

上一轮这里标着「未能验证」，因为三种触发方式的结果互相矛盾。**结论：拦截是好的，
而且我担心的那个真缺陷不存在。**

### 怎么查清的

给能力表加了 `core:window:allow-close`，改用
`window.__TAURI__.window.getCurrentWindow().close()` —— 这条**确实**走 Rust 的
`Window::close()`，会发 `CloseRequested`：

```json
{"hasWindowApi":true,"err":null,"dialog":true,
 "buttons":["取消","最小化到托盘","直接退出"],"pageAlive":true}
```

- `close()` 成功且 `dialog: true` → **事件被拦住了**，`api.prevent_close()` 生效
- `pageAlive: true` → 页面没被销毁
- 也就是说 `frontend_reachable()` **返回了 `true`** —— 「它误判成不可达」这个可能被排除

### 之前为什么矛盾

**WebView2 对页面自己发起的 `window.close()` 在操作系统层面直接关窗，绕过了 Tauri 的
`CloseRequested`。** 所以那条路本来就不经过我们拦的钩子——用它验证拦截，验的是一个
不存在的问题。

### 为什么保留那条权限

`core:window:allow-close` 是目前**唯一**能自动化验证这条路径的手段，而且它走的是**同一条
确认流程**——最坏情况只是弹出确认框，不构成绕过。它同时也是前端「关闭窗口」这一动作
本来就需要的能力。

> **批次二终审补注（2026-09-30）**：本节记的是当时的判断，两处已经变了——
> ① 退出确认框换成了共享 `Dialog`：外壳与页脚的类名是 `.dialog-panel` / `.dialog-footer button`，
> 下面那条探针里的 `.overlay-panel-confirm` / `.overlay-foot` 已不存在（类名随批次一/二改名）；
> ② `core:window:allow-close` **已删除**——前端没有消费者（窗口关闭走 `app:window-close`，
> 由 Rust 侧调 `window.close()`；能力只约束页面发起的 IPC），它作为「自动化验证手段」也随之失效，
> 那条回归方式**不能再跑**。

**回归方式**（没有自动化测试能覆盖它，因为它需要一个真窗口）：

```
$env:CDP_PROBE='(async()=>{const a=window.__TAURI__;await a.window.getCurrentWindow().close();
  await new Promise(r=>setTimeout(r,2000));
  return JSON.stringify({dialog:!!document.querySelector(".overlay-panel-confirm")});})()'
# 以 --remote-debugging-port=9222 启动应用后：
node scripts/cdp-shot.cjs out.png     # 期望 {"dialog":true}
```

### 四条路径的验证状况

| 路径 | 证据 |
| --- | --- |
| 关闭窗口 → 弹确认框 | ✅ 端到端（上面那条） |
| `cancel` → 收起，且**能再次被问到** | ✅ 端到端 |
| `quit` → 跑完清理后应用**真的退出** | ✅ 端到端（进程消失） |
| `minimize` → 隐藏窗口、不退出 | 仅单元测试（`ChoiceAction::Hide` 且 `is_quitting()` 为假）；`hide()` 之后的托盘恢复是既有路径 |

## 7. 门禁

| 项 | 结果 |
| --- | --- |
| `cargo test --workspace` | 全绿（24 个测试目标） |
| `cargo clippy --workspace --all-targets -- -D warnings` | 通过 |
| 前端 `tsc --noEmit` / `vitest run` / `vite build` | 通过 / 45 通过 / 通过 |
| 端到端：退出确认框 | 出现，取消后能重新询问 |
| 端到端：关闭窗口 → 确认框 | 出现，页面未被销毁 |
| 端到端：选「直接退出」 | 应用跑完清理后真的退出 |
| 旧平台 `src/` 改动 | 0 |

## 8. 工具陷阱（这一轮踩到的）

> **`cdp-shot.cjs` 在 CDP 连接断开时会静默退出 0**——Node 的事件循环空了就退出，
> `main()` 的 promise 永远不会 settle。所以「退出码 0」在这里**不代表成功**；而
> `ui-check.ps1` 只看退出码，于是打印了「截图已保存」但**文件并不存在**。
> **判断依据应当是 PNG 是否真的生成**，而不是退出码。

> **PowerShell 里 `... | Select-Object -First N` 会在取够 N 条后关闭管道**，
> 上游进程被提前终止，输出随之丢失。诊断输出一律走 `Out-File` + `Get-Content`。

> **`app.log` 走的是 `TeeWriter`**，进程被强杀时可能还是 **0 字节**——
> 指望它排查「进程为什么没退」不可靠。

> **`window.close()` 不是验证窗口关闭的正确手段**：WebView2 会绕过 Tauri 的窗口事件。
> 用 `window.__TAURI__.window.getCurrentWindow().close()`。
