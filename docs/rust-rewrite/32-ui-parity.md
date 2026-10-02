# 界面一致性对照表（旧平台 × aivc-rs）

> 用户的口径（第 52 轮）：**面齐全、流程一致即可** —— 外观容器可以合并，
> **流程不合并**。「业务实现上不合理」可以改，但交互与信息架构要与旧平台一致。
>
> 这张表是**改界面之前必须先出的东西**。不看完整就改，只会再造一次偏差
> （第 51 轮那两个缺陷就是这么来的）。
>
> 状态：第 52 轮，**骨架已填，标「待补」的格子还需要读旧平台源码**。

---

## 一、已经核对清楚的三处流程差异（证据在手）

### 1. 点会话：**旧平台是进入终端会话**（我一度判断反了）

> **⚠️ 本节曾在第 53 轮写成「旧平台进详情、我直接开终端」，那个结论是错的。**
> 用户在第 58 轮直接纠正：**「点击左侧历史会话进入终端进行会话，不可能是进入详情再进入终端，
> 详情只是终端会话的一个子功能。」**
>
> 我的错误来源：只看到 `app.tsx:970` 的 `onOpenSession={(session) => openSessionDetail(...)}`
> 就认定它是侧栏列表的回调。**而它更可能是详情里「分支关系」面板的回调**
> （`branch-panel.tsx` 的 props 就有 `onOpen: (session) => void`——点父会话/子分支也会「打开那个会话」）。
> 我没读主编排（`app.tsx` L780–980）就下了结论。

**正确的流程**：

| | 旧平台 | aivc-rs |
| --- | --- | --- |
| 点左侧历史会话 | **进入终端会话**（起 CLI，继续那次会话） | **也进入终端** —— 这一层其实没差 |
| 进入之前 | 走**对话框**：工作目录 / 标题 / 提示词 / CLI 参数（恢复历史时是「设置工作目录 → 继续打开」） | **直接就进了** —— **这才是真正的差异** |
| 会话详情 | **终端会话的一个子功能**（时间线 / 加载更早消息 / 从此处分支） | 没有对应物 |

**所以真正的差异不是「进终端 vs 进详情」，而是「进终端之前有没有那一步对话框」。**
用户报的 H2（「默认不选择平台、目标，选中后直接进入终端会话」）说的正是这件事。

这也解释了 `use-session-actions.ts` 里的 `setSelectedSessionDetails`：
选中会话时**顺带准备详情数据**，而不是「选中即只显示详情」。

#### 1.1 编排层也确认了（第 53 轮）

不只是「弹层存在」，而是**点会话调用的就是它**：

```tsx
// src/app/app.tsx
L324  openSessionDetail,                       // 从 use-session-workspace-state 取出
L437  openSessionDetailDialog: openSessionDetail,
L970  onOpenSession={(session) => openSessionDetail(session, detailTargetId, detailTarget)}
```

也就是说：**会话列表的点击 → `openSessionDetail`**，中间没有任何「先选平台/目标再开终端」，
也**不启动 CLI**。终端是另外的流程（「新会话」对话框 / 恢复 / 分支）。

#### 1.2 「从此处分支」的确切语义：**从某一条消息分支**

```ts
// src/renderer/src/features/sessions/create-session-branch-actions.ts
L7   export function createSessionBranchActions({ … })
L30  async function branchFromTurn(session, turn, sourceTargetId = targetId, sourceTarget?) {
L36    const messageLine = getBranchMessageLine(turn);
L38    const branch = await window.codexConsole.branchSession({ targetId, sessionId, messageLine });
```

**分支的粒度是「会话里的一条消息」（`messageLine`）**，不是在会话级别分叉。
所以「从此处分支」必须**在能看到消息时间线的地方**——也就是详情弹层里。
这解释了为什么它在详情里：**不是藏起来，而是那个动作需要详情提供上下文**。

### 2. 新建会话：旧平台是**对话框**，我是一个按钮

| | 旧平台 | aivc-rs |
| --- | --- | --- |
| 入口 | **`新会话` 对话框**（`new-session-dialog.tsx`） | 一个「新建 Codex 终端」按钮（`app.tsx:233`） |
| 内容 | **工作目录** / 标题 / 提示词 / **CLI 参数**；底部 `取消` · `打开` | 无 |
| 恢复历史会话 | 同一对话框切到 **`设置工作目录`** 形态：显示「**原工作目录不存在**」+ 原路径，按钮变成 **`继续打开`** | **没有这条流程** |

### 3. 旧平台有独立的**会话状态提醒**中心，我没有

`components/terminal-attention-center.tsx` 的实际文案：**`会话提醒`** / `会话状态提醒` /
`关闭会话提醒` / `移除提醒` / `关闭`。

这就是「终端需要用户注意」的落点——**用户报的 H1 那个「提示」很可能就属于这一片**。

---

## 二、旧平台的界面面清单（文件级，`src/renderer/src/`）

| # | 旧平台文件 | 作用 | aivc-rs 对应 | 差异 |
| --- | --- | --- | --- | --- |
| 1 | `features/sessions/session-list.tsx` | 会话列表（`收藏会话`/`取消收藏会话`/`选择会话`） | `app.tsx` 内联 | 面在，但收藏等动作待核对 |
| 2 | `features/sessions/session-detail-modal.tsx` | **会话详情弹层**：时间线 + 加载更早消息 + 展开/收起 | **无独立面** | 🔴 需新建 |
| 3 | `features/sessions/new-session-dialog.tsx` | **新建 / 恢复会话对话框** | **无** | 🔴 需新建 |
| 4 | `features/sessions/branch-panel.tsx` + `create-session-branch-actions.ts` | 从此处分支 | 部分内联 | 待补 |
| 5 | `features/sessions/session-duplicate-dialog.tsx` | 复制会话 | `duplicateSession` 命令在 | 面待核对 |
| 6 | `features/sessions/session-rename-dialog.tsx` | 重命名 | `app.tsx` 内联 | 面在，形态待核对 |
| 7 | `features/sessions/session-settings-dialog.tsx` | 会话设置 | 待核对 | 待补 |
| 8 | `features/sessions/session-quick-access-panel.tsx` | 快速访问 | `listSessionQuickAccess` 命令在 | 面待核对 |
| 9 | `features/sessions/global-session-search-panel.tsx` | 全局搜索 | `searchSessions` 命令在 | 面待核对 |
| 10 | `features/sessions/use-session-tabs.ts` | 标签页；`新会话`、`已创建新的分支会话` | `app.tsx` 标签 | 🔴 **建标签的时机**是关键（差异 1） |
| 11 | `components/terminal-attention-center.tsx` | **会话状态提醒** | **无** | 🔴 需新建 |
| 12 | `components/notice-toast.tsx` | 提示 toast | `components/feedback.tsx` | 面在 |
| 13 | `components/sidebar-header.tsx` / `sidebar-controls.tsx` | 侧栏外壳与控件 | `app.tsx` 内联 | 待核对 |
| 14 | `components/status-bar.tsx` | 状态栏 | `app.tsx` + `vendor-route-picker.tsx` | 面在 |
| 15 | `components/pagination.tsx` | 分页 | 待核对 | 待补 |
| 16 | `components/provider-status-dialog.tsx` | 平台状态 | `config-overlay` 内 | 面可能已并 |
| 17 | `components/about-dialog.tsx` / `exit-confirmation-dialog.tsx` | 关于 / 退出确认 | `exit-confirm.tsx` | 退出确认在；关于待核对 |
| 18 | `app/vendor-manager-overlay.tsx` | 供应商管理 | `config-overlay.tsx` | 面在（合并了外观） |
| 19 | `app/skill-manager-overlay.tsx` | 技能管理 | 同上 | 面在 |
| 20 | `app/gateway-port-overlay.tsx` | 网关端口 | 同上 | 面在 |
| 21 | `app/cli-installer-overlay.tsx` | CLI 安装 | 待核对（`installCli` 命令在） | 待补 |
| 22 | `app/session-overlays.tsx` / `workspace-overlays.tsx` | 各类会话/工作区弹层 | `config-overlay` 部分 | 待补 |
| 23 | `app/app-menu-bar.tsx` + `app-menus.ts` | **菜单栏** | **无** | 🔴 **用户已确认要补**，规格见下 |
| 24 | `app/command-palette.tsx` | 命令面板 | `components/command-palette.tsx` | 面在（**H1 的嫌疑落点**） |

### 一个必须单独确认的大件：菜单栏 —— **用户已确认：要补**（第 54 轮）

旧平台有 `app-menu-bar.tsx`（3.9 KB，渲染层）+ `app-menus.ts`（3.1 KB，**结构定义**），
我这边没有菜单栏。

`app-menus.ts` 是纯数据，因此它**就是规格**。原文结构：

| 菜单 | 项 | 条件（`disabled` 的依据） |
| --- | --- | --- |
| **文件** | 管理 skill | 需 `supportsSkills` **且** `hasTarget` |
| | 设置会话 | 需 `supportsSessionSettings` **且** `isWslTarget` |
| | —— 分隔 —— | |
| | **导出** ▸ 导出 Markdown / 导出 JSON / 导出 HTML | 需 `supportsExport` **且** `hasActiveSession`（子菜单三项同步禁用） |
| | —— 分隔 —— | |
| | 退出（`danger`） | |
| **工具箱** | 供应商管理 | |
| | 网关 | |
| **终端** | 新建终端 | |
| **帮助** | 命令面板（快捷键显示为 `Ctrl + K`） | |
| | —— 分隔 —— | |
| | 安装 CLI | |
| | —— 分隔 —— | |
| | 打开日志目录 | |
| | 清理 Gateway 日志 | |
| | 关于 | |

`AppMenuDefinition` 支持：`id` / `label` / `items`，每项可有 `icon` / `disabled` /
`danger` / `separator` / `shortcut` / `children`（子菜单）/ `action`。

**注意「条件启用」本身也是要一致的部分**：例如「设置会话」只对 **WSL 目标**可用，
「导出」没有活动会话时禁用。这不是可选的美化，而是旧平台的行为。

#### 好消息：这份菜单基本是「给已有能力补一个入口」

| 菜单项 | 我已有的能力 | 缺的只是 |
| --- | --- | --- |
| 管理 skill / 供应商管理 / 网关 | `config-overlay` 的对应页签 | 菜单入口 |
| 命令面板 | 已有，且 `app.tsx:74-83` 也是 **Ctrl+K** | 一致，无缺口 |
| 导出（3 种格式） | `app.tsx` 里已有 `runExport` | 子菜单 + 三种格式的入口 |
| 新建终端 | `openSystemTerminal` 已有 | 菜单入口 |
| 退出 | `exit-confirm.tsx` 已有 | 菜单入口 |
| 打开日志目录 | `openPath` 命令已有 | 菜单入口 |
| 清理 Gateway 日志 | `clearGatewayLogs` 已有（现挂在网关面板里） | 菜单入口 |
| 安装 CLI | `installCli` 命令已有 | 菜单入口 |
| **设置会话**（WSL） | **没有** —— 旧平台有 `session-settings-dialog.tsx` | 🔴 面本身要新建 |
| **关于** | **没有** —— 旧平台有 `about-dialog.tsx` | 🔴 面要新建 |

也就是说：**菜单栏本身是一个「入口层」**，它顺带把「哪些面还没有」照出来了
（设置会话、关于）。这比逐个面去猜要可靠。

---

## 三、接下来读什么（按重要性）

| 顺序 | 文件 | 要回答的问题 | 状态 |
| --- | --- | --- | --- |
| 1 | `src/app/app.tsx` | 列表 / 标签 / 详情三者怎么装配；点会话的确切分支 | **✅ 关键点已确认**（§1.1）：点会话 → `openSessionDetail`；还需看标签与详情如何并存（L805 / L932） |
| 2 | `use-session-tabs.ts` | 标签什么时候建、什么时候切 | **部分**：已确认它管的是**历史会话标签**（`historyTabTitle(session)`）；建标签的确切时机待读 |
| 3 | `use-session-details.ts` + `session-detail-modal.tsx` | 详情里「加载 / 从此处分支」的确切语义与入口 | **✅ 已确认**（§1.2）：分支的粒度是**一条消息**（`messageLine` → `branchSession`） |
| 4 | `branch-panel.tsx` | 分支面板在详情里的位置与形态 | 待读 |
| 5 | `terminal-attention-center.tsx` | 提醒从哪来、何时消失（**H1**） | 待读 |
| 6 | `app-menu-bar.tsx` / `app-menus.ts` | 菜单栏是不是必须的面 | **待你确认**（见 §二末） |
| 7 | `release/win-unpacked/AI 可视化控制台.exe` | **截图**：布局与视觉的最终依据 | 待做 |

### 关于第 7 项（截图）：为什么还没做

exe（201 MB，已确认在位）**启动后是空会话列表**，那个状态几乎看不出布局差异。
有信息量的是「有会话、开着详情弹层」的画面，而达到那个状态需要交互
（要真的跑出一次会话），不是启动即得。

因此顺序应当是：**先把流程读清楚 → 再按读出的流程去点开旧平台并截图**，
而不是先截一张空壳。这也避免我从截图里「看出」一个错误的流程假设——
本会话已经有过两次「凭表面判断」出错的记录（§7 的菜单栏、§31.4 之前的模型校验）。

## 四、做法

1. 读上表 1–6，把本文件「待补」全部填实；
2. 跑一次旧平台 exe，截图，补上视觉与布局的差异（尤其侧栏宽度、标签位置、弹层尺寸）；
3. 出一份**改动清单**（按面拆，每个面：新建 / 改造 / 已一致）；
4. **一次改一个面**，每个面配组件测试（本会话已经把 `vendor-route-picker`、
   `gateway-insights`、`refresh-coalescer` 的做法立起来了）；
5. 全部改完再整体跑一次真实 CLI 端到端，确认流程改动没有破坏网关链路。
