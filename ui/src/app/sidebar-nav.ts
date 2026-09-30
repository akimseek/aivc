/**
 * 侧栏的「地方」与「列表内容」。
 *
 * # 一级只剩一处控件，两个取值
 *
 * 「工作台 / 会话」此前是侧栏里的一组 chip（一级导航），现在由**活动栏**承担：
 * 它切的是整块侧栏，与「筛同一批内容」不是一类动作，摆在一起会让人以为层级相同。
 * 于是这里只剩一个 `SidebarPane` 类型，**不再有映射函数**——一级导航的映射
 * （`paneForViewNav` / `showsViewNav` / `viewNavKey`）随旧的两级控件一起删掉了：
 * 两级控件被过滤 chips 取代，chips 直接读写 `view`，中间不需要一层换算。
 *
 * # 收藏夹为什么是 `view` 而不是 `pane`
 *
 * 它看起来像「另一处地方」，但数据来源与筛选都不是按目标取的会话列表：
 * 收藏夹来自 `quickAccess`（跨目标、只读缓存）。因此它留在 `view` 里
 * （用户能一眼看到它与「全部 / 回收站」并列），但由 `use-console` 保证
 * **切到它时不发会话列表请求**。
 */

/** 侧栏一级「地方」。活动栏按它推导当前亮着哪一个。 */
export type SidebarPane = "sessions" | "workbench";

/**
 * 会话列表当前显示哪一类内容。
 *
 * `favorites` 的内容来自 `quickAccess`（收藏 + 最近打开两组），不是当前目标的
 * 会话列表——这一点决定了「切到收藏夹不发列表请求」。
 */
export type SidebarView = "active" | "favorites" | "trash";

/**
 * 当前平台支不支持回收站与批量操作。
 *
 * 实现搬到 `provider-capabilities.ts`：那些能力不只侧栏在用
 * （右键菜单的「复制会话」、详情里的「从此处分支」也按能力门控），
 * 这里只是把它按侧栏的历史路径再导出一次。
 */
export { supportsBatchActions, supportsTrash } from "./provider-capabilities";
