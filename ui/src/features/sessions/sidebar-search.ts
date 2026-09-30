import type { AiSession } from "../../types";
import type { SidebarView } from "../../app/sidebar-nav";

/**
 * 列表区该显示什么。
 *
 * # 为什么是纯函数
 *
 * 判定的顺序就是**优先级**：全局搜索 > 收藏夹 > 过滤结果 > 普通列表。四条分支
 * 在渲染里各写一个 `if` 时，最先写的那个就赢了——而这段顺序是有理由的
 * （见下），靠读 JSX 是读不出来的，改错了也看不出。
 *
 * # 顺序的两条理由
 *
 * - **收藏夹排在过滤之前**：它是活动栏之外唯一「换了一处地方」的视图。
 *   用户在收藏夹里打字是在收藏里筛，而不是跳到别处搜——所以本地关键词
 *   不改变列表来源，只决定要不要把 `filtered` 判给它。
 * - **全局搜索排在收藏夹之前**：`globalQuery` 只可能由「在所有目标中搜索」
 *   这个显式动作写进去（它意味着用户已经决定离开当前视图去别处找），
 *   因此它压过一切。`globalSearching` 也在这一条里：结果还没回来时
 *   列表区不能先闪回当前目标的列表。
 */
export type SidebarListMode =
  | { kind: "list" } // 正常的会话列表
  | { kind: "filtered" } // 本地/后端过滤结果（query 非空）
  | { kind: "global" } // 跨目标搜索结果
  | { kind: "favorites" }; // 收藏夹（quickAccess）

export function resolveSidebarListMode(input: {
  view: SidebarView;
  query: string;
  globalSearching: boolean;
  globalQuery: string;
}): SidebarListMode {
  if (input.globalSearching || input.globalQuery.trim()) return { kind: "global" };
  if (input.view === "favorites") return { kind: "favorites" };
  if (input.query.trim()) return { kind: "filtered" };
  return { kind: "list" };
}

/**
 * 关键词在不在这条会话的**摘要**里。
 *
 * # 为什么前端要再算一遍
 *
 * 后端的 `codex:search-sessions` 是两段式的：先匹配摘要，摘要不中再**读整个文件**
 * 用正文匹配（`matches_session_terms`）。结果里**不带**「它是哪一段命中的」这个信息，
 * 而界面上有一个「搜索消息正文」开关：关掉它时用户要的是「只看摘要命中的那些」。
 * 后端没有对应的开关参数（也不该为一个展示选项加一个），所以这里按
 * `aivc-core/src/text.rs` 的 `session_search_haystack` 逐字对齐地算一遍：
 * 编号、标题、来源标题、工作目录、模型、CLI 版本、预览消息全文。
 *
 * 语义与后端一致：按空白分词、全部词都要命中（AND）。
 */
export function matchesSessionSummary(session: AiSession, query: string): boolean {
  const terms = query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter(Boolean);
  if (terms.length === 0) return true;
  const haystack = [
    session.id,
    session.title,
    session.sourceTitle ?? "",
    session.cwd ?? "",
    session.model ?? "",
    session.cliVersion ?? "",
    ...session.preview.map((message) => message.text)
  ]
    .join("\n")
    .toLowerCase();
  return terms.every((term) => haystack.includes(term));
}
