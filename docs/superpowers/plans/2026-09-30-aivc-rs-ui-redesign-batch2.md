# aivc-rs 界面重做 · 批次二 实现计划（无边框窗口 + 设置中心面板 + 弹框统一）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ① 窗口改为无边框并补齐应用内窗口控制；② 按 demo 重排设置中心十个面板；③ 统一全部弹框与右键菜单样式；④ 清掉批次一的结转小项。

**Architecture:** 就地演进（同批次一）。面板重排以「现有业务逻辑与测试不动、只改结构与样式」为原则：`PanelHeader` 统一面板头，表格/卡片/筛选按 demo 的信息架构重排；弹框统一走既有 `Dialog` 与新的 `.dialog-*` 样式。窗口控制新增 4 条平台命令（`app:window-*`），前端只经 `aivc_invoke`（维持「前端只与 Rust 后端单命令面通信」的约束）。

**Tech Stack:** 同批次一（React 19 + TS + Vite + Vitest；Rust 侧 tauri 2.11.6 / tao 0.35.3）。

**Spec:** `D:/code/ai/docs/superpowers/specs/2026-09-30-aivc-rs-ui-redesign-design.md`（§10 面板清单、§5 设计系统）
**上一批记录（含全部结转项，本批的输入）:** `D:/code/ai/docs/rust-rewrite/39-ui-redesign-batch1.md`（§六 批次二待办、§七 遗留明细、§八 截图核对结论）
**Demo（视觉真源）:** `D:/code/ai/docs/ui-redesign-demo.html`

## Global Constraints

- 依赖零新增（前端与 Rust 均不引新 crate/npm 包）。
- 样式一律用 `ui/src/styles/tokens.css` 的 token；注释用中文、只解释「为什么」。
- 前端门禁：`cd ui && npm run typecheck && npm test && npm run build`。
- **改动 Rust 的任务必须跑**：`powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1`（从 `aivc-rs/` 根；含 fmt/check-layers/clippy/test）。
- 截图核对：本批**每个任务完成后由控制器统一跑**（用户已确认启用）；任务内不自行启动应用。命令：
  `powershell.exe -NoProfile -ExecutionPolicy Bypass -File scripts/ui-check.ps1 -Name <名> [-Probe "..."]`
  （注意：探针里要等 React 提交，用 `MessageChannel` tick；不要用 `setTimeout`——离屏窗口下计时器不触发。
  另：`ui/dist` 变了必须先 `scripts/cargo.ps1 build -p aivc-app`，否则嵌的还是旧界面。）
- 工作区约定：批次二沿用 aivc-rs 主工作区（`main` 分支直接提交；批次一先例已合入 main）。
- 每任务提交中文信息，`feat(ui):`/`fix(ui):`/`refactor(ui):` 前缀。

---

### Task 1: 无边框窗口（Rust 命令面 + 应用内窗口控制）

**背景：** 批次一截图确认存在双标题栏（系统框「AI 可视化控制台」+ 应用内 40px「AI Visual Console」）。用户决定改无边框。
**已核实的既有事实：** tao 0.35 对 `decorations(false) + resizable(true)` 的窗口实现 `WM_NCHITTEST`（无阴影标记时四处边缘全支持，有 undecorated shadow 时由系统提供边缘）⇒ **边缘缩放仍可用**；`tauri_app.rs:484` 已hook `WindowEvent::CloseRequested` 走退出确认链路；前端只用 `aivc_invoke`（`ui/package.json` 的描述即此约束）。

**Files:**
- Modify: `crates/aivc-app/src/tauri_app.rs`（builder 加 `.decorations(false)`；`platform_command` 加 4 条）
- Modify: `crates/aivc-ipc/src/commands.rs`（注册表 + 计数断言：93→97，App 域 3→7）
- Modify: `crates/aivc-ipc/src/router.rs`（`HANDLED_BY_PLATFORM` 9→13）
- Modify: `ui/src/ipc/commands.ts` + `ui/src/ipc/commands.test.ts`
- Modify: `ui/src/shell/title-bar.tsx` + `ui/src/shell/title-bar.test.tsx`（新建）+ `ui/src/app/app.css`
- Modify: `ui/src/features/settings/panels/about-panel.tsx`（退出改走 `app:window-close`）

**Interfaces:**
- 新命令（参数均为空对象 `{}`）：`app:window-minimize`、`app:window-toggle-maximize`、`app:window-start-drag`、`app:window-close`（返回 `null`）。
- 前端桥：`windowMinimize()`、`windowToggleMaximize()`、`windowStartDrag()`、`windowClose()`（均 `invoke<null>`）。
- `TitleBar` 新增 props：`onMinimize: () => void`、`onToggleMaximize: () => void`、`onClose: () => void`、`onStartDrag: () => void`（由 `app.tsx` 注入，内部调桥）。

- [ ] **Step 1: Rust 命令面**
  - `commands.rs` 在 App 域追加 4 条（顺序与现有表一致），`COMMAND_COUNT` 93→97，`commands_of(CommandDomain::App).len()` 3→7；跑 `cargo test -p aivc-ipc` 确认注册表测试通过。
  - `router.rs` 的 `HANDLED_BY_PLATFORM` 追加 4 条（数组长度 9→13）；跑 `cargo test -p aivc-ipc`。
  - `tauri_app.rs::platform_command` 加 4 个分支（`use tauri::Manager as _;` 已在函数所在模块内可用；取窗用 `app.get_webview_window(crate::tray::MAIN_WINDOW)`，取不到返回 `Err("主窗口不存在")`）：
    ```rust
    "app:window-minimize" => Some(window_of(app).and_then(|w| w.minimize().map(|()| Value::Null).map_err(|e| e.to_string()))),
    "app:window-toggle-maximize" => Some(window_of(app).and_then(|w| {
        let target = !w.is_maximized().unwrap_or(false);
        (if target { w.maximize() } else { w.unmaximize() }).map(|()| Value::Null).map_err(|e| e.to_string())
    })),
    "app:window-start-drag" => Some(window_of(app).and_then(|w| w.start_dragging().map(|()| Value::Null).map_err(|e| e.to_string()))),
    "app:window-close" => Some(window_of(app).and_then(|w| w.close().map(|()| Value::Null).map_err(|e| e.to_string()))),
    ```
    （`window_of` 是一个小 helper：`fn window_of(app) -> Result<tauri::WebviewWindow, String>`。）
  - builder 链加 `.decorations(false)`，并加注释说明：tao 对无边框可缩放窗口自带 `WM_NCHITTEST` 边缘处理（已核实 tao 0.35.3 源码），因此边缘缩放与阴影不丢；窗口标题保留（任务栏标签用）。
- [ ] **Step 2: 前端桥 + 测试**：`commands.ts` 加 4 个函数（命令名与后端逐字一致）；`commands.test.ts` 各加一条载荷断言。
- [ ] **Step 3: TitleBar 窗口控制**：右侧在主题按钮之后追加三个 `.title-window-btn`（最小化 `−`、最大化 `□`/还原 `❐`（用 `Icon` 里已有的 `close`/`chevron-*` 之外的字符太丑——**新增两个 Icon**：`minus`、`square`）+ 关闭 `close`）。关闭按钮 hover 用 `--ui-danger`。`aria-label`：最小化/最大化/关闭窗口。
  拖拽：`.title-drag` 上 `onMouseDown={(e) => { if (e.button === 0 && e.detail === 1) onStartDrag(); }}`、`onDoubleClick={onToggleMaximize}`；**移除** `WebkitAppRegion` 内联样式（避免双机制打架），更新那段的注释（说明为什么用命令而不是 `-webkit-app-region`：单命令面约束 + 各 WebView2 版本支持不一）。
- [ ] **Step 4: app.tsx 接线 + 通用退出改道**：`app.tsx` 给 `TitleBar` 传 4 个回调（调桥，失败走 `console.setError`）；`about-panel` 的「退出控制台」由 `window.close()` 改为 `api.windowClose()`（同一「关闭窗口 → 后端退出确认」链路；`app.tsx` 里若还有别的 `window.close()` 一并替换）。
- [ ] **Step 5: 门禁 + 截图**：前端三连全绿；**Rust 侧跑 `scripts/verify.ps1`**（改动 `crates/` 的必做）；然后 `scripts/cargo.ps1 build -p aivc-app` 重建 + 整窗截图（复用批次一的一次性 PS 写法：启动→等 12s→`GetWindowRect`→`CopyFromScreen`）确认**只剩应用内一条标题栏**；再 CDP 截图确认三个窗口按钮渲染正常。
- [ ] **Step 6: 提交**（`feat: 无边框窗口与应用内窗口控制（app:window-* 四条命令）`）。

**验收：** 无系统标题栏；拖拽区能移动窗口；双击拖拽区最大化/还原；三个按钮工作；关闭按钮弹出退出确认；边缘可缩放（**用户手动确认**——自动化证明不了手感）。

---

### Task 2: 设置中心面板头统一 + 供应商面板重排

**Files:** `ui/src/app/config/vendors-panel.tsx`、`ui/src/app/vendor-probe-panel.tsx`（或并入）、`ui/src/features/settings/panels/*`、`ui/src/app/app.css`、相关测试。

**要求（demo 真源：`ui-redesign-demo.html` 第 844-1004 行）：**
- 面板头统一用 `PanelHeader`（标题 + 一句说明 + 右侧操作区）；十个面板全部套上（本轮起，批次一留的只有关于页用了）。
- 供应商面板：头部四按钮（查余额 / 测连接(列模型) / 读本地配置 / ＋新增供应商）= 现有 `vendor-probe-panel` 的能力合并进工具条；厂商过滤 chips（全部/Codex/Claude/Gemini/Qoder，带条数）；表格列 = 名称/端点 · 厂商 · 费率(输入/输出/M) · 健康延时 · 候选池状态 · 操作（编辑 / 启用·禁用 / 删除）；余额以**行内状态**呈现（沿用现有状态机：未获取/查询中/查询失败/(已过期)）。
- **不得丢**：启用/禁用与 `vendor:enable`（写 CLI 配置）的区别、删除确认文案、高级 JSON（模型查询/余额查询配置）与 `<details>` 折叠、表单校验与错误面板、`vendor:read-configs` 白名单说明。

---

### Task 3: 网关面板三卡重排

**Files:** `ui/src/app/config/gateway-panel.tsx`（拆分为 `gateway-port-card.tsx` / `gateway-external-card.tsx` / `gateway-rules-card.tsx` 三个子文件 + 组装），`app.css`，测试。

**要求（demo 第 1006-1371 行）：**
- 头部：`PanelHeader` + 右侧**总开关胶囊**（`gateway:set-enabled`）。
- 卡 1 端口与熔断韧性：端口输入 + 「活跃端口/未监听」状态 + 三条阈值（异常切换 1–10 / 熔断次数 1–20 / 熔断时长 10–86400s）+ **集中保存**按钮（沿用三个 set 命令；端口提示「重启生效」）。
- 卡 2 外部网关：开关 + 共享令牌轮换（一次性令牌横幅）+ **平台端点清单**（路径以 `crates/aivc-gateway/src/routes.rs` 的 `route_prefix`/`build_route_url` **实际形制为准**，不照抄 demo 字符串）+ 各平台认证头说明（`Authorization: Bearer` / `x-api-key` / `x-goog-api-key`）+ 仅监听 127.0.0.1 的提示。
- 卡 3 故障规则：列表（作用域/模式/状态码/自定义返回/状态/编辑·启停·删除）+ 表单（scope 级联、文本包含、状态码 CSV、自定义应答状态码+JSON、启用开关）+ **在线测试引擎**（`gateway:test-failover-rule`，命中理由逐条列出）。
- **不得丢**：`parseCustomResponse` 校验（两字段同时配置或同时留空、JSON 合法）、规则测试的 matchedBy 三态。

---

### Task 4: 用量 / 异常 / 日志面板重排

**Files:** `ui/src/app/gateway-usage-panel.tsx`、`gateway-failure-panel.tsx`、`gateway-log-panel.tsx`、`app.css`、测试。

**要求：**
- 三页统一 `PanelHeader`；范围选择沿用 `GatewayRangePicker`（今天/近7天/近30天/全部/自定义）。
- 用量页：三张汇总卡（请求数 / Tokens / 费用，demo 第 1386-1399 行）+ 按供应商/按模型维度表（现有）。
- 异常页：筛选行（平台/供应商/结果/时间）+ 分页表（20/页，现有）。
- 日志页：筛选（结果类型/时间）+ 全选当前页 + 删除选中 + **清空全部日志**（T6 已加，样式收口到与工具条一致）。
  > **修正（B-R14，2026-09-30 执行时）**：原写的「+ 分页」作废——后端 `gateway:query-logs` 无分页参数、返回全量数组，
  > 客户端分页属新行为且会重定义「全选当前页」。分页改为「下推 Rust」的后续项（见 docs/rust-rewrite/40-ui-redesign-batch2.md §五）。

---

### Task 5: 技能 / 环境 / 预设 / 诊断面板重排

**Files:** `ui/src/app/config/skills-panel.tsx`、`environment-panel.tsx`、`presets-panel.tsx`、`diagnostics-panel.tsx`、`app.css`、测试。

**要求：** 统一 `PanelHeader` + 卡片化；环境页保留 WSL 会话目录卡（最近一次 `app:get-version` 重复取值（doc 39 §七）顺手合并到一处）；诊断页保留能力矩阵/路径探测/目标表；技能页的已安装/回收站切换与冲突覆盖流程原样保留（只换视觉）。

---

### Task 6: 弹框与菜单统一 + Escape 一致性（R35）

**Files:** `ui/src/app/new-session-dialog.tsx`、`session-title-dialogs.tsx`、`session-settings-dialog.tsx`、`exit-confirm.tsx`、`session-context-menu.tsx`、`tab-context-menu.tsx`、`terminal-text-menu.tsx`、`overlay-layer.tsx`（导出提示）、`features/attention/attention-center.tsx`、`features/workspace/messages-modal.tsx`、`features/settings/settings-center.tsx`、`components/*`、`app.css`。

**要求：**
- 全部弹框套 `Dialog` 的统一外观（尺寸档 sm/md/lg 已可用）；右键菜单统一 `.context-menu` 形态与视口夹取（现有 `viewport-menu`）。
- **Escape 一致性**：退出确认框补 Escape → `choose("cancel")`；`settings-center` 与 `messages-modal` 的「内层先吃」白名单补 `.overlay`（R35）；`attention-center` 与两个右键菜单的 Escape 行为对齐（有浮层时让位）。
- 导出提示（`.export-hint`）样式收口；关注中心弹层样式对齐新设计。
- **不得丢**：新建会话的四个字段与恢复形态、重命名/复制的文案与校验、退出确认三选项与「坚持要退」放行、右键菜单全部项与缺动作即禁用的约定。

---

### Task 7: 批次一结转小项

**Files:** 见每条。

- [ ] `ui/src/app/app.tsx`：`beginResumeSession` 的「原目录丢失」分支补 `workspace.selectSession(session)`（R37，1 行 + 测试）。
- [ ] `ui/src/components/list.tsx`：`ListRow` 鼠标点击路径对插槽内可聚焦控件加 `stopPropagation` 判定（R38；与键盘守卫同理）。
- [ ] `ui/src/app/use-console.ts`：`selectProvider` 加值校验（只接受 `""` 或已知 provider id；R39）。
- [ ] `terminal-view.tsx` 补「不可得项不渲染」的直接测试；`messages-modal` 容器补分支/跳转/兜底测试。
- [ ] `Popover` 补 `aria-label` 与 scroll/resize 重定位（或明确记为不修并在注释写明理由）。
- [ ] `sidebar-nav.ts`：把剩下的类型与再导出并回调用方或改名为更贴合的名字。
- [ ] `settings-nav.test.ts` 两条恒真断言改真断言；`.tsx` 内联 `var(--…)` 纳入 token 契约测试的扫描范围（若存在用法）。
- [ ] 核对 `terminal-search` 段混用 `--ui-*`/`--term-*` 的措辞与注释一致（可留可改，改则一处 token 化）。

---

### Task 8: 批次二收尾

- [ ] 全量门禁（前端三连 + `scripts/verify.ps1` 若本批动过 Rust）。
- [ ] 明暗两套截图核对（主界面/设置中心十页/弹框/右键菜单），逐张过 batch-1 §八 的同类清单。
- [ ] 更新 `docs/rust-rewrite/39-ui-redesign-batch1.md` 的后续段落（或新增 40- 记录）：批次二范围/行为变化/遗留。
- [ ] 提交 + 合并 main。

---

## 计划自查记录

- **Spec 覆盖**：spec §10 的十个面板 → T2–T5；§15「其余弹框统一」→ T6；§14 批次二定义 → 文首；无边框（用户新增要求）→ T1；batch-1 记录 §七 的结转项 → T7；R35/R37/R38/R39 → T1/T6/T7。
- **风险**：T1 是本批唯一动 Rust 的任务（命令面计数断言 + verify.ps1 必须过）；T4/T5/T6 是大面积样式改动，以「逻辑与测试不动」为纪律，评审按「行为零变化」核对。
- **已核实的前置事实**（避免实现者重复考古）：tao 无边框缩放；CloseRequested 链路；`PanelHeader` 已存在（batch-1 T5）；`Dialog` 尺寸档可用（batch-1 T2 修复）。
