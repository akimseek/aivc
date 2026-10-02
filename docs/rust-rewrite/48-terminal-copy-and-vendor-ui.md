# 48 · 终端右键复制修复 · 供应商表单与列表重排

> 2026-10-02。用户实机测试（AI.exe release 版）反馈四项，本批完成 1–3；
> 第 4 项（网关统一外部接入）另立设计文档。四项均为真实缺陷/改进，
> 第 1 项经真机探针完整复现后按根因修复。

## 一、终端右键复制失效（真实缺陷，根因复现）

**现象**：选中文字后右键，菜单弹出时选中被清掉，点「复制」无反应。

**根因链**（探针 `scripts/probes/terminal-copy.js` 复现，两个变体对照）：

1. CLI（实测 Codex）开启鼠标上报（DECISET 1000/1003，实测 `"any"`）；
2. 右键 mousedown 被 xterm 的 `CoreMouseService` 作为输入上报给 PTY；
3. xterm `SelectionService` 有一条「用户输入即清选区」（`onUserInput → clearSelection`）；
4. 菜单的可用性快照 `hasSelection()` 取在 contextmenu 时——此时选区已空；
5. 点「复制」读实时选区（空）→ 静默 no-op。**全链无一个环节报错**，因此表现为「没反应」。

**探针矩阵（修复前 → 修复后）**：

| 指标 | V1 关鼠标上报 | V2 开鼠标上报（修复前） | V2（修复后） |
| --- | --- | --- | --- |
| mousedown 后选区 | 存活 | **已被清** | 存活 |
| 菜单「复制」 | 可用 | **禁用** | 可用 |
| 点复制后剪贴板 | = 选中文本 | 哨兵原样（未复制） | = 选中文本 |

**修法**（`terminal-pane.tsx`，两步都在**捕获阶段**——xterm 的监听在 `host` 内部，捕获先于它们）：

1. **快照**：右键 mousedown 时把选中文本拍下来，菜单「复制」用它，不再依赖实时选区；
2. **屏蔽上报**：`stopPropagation()` 拦住 button 2 的 mousedown/mouseup——不给 TUI 送半截点击，
   选区高亮也不再消失。效果：右键=应用菜单（不再兼报给 TUI），与关闭鼠标上报时的行为一致。

键盘路径保留：Shift+F10 等无 mousedown 的路径回退读实时选区；Ctrl+Shift+C 是 xterm 原生复制
（源码注释里「Ctrl+C 可复制」是死承诺，已改正——Ctrl+C 是发给 PTY 的中断信号）。

**回归护栏**：`terminal-pane.test.tsx` 新增 4 条（捕获阶段拦截、快照复制、无选中禁用、键盘回退）；
探针 `terminal-copy.js` 留存，跑法：`.\scripts\app-probe.ps1 -Probe probes\terminal-copy.js`。
探针拿 Terminal 实例走 React fiber 走查（`__reactFiber$` → hooks 里的 termRef），只读、不注入后门。

## 二、新增供应商表单（布局 + 必填 + 就地校验）

- **分区两列**：基本信息（模型厂商全宽 / 名称 / API 地址 / API Key / 排序）＋ 计费（可选）
  （输入·输出费率）；高级查询照旧折叠。原先是 auto-fit 网格，七个字段随宽度任意折行、无分区。
- **必填红 \***：名称、API 地址、API Key（**仅新建**；编辑时留空=沿用原密钥，不给星标）。
- **就地校验**：点「创建/保存修改」时校验，错误以红字落在字段下方 + `aria-invalid` 红框；
  文案沿用旧平台：「请输入供应商名称。」「请输入 API 请求地址。」「请输入 API Key。」。
  创建按钮**不再禁用**——点不动且无提示正是用户报的「没反应」；改哪个字段消哪条错误。
- `Field` 组件扩展 `required` / `error` / `className`（`components.css` 配套样式）。

## 三、供应商列表（余额/操作分列 + 复制）

- 「操作 / 余额」合并列拆成**余额列（左）与操作列（右）**，列头各自如实。
- 操作列新增「复制」＝**克隆为新供应商**（用户 2026-10-02 选定的语义）：预填整份配置
  （名称加「副本」、地址/费率/查询配置照搬），但 **id 留空、API Key 必须重填**
  （接口不下发密钥，这也是「复制」与「编辑」的本质区别）；`enabled=false`（与「新增」同理由：
  启用会写 CLI 配置文件）；`sort` 取当前条数避免撞号。

## 四、验证与门禁

- 前端：`tsc --noEmit` 通过；`vitest run` **88 文件 / 883 项全绿**（含新增回归）；
  `npm run build` 通过；`scripts/verify.ps1 -Quick` 八段通过。
- 真机：修复前探针 REPRO（V2 清选区、复制禁用、剪贴板未动）→ 修复后 OK（V2 三阶段选区存活、
  复制可用、剪贴板命中）。截图存 `shots/probe-terminal-copy.png`。
- 后端（Rust）无代码改动；本批全部在前端。

## 五、未纳入本批

- 第 4 项「网关统一外部接入」（外部路径 `/<平台>` 直达候选池 + 本地/外部概念合一）
  → 设计文档：`docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`（待用户审阅）。
