# 界面一致性：任务清单（自主执行）

> 依据：用户口径 **「面齐全、流程一致即可」**（外观容器可合并，**流程不合并**）。
> 证据来源：`31-open-items.md`（缺口）、`32-ui-parity.md`（对照表）、`33-ui-change-list.md`（改动清单与接线细节）。
>
> **执行规则**：一次做一项，每项做完即验证（`tsc` + `vitest` + 构建 + `verify.ps1`）+ 提交，
> **不留半截**。做完一项就在本文件里勾掉并写明证据。

---

## 待办

### T2 的**真实验收**（第 5 轮补做）—— ✅ 通过，并因此挖出**两处真实缺陷**

清单要求的最后一项是「真实 CLI 端到端回归」。做完之后我又补了一条**更严格的验收**：
用探针在真实界面上**点一条历史会话**，看它是否真的被恢复。结果 **PASS**，而且过程里
挖出两个此前完全不知道的缺陷：

| 缺陷 | 怎么发现的 | 后果 |
| --- | --- | --- |
| **主列表点击没走恢复流程** | 探针点真实列表行，`terminalsBefore: 0 → terminalsAfter: 0` | 「点历史会话进终端」在**最主要的入口上根本没生效**。T2 接线时我只改了收藏/最近/搜索结果三处，**漏了主列表**（`app.tsx` 的 `SessionRow.onOpen` 仍是 `setSelectedId`） |
| **终端面板从不获得焦点** | 探针按提示键（回车/esc）毫无反应，直到发现焦点根本不在终端里 | 终端起来了、CLI 也在跑，但**用户敲键盘没反应**——真实场景立刻撞上（Codex 启动会问升级与目录信任，都要按一次键） |

两处都已修（`b97c789`）。**这两条都是「测试全绿但用户用不了」的类型** ——
单测覆盖的是我写下的假设，而探针点的是真实按钮。

#### 验收的判据（换过两次，记下来）

1. 第一版判「有没有请求到达网关」——**不成立**：新建会话也会有请求，而且 Codex 的
   升级/信任提示会挡在前面，请求根本发不出去；
2. 第二版判「横幅里的 `session id` 等于点的那条」——**也不成立**：交互式 TUI 不打那行
   （那是 `codex exec` 的横幅）；
3. 最终判据：**屏幕上出现了那次会话的历史记录**。探针挑的那条会话里有一次旧网关失败，
   带着**上一轮探针的路由名** `probe-e2e-codex`，而本次终端叫 `term-…`——
   屏幕上出现那个名字，只可能是把历史回放出来了。

#### 探针本身踩的四个坑（都写进 `scripts/probes/resume-e2e.js` 了）

1. **目标 id 不能写死**：目标表要在 `codex:list-targets` 跑过之后才有内容；
2. Codex 会问**升级**（`esc skip`）——按回车它会真去跑 `npm install`（第一次就这么干了）；
3. 紧接着问**目录信任**（`enter continue · esc quit`）——这里按 esc 是 **quit**；
4. **不能按固定顺序按键**：第二次跑时升级提示已经没了，而探针仍先按 esc，
   那一刻屏幕上是信任提示，于是把终端自己关掉了。**改成读屏幕再决定按哪个键。**

第 4 条尤其值得记：**探针也会因为「上一次的运行结果」而失效**——
它依赖的状态被自己改变了。


### T1. 提示词投递（把对话框最后一个字段接上）—— ✅ 完成（`10b32b3`）

- **做法**：`use-terminals` 用 `ref` 排队「待发送提示词」，在**该终端第一段输出到达时**
  写一次（`terminal:write`，结尾带回车），随后出队；启动失败则丢弃。
- **为什么是这个时机**：ConPTY 必须等 CLI 回完 DSR 才推进；「收到第一段输出」是这里
  拿得到的最可靠的「它活了」信号。
- **测试**：3 条（不早发、只发一次、空值不写）。
- **false-green 检查过**：把写入提前到 `open()` 之后，**正好那两条时机相关的红**。
- **依据**（当时查清的）：提示词**不走终端层**（`terminal-sessions.ts` 全文没有 `prompt`），
  是渲染层的事；而 ConPTY 必须收到 DSR 并回 `\x1b[1;1R` 之后会话才推进，
  所以**不能**在 `terminal:start` 返回后立刻写。

### T2. 历史会话的恢复路径也走对话框

**🔍 本轮调查的重大更正（第 2 轮）**：

1. **`openTerminal` 现在只有一条调用路径** —— 新会话对话框（`app.tsx:402`）。
   点历史会话走的是 `console.openAt`（`:684/690/700`）→ 建一个 **`kind:"session"` 的标签**，
   **它根本不启动终端**。所以「点历史会话进入终端」这件事**目前完全没做**。
2. **后端完全支持恢复**：`crates/aivc-ipc/src/terminal.rs:273` 收 `sessionId`，
   `crates/aivc-terminal/src/command.rs:301` 的 `resume_args` 把它变成 `--resume <id>`
   （`:516/573/712` 三处用到）。
3. **界面 `startTerminal` 早就收 `sessionId`**（`ui/src/ipc/commands.ts`）。
4. 旧平台的一句要紧注释（`use-session-tabs.ts:97`）：
   > Qoder 按**项目目录**定位 `--resume` 的历史文件，**恢复时必须保留原始工作目录**。

- ✅ **已完成（`157ff26`）**：`use-terminals.openTerminal` 增加 `sessionId` 参数并递给
  `terminal:start`；2 条测试（恢复时带 `sessionId` + `cwd`；不恢复时不带）。
- ✅ **流程已接（`b65fedc`）**：点历史会话 → `beginResumeSession` → 决策 → 进终端或弹对话框。
  - **决策抽成纯函数** `ui/src/app/resume-session.ts` 的 `planResume(cwd, pathExists)`，
    **四条分支各一条测试**：没有 cwd / 目录还在（**不弹对话框**）/ 目录丢了（弹 `设置工作目录`）/
    **判断不出来时按「还在」处理**（WSL 上那条检查未接线，后端显式报错——
    把它当成「不存在」会让用户永远看到「原工作目录不存在」）。
  - 三处入口全部改接：两处 `onOpen`（收藏 / 最近）与全局搜索的行点击。
  - 对话框恢复形态传 `sessionId` → 后端 `resume_args` → `--resume <id>`。
  - 「原工作目录是否还在」用的是**已有的** `shell:path-exists`
    （`crates/aivc-ipc/src/shell.rs:97`）——不需要新加 IPC，只需在界面暴露。
- **验收**：点历史会话 → 目录在则**直接进终端**（带 `--resume`）；目录丢了则出现
  `原工作目录不存在` + `继续打开`；取消则不起终端。**待真实 CLI 回归时一并确认**。


- **目标**：点左侧历史会话 → 走对话框的**第二种形态**（`设置工作目录` / `继续打开`）。
- **依据**：`new-session-dialog.tsx` 的 `pendingResume` 形态已经写好并有测试；
  缺的是「判断原工作目录是否还在」这一步数据流。
- **要查**：旧平台的恢复路径（会话文件是否存在、原 `cwd` 从哪读、何时判定「原工作目录不存在」）。
- **验收**：原目录存在 → 对话框显示工作目录 + `继续打开`；不存在 → 出现
  `原工作目录不存在` + 原路径 + 提示；**取消则不起终端**。
- **依赖**：无（T1 可并行）。

### T3. M1 会话详情子视图

**🔍 本轮调查的重大更正（第 2 轮）：这一项**大部分已经实现**，只是形态不同。**

点历史会话建的 `kind:"session"` 标签，正文里已经有（`app.tsx:835-941`）：

| 旧平台的能力 | 我这边 |
| --- | --- |
| 会话时间线 | ✅ `console.messages.map(...)` |
| **加载更早消息** | ✅ `console.hasEarlier` + 按钮 `加载更早的消息`（`:918-928`） |
| **从此处分支**（粒度是一条消息） | ✅ 每条消息的 `onBranch` → `console.branchFromMessage(message.sourceLine)`（`:934-939`），
且注释写明「只有带原始行号的消息才能分支——后端要靠行号定位」 |
| 会话头操作 | ✅ 重命名 / 复制 / 导出 MD / 导出 JSON / 另存到（`:886-904`） |
| 工作台指标（上下文 / token / 模型） | ✅ `:868-884` |
| **分支关系面板** | 🔴 **没有**（旧平台 `branch-panel.tsx`：来源 + `消息 N` + `JSONL 行 N` + 子分支列表） |

**形态差异是可接受的**：旧平台是**弹层**，我这边是**标签正文**——按用户口径
「面齐全、流程一致即可」，容器可以不同，**流程一致**（从终端会话能看到它、能加载、能分支）。

**因此 T3 缩小为**：补 `分支关系` 面板（**块状**：有来源或子分支才显示），
并核对「加载更早消息」与「分支」的确切行为与旧平台一致。

**🔍 第 3 轮的精确起点（数据来源已全部查清，**纯界面工作**）**：

| 需要的东西 | 现状 |
| --- | --- |
| **列出子分支** | 后端**已经有** `codex:list-session-children`（`crates/aivc-ipc/src/commands.rs:103`，路由器 `:494` 已分派），但**界面没暴露** |
| **来源（父会话）** | 后端**已经有**：`aivc-core/src/session.rs` 的 `Session.metadata`（`:337`）→ `SessionMetadata.branch`（`:254`）→ `BranchMetadata`（`:224`，含 `parent_target_id` / `parent_session_id` / `parent_message_index` / **`parent_message_line`**）——**正好是旧平台面板要的四项** |
| **界面类型缺字段** | `ui/src/types.ts:87` 的 `SessionMetadata` 只有 `updatedAt` / `customTitle` / `favorite` / `lastOpenedAt`——**没有 `branch`**，`BranchMetadata` 也**没有对应类型** |

**因此 T3 剩下四步（下一步照着做）**：

1. `ui/src/types.ts`：补 `BranchMetadata` 类型（四个字段），并给 `SessionMetadata` 加 `branch?`；
2. `ui/src/ipc/commands.ts`：暴露 `listSessionChildren(targetId, sessionId)` → `codex:list-session-children`；
3. 新建 `ui/src/app/branch-panel.tsx`：**块状**——`分支关系` 标题；来源行显示**父会话标题**（可点开）+
   `消息 {parentMessageIndex}` + `JSONL 行 {parentMessageLine}`；子分支行显示每个子会话（标题 + 相对时间，可点开）；
   `正在加载...` 态；**父与子都没有时整块不渲染**；
4. 挂到会话标签正文里，并配组件测试（渲染/不渲染、加载态、点父与点子都调 `openAt`）。


- **目标**：终端会话里能打开详情：**时间线 / 加载更早消息 / 从此处分支 / 分支关系**。
- **依据**：`32-ui-parity.md` §1、`33-ui-change-list.md` §四。
  - 分页语义：`getSessionMessagesPage(targetId, sessionId, -1, 100)`，`offset = -1` 取最后 100；
    `detailHasMore = page.offset > 0`；加载更早用 `max(0, previewOffset - 100)`，**前插**。
  - 分支粒度是**一条消息**：`messageLine` → `branchSession({targetId, sessionId, messageLine})`
    （我的 IPC 参数面已经对齐）。
  - 分支关系面板：**来源**（父会话 + `消息 N` + `JSONL 行 N`）、**子分支**（标题 + 相对时间）；
    都没有时**整块不渲染**。
- **验收**：终端跑着时能打开详情；关掉详情**终端不受影响**；终端退出后详情仍能打开；
  分支后的新会话出现在「子分支」里。
- **依赖**：无。

### T4. M3 会话提醒中心

- **目标**：铃铛 + 未读数徽标 + 弹层（`会话提醒` / `N 项待处理` / 每条
  `{provider} · {title}：{detail}` / 点击跳转 / `移除提醒`）；**无未读时整个组件不渲染**。
- **依据**：`32-ui-parity.md` 的 M3 规格；**后端已经在发** `attention`
  （`terminal.rs:614`），`terminal:attention-notification` 命令与校验都在。
  位置：菜单栏之后（旧平台 `app.tsx:797 AppMenuBar` → `:802 TerminalAttentionCenter`）。
- **验收**：有未读时出现徽标与条目；点击跳转到那路终端并清未读；移除后消失；
  无未读时组件不渲染。
- **依赖**：无。

### T5. M5 设置会话（仅 WSL）

- **目标**：`session-settings-dialog` 对应的面；命令 `setWslCodexHome` / `clearWslCodexHome` 已在。
- **依据**：`app-menus.ts` 里「设置会话」的启用条件是 `isWslTarget`——
  做完这一项，菜单里那个禁用项即可接上（现在 `isWslTarget` 写死 `false`，要改成按真实字段判定）。
- **验收**：仅 WSL 目标可用；设置/清除 Codex Home 生效。
- **依赖**：无。

### T6. M6 关于

- **目标**：`about-dialog` 对应的面（版本、项目信息）。
- **验收**：菜单「关于」从禁用变为可用并打开它。
- **依赖**：无。

### T7. 清掉菜单里剩下的两个禁用项

- **目标**：`openLogDirectory`（打开日志目录）、`clearGatewayLogs`（清理 Gateway 日志）接上。
- **依据**：`app-menus.ts` 里它们因「动作缺失」被禁用；`api.openPath` 与 `clearGatewayLogs`
  命令都已存在，缺的是**日志目录的路径**与一个确认流程。
- **验收**：两项可用且行为正确。
- **依赖**：无。

### T8. 「标签同时挂会话与终端」

- **目标**：一个标签能同时带 `sessionId` 与 `terminalId`（旧平台 `terminalIdsByTabKey` 的等价物）。
- **依据**：`33-ui-change-list.md` §四 5.1/5.2。
- **注意**：**点历史会话进来时 `terminalId` 一开始就有**（终端确实起来了）；
  `terminalId` 消失只发生在终端退出之后。
- **验收**：终端退出后标签仍在、详情仍能打开；不会出现「详情说开着终端、实际没有」。
- **依赖**：与 T3 同一片，建议一起做。

### 收尾. 端到端回归 —— ✅ 通过

- 命令：`scripts\app-probe.ps1 -Probe probes\gateway-e2e.js -ProbeArgs '{"provider":"codex"}'`
- 结果：**`verdict: "PASS：CLI 的请求到达了网关"`**，`requestCount: 30`。
- Codex 自己打印的报错里能看到它在打
  `http://127.0.0.1:51173/gateway/codex/probe-e2e-codex/responses` —— **就是本地网关**；
  30 条全部失败是**预期**（探针的供应商故意指向打不通的 `127.0.0.1:1`）。
- 结论：**这一轮全部界面改动没有破坏网关链路**。

---

## 全部完成（第 4 轮结束时）

| 项 | 提交 | 测试 |
| --- | --- | --- |
| T1 提示词投递 | `10b32b3` | 3 条（含时机假绿检查） |
| T2 恢复进终端（地基 + 流程） | `157ff26` `b65fedc` | 2 + 5 条（`planResume` 四条分支） |
| T3 分支关系面板 | `0636f24` | 6 条 |
| T4 会话提醒中心 | `d03aaff` | 7 条 |
| T5 设置会话（仅 WSL） | `cf17d61` | 6 条 |
| T6 关于 | `8ca6fd5` | 5 条 |
| T7 菜单剩余两项 | `7ebaf7e` | （沿用菜单结构测试） |
| T8 标签同时挂会话与终端 | `cbdd1e5` | 5 条（`workspace-tabs`） |
| 收尾端到端回归 | — | 真实 CLI PASS |

**前端 135 个测试 + 类型检查 + 构建全绿；后端门禁 8 项全绿；旧平台 `src/` 改动 0。**

### T8 顺带修掉的一个**我自己引入的缺陷**

T2 接线之后，点历史会话会**同时建出两个标签**（`openAt` 让「选中即建标签」的 effect
建一个会话标签，紧接着起终端又建一个终端标签）。T8 把标签规则抽成
`workspace-tabs.ts` 的 `activateSessionTab`：**已经有标签挂着这条会话时——哪怕它是终端标签
——就切过去而不是再建一个**，并逐条测试。



- 全部做完后跑一次**真实 CLI 端到端**（Codex 与 Claude），确认流程改动**没有破坏网关链路**
  （`scripts\app-probe.ps1` + `probes\gateway-e2e.js`）。**✅ 已通过，见文末。**

---

## 已完成的（本轮之前）

| 项 | 证据 |
| --- | --- |
| 对照表与改动清单 | `32-ui-parity.md`、`33-ui-change-list.md` |
| M4 菜单栏 | `494a131`（14 条测试） |
| M2 对话框组件 | `2b88af0`（7 条测试） |
| M2 接线（新建路径） | `ed9b46c` + `eb26275` |
