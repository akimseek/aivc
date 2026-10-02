# 41 · 界面重做 · 批次三记录（收尾：重查语义 / 探针现代化 / 分页下推 / 对比度与基线）

> 日期：2026-10-01 · 直接提交在 `main`（起点 `8893de6`，终点头 `a8aaea4`，11 个提交）
> 计划：[`../superpowers/plans/2026-09-30-aivc-rs-ui-redesign-batch3.md`](../superpowers/plans/2026-09-30-aivc-rs-ui-redesign-batch3.md)
> 前序：[`39-ui-redesign-batch1.md`](39-ui-redesign-batch1.md)、[`40-ui-redesign-batch2.md`](40-ui-redesign-batch2.md)

## 一、范围与结果

① 修 **B-R15 重查语义**（`useAsyncData` 跟随 `load` 身份重查）；② **探针现代化**（回归脚本锚点全部对齐新类名 +
新增类名守卫进 `verify.ps1` + 基线五步复跑）；③ **日志分页下推 Rust**（`gateway:query-logs` 可选 `page/pageSize` +
前端分页控件）；④ 剩余小项批（card/popover/网关三小项/令牌防连点/flake）；⑤ 明暗走查、**对比度修复**、**明暗两套基线集重建**。

门禁：前端 `typecheck / npm test`（**75 文件 · 790 用例全绿**）/ `build`；`scripts/verify.ps1` 七步全绿（含类名守卫）。
截图：走查 23 张 + 基线 10 张（`shots/baseline-light/`、`shots/baseline-dark/` 各 5 张；旧基线留档）。

## 二、提交（`main`）

| 提交 | 内容 |
| --- | --- |
| `e1fdbc1` | `useAsyncData` 跟随 `load` 身份重查（观测三页与工作台）；三个面板的 `load` 原来挂着渲染期新建值——已把区间搬进 `load` 内部现算 |
| `6a322a5` `02c7372` | 探针锚点全量对齐新类名；新增 `check-probe-selectors.cjs` 类名守卫接入 `verify.ps1`；修 `ui-surfaces`/`startup-and-detail` 的假 FAIL（徽章破坏精确匹配、恒假禁用断言）与守卫的 `.toolchain` 环境相关误报 |
| `e76740d` `7c6bebb` `da67ac2` `9ab857f` | `gateway:query-logs` 分页下推（缺省全量、旧调用形状逐字不变）+ 日志页分页控件；字段名对齐先例改 `items` |
| `f6d4a0e` `f83e1e7` | card 判 `false` 不出空动作区、popover 跟随 scroll/resize、网关页失败可辨 + 重试、平台名用后端 label、令牌轮换在途禁用、flakes 加固 |
| `8a4be26` `a8aaea4` | 暗色对比度修复（`--ui-on-accent` 改深墨 `#052e26`：2.54→5.81；`--ui-faint` 提亮 `#8896ab`：3.59→5.70）+ **tokens 对比度守卫**（双主题关键配对按 WCAG 钉死，7 条） |

## 三、行为变化（需知情）

1. **取数语义**：观测三页与工作台的区间/筛选/翻页/手动刷新**现在真的会重新取数**——顺带查明**工作台的刷新（事件 + 按钮）此前是死链**（批次一引入），本批修复后生效。
2. `gateway:query-logs` 加可选 `page/pageSize`（缺省 = 全量，旧调用返回形状逐字不变，两侧都有断言钉住）；日志页 20/页，「全选当前页」= 当前页、翻页清空勾选、整页删光退回上一页、清空回到第 1 页。
3. 卡片 `actions={false}` 不再渲染空动作区；`Popover` 跟随页面滚动/窗口缩放重定位；规则测试的状态码输入 `"0"` 视为未填（不再发出）；令牌轮换请求在途禁用；网关页 external/端口读取失败可辨且有重试。
4. **暗色两个 token 变了**：`--ui-on-accent`（暗色 → `#052e26`）与 `--ui-faint`（暗色 `#64748b` → `#8896ab`）。

## 四、裁定（C-*）

- C-R1（T1 契约）：`useAsyncData` 的调用方**必须传稳定身份的 `load`**（`useCallback` 且依赖恰好 = 查询输入）——已写进 hook 顶部注释，四个调用点逐个核查过（三个原不稳，已修）。
- C-R2：T3 允许在改动面超预期时停在「Rust 支持分页 + 前端不改」——**未触发**（三处语义在同任务内收敛）。
- C-R3：类名守卫的级别定为**失败级**（与 `verify.ps1` 的 BOM 检查同先例），白名单逐条写理由，node 缺失时 fail-closed。
- C-R4：截图/基线由控制器统一跑（沿用批次一/二做法）。
- C-R5：T2 的「折叠条判据」改 `.sidebar.hidden` + 严格变窄（原 0px 判据在新折叠机制下恒假）。
- C-R6：分页返回字段名以**先例 `items`** 为准（派单里举例的 `entries` 作废）。

## 五、承接自 40 §五的状态

| 40 的结转项 | 状态 |
| --- | --- |
| B-R15 `useAsyncData` 不自动重查 | ✅ 本批 T1 修复（并要求 `load` 稳定身份） |
| 日志无分页（下推 Rust） | ✅ 本批 T3 |
| 网关页三小项（external 不可辨 / 端口无重试 / 试规则 `"0"`） | ✅ 本批 T4 |
| 规则表平台名漂移 | ✅ 规则表已用后端 label；⚠️ **外部接入端点清单仍用本地平台表**（本批 T4 披露，见 §六） |
| `vendors-panel` 测连接 flake | ✅ 本批 T4 加固（根因：disabled 按钮点击不派发） |
| 探针口径（`gap-closeout` 3 FAIL、`ui-baseline` 01–03 步） | ✅ 本批 T2 |
| `Popover` 无 scroll/resize 重定位 | ✅ 本批 T4 |
| `card.tsx` `actions={false}` 空容器 | ✅ 本批 T4 |

## 六、遗留（未做，附去向）

- `--ui-user`（头像蓝）在暗色下白字对比度 **4.01:1**（仍 <4.5）——需改 `--ui-user` 本身（亮/暗各调一档），单独小项。
- 外部接入端点清单仍用前端本地平台表（`gateway-external-card.tsx` 的 `EXTERNAL_PROVIDERS`/`PROVIDER_LABELS`）；与规则表的后端 label 存在同屏不一致风险（I1 类）。
- 探针守卫的**成员存在性**规则抓不到「组合上下文错」（如旧 `.sidebar .rows .row`——`.row` 在别处仍存在）；该类只能靠「锚点改动时人工核对」。
- `LOGS_PAGE_SIZE_MAX=50` 是同一数字的第三份拷贝（可改引 `aivc_core::constants`）；`pageCount` 用本地 PAGE_SIZE 而非响应回显；分页控件两个面板都没有「末页」按钮（计划括号里的「首/上/下/末」记为被「与异常页同形」取代）。
- `popover` 不跟随锚自身尺寸变化（要等下次滚动/resize）；`gateway-insights` 注释半句已改准。
