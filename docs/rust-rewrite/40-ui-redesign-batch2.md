# 40 · 界面重做 · 批次二记录（无边框窗口 + 设置中心面板 + 弹框统一）

> 日期：2026-09-30 · **直接提交在 `main`**（批次二无独立分支；收尾 HEAD 见下）
> 计划：[`../superpowers/plans/2026-09-30-aivc-rs-ui-redesign-batch2.md`](../superpowers/plans/2026-09-30-aivc-rs-ui-redesign-batch2.md)
> 上一批：[`39-ui-redesign-batch1.md`](39-ui-redesign-batch1.md)

## 一、范围与结果

① **无边框窗口 + 应用内窗口控制**；② 设置中心十个面板按 demo 重排（面板头统一、供应商六列表格、
网关三卡拆分、观测三页汇总卡与收口、扩展四页卡片化）；③ 全部弹框与右键菜单统一 + Escape 让位判定收口；
④ 批次一结转项（R35/R37/R38/R39 等）与终审残留。

质量门禁：前端 `typecheck / npm test`（**71 文件 · 759 用例全绿**）/ `build`；Rust 侧 `scripts/verify.ps1`
（T1 动过 `crates/`，全绿）。**截图核对**：16 张（`shots/b2-*.png`），10 ✅ / 2 ⚠️（两条已修 + 复截确认）；
终审修复波又补 3 张（`b2-fix*`）与 5 张基线图（`shots/final-fix-baseline/`），口径见 §四。

## 二、提交（`main`）

| 提交 | 内容 |
| --- | --- |
| `79357b7` `d599aba` `906769c` | 无边框窗口：`app:window-minimize/toggle-maximize/start-drag/close` 四条平台命令（注册表 93→97、App 域 3→7、平台清单 9→13）+ 标题栏三按钮 + 拖拽改命令驱动（双击最大化与 Tauri `drag.js` 同构）；顺带修 `ui-baseline.ps1` 缺失 BOM（让 `verify.ps1` 可绿） |
| `de0a45b` `513beb2` | 十面板统一 `PanelHeader` + 供应商面板按 demo 重排（四按钮 / 厂商 chips / 六列表格 / 行内余额 / 健康列只用真字段） |
| `bcb06d2` | 网关面板三卡拆分（端口与熔断 / 外部网关与端点清单 / 故障规则 + 在线测试），端点以后端 `routes.rs` 真实形制生成 |
| `94c08f6` | 观测三页收口 + 用量页三张汇总卡（缺数据显示 `-`） |
| `f686a77` | 扩展四页卡片化；环境页版本来源合并（全仓只剩一处 `getAppVersion`） |
| `8c99c07` | 弹框与菜单统一、`overlay-stack.ts` 收口 Escape 让位、`HintBar` 收口；顺带修 `vendors-panel` 时序 flake |
| `37db3d9` `3e6f486` | 退出确认框置顶（DOM 末尾）、**Dialog 实例栈让位**（叠放时一次 Escape 只关上层）、恢复会话激活标签（R37）、插槽点击让位（R38）、`selectProvider` 值校验（R39）、`Popover` 让位与 `aria-label`、删 `OutcomeChips` 死代码 |
| `45ae106` `385566d` | `SessionSettingsDialog` 置顶、试规则空状态码语义、网关页 external 失败降级、规则表显示名、复制按钮 aria、`SwitchPill` 在途禁用、用量成功率缺数、日志防误读注释等 |
| `bc1bc78` `b411bb2` | 测试与注释清理（设置导航真断言、终端信息条直测、令牌契约扩到内联、六处注释改准） |
| `900804b` | 截图发现两条修复（禁用主按钮去绿底、清字面 `**`）+ chips 不折行 + 筛选框留宽 |

> 2026-10-02：`bcb06d2` 一行的端点形制已更新——外部端点从占位符
> `/gateway/<平台>/<routeId>`（从未可用）改为 `/<平台>` 短路径；
> 见 `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md` 与 49 号。

## 三、行为变化（需知情）

1. **窗口无系统边框**：拖拽标题栏空白移动、双击最大化/还原、三个窗口按钮、边缘可缩放（tao 自带 `WM_NCHITTEST`），关闭走既有退出确认链路。用户已实机确认。
2. **删除供应商新增确认**（`window.confirm`，文案逐字取自旧平台：「此操作不会还原已写入的 CLI 配置」类）。
3. **编辑供应商时点「＋ 新增供应商」会先确认**（避免静默丢弃草稿）。
4. **Escape 让位语义**：叠放浮层只收最上面一层（Dialog 之间用实例栈、跨类用 `overlay-stack` 的类名单）；退出确认框按 Escape = 取消。
5. 环境页版本号改为经 `SettingsCenter` 的 `version` prop 下传（少一次重复 IPC；失败兜底文案从 `-` 变为「无法读取」）。

## 四、截图核对（`shots/b2-*.png`）

十页（明亮 10 + 暗色 2 + 补充 4）逐张过：面板头/卡片/表格/空态/暗色翻转均正常。
截图报告（`.superpowers/sdd/2026-09-30-aivc-rs-ui-redesign-batch2/task-8-screenshots.md`）
§2 列的 **4 条轻微项全部已修**（问题 1 禁用主按钮绿底灰字、问题 2 字面 `**`、
问题 3 chips 折行大椭圆、问题 4 异常页占位被裁），修后**复截三次**：

| 复截 | 图 | 结论 |
| --- | --- | --- |
| 收尾 | `shots/b2-fix-presets.png` | 禁用主按钮不再绿底灰字（中性底 `--ui-bg-badge` + `--ui-faint`） |
| 终审 | `shots/b2-fix2-vendor-form.png` | 供应商表单：字面 `**` 计数 0、改为 `<strong>`；禁用「创建」中性灰（实测底 `#e2e8f0` = 226/232/240） |
| 终审 | `shots/b2-fix2-diagnostics.png` | 「按供应商筛选」占位完整（需要 144px / 可用 179px，`clipped=false`） |

终审复截时又抓出问题 2 的**副作用**：`.row-meta` 是 `display: flex`，`<strong>` 会自成一项，
那句说明被拆成三列（对照修复前的 `b2-extra-vendor-editor.png` 是一整段）。已改成整句包一层
`<span>`，复截为整段两行。

**未核验项**（本轮没有出图，只有单元测试兜底）：

- **关键弹框与右键菜单**（供应商编辑内的确认框、消息与分支弹框、**退出确认**、三个右键菜单）：
  截图报告 §5 明确记「本轮未出图」；终审用 `ui-baseline.ps1` 的 04/05 步把设置中心与退出确认
  补成了 PNG（`shots/final-fix-baseline/04-配置.png`、`05-退出确认.png`），其余仍以 T6/T7a
  的单元测试为准。
- 空库冷启动无数据，只到空态：供应商表格行、用量维度表行、环境工具链/WSL 卡、诊断三卡的内容态。

## 五、遗留（批次三或单开）

- **截图基线只覆盖到 04/05 步**：`ui-baseline.ps1` 的 01/02/03 步仍是批次一之前的口径
  （`.rows .row` 等），空运行根下分别返回 `rows:0` / `NO-ROW` / `NO-BUTTON`——03 步还需要先选
  平台（启动不预选是 2026-09-29 那批 `6ed3f9b` 改的，早于本批）。终审只把 04/05 步改指新类名
  （`fix(scripts)`），这三步留给
  「回归探针现代化」单开一轮（`scripts/probes/` 里另有一批旧标签与旧选择器，清单见
  `.superpowers/sdd/2026-09-30-aivc-rs-ui-redesign-batch2/final-fix-report.md`）。
- 截图报告 §2「轻微 / 观察」四条**没有动作**（技能/环境空态只有一句 `message` 且与诊断页口径不一、
  诊断页无目标时不渲染能力矩阵、端点 URL 用省略号截断、模态遮罩覆盖标题栏）：都是记录性观察、
  非缺陷，要改属批次三。
- **`useAsyncData` 只在挂载时取数**（B-R15）：区间/筛选/翻页不自动重查；汇总卡放大了「旧区间数字与新选中区间并排」的误读风险。修法二选一（hook 跟 `load` 身份重查 / 变化时清空 data 或打「已过期」）。
- **日志页无分页**（B-R14）：后端 `gateway:query-logs` 返回全量、无分页参数 ⇒ 计划里那行已按本批修正（见 §六）。优选把分页下推 Rust，与异常页同形。
- 网关页：external 读失败时退化值在界面上不可区分（仅靠 onError 提示）；端口读失败无重试入口；试规则手输 `"0"` 仍会发出。
- 规则表平台名取前端 `PROVIDER_LABELS`（与标签栏的后端 label 可能漂移）；轮换令牌按钮可连点。
- `vendors-panel.test.tsx` 的「测连接」waitFor 超时在并发下偶发（第二次观察到的 flake；与改动无关，批次三收口）。
- 拖拽区对 `detail >= 3` 与非主键一律忽略（与 Tauri 模式一致，有意行为）；最大化按钮恒 `square`（不镜像还原态，既定设计）。

## 六、计划修正（B-R14）

计划 `2026-09-30-aivc-rs-ui-redesign-batch2.md` 的 Task 4 曾写「日志页…+分页」——**该行作废**：
`gateway:query-logs` 无分页参数、返回全量数组，客户端分页属新行为且会重定义「全选当前页」，
与「行为零变化」冲突。分页需求改为「下推 Rust」的后续项（见 §五）。
