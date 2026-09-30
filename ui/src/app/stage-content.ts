/** 画布当前该显示什么。 */
export type StageContent =
  | { kind: "terminal"; terminalId: string }
  | { kind: "history"; sessionId: string }
  | { kind: "empty" };

/**
 * 判定规则：**终端优先**。
 *
 * 一张标签同时挂着会话与终端（spec D3 的统一模型），两者都在时显示终端——
 * 用户刚启动的就是它。终端不在了（还没启动、或被关掉）才回落到历史。
 * 这一步必须是纯函数：它是「切换画布」唯一的判据，错了整块区域就是错的。
 */
export function resolveStageContent(
  tab: { sessionId?: string; terminalId?: string } | null,
  hasTerminal: (terminalId: string) => boolean
): StageContent {
  if (!tab) return { kind: "empty" };
  if (tab.terminalId && hasTerminal(tab.terminalId)) {
    return { kind: "terminal", terminalId: tab.terminalId };
  }
  if (tab.sessionId) return { kind: "history", sessionId: tab.sessionId };
  return { kind: "empty" };
}
