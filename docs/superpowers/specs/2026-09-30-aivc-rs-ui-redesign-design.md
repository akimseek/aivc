# aivc-rs 界面重做设计（对齐 `docs/ui-redesign-demo.html` 第九版）

> 日期：2026-09-30
> 范围：`D:\code\ai\aivc-rs\ui`（React 19 + TS + Vite 前端）。Rust 后端原则上不改。
> 依据：用户决策（2026-09-30，四项均已确认）+ 对旧平台（`D:\code\ai\src\renderer`）、
> Rust 命令面（93 条全部已接线）、现有前端（118 文件 / 40 测试文件）的逐面审计。

## 1. 背景与目标

旧平台（Electron + TS）已用 Rust（Tauri 2 + 100% Rust 内核）重做，工程在 `aivc-rs`。
现在按**新版前端 demo**（`docs/ui-redesign-demo.html`，第九版"现代精研标签页版"）
重做 Rust 版界面。

硬性要求：

1. **功能一致**：demo 只是视觉与信息架构的目标；现有已实现的功能（含比 demo 更完整的
   部分）一个都不能丢。demo 中后端无对应能力的元素不实现。
2. 旧平台 / Rust 版实现中**不合理、可优化、有更好实现**的，顺势调整（见第 12 节）。
3. **组件化**：可复用的必须封装；不造上帝组件，能拆就拆。

Rust 版已移除且不再恢复的功能：命令面板、系统终端（含 `terminal:start-system`）。
本设计与二者无关。

## 2. 已确认决策（用户 2026-09-30 拍板）

| # | 决策 | 结论 |
| :-: | --- | --- |
| D1 | 顶部菜单栏（文件/工具箱/帮助） | **按 demo 移除**；全部菜单动作迁移到设置中心与上下文菜单（映射表见 §11） |
| D2 | 暗黑主题 | **实现明暗双主题**：纯前端 token 双套 + localStorage 记忆 + xterm 主题联动 + 「跟随系统」选项 |
| D3 | 工作区标签模型 | **合并为单一会话标签**：一个会话一张标签；运行中显示终端，未运行显示历史；完整历史走「消息与分支」弹框 |
| D4 | 交付节奏 | **分阶段交付**（三批，见 §14），每批过门禁 + 截图验证 |

## 3. 事实基线（审计结论，均为代码核实，非推测）

### 3.1 后端能力（crates/aivc-ipc，93 条命令，PENDING 为空）

- 会话：列表/回收站/批量/彻底删除/恢复/重命名/收藏/复制/分支/导出(md/json/html)/
  分页消息/子分支/快速访问(收藏+最近)/WSL CODEX_HOME 覆盖/搜索。
- 终端：`terminal:start|write|resize|stop`，四平台(Codex/Claude/Gemini/Qoder) ×
  local/WSL；事件 `terminal:data|status|exit`。
- 网关：启停、端口、三条韧性阈值(失败切换 1–10 / 熔断次数 1–20 / 熔断时长 10–86400s)、
  故障规则 CRUD（scope global/provider/vendor + 文本包含 + 状态码 + 自定义应答）
  与 `gateway:test-failover-rule` 在线测试、请求日志(查询/删除/清空)、用量汇总与报表、
  失败诊断分页、供应商健康、外部网关开关与共享令牌轮换（明文仅一次）。
- 供应商：CRUD、`vendor:list-models`、`vendor:refresh-balance(s)`、`vendor:read-configs`、
  `vendor:enable`（写真实 CLI 配置，带备份）、候选池（enabled+sort+lastEnabledAt）、
  `vendor:route-switch` / `vendor:route-mode`（dynamic|locked）。
- 技能（仅 Codex 目标）、CLI 环境检测/安装、工作区预设、`app:get-version`、
  `app:command`（仅 quit / openLogDir）、`app:exit-choice`、`models:list`（仅 Qoder）。
- 事件共 6 个：`terminal:data/status/exit`、`gateway:request-recorded`、
  `gateway:vendor-switched`、`app:request-exit`。

**后端不具备**（demo 有对应元素但需裁剪）：主题（无设置项，故主题纯前端）、
窗口控制、检查新版本、数据库健康诊断命令。

### 3.2 现有前端（aivc-rs/ui）

- 壳：`.shell` = 菜单栏 /（侧栏 360px + 工作区）/ 状态栏；无活动栏；无暗黑。
- 侧栏：一级 pane（工作台/会话/收藏/搜索）× 二级 view（当前/收藏/回收站）；
  `.pickers` 平台+目标选择；WSL 不可用 Notice + `WslHomeForm`；批量条；会话行 + 快捷分组。
- 工作区：标签(kind=session|terminal) + `.terminal-stack` + 会话正文（`.session-head`/
  BranchPanel/`.thread`）；输入区（附件/粘贴压缩/拖拽/图片预览/自管输入↔终端输入）。
- 设置：`config-overlay.tsx` 9 个扁平标签（供应商/技能/网关/用量/异常/日志/环境/
  工作区预设/诊断）；另有关于弹框、会话详情弹框、右键菜单×3、退出确认、关注中心。
- 设计系统：`tokens.css` 单主题 + `components.css` + `app.css`（1853 行）；
  基础组件 primitives/card/dialog/feedback/list/loading/tabs/use-async-data。
- 测试：40 个 vitest 文件（纯逻辑/hook/组件/面板）。

### 3.3 旧平台中 Rust 版缺失、本轮需补齐的能力（逐条核对结论）

| 能力 | 旧平台 | Rust 现状 | 处置 |
| --- | --- | --- | --- |
| 输入区**模型选择**（选中写 `/model <id>`） | 有（`modelSelectionSupported` 门控） | **缺失** | **补齐**（§9.3） |
| 上下文用量提醒 toast（60/80/90%） | 有 | 缺失（不在本轮范围） | 记为后续项 |
| 其余（批量/搜索/分支/导出/关注中心/输入模式等） | 有 | 已对齐 | 仅重做视觉 |

### 3.4 已发现的缺陷（本轮修复）

| # | 缺陷 | 位置 | 处置 |
| :-: | --- | --- | --- |
| B1 | 「打开所在目录」调用不存在的命令 `codex:open-session-folder`（后端注册为 `shell:open-session-folder`），该功能实际是坏的 | `ui/src/ipc/commands.ts:150` | 改名修复 |
| B2 | 4 个被引用但未定义的 token（`--ui-shadow-md/lg`、`--ui-warning`、`--ui-surface-sunken`），声明被浏览器整条忽略 | `app.css` 内联 fallback | 重做 token 时清零 |
| B3 | 实现中发现的同类问题（如 demo 端点路径与后端不一致） | — | 以后端实际路由为准，发现即修并记录 |

## 4. 总体架构

### 4.1 实施路径（已选 A：就地分层重构）

保留现有工程、状态层与测试资产，按组件边界逐个功能面替换。不采用"平行新壳"（状态层
无法干净复制）与"全量重写"（丢弃已验证行为，违背"功能一致"）。

### 4.2 目录结构（轻量重组：只搬"本来就改到"的文件）

> **2026-10-01 整改后修订**：以下为**现状口径**（批次一~三 + 代码审查整改 P1-C/P1-D
> 搬迁、P1-F 死代码清理、P2-D 收口之后）。与本节初稿的差异见文末注记。

```
ui/src/
  components/          # 设计系统与跨层共用件（业务无关，全部可复用）
    icon.tsx / popover.tsx / progress.tsx / badge.tsx / status-dot.tsx
    primitives.tsx / card.tsx / dialog.tsx / feedback.tsx / list.tsx
    loading.tsx / index.ts / components.css
    pager.tsx / context-menu.tsx / icon-button.tsx    # 整改新增：分页 / 右键菜单外壳 / 图标钮
    foldable-text.tsx / error-text.ts                 # 整改新增：长文折叠 / 错误文案归一
    overlay-stack.ts / viewport-menu.ts / use-async-data.ts   # 层次判定 / 视口定位 / 取数 hook
  shell/               # 应用骨架
    app-shell.tsx / title-bar.tsx / activity-bar.tsx / status-bar.tsx
  features/
    sessions/          # 会话侧栏（会话卡、右键菜单、新建/重命名/复制/设置对话框、WSL 表单）
    workbench/         # 工作台侧栏（运行上下文/速率限制/路由变更/今日用量/最近异常 卡）
    workspace/         # 标签栏 / 画布（终端视图 + 历史视图）/ 输入区 / 消息与分支弹框 / 终端搜索与文本菜单
    settings/          # 设置中心外壳 + 10 面板（panels/）+ 网关区间选择器等共用件
    attention/         # 关注中心（铃铛 + 弹层）
  app/                 # 接线、hooks 与纯逻辑：app.tsx / overlay-layer / error-boundary /
                       # exit-confirm / session-workspace（布局接线）；
                       # hooks：use-console / use-terminals / use-workspace-tabs / use-theme /
                       #   use-session-detail；纯逻辑：session-display / gateway-range /
                       #   gateway-format / search-scope / workspace-tabs / theme /
                       #   resume-session / refresh-coalescer / provider-capabilities 等
  ipc/ styles/ types.ts main.tsx
```

搬迁与收口口径（整改后的实际状态）：

- `app/config/*` 面板整族（21 文件）→ `features/settings/panels/`（P1-C）；
- 终端/会话/供应商三族组件（10 源文件 + 9 测试）→ `features/{workspace,sessions,workbench}/`（P1-D）；`app/` 只留**接线、hooks 与纯逻辑**（全部可单测）；
- `sidebar-nav.ts` 已摘除（P2-D）：两个类型并回 `types.ts`，能力函数只从 `provider-capabilities` 进；
- `gateway-range.ts` 拆出 `gateway-format.ts`（P2-D）：区间计算与显示层折叠分开；
- 初稿清单里的 `toggle.tsx` 未保留（零调用方，P1-F 删除；网关开关是业务组的 `switch-pill`）、`tabs.tsx` 已不存在（标签呈现归 `features/workspace/tab-bar.tsx`）。

## 5. 设计系统

### 5.1 Token 双主题（`styles/tokens.css` 重写）

`:root` = 明亮，`:root[data-theme="dark"]` = 暗黑。值取自 demo（emerald + slate），
**按角色命名**，组件不感知主题（只引用语义 token）：

| Token | 明亮 | 暗黑 | 用途 |
| --- | --- | --- | --- |
| `--ui-bg` | `#f8fafc` | `#0b0d13` | 应用/工作区底 |
| `--ui-bg-header` | `#ffffff` | `#11131c` | 顶栏 |
| `--ui-bg-activity` | `#f1f5f9` | `#0d0f16` | 活动栏（永远比侧栏"凹"） |
| `--ui-bg-sidebar` | `#ffffff` | `#131622` | 侧栏 |
| `--ui-bg-card` | `#ffffff` | `#181b29` | 卡片/面板 |
| `--ui-bg-card-hover` | `#f1f5f9` | `#1f2436` | 卡片悬停 |
| `--ui-bg-card-active` | `#e2e8f0` | `#262c42` | 卡片选中 |
| `--ui-bg-input` | `#ffffff` | `#171a27` | 输入控件 |
| `--ui-bg-badge` | `#e2e8f0` | `#1f2436` | 徽章/胶囊 |
| `--ui-bg-tabbar` | `#f1f5f9` | `#0f121d` | 标签栏 |
| `--ui-bg-modal` | `#ffffff` | `#151824` | 弹框体 |
| `--ui-bg-modal-nav` | `#f8fafc` | `#11131c` | 弹框导航列 |
| `--ui-bg-statusbar` | `#ffffff` | `#0d0f16` | 状态栏 |
| `--ui-border` | `#e2e8f0` | `#1f2537` | 常规边框 |
| `--ui-border-strong` | `#cbd5e1` | `#2f3750` | 强边框（输入框、按钮） |
| `--ui-divider` | `#e8edf3` | `#1a2032` | 分隔线 |
| `--ui-text` | `#0f172a` | `#f8fafc` | 正文 |
| `--ui-text-soft` | `#334155` | `#cbd5e1` | 次级正文 |
| `--ui-muted` | `#475569` | `#94a3b8` | 弱化文字 |
| `--ui-faint` | `#64748b` | `#64748b` | 最弱（时间、元数据） |
| `--ui-accent` | `#059669` | `#10b981` | 唯一强调色 |
| `--ui-accent-strong` | `#047857` | `#34d399` | 强调悬停 |
| `--ui-accent-soft` | `#ecfdf5` | `rgba(16,185,129,.15)` | 强调底 |
| `--ui-accent-border` | `#a7f3d0` | `rgba(16,185,129,.35)` | 强调边框 |
| `--ui-focus` | `rgba(5,150,105,.25)` | `rgba(16,185,129,.35)` | 焦点环 |
| `--ui-warn[-soft/-border]` | `#b45309` / `#fffbeb` / `#fde68a` | `#fbbf24` / `rgba(251,191,36,.12)` / `rgba(251,191,36,.35)` | 等待确认 |
| `--ui-danger[-soft/-border]` | `#dc2626` / `#fef2f2` / `#fecaca` | `#f87171` / `rgba(248,113,113,.12)` / `rgba(248,113,113,.35)` | 错误 |
| `--term-bg/-border/-text/-prompt` | `#f8fafc/-#e2e8f0/-#1e293b/-#059669` | `#0d0f17/-#1e2434/-#e2e8f0/-#10b981` | xterm 与终端画布 |
| `--term-card-bg/-border/-cursor` | `#ffffff/-#cbd5e1/-#0f172a` | `#141824/-#232b3e/-#10b981` | 终端内卡片与光标 |

沿用（值不变）：字号阶 `--ui-text-xs…xl`（补 `--ui-text-base: 13.5px`）、行高、间距阶
`--ui-space-1…6`、圆角阶（补 `--ui-radius-xl: 16px` 供弹框）、层级阶、字体栈
（系统栈 + Cascadia 等宽，不内置字体、不引 CDN）。

**唯一强调色的用法约定不变**：只用于「选中 / 主操作 / 焦点环」；等待确认走 warn、
错误走 danger。金色收藏星是刻意保留的第二色相（沿用现有注释理由）。

### 5.2 主题机制

- `app/theme.ts`（纯逻辑，可单测）：`ThemePreference = "system"|"light"|"dark"`；
  `resolveTheme(pref, systemDark) → "light"|"dark"`；读写 localStorage 键
  `aivc.theme`（容错：非法值回退 `system`）。
- `app/use-theme.ts`：`useTheme()` → `{preference, resolved, setPreference}`；
  监听 `matchMedia("(prefers-color-scheme: dark)")` 变化；把 `resolved` 写到
  `document.documentElement.dataset.theme`。
- `index.html` 内联脚本：首帧前读取 localStorage + 系统偏好并设好 `data-theme`，
  避免暗色用户开机闪白（FOUC）。
- xterm：`TerminalPane` 从 `--term-*` 计算主题对象，主题切换时 `term.options.theme = …`
  即时更新（不重建终端）。
- 切换入口：顶栏按钮（循环 明亮→暗黑→跟随系统，按钮文案显示当前态）。

### 5.3 图标系统

`components/icon.tsx`：内联 SVG 注册表，`<Icon name="chat" size={16} />`。
约 28 个：`chat / chart / settings / sun / moon / monitor / bell / search / plus / close /
chevron-down / chevron-right / trash / star / branch / paperclip / image / file / folder-open /
external / refresh / copy / alert / check / info / play / stop / panel-left / send / circle-dot /
globe / plug / wrench / heart-pulse / list / clock / git-branch` 等（以实现时实际用到为准）。
统一 24 viewBox、`stroke=currentColor`、`stroke-width=1.75`。
**demo 中的 emoji（📊🔌🌿⭐ℹ️ 等）一律替换为 SVG**——emoji 跨平台渲染不一致，
旧平台 L19 亦要求图标统一。

### 5.4 组件清单（全部放 `src/components/`，业务无关）

| 组件 | 变体/能力 | 说明 |
| --- | --- | --- |
| `Button` | primary / secondary / ghost / danger；sm / md | 现有扩展 |
| `Badge` | tone: accent / neutral / warn / danger；mono / small 可选 | 徽章（消息数、候选池状态、表格结果标记等；`small` 供密集表格） |
| `Chip` / `ChipGroup` | 可选中 | 现有 |
| `StatusDot` | tone: ok / warn / danger / idle；pulse 可选 | 运行中脉冲点、就绪点 |
| `ProgressBar` | tone: accent / warn / danger；value% | 上下文窗口、速率限制 |
| `Popover` | 锚定 + 点击外部/Esc 关闭 + 视口夹取 | 供应商锁定浮窗、附件菜单 |
| `Modal`(Dialog) | 尺寸 sm/md/lg/xl + `twoColumn` 内容区 | 设置中心、消息与分支等 |
| `Card` | panel / metric / list 三形态 | 统一卡片外观（细边框、`--ui-radius-lg`） |
| `Field` / `TextInput` / `Select` / `SearchInput` | 现有扩展 | 表单 |
| `EmptyState` / `Notice` / `LoadingBlock` | 现有 | 反馈（`Spinner` 为 `LoadingBlock` 的内部件，不再对外再出） |
| `Toast` | 成功/错误 tone；可选 action 按钮 | 现有 `useToast` 扩展 |
| `Icon` | §5.3 | 新增 |
| `Pager` | Fragment 返回；`extra` / `withLast` / `labelClassName` / `ariaLive` | 三处分页的唯一实现（整改 P1-E①） |
| `ContextMenu` | 遮罩 / 视口定位 / 先做事再关 / Escape 让位 | 三个右键菜单的外壳（整改 P1-E②） |
| `IconButton` | `label` 必填；`title` 缺省取 label；`active` | 图标钮（整改 P1-E④） |
| `FoldableText` / `FOLD_CHARS` | 超过 4000 字折叠、展开即全文 | 弹框与历史正文共用（整改 P1-J） |
| `errorText` | `unknown → string` | 全仓错误文案归一（整改 P2-C；`ipc/client.ts` 为唯一例外） |
| `Tooltip` | 用原生 `title` | 不造轮子 |

注（整改裁定）：初稿的 `Tag` 行**不再实现**——平台徽章与状态标记都由 `Badge`（含
`small`）承担，一个组件一份实现（整改 P1-E③）；`Toggle` 行作废（零调用方，P1-F）。

## 6. 应用骨架（`shell/`）

```
┌ TitleBar 40px ───────────────────────────────────────────────┐
│ ▣AI  AI Visual Console  [v0.1.0-rs]   ←拖拽区→   [☀/🌙] [🔔] │
├──────┬────────────────────────┬───────────────────────────────┤
│活动栏 │ 会话侧栏 310px          │ 工作区                        │
│50px  │ 或 工作台侧栏 340px      │ ┌ TabBar 44px ──────────────┐ │
│ 会话 │                        │ │ 标签…  ＋    查找 消息与分支 │ │
│ 工作台│                        │ ├ 画布（终端 / 历史）────────┤ │
│      │                        │ ├ ComposerBar ──────────────┤ │
│ 设置 │                        │ └───────────────────────────┘ │
├──────┴────────────────────────┴───────────────────────────────┤
│ StatusBar 28px：会话 · 更新 · 目录 · 供应商▾ ｜ 输入模式         │
└───────────────────────────────────────────────────────────────┘
```

- `AppShell`：CSS Grid（rows: auto 1fr auto；body: columns 50px auto 1fr）。
  `data-theme` 在 `<html>`；`body` 保持 `min-width:980px; min-height:640px`。
- **活动栏**（50px）：上「会话」「工作台」，下「设置」；选中态=accent-soft 底 +
  accent 边框 + 左侧 2px 指示条；`title` 提示；设置按钮打开设置中心。
- **侧栏折叠**：保留现有能力（终端列数敏感），入口改为侧栏头右侧的 `panel-left`
  图标按钮；折叠后仅活动栏可见（现有 36px 折叠条不再需要）。
- **标题栏**：左侧品牌（渐变方块 AI + 名称 + 版本徽章，版本取 `app:get-version`）；
  中间 `-webkit-app-region: drag`；右侧主题切换 + 铃铛（关注中心，含未读角标）。
- **状态栏**：左「会话: `shortId`（点击复制）| 更新: … | 目录: …（截断+title）|
  目标: …（窄窗隐藏）| 供应商: 名称（锁定徽章）▾（Popover：供应商列表含健康与延时、
  锁定复选框、模式切换）」；右「网关: 端口/未启用 · 输入模式胶囊（自管输入 /
  终端原生输入，点击切换）」。保留现有 busy/searching 状态文案与错误条、
  以及窄窗口三级降级（1180/1040/900，优先级低者先隐藏）。
  纯计数（会话数/终端数）不再单列——侧栏摘要与工作台已覆盖。

## 7. 会话侧栏（`features/sessions/`）

自上而下：

1. **头部**：网关胶囊（`网关: <port>` / `未启用`，状态点，点击→设置中心·网关面板）+
   「＋ 新建会话」主按钮（打开现有新建会话对话框）。
2. **平台/目标选择器**：`1. 平台`（右侧显示当前平台 CLI 提示）+ `2. 目标`
   （右侧状态点 + 就绪/探测中/不可用文案）。保留目标不可用 Notice 与
   `WslHomeForm`（WSL 无 CODEX_HOME 时），样式重做。
3. **搜索**：`SearchInput`（本地即时过滤 + 250ms 防抖全文搜索，沿用现有逻辑）。
   输入后列表区切换为结果视图：结果行显示 目标 + 标题 + 匹配来源；
   结果区头部提供「在所有目标中搜索」按钮与「搜索消息正文」开关
   （承接现「搜索」pane 的能力，不再占一级导航）。
4. **过滤 chips**：`全部 (N)` / `⭐ 收藏 (N)`；右侧回收站图标按钮（切换 trash 视图，
   平台不支持时禁用+`title` 说明）。
5. **会话卡片**（`SessionCard`）：
   - 行 1：状态点（有存活终端=accent 脉冲；否则灰）+ 标题（粗）+ 右侧时间（等宽）。
   - 行 2：预览一行省略（沿用 `pickPreview`）。
   - 行 3：徽章 `N 条消息`（有分支时 `· N 分支`）；悬停出现动作：
     `⭐ 收藏`、`🌿 分支`（打开消息与分支弹框）、`⋮ 更多`（同现有右键菜单）。
   - 选中态：`--ui-bg-card-active` + accent 边框。
   - 批量勾选框：平台支持时显示（现有 `selectable` 语义），勾选后顶部出现批量条
     （已选 N / 全选 / 恢复 / 彻底删除 / 移入回收站，沿用现有逻辑）。
6. **底部摘要**：`<平台> · 共 N 条会话` + `⟳ 刷新`。

右键菜单（`SessionContextMenu`）项不变：打开所在目录（**修 B1**）/重命名/复制会话/
收藏/复制会话编号/复制文件路径/导出 Markdown|JSON|HTML（新增子菜单，承接菜单栏）/
恢复/移入回收站|彻底删除。

## 8. 工作台侧栏（`features/workbench/`）

头部：`📊 运行概览与工作台`（图标用 SVG）+「实时监控」徽章 + 关闭按钮（回会话）；
内容卡（数据沿用现有 `sidebar-workbench`，只重排视觉）：

| 卡 | 内容 |
| --- | --- |
| 运行上下文 | 目标/平台徽章；当前模型；会话已消耗 tokens；上下文窗口进度条 + `已用% / 窗口` |
| CLI 速率限制监控 | 主窗口 / 次窗口：`usedPercent% (已用/上限)` + 进度条 + 重置倒计时；≥80% 转 warn；无数据显示「无数据」 |
| 最近路由变更 | 供应商 A ➔ B（accent）+ 原因 + 时间；无变更显示空态 |
| 今日 Gateway 用量 | 请求数（成功率）+ 平均延时；`用量明细 ↗` → 设置中心·用量面板 |
| 最近 Gateway 异常 | 保留现有卡（时间 + 供应商一句话），点击 → 设置中心·异常面板 |

刷新：现有 300ms 事件合并 + 手动刷新按钮（保留，不轮询）。

## 9. 工作区（`features/workspace/`）

### 9.1 统一标签模型（D3）

```ts
type WorkspaceTab = {
  key: string;              // sessionId（已知）或 terminalId（新会话尚未拿到 id）
  sessionId?: string;
  terminalId?: string;
  title: string;
  providerId: AiProviderId;
  messageCount?: number;    // 「消息与分支 (N)」用
};
```

行为表（替换现有 `use-workspace-tabs.ts` 的 session/terminal 双 kind 模型）：

| 动作 | 行为 |
| --- | --- |
| 点击会话行 | **直接进入该会话的终端**（2026-10-02 用户裁定，替换「打开标签看历史」的批次一口径）：该会话的终端在跑 → 激活其标签；没在跑（含异常退出残场）→ 带 `--resume` 起一路（原目录不在时弹「设置工作目录」）。恢复在途的连点/双击由 `resumingRef` 守卫收敛。**详情走卡片/标签栏的「消息与分支」**，不再途经画布历史正文 |
| 启动/恢复终端 | 若该会话已有标签 → 挂载 terminalId 到同一标签（不新建）；新会话（无 sessionId）→ 以 terminalId 为 key 新建 |
| 激活标签 | 有 terminalId 且终端存在 → 终端视图；否则历史视图 |
| 关闭标签 | 先 `terminal:stop`（若在运行）→ 移除标签；激活键切到剩余最后一张 |
| 干净退出 | 自动收标签（沿用现有语义与注释理由） |
| 异常退出 | 保留标签，标题缀 `（退出码 N）`，画布停留在终端最后画面 |
| 批量关闭 | 关闭其他 / 关闭全部（语义按新模型重定义：作用于全部标签，均先停终端） |
| 进入会话前清理 | 保留 `clearDeadTabs`（判据＝终端不在了，而非标签存在） |
| 标签 ↔ 选中联动 | 激活任一标签（含终端标签，其 sessionId 已知时）→ `setSelectedId(sessionId)`；关闭激活标签后切到剩余最后一张并同步选中，无标签则清空选中 |
| 持久化 | 不做（与现状一致：标签是会话内的临时状态） |

纯逻辑（`app/workspace-tabs.ts`）与 hook（`app/use-workspace-tabs.ts`）随新模型重写并
更新单测；`attention` 标记按 tab.terminalId 映射到标签点。

### 9.2 画布

- **终端视图**：xterm 占满余下空间（`TerminalPane` 保留，主题随明暗联动）；终端搜索条
  （`Ctrl+F`，现有能力）浮于画布右上。~~顶部会话信息条（标题 · 短编号 · 目标 · 供应商+模式 ·
  网关 Base URL）~~ **2026-10-02 用户裁定移除**——那几项在状态栏与设置中心都有，不常驻终端上方。
- **历史视图**：消息流沿用现有角色区分并重做视觉——用户=引述块（accent-soft 底 +
  左 3px 竖线）、助手=无框正文、工具/系统=等宽小字块；顶部「加载更早消息」；
  每条 hover 显示「从此处分支」；顶部血缘条（`来源：… · 子分支：N`，点击开弹框）。
- **空态**：无标签时显示引导（选择左侧会话继续，或「新建会话」），含快捷键提示。

### 9.3 输入区（`ComposerBar`）

- 视觉按 demo：卡片 + `focus-within` 焦点环；底部工具行（模型选择｜`📎 附件 ▾`｜
  `Enter 发送`｜发送按钮）；高度可拖拽（保留 56–420px）。
- 保留全部现有能力：附件 chips、大段粘贴压缩（`[Pasted Content N chars]`）、
  拖拽文件、图片粘贴→`shell:save-image`、路径识别为附件、`Shift+Enter` 换行、
  括号粘贴提交、图片预览（缩放/复制）。
- 附件下拉合并为**一个**菜单（添加图片 / 添加文件，`Popover` 实现）——与 demo 一致，
  取代现有两个按钮。
- **补齐模型选择**（§3.3）：下拉数据 = 该终端供应商的 `vendor:list-models`
  （Qoder 走 `models:list`）；选中即向 PTY 写 `/model <id>`（同旧平台语义）；
  列不到模型时不渲染该下拉；加载/失败静默降级为不显示（旧平台同）。
- 输入模式指示移至状态栏（§6）；输入区自身不再放切换按钮。

### 9.4 消息与分支弹框（`MessagesModal`）

由现有 `session-detail-dialog` + `branch-panel` 重构（逻辑与测试沿用，外观重做）：

- 头部：`🌿 会话消息历史与分支演进` + `ID: <shortId>` 徽章 + 关闭。
- 血缘条：`根会话 ➔ … ➔ 当前会话（accent）➔ 子分支/无下游分支`，点击父/子跳转
  （沿用现有 `onOpenSession`）。
- 消息区：头像（我 = 蓝底「我」，助手 = accent 底「AI」）+ 角色/时间 + 内容卡片；
  hover「从此处分支」（能力门控 `supportsBranch` 保留）；分页控件保留
  （20/50/100/200 + 首/上/下/末）。
- 底部：`共 N 条历史消息` +「导出 ▾」（Markdown/JSON/HTML）+ 关闭。
- 入口：TabBar 右侧「消息与分支 (N)」；会话卡 🌿；历史视图血缘条。

## 10. 设置中心（`features/settings/`）

`SettingsCenter`：Modal（1024×660，`--ui-radius-xl`），左侧 224px 分组导航 + 右侧面板区。
**10 个面板全部复用现有业务逻辑与测试**，按 demo"面板头（标题+说明+操作区）+ 内容卡"
重排。分组与标题：

| 分组 | 面板 key | 标题 | 迁移来源 |
| --- | --- | --- | --- |
| 🔌 服务与路由 | `vendors` | 供应商管理 (N) | `config/vendors-panel.tsx` |
| | `gateway` | 本地网关 & 路由 | `config/gateway-panel.tsx`（三卡重排） |
| 📊 观测与审计 | `usage` | 用量报表 | `config/gateway-usage-panel.tsx` |
| | `failures` | 异常诊断 | `config/gateway-failure-panel.tsx` |
| | `logs` | 请求日志 | `config/gateway-log-panel.tsx`（补「清空全部日志」） |
| 🛠️ 环境与扩展 | `skills` | 平台技能 (Skills) | `config/skills-panel.tsx` |
| | `environment` | 运行时环境 | `config/environment-panel.tsx`（补「WSL 会话目录」卡） |
| | `presets` | 工作区预设 | `config/presets-panel.tsx` |
| | `diagnostics` | 系统诊断 | `config/diagnostics-panel.tsx`（保留能力矩阵/路径探测） |
| ℹ️ 系统与关于 | `about` | 关于控制台 | `about-dialog.tsx` + 新增退出入口 |

面板要点（与 demo 对齐处的差异，全部以现有能力为准）：

- **供应商管理**：头部四按钮（查余额 / 测连接(列模型) / 读本地配置 / ＋新增供应商）=
  现有 `vendor-probe-panel` 的能力合并进工具条 + 表单视图；厂商过滤 chips；
  表格列 = 名称/端点 · 厂商 · 费率(输入/输出/M) · 健康延时 · 候选池状态 · 操作
  （编辑 / 启用·禁用 / 删除）；余额查询结果以行内状态呈现（沿用现有状态机
  「未获取/查询中/查询失败/(已过期)」+ 工具条"刷新全部余额"）。
- **本地网关 & 路由**：① 头部总开关胶囊（`gateway:set-enabled`）；② 端口与熔断韧性卡
  （端口 + 三条阈值，**集中保存**，沿用三个 set 命令，端口提示"重启生效"，阈值即时）；
  ③ 外部网关卡（开关 + 共享令牌轮换 + **端点清单**（路径以后端 `routes.rs` 实际为准，
  不照抄 demo 字符串）+ 各平台认证协议说明 + 一次性令牌横幅）；④ 故障规则卡
  （列表 + 表单：作用域 global/provider/vendor 级联选择、文本包含、状态码 CSV、
  自定义应答状态码+JSON 正文、启用开关 + **在线测试引擎**（`gateway:test-failover-rule`））。
- **用量报表**：范围（今天/近7天/近30天/全部/自定义）+ 汇总卡（请求数 / Tokens /
  费用，demo 三卡）+ 按供应商/按模型维度表（现有）。
- **异常诊断**：筛选（平台/供应商/结果/时间）+ 分页表（现有 20/页）。
- **请求日志**：筛选（结果类型/时间）+ 全选当前页 + 删除选中 + **清空全部日志** + 分页。
- **平台技能**：已安装/回收站 + 导入（文件夹 / .md）+ 冲突覆盖 + 打开目录
  （信息架构沿用现有，样式重做）。
- **运行时环境**：工具状态行（Node/npm/nvm/CLI）+ Node 版本选择 + 安装 + 重新检测；
  **新增「WSL 会话目录」卡**：承接被移除的菜单「设置会话」（distro + CODEX_HOME +
  恢复自动探测）。
- **工作区预设**：列表 + 新增/删除（沿用）。
- **系统诊断**：目标表 + 路径探测 + 平台能力矩阵（沿用）。
- **关于控制台**：品牌块 + 版本卡（应用版本 / 渲染核心）+ 描述 + 按钮
  「打开日志目录」（`app:command openLogDir`）、「退出控制台」（`app:command quit`，
  仍走退出确认）。**不实现 demo 的"检查新版本"**（后端无此能力）。

## 11. 入口迁移与移除清单（D1）

移除：`app-menubar.tsx`、`app-menu-bar.tsx`、`app-menus.ts` 及其测试
（`app-menu-bar.test.tsx`）；`app.tsx` 中的 menubar 装配。

| 原菜单项 | 新落点 |
| --- | --- |
| 管理 Skill | 设置中心·平台技能 |
| 设置会话（WSL CODEX_HOME） | 设置中心·运行时环境「WSL 会话目录」卡 |
| 导出 Markdown / JSON / HTML | 消息与分支弹框底部「导出 ▾」+ 会话右键菜单「导出 ▾」 |
| 退出 | 设置中心·关于「退出控制台」（经退出确认流程） |
| 供应商管理 / 网关 | 设置中心对应面板（活动栏「设置」默认打开供应商） |
| 安装 CLI | 设置中心·运行时环境 |
| 打开日志目录 | 设置中心·关于（demo 同款按钮） |
| 清理 Gateway 日志 | 设置中心·请求日志「清空全部日志」 |
| 关于 | 设置中心·关于 |

关注中心（铃铛）从菜单栏右侧迁至 TitleBar 右侧；导出完成提示沿用现有 hint
（重做样式，含「打开」按钮）。

## 12. 顺带修复与调整（B1–B3 及实现建议）

1. **B1**：`commands.ts` 的 `openSessionFolder` → `shell:open-session-folder`（含测试）。
2. **B2**：token 重写时清零未定义引用；`components.css` / `app.css` 中散落的硬编码
   颜色与尺寸在重排时逐处换 token。
3. **B3**：实现中发现的同类问题（如 demo 与后端端点路径不一致）以后端为准，发现即修。
4. 旧文件不再需要的（如 `.sidebar.collapsed` 折叠条、`.system-dock` 残留样式）删除，
   不留"看起来在用"的死规则。

## 13. 测试与验证

- **单元/组件测试**：现有 40 个文件全部保留并同步更新（类名/结构断言）；
  新增纯逻辑单测：`theme.ts`（偏好解析与持久化）、`workspace-tabs.ts` v2（标签行为表
  逐条）、`settings-nav.ts`（分组模型）；新增组件测试：`Icon`（渲染与 a11y）、
  `SettingsCenter`（导航切换）、`MessagesModal`（血缘与分支回调）、
  `ComposerModelSelect`（写 `/model`、空列表隐藏）。
- **门禁**：`cd ui && npm run typecheck && npm test && npm run build`。
- **界面验证**：`pwsh scripts/ui-check.ps1 -Name <阶段名>`（启动+CDP 离屏截图）；
  `pwsh scripts/ui-baseline.ps1 -Name <阶段名>` 视图基线集（明/暗各一套），
  逐张 diff；旧基线（`shots/after-tokens/` 等）留档不删。
- **Rust 侧**：不动。若发现必须的后端缺口（例如端点路径确有问题），先报告再决定，
  如改动则跑 `scripts/verify.ps1`。
- 每批交付前必须：门禁全绿 + 截图核对（原尺寸看图，遵 doc 29 的教训）。

## 14. 阶段划分（D4）

| 批次 | 内容 | 交付后状态 |
| --- | --- | --- |
| **一** | 设计系统（token 双主题 + Icon + 组件库扩展 + theme 机制）；应用骨架（TitleBar/ActivityBar/侧栏容器/StatusBar）；会话侧栏；工作台侧栏；TabBar + 终端画布 + ComposerBar；统一标签模型；消息与分支弹框；设置中心**外壳**（新导航 + 现有 9 面板挂入 + 新增关于）；移除菜单栏并完成入口迁移 | 应用完整可用，新视觉成形，功能零丢失 |
| **二** | 设置中心 10 面板按 demo 重排；其余弹框统一新样式（新建会话/重命名/复制/会话设置/退出确认/右键菜单/导出提示/关注中心） | 全部界面完成新视觉 |
| **三** | 暗色逐屏走查修正；死代码清理；基线重建；`docs/rust-rewrite/` 补一篇重做记录（含截图与遗留项） | 收尾 |

## 15. 明确不做

- 命令面板、系统终端（Rust 版已移除，用户明确）。
- demo 中后端无能力项：状态栏/关于的「检查新版本」、数据库健康诊断面板、
  窗口控制按钮。
- 内置字体（保留有意的系统字体栈；后续换字体只改两个 token）。
- Rust 后端功能扩展（本轮纯前端；发现缺口另行报告）。

## 16. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| 标签模型重构触碰 `app.tsx`/hook 深水区（竞态注释里记录过多起历史 bug） | 行为表逐条落地为单测；保留既有全部语义（干净退出自动收、异常保留、死标签清理） |
| 双主题导致视觉验证工作量翻倍 | token 化彻底（组件不写死颜色）；基线集明暗各一套，逐屏 diff |
| 40 个测试文件同步更新量大 | 按批次推进，每批只改受影响文件；纯逻辑测试尽量不动（模型不变的模块） |
| demo 与后端实际能力不一致（端点路径、字段） | 以后端为准（B3），差异记录在设计文档"实现记录"节 |
| 侧栏 310px 收窄影响信息密度 | 卡片两行布局 + 徽章换行策略，截图核对；必要时按 demo 340px 上限微调 |
