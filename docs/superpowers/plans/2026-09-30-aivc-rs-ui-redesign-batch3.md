# aivc-rs 界面重做 · 批次三 实现计划（收尾：重查语义 + 探针现代化 + 分页下推 + 暗色走查）

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development。
> **Goal:** 清掉批次一/二的全部结转项，把「验证设施」修回可信（探针/基线），并让观测页的取数语义正确。
> **Spec:** `docs/superpowers/specs/2026-09-30-aivc-rs-ui-redesign-design.md`
> **输入（结转清单）:** `docs/rust-rewrite/39-ui-redesign-batch1.md` §六/§七、`docs/rust-rewrite/40-ui-redesign-batch2.md` §五。

## Global Constraints

- 依赖零新增；样式用 token；注释中文、只解释「为什么」；提交中文前缀。
- 前端门禁：`cd ui && npm run typecheck && npm test && npm run build`；**动 `crates/` 的任务**加跑 `scripts/verify.ps1`。
- 截图/探针：`scripts/ui-check.ps1` 每次会清 `data/`；改过 `ui/dist` 必须 `scripts/cargo.ps1 build -p aivc-app` 再出图；探针等 React 提交用 `MessageChannel` tick（`setTimeout` 在离屏窗口不触发）。
- 回归脚本改动后：`ui-baseline.ps1` 至少跑一遍并确认每步有 PNG。

## Task 1: 重查语义（B-R15）——`useAsyncData` 跟随 `load` 身份重查

**问题**：`ui/src/components/use-async-data.ts:76-79` 的 effect 只依赖 `[auto, reload]`（`reload` 恒稳定）⇒ 只在挂载取数一次。观测三页的区间/筛选/翻页、工作台的多处，改了输入不重查；批次二加的汇总卡让「旧区间数字与新选中区间并排」更醒目。
**修法**：effect 依赖加 `load`（`[auto, reload, load]`），并在 hook 顶部注释写明**契约**：「调用方必须传稳定身份的 `load`（`useCallback` 且依赖 = 查询输入）」。
**必做的核查**（否则会打成死循环）：`grep -rn "useAsyncData(" ui/src` 列出全部调用点，逐个确认 `load` 是 `useCallback` 且依赖完整（少一个输入 ⇒ 改它不重查；多一个非稳定值 ⇒ 死循环）；不稳的调用点就地补 `useCallback`。修完跑全量测试——`use-async-data` 若有既有测试（或调用方测试）要补「输入变化 ⇒ 重新取数」「输入不变 ⇒ 不重复取数」两条断言（可在一个代表性面板上做）。
**验收**：观测三页改区间/翻页会真的重新请求；工作台不因重渲染而重复请求。

## Task 2: 探针现代化（B-R18）

**问题**：批次一/二改了类名（`.pane-nav .chip`、「全部会话」等已移除）、弹层类名（`.overlay-*` → `.settings-*`/`.dialog-*` 已在批次二终审修过一轮，但仍有残留）。`scripts/probes/gap-closeout.js` 3 条 FAIL；`ui-baseline.ps1` 01–03 步仍是批次一口径（空库返回 `rows:0`/`NO-ROW`/`NO-BUTTON`）。
**修法**：① `grep` 全部 `scripts/probes/*.js` 与 `scripts/ui-baseline.ps1` 里的 CSS 选择器/中文文本锚点，逐个在现源码里核对存在性，改到当前类名/文案；② **加一条机械守卫**（放进 `scripts/verify.ps1` 或一个 node 脚本）：扫 `scripts/**/*.{ps1,js}` 里的 `\.([a-z][a-z0-9-]{2,})` 类名与 `ui/src/**/*.{tsx,css}` 的实际类名对照，报「脚本引用但源码不存在」的名单；③ 跑 `pwsh scripts/ui-baseline.ps1 -Name b3-baseline-check` 确认每步有 PNG 且探针无 FAIL。
**验收**：`gap-closeout` 全 PASS；`ui-baseline` 五步全出图；机械守卫进 `verify.ps1` 且当前零命中（或白名单注明）。

## Task 3: 日志分页下推 Rust（B-R14）

**问题**：`gateway:query-logs` 返回全量、无分页；日志页无分页（批次二裁定不做客户端分页）。
**修法**：Rust 侧给 `gateway:query-logs` 加可选 `offset`/`limit`（与 `gateway:get-failure-diagnostics` 同形：默认不分页、限流上限），仓储层 `cleanup_entries` 同形加 LIMIT/OFFSET（`crates/aivc-infra/src/db/repos/gateway_request_log.rs`），路由与命令表同步（命令数不变、只是参数扩展）；前端 `queryGatewayLogs` 透传并在日志页加分页控件（与异常页同形：20/页 + 首/上/下/末 + 全选当前页语义不变）。
**注意**：这是本批唯一动 Rust 命令**语义**的任务——`06-l-audit` 的 parity 结论要注明「命令名不变、参数扩展、前端旧调用（不带 offset/limit）行为不变」；`scripts/verify.ps1` 必跑。若评估后发现改动面超预期（分页语义涉及「全选当前页/删除选中/清空」三处），可停在「Rust 支持分页 + 前端不改」并在报告里说明。

## Task 4: 剩余小项批

- `ui/src/components/card.tsx`：`actions`/`title`/`description` 由 `!== undefined` 改 `!= null`（`actions={false}` 不再渲染空 `.card-actions`）；补断言。
- `ui/src/components/popover.tsx`：补 scroll/resize 重定位（或明确记为不修 + 注释写明理由——二选一，倾向补，因为右键菜单已有 `viewport-menu` 先例）。
- 网关页三小项：external 失败时界面上可辨「读取失败」（不只是 onError 提示）；端口读失败给重试入口；试规则状态码输入 `"0"` 不发出（与空串同处理）。
- 规则表平台名与标签栏后端 label 漂移：改为优先用后端 `providers` 的 label（经 props 传入）或记明接受漂移。
- 令牌轮换按钮请求在途禁用（防连点）。
- `vendors-panel.test.tsx` 的「测连接」waitFor 超时 flake：定位并加固（把它纳入 waitFor 判据或稳定假运行时）。

## Task 5: 暗色走查与基线重建 + 收尾

- 明/暗两套完整截图核对（主界面/十页/弹框/菜单/关注中心），逐张看；修掉发现的对比度/漏 token 问题（同一批内完成，一笔修复轮）。
- **基线集重建**：`shots/baseline-light/` 与 `shots/baseline-dark/`（沿用 `ui-baseline.ps1` 机制，暗色用它支持的探针切换）；旧基线留档不删。
- 记录：`docs/rust-rewrite/41-ui-redesign-batch3.md`（范围/提交/遗留/基线路径）；更新 39/40 的「已结转」状态。
- 全量门禁（前端三连 + `verify.ps1`）+ 提交。
