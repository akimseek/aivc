# 界面改动清单（按面拆）

> 依据：[`32-ui-parity.md`](./32-ui-parity.md)（对照表）与用户口径
> **「面齐全、流程一致即可」**（外观容器可合并，**流程不合并**）。
>
> 这份清单是**动手之前给你过一眼**的。每项都标了「已确认」或「待核对」，
> 没读过的我不装读过。

---

## 一、必须新建的面

### M1. 会话详情弹层 🔴 最高优先

**旧平台**：`features/sessions/session-detail-modal.tsx`
**已知内容**（来自源码文案）：标题 **`会话详情`**、**`会话时间线`**、**`加载更早消息`**
（加载中显示 `正在加载更早消息...`）、`展开` / `收起`。

**里面还嵌着 `分支关系` 面板**（`branch-panel.tsx`，本轮已读全文）：

| 区域 | 内容 |
| --- | --- |
| 来源 | 父会话（可点开）+ **`消息 {parentMessageIndex}`** + **`JSONL 行 {parentMessageLine}`** |
| 子分支 | 每个子会话一个按钮（标题 + 相对时间） |
| 加载中 | `正在加载...` |
| 空 | **整个面板不渲染**（没有父也没有子时） |

**以及「从此处分支」**：粒度是**一条消息**
（`create-session-branch-actions.ts:36` `getBranchMessageLine(turn)` → `branchSession({ targetId, sessionId, messageLine })`）。
我的 IPC `branchSession` 已经收 `{targetId, sessionId, messageLine}` —— **参数面已经对齐**。

### M2. 新建 / 恢复会话对话框 🔴

**旧平台**：`features/sessions/new-session-dialog.tsx`（已读全文）

| 形态 | 标题 | 字段 | 按钮 |
| --- | --- | --- | --- |
| 新建 | `新会话` | 工作目录 / 标题 / 提示词 / CLI 参数 | `取消` · **`打开`** |
| 恢复历史 | `设置工作目录` | 顶部提示「**原工作目录不存在**」+ 原路径 + 「请选择新的工作目录后继续打开此历史会话。」 | `取消` · **`继续打开`** |

### M3. 会话状态提醒中心 🔴

**旧平台**：`components/terminal-attention-center.tsx`（已读全文）

- **未读数为 0 时整个组件不渲染**；否则是一个铃铛按钮 + 未读数徽标（>99 显示 `99+`）；
- 点开是一个弹层，标题 **`会话提醒`**，副标题 `N 项待处理` / `N 项记录`；
- 每条：图标（按 kind 分）+ 终端标题 + **`{provider} · {title}：{detail}`**；
- 点条目 → **跳转到那个终端**并清除其未读，同时关闭弹层；
- 每条右侧有 **`移除提醒`**；
- Escape 关闭、点外面关闭。

**✅ 更正（第 55 轮）：注意力在我的实现里**已经有了**，这块功能没有缺。**

我一度据文件表面判断「我的终端层很可能完全没有注意力的概念」。查完发现是错的：

| 位置 | 事实 |
| --- | --- |
| `ui/src/types.ts:170` | 有 `TerminalAttention`，`:180` 有 `attention?: TerminalAttention` |
| `crates/.../terminal.rs:614` | 后端**会发** `"attention": ev.attention` |
| `terminal:attention-notification` | 命令面、路由器、参数校验（`validate.rs:302`）**都在** |
| 旧平台的 kind 定义 | `TerminalAttentionKind = Exclude<TerminalStatus, "running">`——**注意力类型就是「终端状态去掉 running」** |

**所以 M3 只是「补一个面」**，不是补一层能力。这比我上一轮的判断便宜得多。

> 这已经是本会话**第三次**「凭文件表面下结论」出错（§7 说菜单栏缺失、§31.4 之前认定是模型校验、
> 以及刚才这条）。共同的教训是：**结论要落在「谁产生它」上，而不是「有没有同名文件」**。

### M4. 菜单栏

**旧平台**：`app/app-menu-bar.tsx` + `app-menus.ts`（规格已在 `32-ui-parity.md` 写完）

四个菜单、条件启用规则、导出子菜单（3 种格式）都照抄。
**它同时是「入口是否齐全」的检验器** —— 逐项接的时候会立刻暴露哪个面还不存在。

### M5. 设置会话（仅 WSL 目标）

**旧平台**：`session-settings-dialog.tsx`。命令面的 `setWslCodexHome` / `clearWslCodexHome`
**已经在我的 IPC 里**，缺的是面。

### M6. 关于

**旧平台**：`components/about-dialog.tsx`。

---

## 二、必须改的流程（**流程不合并**）

> **⚠️ 第 58 轮更正**：本节原本的 **P1**（「点会话只出详情、不起终端」）**是错的**。
> 用户纠正：**点左侧历史会话就是进入终端会话；详情只是终端会话的一个子功能。**
> 真正的差异在「进终端**之前**有没有那一步对话框」。P1 已删除，理由见 `32-ui-parity.md` §1。

| # | 现状（aivc-rs） | 应改成 |
| --- | --- | --- |
| ~~P1~~ | ~~选中会话即建标签、即开终端~~ | **删除**——这一层旧平台也是进终端，不是差异 |
| P2 | 「新建 Codex 终端」按钮**直接开**；点历史会话也**直接就进** | 走 **M2 的对话框**：工作目录 / 标题 / 提示词 / CLI 参数；恢复历史时是「设置工作目录 → 继续打开」。**这才是 H2 真正说的那件事** |
| P3 | 恢复历史会话没有「工作目录不存在」的补救流程 | 走 M2 的 `设置工作目录 → 继续打开` |
| P4 | 没有会话提醒入口 | 走 M3 |
| P5 | 没有「会话详情」这个子视图 | 从终端会话里能打开 **M1**（时间线 / 加载更早消息 / 从此处分支 / 分支关系） |

**优先级随之改变**：**M2 升为第一优先**（它是用户实际撞到的那一步），
M1 从「主链前置」降为「终端会话里的子视图」。

---

## 三、还需读的东西

### ✅ 「加载」的语义已确认（第 56 轮）：是**详情里的分页**，不是「加载进终端」

全文 grep `features/sessions/*` 里的「加载」，**每一处都是会话正文的加载**：

| 位置 | 文案 |
| --- | --- |
| `use-session-details.ts:68` | `加载完整会话失败。` |
| `use-session-details.ts:171` | `加载更多会话消息失败。` |
| `session-detail-modal.tsx:130` | `正在加载完整会话...` |
| `session-detail-modal.tsx:133` | 按钮 **`重新加载`** |
| `session-detail-modal.tsx:148` | **`加载更早消息`** / `正在加载更早消息...` |
| `use-session-loader.ts:96` | `加载 AI 会话失败。`（列表加载，另一回事） |

**没有**任何一处是「把历史加载进一个终端会话」。所以用户说的
「详情才是**加载**/从此处分支的」= **详情是看/继续加载会话正文、以及从那一条消息分支的地方**。
**P1 的语义到此完全确定，可以动手。**

### ✅ P1 的机制也确认了：`setSelectedSessionDetails`

`use-session-actions.ts` 的入参里有 **`setSelectedSessionDetails: Dispatch<SetStateAction<AiSession | null>>`**。
也就是说：**选中会话 → 设置 `selectedSessionDetails` → 驱动详情弹层**。
这正是「点会话进详情」，与 `app.tsx:970` 的 `onOpenSession → openSessionDetail` 互相印证。

### 一个要注意的相邻概念：会话可以「有开着的终端」

同一个入参列表里有 **`closeOpenSessionTerminal: (session) => Promise<void>`**。
说明旧平台的模型是：**一个会话可以对应一路开着的终端**，而详情会与它互动
（例如会话文件不在了要关掉那路终端 —— `use-session-tabs.ts` 里就有
`该会话文件已不存在，已刷新会话列表。`）。

我的实现里「终端」与「会话」是两套东西（`use-terminals` vs 会话缓存），
**这一处关系需要在实施 M1 时一并设计**，否则会出现「详情说这个会话开着终端，而实际上没有」。

### ✅ 最后三个面也读完了（第 57 轮）

#### 1. 分页语义（`use-session-details.ts`）—— 具体到数字

| 动作 | 调用 |
| --- | --- |
| 初次打开 | `getSessionMessagesPage(targetId, sessionId, **-1**, **100**)` —— `offset = -1` 表示**取最后 100 条** |
| 还能往前翻？ | `detailHasMore = page.offset > 0` |
| `加载更早消息` | `offset = max(0, previewOffset - 100)`，再取 100 条，**前插**到 `preview` |

**每页 100 条、从最新往旧翻、`offset > 0` 表示还有。**
我的 IPC 里 `getSessionMessagesPage` 已经在用 —— **M1 用现成命令就能实现**。

#### 2. 各个面挂在编排的哪里（`app.tsx`）

```tsx
L797  <AppMenuBar
L802  <TerminalAttentionCenter
L907  … : <SessionList
```

顺序是 **菜单栏 → 会话提醒中心 → … → 会话列表**。也就是**提醒中心在顶部工具区、会话列表在侧栏** ——
M3 的位置不用猜。

#### 3. 🔑 标签就是「会话 ↔ 终端」的连接点（**这条回答了我前面标出的设计问题**）

`useSessionTabs` 的入参里有：

```ts
terminalIdsByTabKey: Record<string, string>;   // 一个标签 → 它那路终端
releaseTabVendor: (tabKey: string) => void;
```

以及 `closeSessionTab` / `closeOtherSessionTabs` / **`handleTerminalExit(tabKey, exitCode)`**。

**所以旧平台不是「会话」和「终端」两套并列的东西，而是一个标签同时挂着会话（历史）
与终端（运行时）；终端退出时会回到标签层处理。**

我前面担心的「详情说这个会话开着终端、实际却没有」在这个模型下自然消解：
**关系在标签上，不在两张表之间**。实施 M1 时要按这个模型来，而不是我原先想的
「在会话缓存和终端列表之间再建一个映射」。

---

## 四、M1 的内部结构切法（动手前先定）

> **⚠️ 第 58 轮更正**：本节原先写的是「点会话只出详情、不起终端」，
> 那是**搞反了**。下面的切法已按用户的纠正重写：**详情是终端会话的子视图**，
> 而「一个标签同时挂会话与终端」这个模型本身**仍然成立**（旧平台就是 `terminalIdsByTabKey`），
> 只是它的用途变了——不是「点会话只开详情」，而是「进了终端之后，详情与终端是同一条标签的两个面」。

### 5.1 现有的差距：两种标签**互不相干**

我现在的模型（`app.tsx`）：

```ts
type WorkspaceTab = {
  key: string;
  kind: "session" | "terminal";   // ← 二选一
  title: string;
  sessionId?: string;             // kind === "session"
  terminalId?: string;            // kind === "terminal"，同时用作 key
};
```

**一个标签要么是会话、要么是终端，两者之间没有任何连接。**
而旧平台是：

```ts
terminalIdsByTabKey: Record<string, string>;   // 一个标签 → 它那路终端
```

**旧平台是「一个标签，同时挂着会话（历史）与终端（运行时）」。**
差距在这里：**不是进哪里，而是进去之后这两个面各走各的** ——
所以「终端在跑，但看不到它的会话正文」「详情里说这个会话开着终端，实际却没有」这类问题都会出现。

### 5.2 切法：让一个标签能同时带两个编号

```ts
type WorkspaceTab = {
  key: string;
  title: string;
  /** 这条标签对应的会话（历史 / 正文）。视图与详情都靠它。 */
  sessionId?: string;
  /** 这条标签上活着的终端。**点历史会话进来时它就已经有了**。 */
  terminalId?: string;
  /** 只影响图标与排序；**行为一律由上面两个字段决定**。 */
  kind: "session" | "terminal";
};
```

**关键更正**：点历史会话进来时，`terminalId` **一开始就有**（终端确实起来了）——
不是「先只有 sessionId、再点一下才有 terminalId」。
`terminalId` 消失只发生在**终端退出之后**，那时会话仍可查看、详情仍可打开。

| 状态 | `sessionId` | `terminalId` | 表现 |
| --- | --- | --- | --- |
| 点历史会话进来 | ✅ | ✅ | **终端视图**（默认）；从终端里可打开**详情**子视图 |
| 打开详情 | ✅ | ✅ | 时间线 / 加载更早消息 / 从此处分支 / 分支关系；**终端仍在跑** |
| 终端退出 | ✅ | — | 会话还在，详情仍可打开 |

### 5.3 要改的三处

| # | 位置 | 改动 |
| --- | --- | --- |
| 1 | `use-terminals.ts` | `open()` 接受一个**已存在的标签 key**，把终端挂到会话标签上（旧平台 `terminalIdsByTabKey` 的等价物）；补 `handleTerminalExit(tabKey, exitCode)` |
| 2 | 新建 `session-detail.tsx` | 详情子视图：时间线 + `加载更早消息`（每页 100、`offset=-1` 起）+ **分支关系**面板 + 「从此处分支」 |
| 3 | `app.tsx` | 从终端视图能打开详情；**并接上 M2 的对话框**（真正的前置步骤） |

**`use-terminals` 的命令面不用动**：`terminal:start` 已经收 `terminalId`。

### 5.4 M1 的验收标准（做过更正）

1. 点左侧历史会话 → **先出 M2 的对话框**（工作目录/标题/提示词/参数），确认后**进终端**；
2. 终端跑着的时候能打开**详情**，看到时间线；关掉详情**终端不受影响**；
3. 终端退出后，那条标签仍在，详情仍能打开；
4. `从此处分支` 传的是**那一条消息的 `messageLine`**，新会话出现在「子分支」里。

四条都要有测试：1、2、3 是组件/钩子测试，4 走 `branchSession` 的命令断言。

### 5.5 M2 的接线：两条命令层事实（第 59 轮查清）

对话框组件已经写好（`new-session-dialog.tsx`，7 条测试），但「接上主链」前必须知道
那些字段往哪走。查完了，**一好一待定**：

#### ✅ CLI 参数：**已经支持，不用改**

| 侧 | 证据 |
| --- | --- |
| 旧平台 | `terminal-sessions.ts:415/539/585/631` 四处都是 `const extraArgs = parseCliArgs(params.cliArgs \|\| "")` —— **追加为额外命令行参数** |
| aivc-rs | `crates/aivc-ipc/src/terminal.rs:276` **已经有** `cli_args: optional_string(args, "cliArgs")` |

所以对话框只管把它传下去。

#### ⚠️ 启动提示词：我这边**没有这个入口**，且投递时机要确认

| 侧 | 证据 |
| --- | --- |
| 旧平台 | `app.tsx:407` `onCreate: ({cwd, title, prompt, cliArgs}) => sessionTabs.openNewSessionTab(targetId, cwd, title, prompt, cliArgs)`；`use-session-tabs.ts:141` `prompt: prompt.trim() \|\| undefined` |
| aivc-rs | `terminal.rs` / `validate.rs` 里**搜不到** `prompt` / `initial` —— `terminal:start` 不收这个参数 |

**这是真正的缺口**：界面上的「启动提示词」目前无处可去。

**而且它的投递时机不能想当然**：这个项目的终端是 ConPTY，**必须收到 DSR（`\x1b[6n`）
并回 `\x1b[1;1R` 之后会话才开始推进**（这是本会话早先实测出来的）。
所以提示词**不能简单地在 start 之后立刻 write** —— 那时 CLI 可能还没准备好读输入，
写进去会丢。

**因此接主链之前要再查一处**：旧平台是在什么时机把 `prompt` 写进终端的
（`terminal-sessions.ts` 里用 `prompt` 的地方，以及它是否等某个「就绪」信号）。
**在查清之前我不实现这条**，否则会做出一个「多半能成、偶尔丢字」的功能。

#### 5.5.1 查完了：提示词**根本不走终端层**（第 60 轮）

真正的文件是 `src/electron/terminal/terminal-sessions.ts`（不在 `src/electron/` 根下，
我上一轮路径猜错了一次）。在里面全文搜 `prompt`：**一处都没有。**

它只有：

```ts
L299  session.pty.write(data);                                    // PTY 写入的唯一出口
L344  session.outputTimer = setTimeout(() => flushTerminalOutput(session), 16);
L376  new Promise<void>((resolve) => setTimeout(resolve, 3_000));  // 某个 3 秒等待
```

**结论：启动提示词是「渲染层的事」，不是后端的事。**

它从 `app.tsx:407` 进 `openNewSessionTab(...)`，之后由**界面**在终端起来之后写进 PTY
（`terminal:write`）。这也解释了为什么后端拿它没办法——后端只知道命令与参数。

**对我实现的直接影响**：

- 那条 DSR（`\x1b[6n` / `\x1b[1;1R`）的顾虑**依然成立**，而且现在明确了它是**界面侧**要处理的：
  界面必须等到「会话可交互」再写提示词，否则会丢；
- 所以我这边应当**先不做提示词**，或者做一个明确的「等首个输出再写」的策略——
  而不是在 `terminal:start` 返回后立刻写。**这正是值得单独一轮的事**，
  与「接 M2 主链」是两件不同的工作。

**因此接主链的顺序应当是**：先接对话框（`cliArgs` 已通、`prompt` 先禁用并注明原因），
等提示词的投递策略单独做完再打开那个字段。**不为一个次要字段拖住主要修复。**


#### 接线的完整形状（据此可以动手）

```
点新建 / 点历史会话
  → 打开 NewSessionDialog（cwd / title / prompt / cliArgs；恢复历史时 pendingResume）
  → 确认
  → terminal:start { terminalId, providerId, targetId, cwd, cliArgs, ... }   ← cliArgs 已支持
  → （prompt 待定：确认时机后再接）
```



1. 点左侧历史会话 → **先出 M2 的对话框**（工作目录/标题/提示词/参数），确认后**进终端**；
2. 终端跑着的时候能打开**详情**，看到时间线；关掉详情**终端不受影响**；
3. 终端退出后，那条标签仍在，详情仍能打开；
4. `从此处分支` 传的是**那一条消息的 `messageLine`**，新会话出现在「子分支」里。

---

## 五、M2 接线的可执行清单（下一轮照着做，不必重新调研）

前置调研已全部完成（§5.5、§5.5.1）。下面是**具体到位置**的改动，共四处：

| # | 位置 | 改动 | 风险 |
| --- | --- | --- | --- |
| 1 | `use-terminals.ts:301-319`（`openTerminal`） | 入参增加 `cliArgs?: string`，在 `api.startTerminal({...})` 里带上 `cliArgs`（后端 `terminal.rs:276` 已经收） | 低 |
| 2 | `app.tsx:106-109`（`openTerminal`） | 透传 `cliArgs`；签名改成接受一个可选的 `{ cwd, title, cliArgs }` | 低 |
| 3 | `app.tsx` 新建状态 | 加 `newSessionOpen` + `cwd/title/prompt/cliArgs` 四个受控状态；把两处「新建」入口（`:233` 命令面板项、`:618`/`:688` 按钮）改成 `setNewSessionOpen(true)` | 中 |
| 4 | `app.tsx` 渲染 | 渲染 `<NewSessionDialog ...>`：`onConfirm` → `openTerminal({cwd, title, cliArgs})` 然后关闭 | 低 |

**`prompt` 字段暂时禁用**，并在界面上注明原因（投递时机要单独处理，见 §5.5.1）。

### 验收（这一处单独验证，不掺别的）

1. 点「新建终端」/ 点历史会话 → **先出现对话框**，不再直接进终端；
2. 对话框里填的 **CLI 参数** 真的到了命令上（可用探针断言命令面，或看终端的启动日志）；
3. 取消 → **一个终端都不起**（`terminal:start` 未被调用）；
4. 「恢复历史会话但原目录不存在」→ 出现 `原工作目录不存在` + `继续打开`。

**改动 3 是唯一有风险的一处**（动主流程的状态），所以**先把 1、2 做完并验证**
（它们只是让 `cliArgs` 能流下去、不影响现有行为），再做 3、4。

### 还没做、已知要做的（按顺序）

| 顺序 | 事 | 依据 |
| --- | --- | --- |
| 1 | **M2 接线**（上面的清单） | §5.5 前置调研已完成 |
| 2 | **提示词的投递策略** | §5.5.1：渲染层的事，必须等「可交互」再写，否则丢字 |
| 3 | **M1 详情子视图** | §5.1–5.4：标签同时挂会话与终端 |
| 4 | **M3 会话提醒中心** | 后端 `attention` 已经在发，只需补面 |
| 5 | **M5 设置会话 / M6 关于** | 做完后菜单里那两个禁用项即可接上 |


1. **先读第三部分那 5 个文件**（尤其 `terminal-tab-state.ts` 与 `use-session-actions.ts`）；
2. 出一份「按面拆」的实施顺序（M1 → M4 → M2 → M3 → M5 → M6），
   每个面**一次改完并配组件测试**（本会话已把 `vendor-route-picker` /
   `gateway-insights` / `refresh-coalescer` 的测试做法立起来了）；
3. M4 菜单栏建议**早做**：它是入口总检，能立刻照出还缺哪个面；
4. 全部改完跑一次真实 CLI 端到端，确认流程改动没有破坏网关链路。
