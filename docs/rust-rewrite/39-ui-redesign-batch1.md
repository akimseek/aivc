# 39 · 界面重做 · 批次一记录（应用骨架与主界面）

> 日期：2026-09-30 · **已本地合并回 `main`**（fast-forward，`main = b8c4d31`；特性分支 `refactor/modern-ui` 已删除）。最终树门禁：typecheck 干净 / `npm test` 61 文件 639 用例 / build 干净。
> 设计规格：[`../superpowers/specs/2026-09-30-aivc-rs-ui-redesign-design.md`](../superpowers/specs/2026-09-30-aivc-rs-ui-redesign-design.md)
> 实现计划：[`../superpowers/plans/2026-09-30-aivc-rs-ui-redesign-batch1.md`](../superpowers/plans/2026-09-30-aivc-rs-ui-redesign-batch1.md)

## 一、范围与结果

按 `docs/ui-redesign-demo.html`（第九版）重做**应用骨架与主界面**：双主题设计系统、
TitleBar 40px / ActivityBar 50px / 两侧栏 / StatusBar 28px 新壳、统一会话标签、
标签栏 + 终端/历史画布 + 输入区、消息与分支弹框、设置中心外壳（现有九面板挂入 + 关于页）、
菜单栏移除与入口迁移。**12 个任务 + 1 个收口批次，全部过任务审查**（其中 7 个任务各经
1 轮修复循环）。

质量门禁（前端）：`npm run typecheck` / `npm test`（**59 文件 · 618 用例全绿**）/
`npm run build` 全绿。**界面截图核对未执行**（用户指示：全部完成后统一启动应用核对）。

## 二、提交（aivc-rs 仓库，分支 `refactor/modern-ui`）

| 提交 | 内容 |
| --- | --- |
| `dc76877` | 双主题设计令牌与主题机制（token 双套 + `theme.ts` + `useTheme` + index.html 首帧引导） |
| `75eefe5` `91c50b1` | 设计系统基础组件（Icon/Badge/StatusDot/ProgressBar/Toggle/Popover）+ Dialog 尺寸档级联修复 |
| `39b3724` | 统一标签模型纯逻辑（`openSessionTab`/`attachTerminal`/`closeTab`/`tabSuffix` + `resolveStageContent`） |
| `cfc3b2b` `69c0ebe` | 标签 hook 接线（一个会话一张标签）+ 并发写入与批量收标签修复 |
| `8c9423f` | 设置中心外壳（分组导航 + 关于面板）+ **修 B1**（`shell:open-session-folder`） |
| `4dfd51c` `76b37f3` | 应用新壳 + 移除菜单栏 + 输入模式上收；主区高度链修复 + R20 竞态 |
| `71ed6fe` `3450979` | 会话侧栏重做（卡片/chips/收藏语义/网关胶囊/折叠新形态） |
| `fa1ab84` `28e9aca` | 工作台侧栏重做（五卡）+ 类名撞车与倒计时修复 |
| `a6d24cc` `8253f6c` | 现代流体标签栏与画布；查找信号与死快捷键修复 |
| `4b7207d` | 输入区重做 + **模型选择补齐**（旧平台有、Rust 版缺）+ composer 换 token |
| `8d151d4` | 消息与分支弹框重构 |
| `a2b7a88` `7a5f29f` `ec6dc0c` | 收口：分支会话错配、血缘条根会话、a11y 四项、结转小修、死代码清理、R32 拆分与 stale 响应修复 |

## 三、行为变化（需要知情）

1. **点会话行 = 打开/激活该会话的标签（未运行显示历史）**；恢复终端改为显式动作
   （历史视图头部「继续会话」）。理由：单击就拉起 CLI 进程既慢又意外。
   > **2026-10-02 用户裁定修正**：改为「**单击会话行 = 直接进入该会话的终端**」
   > （在跑切过去、没跑起 `--resume`；连点/双击由在途守卫收敛）。详情走
   > 「消息与分支」——单击先停在历史与预期相反。见
   > `docs/rust-rewrite/45-session-click-enter.md`；spec §9.1 行为表已同步。
2. **一个会话一张标签**（`sessionId` 为 key；全新会话拿到编号前以 `terminalId` 为 key）。
   终端挂到已有标签上、被替换的旧终端会被回收。**不做 re-key**：新建会话的终端标签在
   拿到会话编号后不改 key，因此「终端在跑 + 从列表点同一条会话」会出现第二张（历史）标签
   ——与旧平台同形，已知并接受。
3. **收藏夹语义保持旧口径**：chips「⭐ 收藏」进入的是「收藏 + 最近打开」两组视图
   （跨目标、cache-only），chip 计数只数收藏。
4. **移除「搜索消息正文」开关**：后端搜索第二阶段命中即返回全量解析结果，命中词必然出现在
   `preview` 里，前端按 haystack 重算必然也命中——该开关恒等于不过滤，故删除（不保留恒真控件）。
5. **移除 `＋` 的 `(Ctrl+T)` 文案**：批次一不实现全局快捷键（终端焦点下的按键归属需单独设计）。
6. 直连终端不再显示「动态/锁定」开关（无网关绑定时本就无意义），改显「直连（用自己的密钥）」。

## 四、结构（aivc-rs/ui/src）

```
components/       设计系统（+ icon/badge/status-dot/progress/toggle/popover）
shell/            app-shell / title-bar / activity-bar / status-bar
features/
  sessions/       新会话侧栏（卡片/筛选 chips/搜索/批量/摘要/WSL 表单/折叠）
  workbench/      工作台侧栏（头部 + 五卡 + workbench-cards 纯逻辑）
  workspace/      标签栏/画布三视图/消息与分支弹框/composer 四件
  settings/       设置中心外壳 + settings-nav + PanelHeader + 关于面板
  attention/      关注中心（从 app/ 迁入）
app/              接线与状态（app.tsx、use-*、纯逻辑模块）
```

新 token：`--ui-user`（头像蓝，demo 色）、`--ui-scrim`、`--ui-on-accent`、`--ui-brand-gradient`；
`--term-*` 七色供终端与 xterm（`terminal-theme.ts` 与 `tokens.css` 有测试钉住一致）。

## 五、顺带修掉的真实缺陷（原计划外，均为「测试全绿而功能失效」型）

| 缺陷 | 后果 |
| --- | --- |
| B1：`openSessionFolder` 调 `codex:open-session-folder`（后端注册为 `shell:…`） | 「打开所在目录」从未生效 |
| `Dialog size="lg"` 被 `app.css` 基档按导入顺序反超（同权重、后写胜） | 大尺寸弹框静默失效 |
| 新壳 `.workarea` 缺 `flex: 1` | 主区高度链断裂（终端不可见/历史不可滚动） |
| 工作台侧栏类名 `workbench` 与既有全局规则撞车 | 灰标题、说明文字溢出被裁、12px 缝 |
| `attachTerminalTab` 渲染期快照写入 | 并发挂载丢标签并泄漏活终端 |
| 干净退出循环关闭只生效最后一次 | 残留死标签（不自愈） |
| `findSignal` 未跳过首次挂载 | 新终端自弹查找栏并抢焦点 |
| 恢复会话输入框初值竞态（`noteOutput` 扑空） | 输入框可能一直不出现 |
| 恢复会话的 `initialComposerVisible` 传了不存在的字段 | 恢复会话本该隐藏输入框却一直可见 |

## 六、批次二 / 批次三待办（已入台账）

- **批次二**：设置中心 10 面板按 demo 重排（供应商表格列、网关三卡、用量/异常/日志/技能/环境/预设/诊断/关于）；
  其余弹框统一新样式（新建会话/重命名/复制/会话设置/退出确认/右键菜单/导出提示/关注中心）；
  「历史视图消息源跟随活动标签」的取舍（含「继续会话」判据、消息与分支按钮的计数同源）；
  `messages-modal` 容器级测试；`aria-controls`/tabpanel 全量关联。
- **批次三**：暗色逐屏走查与修正；明暗基线集重建（旧基线留档）；清理残余注释口径
  （`app.tsx:781-785`、`history-view.tsx:47-50` 等）；`docs/rust-rewrite` 收尾记录。
- **待用户确认后执行（本批次末尾一步）**：启动应用 + 截图核对（明/暗各一套），
  重点看：主区高度链、侧栏折叠留位、终端区在明亮主题下的配色、双主题对比度、
  卡名与对齐、焦点环。

## 八、截图核对结果（2026-09-30，用户确认后执行）

截图落在 `aivc-rs/shots/b1-*.png`（`shots/` 已被 gitignore，属过程证据）。

| 图 | 结论 |
| --- | --- |
| `b1-light-main.png` / `b1-dark-main.png` | 明亮/暗黑两套主界面与 demo 结构一致；暗色整体配色正确，胶囊/徽章/空态可读 |
| `b1-settings.png` | 设置中心两栏就位，导航 **10/10**（4 组），供应商面板正确挂入（面板重排留批次二） |
| `b1-workbench.png` | 工作台侧栏：头部 + 实时监控徽章 + 刷新 + 卡片；**自带滚动条**（R24 生效） |
| `b1-min-window.png` | 980×640：无裁切；状态栏按设计降级（丢更新/目标） |
| `b1-light-terminal-css.png` + 计算样式探针 | **R7 收口**：`.terminal-view` bg=`#f8fafc`、`.composer` bg=`#f8fafc` 底、`.composer-text` 文字=`#0f172a`、`.composer-card` 边=`#cbd5e1` —— 明亮主题下终端/输入区不再有硬编码深色 |
| 探针（主区高度链） | `vh 820 / shell 820 / body 752 / main 752 / **workarea 752** / workspace 752 / statusbar 28 / tabbar 43` —— 高度链完整（那条 Critical 修复实证） |
| `b1-window-frame.png`（整窗） | **确认存在双标题栏**：系统标题栏「AI 可视化控制台」+ 应用内 40px 标题栏。批次三决定：改无边框（需先设计窗口控制按钮）或保留（与会话内标题文案统一为「AI Visual Console」二选一） |

未通过截图覆盖、留待日后：真实会话下的终端/历史画布与消息弹框、原生对话框（导出/技能导入）、
焦点环在真实键盘路径下的观感。这些小项在首次真实使用中顺带核对即可。

> 以下来自执行台账（已随 SDD 工作区删除；此处为去重后的清单）。
> 每条都带定位线索；「R38/R39 等」为终审残留披露项。

**批次二（设置中心面板与其余弹框）**
- 十个面板按 demo 重排（供应商表格列/工具条、网关三卡 + 端点清单以 `routes.rs` 实际路径为准、
  用量/异常/日志/技能/环境/预设/诊断/关于的面板头与卡片化）；其余弹框统一新样式
  （新建会话/重命名/复制/会话设置/退出确认/右键菜单×3/导出提示/关注中心）。
- `beginResumeSession` 的「原目录丢失」对话框分支只走 `console.openAt`、不调 `selectSession`
  ⇒ 弹完框标签不激活（1 行修法）。
- 「消息与分支」按钮：计数取选中口径、打开对象取标签口径（瞬态不一致，顺手统一）。
- 历史视图消息源是否随活动标签（含「继续会话」判据）——需要一次设计裁定。
- Escape 白名单统一：`settings-center.tsx` 缺 `.overlay`；退出确认框自身无 Escape 绑定
  （补 Escape → cancel）；`attention-center`/两个右键菜单同类不一致。
- `ListRow` 鼠标点击对插槽内可聚焦控件仍冒泡（今天零调用方，放勾选框时一并修）。
- `use-console.selectProvider` 无值校验（防御只在侧栏 onChange）。
- 清理残余注释口径：`app.tsx` 的 `onBranch` 理由注释与 `history-view.tsx:47-50` 仍按旧假设写。
- `Popover`：`role="dialog"` 缺 `aria-label`；只在 `[open, anchorRef]` 变化时重定位（无 scroll/resize）。
- `terminal-view.tsx`（含「不可得项不渲染」）与 xterm 主题更新缺直接测试。
- `sidebar-nav.ts` 现仅剩类型与一行再导出，定位尴尬。
- `settings-nav.test.ts` 两条构造上恒真的断言；`MessagesModalContainer` 零测试；
  `environment-panel.tsx` 自己又取了一次 `app:get-version`（重复 IPC）；
  `.tsx` 内联 `var(--…)` 不在 token 契约扫描范围（当前无此类用法）。
- 视觉相关（截图核对的替代品：参考批次三）：`.hint-float` 是否压住标签栏、
  composer 的 shadow 焦点环与全局 `:focus-visible` 是否双环。

**批次三（收尾）**
- 暗色逐屏走查与修正；明暗基线集重建（旧基线留档）。
- Rust 侧两个窗口问题：`tauri_app.rs` 未 `.decorations(false)` ⇒ 系统标题栏 + 新 40px 标题栏**双标题栏**；
  窗口背景硬编码 `#f6f7f9` ⇒ 暗色用户启动闪白（`index.html` 引导脚本救不了首帧前的窗口底色）。
- `app.css`/`components.css` 残余硬编码复查（本轮已清零引用方，但新增样式仍可能有漏网）。
- 「读屏焦点」、`aria-controls`/tabpanel 全量关联（R33）。
- `theme.test.ts` 把「localStorage 抛错 → 即使系统偏好暗色也 light」钉成契约（既有行为，可重新裁定）。
