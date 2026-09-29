import type { SessionView } from "../types";

/**
 * 侧栏的两级导航。
 *
 * # 为什么把它抽成纯函数
 *
 * 一级与二级是**两个控件**，但它们要表达的是同一件事的两个层次：
 * 「工作台 / 会话」是地方，「当前会话 / 收藏夹 / 回收站」是同一批内容的筛选。
 * 两处各写一遍映射，就会出现「二级说在收藏夹、一级还高亮着会话」这种自相矛盾的状态。
 *
 * 所以状态只有一份（`pane` + `view`），两个控件都从它派生，改变也都回到它。
 *
 * # 旧平台的形状（`sidebar-controls.tsx`）
 *
 * - 一级 `sidebar-navigation`：**工作台 / 会话**（只有两个）
 * - 二级 `view-switch has-trash`：**当前会话 / 收藏夹 / 回收站**
 * - 搜索行：输入框 + **全部会话**按钮（全局搜索）
 *
 * 此前我这里是「工作台 / 会话 / 快捷 / 搜索」四个一级项——收藏与全局搜索
 * 被当成了一级地方，而它们在旧平台是**二级筛选**与**一个按钮**。
 */

/** 侧栏一级「地方」。 */
export type SidebarPane = "workbench" | "sessions" | "favorites" | "search";

/**
 * 二级导航的取值。
 *
 * 它**不是** `pane`：收藏夹与回收站分属不同的 `pane`/`view` 组合，
 * 但在二级导航上它们是并排的三个按钮。
 */
export type ViewNavKey = "active" | "favorites" | "trash";

/** 从状态算出二级导航该高亮哪一个。 */
export function viewNavKey(pane: SidebarPane, view: SessionView): ViewNavKey {
  if (pane === "favorites") return "favorites";
  return view === "trash" ? "trash" : "active";
}

/** 二级导航被点时，状态该变成什么。 */
export function paneForViewNav(key: ViewNavKey): { pane: SidebarPane; view: SessionView } {
  if (key === "favorites") return { pane: "favorites", view: "active" };
  return { pane: "sessions", view: key === "trash" ? "trash" : "active" };
}

/**
 * 二级导航那一行该不该出现。
 *
 * 工作台是一处**独立的地方**（它不列会话，也就没有「哪种会话」可筛）；
 * 全局搜索也是。旧平台的条件是 `!workbenchOpen && !globalSearchOpen`。
 */
export function showsViewNav(pane: SidebarPane): boolean {
  return pane === "sessions" || pane === "favorites";
}

/**
 * 搜索行（输入框 + 「全部会话」按钮）该不该出现。
 *
 * 旧平台在收藏夹里把它藏起来：收藏夹是「我已经挑好的那几条」，
 * 在那里再放一个搜索框会让人以为搜的是收藏夹内部。
 */
export function showsSessionSearchRow(pane: SidebarPane): boolean {
  return pane === "sessions";
}

/**
 * 当前平台支不支持回收站与批量操作。
 *
 * 实现搬到 `provider-capabilities.ts`：那些能力不只侧栏在用
 * （右键菜单的「复制会话」、详情里的「从此处分支」也按能力门控）。
 */
export { supportsBatchActions, supportsTrash } from "./provider-capabilities";
