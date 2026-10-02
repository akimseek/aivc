# aivc-rs 代码审查整改计划（P0 → P1 → P2）

> 来源：2026-10-01 四维度企业级审查（目录结构/规范/命名 · 可拓展性 · 兼容性 · 性能）。
> 每条发现都带 `file:line`；本计划把它们重排为「按影响排序的执行单元」。
> 全局约束（沿用三批次）：依赖零新增（前后端）；注释中文、只解释「为什么」；样式只用 token；
> 前端门禁 `cd ui && npm run typecheck && npm test && npm run build`；动 `crates/` 加跑 `scripts/verify.ps1`；
> 截图/基线由控制器统一跑（`ui-check.ps1` 会清 `data/`；改过 dist 先 `scripts/cargo.ps1 build -p aivc-app`；探针等 React 提交用 MessageChannel tick）。

## P0 · 真实缺陷（先修）

### P0-A：能力门控接线（跨平台数据污染风险）
**证据**：`ui/src/app/overlay-layer.tsx:223` 传 `supportsCustomCwd`（JSX 布尔简写 = 恒 true）、`:346` 传 `supportsSessionSettings`（同样恒 true）；`ui/src/app/provider-capabilities.ts:40-63` 只导出 `trash/batchActions/duplicate/branch` 四个查询，**没有** `skills/export/sessionSettings/customCwd/vendorManagement/usage`。
**后果**：环境页「WSL 会话目录」卡（`ui/src/app/config/environment-panel.tsx:251`）对 claude/qoder 的 WSL 目标也显示，保存会写 Codex 的全局设置（`codex:set-wsl-codex-home`），而后端这三平台 `sessionSettings=false`（`crates/aivc-core/src/target.rs:126-183`）。`session-settings-dialog.test.tsx` 仍在测生产不可达的 false 分支。
**要求**：① 在 `provider-capabilities.ts` 按后端能力键补查询函数（至少 `supportsSessionSettings/supportsCustomCwd/supportsSkills/supportsExport`，键名与 `types.ts:24` 的 capabilities 一致）；② `overlay-layer.tsx` 两处按真实能力传值（缺能力缺键 = false 的既有约定）；③ 环境页 WSL 卡加 `supportsSessionSettings && isWsl` 门控；④ 测试：claude 的 WSL 目标不显示该卡、菜单/对话框路径由 `openSessionSettings` 的可得性决定；删掉/改掉不可达断言的测试。
**验收**：`grep -n "supportsCustomCwd\|supportsSessionSettings" ui/src` 不再出现恒 true 的传法；新测试能在「去掉门控」时变红。

### P0-B：`use-session-detail` 的真 seq 保护
**证据**：`ui/src/app/use-session-detail.ts:12-14` 注释声称"自增号解决旧响应覆盖"，实际只有 `revision`，无 seq 判定（对照实现见 `ui/src/app/use-console.ts:313,330-345`）；测试 `use-session-detail.test.tsx:125-149` 的假运行时固定延迟 ⇒ 断言恒真。
**要求**：① `load` 里加 `const seq = ++seqRef.current`，await 后比对，不一致丢弃；② 测试改成能真的失败的构造（按 offset 给不同延迟，或手动 deferred 让第 2 页晚于第 3 页返回）；③ 注释与实现一致（若保留"自增号"说法，必须真做）。
**验收**：修复前该测试红（先写红），修复后绿；连点翻页的旧响应不覆盖新结果。

### P0-C：`pathExists` 的 targetId 与 WSL 恢复流程
**证据**：`ui/src/ipc/commands.ts:802-817` 两段叠放 JSDoc 自相矛盾；`resume-session.ts:11-18` 的"判断不出来 → 按还在处理"分支因 `app.tsx:445` 的 `api.pathExists(path)` **不传 targetId** 而永不可达 ⇒ WSL 会话恢复恒弹「原目录丢失」。
**要求**：① 恢复流程把 `targetId` 传下去（`planResume` 的调用点：`app.tsx` 的 `beginResumeSession`）；② 后端若对 WSL 目标返回"不支持探测"错误，前端按既有约定当"不知道"（走"按还在"分支）——核对 `resume-session.ts` 的四条分支与注释；③ 删掉 `commands.ts` 里被取代的那段 JSDoc；④ 测试：WSL 目标下 `pathExists` 带上 targetId；不支持探测时走"按还在"。
**验收**：WSL 恢复不再无脑弹重设目录（代码路径可证）。

### P0-D：白屏风险与 ErrorBoundary
**证据**：`ui/src/app/use-theme.ts:28` 读 localStorage 无 try/catch（同文件 `:49-53` 有）；`ui/src/features/workspace/terminal-view.tsx:13-15` 懒加载 chunk 失败无边界；全仓无 ErrorBoundary（`main.tsx` 只判 `#root`）。
**要求**：① `use-theme.ts:28` 包 try/catch（与 `:49-53` 同构；catch → `system`）；② 新增 `ui/src/app/error-boundary.tsx`（类组件，`componentDidCatch` 落 `api.logPerformance` + 显示「界面出错 + 重试按钮」），套在 `<App/>` 外层与终端 `Suspense` 外层（后者文案「终端加载失败 + 重试」）；③ 测试：抛错的子组件被边界接住并显示重试（jsdom）。
**验收**：`localStorage` 抛错不再中断首帧；chunk 失败有可见出口。

### P0-E：网关页「最近请求」的分页与限流
**证据**：`ui/src/app/gateway-insights.tsx:70` 用**不带分页**的 `queryGatewayLogs`（后端缺省返回全量），`:92-109` 按 `gateway:request-recorded` 做 200ms 合并自动刷新 ⇒ 有流量时 5 次/秒 ×（3 IPC + 全量日志解析 + 整表重渲）。
**要求**：① 改用分页重载（`{page:1, pageSize:20}` 或固定最近 50 条——选一种并写理由）；② 自动刷新加**最小间隔 ≥1s**（或复用 `refresh-coalescer` 的窗口语义）；③ 刷新响应加 seq 防旧覆盖；④ 测试：自动刷新不高于 1Hz、只取首页。
**验收**：有流量时每刷新最多 1 次/秒、payload 有界。

### P0-F：统一 `refresh-coalescer`（工作台刷新饿死）
**证据**：`ui/src/app/app.tsx:337-359` 用 `clearTimeout+setTimeout(300)`（纯 trailing 去抖，连续事件下永不触发），而 `ui/src/refresh-coalescer.ts:23-29` 的注释恰好写明"排到后面会让刷新无限推迟"；`ui/src/ipc/commands.ts:788-789` 指示"见 `refresh-coalescer.ts`"，实际只有 `gateway-insights.tsx` 用它。
**要求**：① `app.tsx` 改用 `createRefreshCoalescer(300, …)`；② 300/200ms 两个窗口常量提为一处（放 `refresh-coalescer.ts` 导出常量）；③ 测试：连续事件（间隔 < 窗口）在窗口结束后**一定**触发一次刷新。
**验收**：工作台在持续流量下也会更新（代码路径可证 + 单测）。

## P1 · 一致性与扩展成本

### P1-A：命令名机械守卫
`crates/aivc-ipc/src/commands.rs:15-16` 承诺的比对脚本不存在；前端 93 处 `invoke` 只有 7 条手写断言。**要求**：新增 `scripts/check-command-names.cjs`（抽 `ui/src/ipc/commands.ts` 的命令字面量 vs `crates/aivc-ipc/src/commands.rs` 的 `COMMANDS` 表；排除事件通道常量），接进 `verify.ps1`（与 `check-probe-selectors.cjs` 同位置、失败级）；当前应零差异；给一条注入式自检（临时改一个名字必须红）。

### P1-B：平台清单单点真源
`ui/src/app/config/gateway-external-card.tsx:28-46`（`EXTERNAL_PROVIDERS` 数组无守卫 + `PROVIDER_LABELS` 已漂移：`"Claude"` vs 后端 `"Claude Code"`）、`ui/src/app/config/gateway-rule-form.tsx:17`（`PROVIDER_IDS` 无守卫）。**要求**：两处数组改从 `providers` prop 派生（`settings-center` 已把 `providers` 传下去）；本地表只留兜底并加注释；修正 `"Claude" → "Claude Code"`；测试：清单随 `providers` 变化。

### P1-C：设置面板搬迁（spec §4.2）
九个面板散落 `ui/src/app/config/`(6) + `ui/src/app/`(3) + `features/settings/panels/`(1)。**要求**：`git mv` 到 `features/settings/panels/`（含 `vendor-probe-panel`、`gateway-range*.ts(x)`、`gateway-insights.tsx` 等同族），修正全部 import 与测试 mock 路径；`app/config/` 目录删除；跑全量门禁。若发现某文件搬迁会引出新耦合，在报告里单列并停在最小可行范围。

### P1-D：`features ↔ app` 归位
`terminal-pane.tsx`/`terminal-search-bar.tsx`/`terminal-text-menu.tsx`/`branch-panel.tsx` → `features/workspace/`；`session-context-menu.tsx`/`new-session-dialog.tsx`/`session-title-dialogs.tsx`/`session-settings-dialog.tsx`/`session-row.tsx` → `features/sessions/`；`vendor-route-picker.tsx` → `features/workbench/`。**要求**：`git mv` + import/mock 修正 + 全量门禁；`app/` 只留接线、hooks、纯逻辑。

### P1-E：抽公共组件（消重复）
① `components/pager.tsx`（三处分页控件：`gateway-log-panel.tsx:262-281`、`gateway-failure-panel.tsx:175-198`、`messages-modal.tsx:170-200`）；② `components/context-menu.tsx` 外壳（三个菜单 + Escape 单点：`session/tab/terminal` 三个 menu 的文件头注释同步）；③ `Badge` 支持 `small` 或实现 `Tag`（收敛 12 处 `.tag ok|off`），删 `.data-table .tag` 特例；④ `IconButton`（6 处裸 `.icon-btn` button）。

### P1-F：死代码与 lint 伪影
① 删 `ui/src/components/toggle.tsx` + `components.css` 的 `.toggle*`（含全仓最后一处硬编码 `#fff`）+ `index.ts` 导出；② `StatusPill` 删或复用（工作台/用量页状态文字处正是其场景，二选一并说明）；③ `index.ts` 的 `Spinner` 再导出只留 `LoadingBlock`；④ 5 处 `eslint-disable`（`terminal-pane.tsx:338,450`、`terminal-search-bar.tsx:69,77`、`workbench-sidebar.tsx:101`）：**先评估**加 eslint+react-hooks 的可行性（devDependency 需联网装；装不上就删这 5 行 + 在 `ui/README` 成文"deps 正确性靠 review"），`workbench-sidebar.tsx:101` 那行本来就冗余（依赖数组已含 `loadKey`），直接删。

### P1-G：capabilities 收窄
`crates/aivc-app/capabilities/default.json:6-15` 删 6 条前端不用的插件权限与重复的 `core:event:default`（`core:default` 已含）；跑 `verify.ps1` + `ui-baseline.ps1` 复验剪贴板/对话框/通知三条链路（都走 `aivc_invoke`，不受 ACL 影响——以实跑为准）。

### P1-H：Windows 拖放提示
`ui/src/features/workspace/composer/composer-input.tsx:355-361` 的 DOM `dragover` 在 Windows 上不触发（wry `SetAllowExternalDrop(false)`）。**要求**：改听 `tauri://drag-enter/drag-over/drag-leave`（`client.ts` 加订阅；载荷含 position），用输入区 `getBoundingClientRect` 做命中判定；删掉那条错误注释；测试：drag-enter 在输入区内 → `dragging` 为真、移出 → 假。

### P1-I：会话列表渲染成本
`ui/src/features/sessions/session-card.tsx` 无 `memo`、8 个内联回调。**要求**：`SessionCard` 加 `memo` + 回调稳定化（`useCallback` 或按 id 的事件委托），在注释里写明阈值（300+ 会话才明显）；测试：父组件无关状态变化不重渲卡片（可用计数器或 React Profiler 断言不现实——用「props 不变时 memo 生效」的 render-count 测试）。

### P1-J：历史视图上限与折叠
`ui/src/app/use-console.ts:413` 累计不封顶；`features/workspace/history-view.tsx:117,160` 正文无折叠。**要求**：① 复用弹框的 4000 字折叠（抽一个共享组件或函数）；② 累计上限（例如 >400 条丢弃最早一页）并在报告说明取舍。

### P1-K：composer 渲染期拷贝
`composer-input.tsx:429-439` 每次渲染算 `composeSubmission`。**要求**：`useMemo`（依赖 text/blocks/attachments）或维护 `hasContent` 布尔；测试不变。

### P1-L（跨层，可后置）：终端输出通道
`types.ts:265` `data: number[]` + `crates/aivc-ipc/src/terminal.rs`。**要求**：仅**评估与记录**（base64 / 二进制通道的改法与收益、需要后端配合）；本计划不实施，产出评估段落。

## P2 · 规范与收尾

- **P2-A**：`engines.node`（Vite 7 需 ≥20.19/22.12）+ 根 `.nvmrc`；`sourcemap` 按环境切（生产关或 `hidden`）；`color-mix` 补回退（`app.css:2462` 前加 `background: var(--ui-warn-soft)`）；`Consolas` 两处改 `var(--ui-font-mono)`（`app.css:2824`、`terminal-pane.tsx:222`）；`tauri.conf.json` 的 `bundle.icon` 补 `icon.icns`/PNG（P8 打包前置）。
- **P2-B**：emoji → `Icon` 七处（`attention-center.tsx:96,48-50,116,150`、`composer-input.tsx:325,337`、`terminal-search-bar.tsx:171`）；`✕` 统一 `Icon name="close"`。
- **P2-C**：`errorText(problem: unknown)` 归一出（35 处替换，`use-console.ts:47-49` 的私有版并入）；面板头同文件复用（`environment-panel`/`gateway-panel`）。
- **P2-D**：`sidebar-nav.ts` 定位收口（类型挪 `types.ts` 或更名；再导出删除，能力函数只从 `provider-capabilities` 进）；`gateway-range.ts` 拆分（`gateway-format.ts`）；`presets-panel.test.tsx` 补（保存/删除/失败上报）；测试命名规则成文（`ui/README` 或 `docs`）。
- **P2-E**：注释/格式修正清单（`commands.ts:281,682,811` 叠放 JSDoc、`:836` 双空行、`app.tsx:574-575` 的"批次二再议"改成已裁定、`use-terminals.ts:18` 中英夹杂、`environment-panel.tsx:305` 行尾空行）。
- **P2-F**：文档收口：spec §4.2 按搬迁后的目录改口径；`docs/rust-rewrite/42-review-remediation.md` 记录整改范围/提交/遗留。

## 执行顺序与验收

P0（A→F）→ P1（A→L）→ P2（A→F），每任务：TDD（能红的测试）→ 门禁 → 任务审查 → 修复轮 → 复审；控制器统一跑截图/基线（P1-C/D 的搬迁后必须跑一次 `ui-baseline.ps1` 复验，探针类名守卫会替我们盯旧类名）。
