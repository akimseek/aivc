# 43 · 旧平台对齐终态对照复核（2026-10-02）

> 目的：把 [`36-comprehensive-gap-list.md`](36-comprehensive-gap-list.md)（44 条）与
> [`38-legacy-parity-audit.md`](38-legacy-parity-audit.md)（真缺功能 12 项 + 操作反馈约 70 条）
> 两份**审计快照**，逐条对照**当前 HEAD**（`aivc-rs` `2eccb4b`：UI 重做批次一~三 + 代码审查整改 P0–P2 之后）
> 给出终态。
>
> 方法：**每条用代码取证**（grep/读码，写明文件或命令名），不采信旧文档的状态列——
> 这正是 36 §12.1 记下的教训（「信任文档而不是真实契约」）。截图佐证取 `shots/b-final` 与
> `shots/final-walk`（2026-10-02 验收）。

## 一、总账

| 来源 | 条数 | ✅ 终态满足 | 🚫 有意移除 | ⚠ 仍缺 |
| --- | ---: | ---: | ---: | ---: |
| 36 全面差距清单 | 44 | **40** | 3 | 1 |
| 38 真缺功能 A1–A12 | 12 | **10** | 2 | 0 |
| 38 C 类（操作反馈 ~70 条） | ~70 | ✅ 已收口 | — | — |

**结论一句话**：两份审计列出的「真缺功能」面**已全部收口**；36 的 44 条里只剩
**1 条按决策保留的缺口**（命令清单/目录表，需 provider 层接口）；「有意移除」的 3+2 条
均为明确决策。复核中发现的 **1 处新缺口**（技能移入回收站）已**当日修复**（见 §四.1）。

---

## 二、36 清单逐条终态

### 布局骨架（1.x）

| # | 终态 | 判据（当前代码） |
| --- | --- | --- |
| 1.1 侧栏+工作区并排 | ✅ | 截图 00/12；`.sidebar` + `.workarea` 结构 |
| 1.2 侧栏折叠 | ✅ | `sessions-sidebar.tsx` 的 `IconButton label="折叠侧栏"`（整改 P1-E④ 收敛后仍在）；截图可见 |
| 1.3 一级导航 工作台/会话 | ✅ | `shell/activity-bar.tsx:19-38`（`"sessions" \| "workbench"`） |
| 1.4 二级视图 当前/收藏/回收站 | ✅ | 侧栏 chips「全部 / ★收藏 / 回收站」；`SidebarView = "active"\|"favorites"\|"trash"`（`types.ts`） |
| 1.5 搜索入口+范围 | ✅ | `sessions-sidebar` 搜索行；范围见 4.6 |
| 1.6 侧栏工作台视图 | ✅ | `features/workbench/workbench-sidebar.tsx`（四卡）；截图可切 |
| 1.7 顶部菜单栏 | 🚫 | **有意移除**（批次二，决策 D1）——功能入口分流到标题栏/状态栏/设置中心 |

### 终端区（2.x）

| # | 终态 | 判据 |
| --- | --- | --- |
| 2.1 xterm 本体+标签 | ✅ | `terminal-pane.tsx` / `tab-bar.tsx` |
| 2.2 自管输入 composer | ✅ | `composer/composer-input.tsx`（含图片面：`composer-image-preview.tsx`；整改 P1-H 拖放、P1-K memo） |
| 2.3 终端内搜索 | ✅ | `terminal-search-bar.tsx`（P2-B 关闭钮换 Icon） |
| 2.4 标签右键菜单 | ✅ | `features/workspace/tab-context-menu.tsx`（P1-E② 抽外壳后） |
| 2.5 终端文本右键 | ✅ | `terminal-text-menu.tsx` |
| 2.6 系统终端最小化/关闭 | 🚫 | **有意移除**（用户要求；`use-terminals.ts:28` 注释留痕） |
| 2.7 标签退出码 | ✅ | `use-workspace-tabs.ts:286`（`（退出码 N）` 后缀） |

### 会话详情/操作（3.x/4.x）

| # | 终态 | 判据 |
| --- | --- | --- |
| 3.1 详情弹层+重新加载 | ✅ | `messages-modal.tsx:208`「重新加载」（换形为「消息与分支」，功能含分支面板+分页） |
| 3.2 分支面板 | ✅ | `branch-panel.tsx` |
| 3.3 分页控件 | ✅ | `components/pager.tsx` + 每页数量选择器（整改 P1-E①） |
| 3.4 会话行信息 | ✅ | `session-card.tsx`（P1-I memo 化） |
| 4.1 右键菜单补齐 | ✅ | `session-context-menu.tsx:97-100`（打开所在目录/重命名/复制/收藏/复制编号/复制路径/导出三种格式/移入回收站或彻底删除/恢复） |
| 4.2 重命名 | ✅ | `session-title-dialogs.tsx` |
| 4.3 复制问名 | ✅ | 同上 |
| 4.4 批量条 | ✅ | `.batchbar`（全选/删除/恢复/彻底删除） |
| 4.5 收藏夹 | ✅ | chips「★收藏」+ `quickAccess` |
| 4.6 全局检索按平台筛选 | ✅ | `sessions-sidebar` 的 `searchProviderId`/`searchTargetId` 两个下拉 |

### 供应商与网关（5.x）

| # | 终态 | 判据 |
| --- | --- | --- |
| 5.1 供应商管理 | ✅ | `vendors-panel.tsx`（表单含模型厂商、费率、高级 JSON；列表跨平台） |
| 5.2/5.3/5.4 测连接/余额/模型/读配置 | ✅ | `vendors-panel.tsx:572-589`（查余额/测连接（列模型）/读本地配置）+ `vendor-probe-panel.tsx` |
| 5.5 诊断页四表 | ⚠ | 能力矩阵✅ + 路径存在性✅；**命令清单与历史/回收站目录未做**（刻意，理由见 36 §12.5：目录布局是 Rust 侧约定、旧命令清单与后端不一致）；`diagnostics-panel.tsx:18-27` 写明 |
| 5.6 平台 Base URL | ✅ | `gateway-port-card.tsx`（含复制与「端口是否固定」提示）；截图 01/11 |
| 5.7 日志查询与清理 | ✅ | `gateway-log-panel.tsx`（分页 Pager + 清理 `delete-log-entries`，`:173` 报条数） |
| 5.8 异常诊断 | ✅ | `gateway-failure-panel.tsx` |
| 5.9 用量报表 | ✅ | `gateway-usage-panel.tsx`（含缓存输入等明细） |
| 5.10 供应商健康度 | ✅ | `gateway-insights.tsx` |

### 环境/反馈（6.x/7.x）

| # | 终态 | 判据 |
| --- | --- | --- |
| 6.1 CLI 安装 | ✅ | `environment-panel.tsx`（Node 版本选择「24 LTS 推荐」，截图 06） |
| 6.2 技能管理 | ✅* | `skills-panel.tsx`（已安装/回收站、启停、导入、还原、彻底删除）——*但缺「移入回收站」，见 §四 |
| 6.3 工作区预设 | ✅ | `presets-panel.tsx`（P2-D 补 6 条测试） |
| 7.1 状态栏 | ✅ | `status-bar.tsx`（就绪/编号/更新/目录/目标/输入模式/网关） |
| 7.2 自动消失 toast | ✅ | `components/use-async-data.ts` 的 `useToast`（计时器清理） |
| 7.3 退出确认 | ✅ | `exit-confirm.tsx` |
| 7.4 关于 | ✅ | `about-panel.tsx` |
| 7.5 提醒中心 | ✅ | `attention-center.tsx`（P2-B 全 Icon 化） |
| 7.6 命令面板 | 🚫 | **有意移除**（用户最初说明「Rust 版移除了命令面板」） |

---

## 三、38 审计逐条终态

| # | 事项 | 终态 | 判据 |
| --- | --- | --- | --- |
| A1 | 规则自定义返回 | ✅ | `gateway-rule-form.tsx:156-177` 输入框 + `gateway-rule-model.ts:94-113` 四条校验（32KB 按字节） |
| A2 | 试规则 | ✅ | `gateway-rule-form.tsx` 的 `会命中：状态码匹配/错误文本包含/不会命中`（同源 `match_rule`） |
| A3 | 供应商费率 | ✅ | `vendors-panel.tsx:68-181`（`$0.15 / $0.60` 展示与回填） |
| A4 | 启用写 CLI 配置 | ✅ | `vendors-panel.tsx:329` 调 `api.enableVendor` → `vendor:enable`（写文件+备份再翻位，按用户决定落地） |
| A5 | 高级 JSON | ✅ | `vendors-panel.tsx:82-83`（`balanceQuery`/`modelQuery`） |
| A6 | 网关三阈值 | ✅ | `gateway-port-card.tsx` 的「失败转移与熔断阈值参数」卡（截图 01/11）；`gateway:set-failure-threshold` 等已在命令面 |
| A7 | 系统终端类型选择 | 🚫 | **随系统终端整体移除而失去对象**（`openSystemTerminal` 已不在 ui/src） |
| A8 | usage 明细 | ✅ | `gateway-usage-panel.tsx:125-126`（缓存输入等分项） |
| A9 | 自定义日期区间 | ✅ | `gateway-range-picker.tsx`（开始/结束日期 + `customBounds` 两条校验） |
| A10 | 日志模式 | 🚫 | **有意不做**：`gateway.log` 本工程不存在（`paths.rs:21`/`logging/mod.rs:10` 已改为说明），访问器已删 |
| A11 | 输入模式切换 | ✅ | `status-bar.tsx:144-154`（「自管输入 / 终端原生输入」胶囊） |
| A12 | 网关引导文案 | ✅ | `gateway-port-card.tsx:223-227`（关着→开、开着未监听→重启的四态指引） |

**C 类（操作反馈）**：✅ 已收口为单通道（`useConsole({notify})` + 面板 `onNotify` → `useToast`），
覆盖会话/供应商/技能/环境/网关各类动作并报对象名与条数；探针与测试见
`action-feedback.test.tsx` 与历史 `action-feedback.js` 9/9。UI 重做后动作换形（如详情→消息与分支），
反馈通道未变。

---

## 四、复核中新发现（此前两份文档都没有的）

1. **✅ 已补（2026-10-02 当日，`aivc-rs` `62d5245`）：技能「移入回收站」入口**。
   - 复核时发现：后端有 `skill:delete`（语义即「移进回收站」、返回 `movedTo`；命令面 97 条之一），
     但前端从未接线（`verify.ps1` 的 IPC 命令名守卫一直以提示级列着它）。技能行只有
     启用/禁用；回收站视图的「还原/彻底删除」只能作用于**别的途径**进了回收站的条目——
     即**用户没有入口把已安装技能放进回收站**。旧平台有此动作（38 号文档 C 类列过其反馈文案）。
   - 补法：`commands.ts` 的 `api.deleteSkill`（按真实返回形状 `{ movedTo }` 声明）+ 已安装行内
     「移入回收站」按钮（danger 档，与会话同类操作一致）+ 消息带 Skill 名走既有 `act`/notify
     通道 + 删后重载；不可逆的「彻底删除」仍只在回收站视图（两个动作、两个位置）。
   - 判据：`action-feedback.test.tsx` 新增一条（**RED 实测**：按钮不存在时该用例超时红→实现后绿）；
     全量 **862 用例** / `verify.ps1` 全绿；守卫「后端有、前端无」**4 → 3 条**；
     截图 `shots/skills-trash.png`（每行可见「移入回收站」）。
2. **📝 其余 3 条提示级命令为「被替代」而非缺口**：`codex:get-session` /
   `codex:get-session-summary`（详情走 `codex:get-session-messages-page` + 列表数据；
   导出走 `codex:export-session`）、`gateway:get-recent-failures`（工作台走
   `gateway:get-failure-diagnostics`）——后端保留，前端无调用点。

---

## 五、有意的移除/差异（汇总，需知情）

| 项 | 决策 | 留痕 |
| --- | --- | --- |
| 菜单栏 | 批次二移除（D1） | 批次二记录；顶栏动作分流 |
| 命令面板 | 用户最初说明移除 | 设计 spec §3 |
| 系统终端（含最小化、类型选择 A7） | 用户要求移除 | `use-terminals.ts:28`、`app.css:2827` 注释 |
| 「另存到…」自选导出目录 | 审计 §15.3 移除（旧平台没有；属菜单栏文件/导出） | 38 §15.3 |
| 日志模式（A10） | 有意不做（不存在第二份文本日志） | 38 §12.3；`paths.rs` 文档 |
| 诊断页「命令清单」 | 有意不做（旧清单与后端不一致） | 36 §12.5 |

---

## 六、仍然开放的其他线（非「对齐」缺口，但影响交付）

- 整改遗留（详见 [`42-review-remediation.md`](42-review-remediation.md) §五）：历史窗口头页重叠、
  `loadEarlierMessages` 无会话切换判别（两处**先于整改存在**）、终端输出通道仅评估（P1-L）。
- **P8 打包**：`bundle.active=false`；图标等前置已就位（`42` §五.5）。
- 本复核的局限：与 38 号文档相同——以「旧平台有、新平台没有」的**功能面**为主，
  不比对样式类名/快捷键绑定/窗口行为；「至少」口径（旧平台 main/preload 未扫）。

## 七、建议

1. ~~技能「移入回收站」小修~~ —— **已补**（§四.1，`62d5245`）；38 号 C 类的动作面自此完整。
2. 其余开放线按 [`42`](42-review-remediation.md) §五的优先级表推进；P8 打包可独立立项。
