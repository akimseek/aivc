/**
 * 工作区标签的规则。
 *
 * # 一个标签同时挂会话与终端
 *
 * 旧平台是 `terminalIdsByTabKey: Record<tabKey, terminalId>`——**一个标签，同时挂着
 * 会话（历史）与终端（运行时）**。我这边此前是「一个标签要么是会话、要么是终端」，
 * 两者互不相干，于是出现两个真问题：
 *
 * 1. **一次点击建出两个标签**：点历史会话时，`openAt` 让「选中会话即建标签」的 effect
 *    建一个会话标签，紧接着起终端又建一个终端标签；
 * 2. 终端在跑，但标签上不知道它属于哪个会话（详情里说开着终端、实际对不上）。
 *
 * 这里把规则收敛成一个纯函数，逐条钉住。
 */

/** 标签。链路上只关心这几个字段。 */
export type WorkspaceTabLike = {
  key: string;
  kind: "session" | "terminal";
  title: string;
  /** 这条标签对应的会话（历史 / 正文）。 */
  sessionId?: string;
  /** 这条标签上活着的终端。 */
  terminalId?: string;
};

/**
 * 选中一个会话时，标签该怎么变。
 *
 * - 已经有标签挂着这个会话（**无论它是会话标签还是终端标签**）→ 切过去，
 *   **不新建**。这一条正是「一次点击两个标签」的解药；
 * - 否则建一个会话标签。
 *
 * 返回新的标签数组与应该激活的 key。
 */
export function activateSessionTab<T extends WorkspaceTabLike>(
  tabs: T[],
  session: { id: string; title: string }
): { tabs: T[]; activeKey: string } {
  const existing = tabs.find(
    (tab) => tab.sessionId === session.id || tab.key === session.id
  );
  if (existing) return { tabs, activeKey: existing.key };

  const created = {
    key: session.id,
    kind: "session",
    title: session.title,
    sessionId: session.id
  } as T;
  return { tabs: [...tabs, created], activeKey: session.id };
}
