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
      tab.key === existing.key
        ? {
            ...tab,
            terminalId: input.terminalId,
            ...(tab.sessionId ? {} : input.sessionId ? { sessionId: input.sessionId } : {})
          }
        : tab
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

/** 有没有能导出的活动会话（导出动作的启用条件）。 */
export function hasSessionTab(tabs: WorkspaceTabLike[]): boolean {
  return tabs.some((tab) => tab.sessionId !== undefined);
}

/**
 * 旧模型的遗留导出，**仍然被 `use-workspace-tabs.ts` 引用**（Task 4 接线时随该
 * hook 的重写一起删掉）。留着是为了让本次改动只动纯逻辑一层：删掉它们会让中间
 * 这一步无法通过 typecheck，而接线是下一个任务的事。
 *
 * 它们与上面的新函数**并存但不互用**：调用方仍是旧的 hook，新函数由 Task 4 接。
 */

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

  // `kind` 是旧 hook 运行时读的字段（它按 `kind === "terminal"` 算可关标签）：
  // 新模型的 `WorkspaceTabLike` 里已经没有它的类型位置，而旧函数还得靠它工作，
  // 所以这里只能经 `unknown` 转回 T。本函数与这一行由 Task 4 一起删掉。
  const created = {
    key: session.id,
    kind: "session",
    title: session.title,
    sessionId: session.id
  } as unknown as T;
  return { tabs: [...tabs, created], activeKey: session.id };
}

/**
 * 找出「挂着这条会话、但终端已经不在跑」的标签。
 *
 * # 为什么必须有这一步
 *
 * 用户报的现象是「`/exit` 退出了会话终端，再次进入会被锁住，并没有释放资源」。
 * 根因有两层，这是第二层：
 *
 * 1. 干净退出时标签没收掉（已修）——但那一条**只覆盖干净退出**，
 *    因为异常退出要保留现场让用户看得到报错；
 * 2. 于是**异常退出的那一张标签会留在那里**，而 `activateSessionTab` 的逻辑是
 *    「已经有标签挂着这条会话就切过去」——再次进入时用户切到的是
 *    **那一路已经死掉的终端**：不能输入、不再产出，看起来就是被锁住。
 *
 * 所以再次进入一条会话之前，要先把这类的死标签清掉，再开新的。
 * 判据是「终端不在了」，不是「标签存在」。
 */
export function deadTabsForSession<T extends WorkspaceTabLike>(
  tabs: T[],
  sessionId: string,
  isTerminalRunning: (terminalId: string) => boolean
): string[] {
  return tabs
    .filter((tab) => tab.sessionId === sessionId)
    // 没有终端编号的会话标签（只是「看正文」那一种）不算死的。
    .filter((tab) => tab.terminalId !== undefined && !isTerminalRunning(tab.terminalId))
    .map((tab) => tab.key);
}
