/**
 * 工作区标签的规则（统一模型）。
 *
 * # 一张标签同时挂会话与终端
 *
 * 旧模型里「会话标签」与「终端标签」是两种东西，点一次历史会话会**同时**建出两张
 * （选中会话的 effect 建一张、起终端又建一张）。新模型（spec D3）把两者合成一张：
 *
 * - key 优先用**会话编号**（会话已知时），否则用终端编号（全新会话还没编号）；
 * - 终端通过 `attachTerminal` **挂到**已有标签上，绝不新建第二张；
 * - 画布显示什么由 [`resolveStageContent`](./stage-content.ts) 判定（终端优先）。
 *
 * 这里全部是纯函数：标签的增删是最容易出竞态的一块，纯函数才能逐条钉住。
 */

/** 标签。链路上只关心这几个字段。 */
export type WorkspaceTabLike = {
  key: string;
  /** 这条标签对应的会话（历史正文）。新会话在拿到编号前没有它。 */
  sessionId?: string;
  /** 这条标签上活着的终端。 */
  terminalId?: string;
  title: string;
};

/** 打开（或激活）一条会话的标签。已有挂着该会话的标签就切过去，不新建。 */
export function openSessionTab<T extends WorkspaceTabLike>(
  tabs: T[],
  session: { id: string; title: string }
): { tabs: T[]; activeKey: string } {
  const existing = tabs.find((tab) => tab.sessionId === session.id || tab.key === session.id);
  if (existing) return { tabs, activeKey: existing.key };
  return {
    tabs: [...tabs, { key: session.id, sessionId: session.id, title: session.title } as T],
    activeKey: session.id
  };
}

/**
 * 把一路终端挂到它的会话标签上；没有标签就建一个。
 *
 * 返回 `replacedTerminalId`：同一张标签上原来那一路终端（若有）——调用方要把它
 * `closeTerminal` 收掉，否则后端登记项与 xterm 会泄漏（这正是「再次进入会话被锁住」
 * 那个历史缺陷的成因之一）。
 */
export function attachTerminal<T extends WorkspaceTabLike>(
  tabs: T[],
  input: { terminalId: string; sessionId?: string; title: string }
): { tabs: T[]; activeKey: string; replacedTerminalId?: string } {
  const existing = input.sessionId
    ? tabs.find((tab) => tab.sessionId === input.sessionId)
    : tabs.find((tab) => tab.key === input.terminalId || tab.terminalId === input.terminalId);

  if (existing) {
    const replacedTerminalId =
      existing.terminalId && existing.terminalId !== input.terminalId
        ? existing.terminalId
        : undefined;
    const next = tabs.map((tab) =>
      tab.key === existing.key ? { ...tab, terminalId: input.terminalId } : tab
    );
    return { tabs: next, activeKey: existing.key, ...(replacedTerminalId ? { replacedTerminalId } : {}) };
  }

  const key = input.sessionId ?? input.terminalId;
  return {
    tabs: [
      ...tabs,
      {
        key,
        terminalId: input.terminalId,
        ...(input.sessionId ? { sessionId: input.sessionId } : {}),
        title: input.title
      } as T
    ],
    activeKey: key
  };
}

/** 关掉一张标签。关掉的是激活标签时切到剩余最后一张（空工作区看起来像「什么都没了」）。 */
export function closeTab<T extends WorkspaceTabLike>(
  tabs: T[],
  key: string,
  activeKey: string
): { tabs: T[]; activeKey: string } {
  const next = tabs.filter((tab) => tab.key !== key);
  return { tabs: next, activeKey: key === activeKey ? (next.at(-1)?.key ?? "") : activeKey };
}

/** 标签标题后缀。异常退出必须带退出码：`1` 与 `137` 要查的地方完全不同。 */
export function tabSuffix(state?: {
  running: boolean;
  exitedCleanly?: boolean;
  exitCode?: number;
}): string {
  if (!state || state.running) return "";
  if (state.exitedCleanly) return "（已退出）";
  return `（退出码 ${state.exitCode ?? "?"}）`;
}
