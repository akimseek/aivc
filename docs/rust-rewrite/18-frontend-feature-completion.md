# 18 · 新前端：功能补完（一次做完的一轮）

> 承接 [15](15-new-frontend-in-aivc-rs.md) / [16](16-session-module-complete.md) /
> [17](17-components-and-terminal.md)。这一轮把剩余功能一次补齐，不再逐步确认。

---

## 1. 做了什么

| 项 | 结果 |
| --- | --- |
| 终端按需加载 | 主包 **573 KB → 241 KB**，xterm 单独成块（332 KB）按需加载 |
| 系统终端 | 接线 `terminal:start-system`（原先在 `PENDING` 里） |
| 终端关注中心 | 把已有的状态检测器接进输出泵，标签上出现「等待确认」提示 |
| 会话高级操作 | 分支 / 复制 / 导出 / 重命名 |
| 快捷访问 | 收藏 + 最近打开（跨目标，标明归属） |
| 跨平台全局搜索 | 并发搜所有目标，**一个目标失败不拖垮整次搜索** |
| 配置层 | 供应商 / 技能 / 网关 / 环境 四个页签 |
| 命令面板 | ⌘K / Ctrl+K |
| 诊断面板 | 把**全部目标**的四态状态并排列出来 |

## 2. Rust 侧的三处改动

### `terminal:start-system`：系统终端

新增 `aivc-terminal/src/shell.rs`：`ShellKind`（`auto`/`powershell`/`cmd`/`git-bash`/
`wsl`/`wsl-default`/`bash`/`zsh`/`fish`）与**纯函数** `resolve_shell`。

- **找不到就报错，不回退**：用户选了 zsh 却拿到 bash，会以为配置文件全没生效；
- **Git Bash 单独找**：它默认不在 PATH 里（Git for Windows 只加 `cmd` 目录），
  因此先查常见安装位置；
- **WSL 目标自动开该发行版的 shell**：`auto_kind_for` 只加这一条规则，平台判断
  仍归 `resolve_shell`——两处都判会让「谁说了算」变得难查。

顺带把「起 PTY、登记、泵输出」抽成 `spawn_session`：Codex 终端与系统终端**只有
「拼什么命令」不同**。

### 关注中心：把状态检测接进输出泵

`aivc-terminal/src/status.rs` 里本来就有 `TerminalStatusDetector`（带跨数据块的缓冲，
用于识别被切开的提示文本），但**从未接进任何地方**。

现在输出泵每路终端持一个检测器，检测到需要提醒时发 `terminal:status` 事件。
只喂 `inspect_output` 不喂 `mark_input`——后者需要知道「用户提交了什么」，而写回路径
与泵是两个独立任务；只做输出侧已经覆盖关注中心真正要提醒的那件事：**程序在等你确认**。

通道与 `terminal:data` **分开**是有意的：数据是高频原始字节，状态是低频语义事件，
混在一条上会让前端在每个字节块上重跑检测。

### 导出：由后端决定落点

原生「保存到…」对话框需要 Tauri 的 dialog 插件（离线装不上），而导出**不该因此
不可用**。因此 `codex:export-session` 的 `filePath` 改为可选：没给就落到
`<运行根>/data/exports/session-<短编号>.<ext>`，并把**实际路径**回给界面显示。

用编号而不是标题命名：标题可能含路径分隔符与控制字符，清洗它要处理一堆边界。

## 3. 两个**明确的缺口**（不是遗漏，是有理由的）

### ① `terminal:attention-notification` 未接线

它是**桌面通知 + 点击聚焦标签**，需要 `tauri-plugin-notification`——离线 cargo
拿不到这个 crate。

我实现的是**应用内**的关注提示（标签上一个小点），那才是真正解决问题的部分：
用户切走后能看见「那一路在等我」。桌面通知只是锦上添花。

### ② 网关的健康/日志/失败转移、供应商的新增/编辑未接入界面

这几条命令（`gateway:get-vendor-health`、`gateway:query-logs`、
`gateway:list-failover-rules`、`vendor:save`…）返回的是**手拼的 `json!`**，而不是某个
Rust 结构体序列化的结果。字段名没有类型层面的约束——**改错不会有编译错误**。

`skill:list` 也是一样（见 `types.ts` 里 `SkillEntry` 的注释）。

先补后端的类型契约，再接界面；否则前端只能靠猜字段名，而猜错是**静默**的。

## 4. 验证

| 项 | 结果 |
| --- | --- |
| `tsc --noEmit` | 通过 |
| `vitest run` | **12 项通过**（2 个文件） |
| `vite build` | 260.6 KB（gzip 81.4 KB）+ 终端块 332 KB |
| 七项门禁 | **全部通过** |
| 真实应用 | 命令序列正常、`ui.started` 正常 |

**仍然没有肉眼看过界面。** 上面的证据说明「类型正确、测试通过、构建成功、应用能跑」，
不说明它**看起来对**——尤其是新增的覆盖层、命令面板、终端标签。

## 5. 下一步（按价值排序）

1. **补后端的类型契约**：把 `skill:list`、`gateway:*` 的手拼 JSON 换成结构体序列化，
   再接界面。这是眼下最该做的一件——它同时消除一类静默错误。
2. **工作区预设与 WSL Codex 目录的编辑界面**（设置模块里剩下的一半）。
3. **安装桌面通知插件**，补上 `terminal:attention-notification`。
4. **前端测试覆盖面**：现在只覆盖纯函数，组件与 hook 还没有测（需要引入 jsdom）。
