import { useCallback, useEffect, useMemo, useState } from "react";

import type { TabItem } from "../components";
import type { AiSession, AiTarget } from "../types";
import { displayTitle } from "./session-display";
import { activateSessionTab, deadTabsForSession } from "./workspace-tabs";
import type { useTerminals } from "./use-terminals";

/**
 * 工作区的标签。
 *
 * # 为什么这段逻辑必须离开 `app.tsx`
 *
 * 它包含三类东西，而它们**都不该出现在渲染代码里**：
 *
 * 1. **标签的增删规则**（已有的就切过去，不新建第二张）——规则本身在
 *    `workspace-tabs.ts` 里且可单测，但「谁在什么时候调用它」有六条路径
 *    （选中会话、开 Codex 终端、开系统终端、手动关闭、干净退出自动收、
 *    再次进入时清死标签）；
 * 2. **一个真实的时间差**：标签的 key 由 `terminal:start` 返回的编号决定，
 *    而 `setTabs` 是异步的——`activeKey` 必须同时设好，否则新终端起来了却不在前台；
 * 3. **一条容易漏的分支**：终端自己退出（干净退出）时不收标签，用户会看到一张
 *    已经死掉的标签，再次进入会话时被它挡住。这条此前真的漏过。
 *
 * 抽出来之后 `app.tsx` 只需在事件里调一个方法，而这里可以按「给定这些事件，
 * 标签变成什么样」来断言。
 */

/** 工作区的一个标签：会话或终端。 */
export type WorkspaceTab = {
  key: string;
  kind: "session" | "terminal";
  title: string;
  /** `kind === "session"` 时是会话编号。 */
  sessionId?: string;
  /** `kind === "terminal"` 时是终端编号（同时用作 key）。 */
  terminalId?: string;
};

export function useWorkspaceTabs(
  terminals: ReturnType<typeof useTerminals>,
  selectedSession: AiSession | null
) {
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeKey, setActiveKey] = useState("");

  /** 加一张终端标签并让它到前台。 */
  const addTerminalTab = useCallback((id: string, title: string, sessionId?: string) => {
    setTabs((current) => [
      ...current,
      {
        key: id,
        kind: "terminal",
        title,
        terminalId: id,
        // **这条终端属于哪条会话。** 有它，标签才能既当终端又当那个会话的入口：
        // 终端退出后会话仍在，选中会话时也不会再建第二个标签。
        ...(sessionId ? { sessionId } : {})
      }
    ]);
    setActiveKey(id);
  }, []);

  /**
   * 关一张标签。
   *
   * 关掉当前标签时**切到剩下最后一张**，而不是回到空：空工作区看起来像「什么都没了」，
   * 而用户通常只是想回到刚才那条。
   *
   * # 那个 `terminalId` 必须从**闭包里的 `tabs`** 取，不能在更新函数里取
   *
   * 这一条踩过：写成
   *
   * ```text
   * let closingTerminalId;
   * setTabs((current) => { closingTerminalId = current.find(...)?.terminalId; ... });
   * if (closingTerminalId) void terminals.closeTerminal(closingTerminalId);  // ← 永远是 undefined
   * ```
   *
   * 看起来等价，实际不等价：**React 不保证同步调用更新函数**（它可能推迟到渲染阶段）。
   * 于是那一行读到的还是 `undefined`，`closeTerminal` 从未被调用——标签消失了，
   * 但终端进程还活着、xterm 还挂在 DOM 上。探针读数直接暴露了它：
   * 退出后 `xterms` 应该是 0，实际是 1，而且每开关一次就多一个。
   */
  const closeTab = useCallback(
    (key: string) => {
      const closing = tabs.find((tab) => tab.key === key);
      setTabs((current) => {
        const next = current.filter((tab) => tab.key !== key);
        setActiveKey((currentKey) =>
          currentKey === key ? (next.at(-1)?.key ?? "") : currentKey
        );
        return next;
      });
      if (closing?.terminalId) void terminals.closeTerminal(closing.terminalId);
    },
    [tabs, terminals]
  );

  /**
   * 清掉某条会话上**已经死掉**的终端标签。
   *
   * 干净退出的标签会被自动收掉，但**异常退出的标签是故意留着的**（要让人看报错）。
   * 于是再次进入这条会话时，`activateSessionTab` 会切到那一张死标签上——
   * 用户看到的是不能输入、不再产出的终端。用户原话：
   * 「`/exit` 退出了会话终端，再次进入会被锁住，并没有释放资源」。
   *
   * 所以进入之前必须先清，判据是「终端不在了」而不是「标签存在」。
   */
  const clearDeadTabs = useCallback(
    (sessionId: string) => {
      const dead = deadTabsForSession(tabs, sessionId, (terminalId) =>
        terminals.terminals.some((item) => item.id === terminalId && item.running)
      );
      if (dead.length === 0) return;
      const removing = new Set(dead);
      setTabs((current) => {
        const next = current.filter((tab) => !removing.has(tab.key));
        setActiveKey((currentKey) =>
          removing.has(currentKey) ? (next.at(-1)?.key ?? "") : currentKey
        );
        return next;
      });
      for (const tab of tabs) {
        if (removing.has(tab.key) && tab.terminalId) void terminals.closeTerminal(tab.terminalId);
      }
    },
    [tabs, terminals]
  );

  /**
   * 选中的会话自动成为当前标签。
   *
   * 由 `selectedId` 驱动而不是在每个点击处手写一次：列表里点击、搜索后跳转、
   * 批量操作后刷新——每条路径都会改 `selectedId`，只在一处响应才不会漏。
   */
  useEffect(() => {
    if (!selectedSession) return;
    let activated = selectedSession.id;
    setTabs((current) => {
      const result = activateSessionTab(current, {
        id: selectedSession.id,
        title: displayTitle(selectedSession)
      });
      activated = result.activeKey;
      return result.tabs;
    });
    setActiveKey(activated);
  }, [selectedSession]);

  /**
   * **干净退出的终端：收掉它的标签。**
   *
   * 与旧平台一致的语义（`handleTerminalExit` 第一行 `if (exitCode !== 0) return;`）：
   * 只有干净退出才收；异常退出保留现场，让用户看得到最后几行报错。
   * 退出码的判断在 `use-terminals` 里做（只有它拿得到），这里只负责执行。
   */
  useEffect(() => {
    const finished = tabs.filter(
      (tab) =>
        tab.kind === "terminal" &&
        tab.terminalId !== undefined &&
        terminals.terminals.some(
          (item) => item.id === tab.terminalId && item.exitedCleanly === true
        )
    );
    for (const tab of finished) closeTab(tab.key);
  }, [tabs, terminals.terminals, closeTab]);

  const tabItems: TabItem[] = useMemo(
    () =>
      tabs.map((tab) => {
        /**
         * 终端进程结束后必须在标签上看得出来，而且**要带上退出码**。
         *
         * 这一条此前**永远不会出现**：`terminal:exit` 只有在我们主动关闭终端时才发得出来
         * ——ConPTY 下子进程自己退出不会让读取线程收到 EOF——所以 `running` 一直是 true，
         * 标签也就一直没有任何变化。用户敲了 `exit`，界面看起来跟还在运行一模一样。
         *
         * 退出码非有不可：`（已退出）` 说不出是「自己结束」还是「被杀掉」，
         * 而这两种要查的地方完全不同。干净退出的标签会被自动收掉，
         * 因此带退出码的那一种恰好就是**需要用户看一眼**的那一种。
         */
        const terminal = terminals.terminals.find((item) => item.id === tab.terminalId);
        const exited = tab.kind === "terminal" && terminal !== undefined && !terminal.running;
        const suffix =
          exited && !terminal?.exitedCleanly
            ? `（退出码 ${terminal?.exitCode ?? "?"}）`
            : exited
              ? "（已退出）"
              : "";
        return {
          key: tab.key,
          label: `${tab.title}${suffix}`,
          // 等待确认的终端在标签上打一个点。这正是「关注中心」要解决的问题：
          // 用户切到别的标签后，那一路还在等他，而界面上没有任何提示。
          tone: terminals.attention[tab.key] ? ("attention" as const) : ("normal" as const)
        };
      }),
    [tabs, terminals.attention, terminals.terminals]
  );

  /**
   * 这张标签是不是**系统终端**。
   *
   * 系统终端与 Codex 终端在数据上是同一种东西（都走 `useTerminals` 的增删与回收），
   * 但**渲染位置不同**：系统终端在底部停靠面板里，Codex 终端在工作区的标签条上。
   * 旧平台就是这样（`system-terminal.tsx` 是一块独立的底板），
   * 好处是「一边看 Codex 一边敲 shell」不需要来回切标签。
   */
  const isSystemTab = useCallback(
    (tab: WorkspaceTab) =>
      tab.kind === "terminal" &&
      terminals.terminals.some((item) => item.id === tab.terminalId && item.kind === "system"),
    [terminals.terminals]
  );

  const workspaceTabItems: TabItem[] = useMemo(
    () => tabItems.filter((item) => !isSystemTab(tabs.find((tab) => tab.key === item.key)!)),
    [tabItems, tabs, isSystemTab]
  );

  /** 底部停靠面板里要显示的标签。 */
  const systemTabItems: TabItem[] = useMemo(
    () => tabItems.filter((item) => isSystemTab(tabs.find((tab) => tab.key === item.key)!)),
    [tabItems, tabs, isSystemTab]
  );

  const activeTab = tabs.find((tab) => tab.key === activeKey) ?? null;

  /** 关得掉的标签（`closable !== false`）。会话标签通常不可关。 */
  const closableKeys = useCallback(
    () => tabs.filter((tab) => tab.kind === "terminal").map((tab) => tab.key),
    [tabs]
  );

  /**
   * 关掉除这一张之外的全部可关标签。
   *
   * 批量关必须**一次算完再逐个调 `closeTerminal`**：逐个 `setTabs` 会在闭包里
   * 反复读到同一份旧列表，最后只剩下最后一次的结果。这正是「关标签时终端没被
   * 终止」那个 bug 的同一类问题（见 `closeTab` 的注释）。
   */
  const closeOtherTabs = useCallback(
    (keepKey: string) => {
      const closing = tabs.filter((tab) => tab.key !== keepKey && tab.kind === "terminal");
      setTabs((current) => current.filter((tab) => tab.key === keepKey || tab.kind !== "terminal"));
      for (const tab of closing) {
        if (tab.terminalId) void terminals.closeTerminal(tab.terminalId);
      }
    },
    [tabs, terminals]
  );

  /** 关掉全部可关标签。会话标签留着——它跟着选中状态走，不归这里管。 */
  const closeAllTabs = useCallback(() => {
    const closing = tabs.filter((tab) => tab.kind === "terminal");
    setTabs((current) => current.filter((tab) => tab.kind !== "terminal"));
    for (const tab of closing) {
      if (tab.terminalId) void terminals.closeTerminal(tab.terminalId);
    }
  }, [tabs, terminals]);

  return {
    tabs,
    activeKey,
    setActiveKey,
    tabItems,
    /** 工作区标签条要显示的（不含系统终端）。 */
    workspaceTabItems,
    /** 底部停靠面板要显示的（只有系统终端）。 */
    systemTabItems,
    isSystemTab,
    activeTab,
    addTerminalTab,
    closeTab,
    closeOtherTabs,
    closeAllTabs,
    closableKeys,
    clearDeadTabs
  };
}

/** 供上层判断「有没有能导出的活动会话」。 */
export function hasSessionTab(tabs: WorkspaceTab[]): boolean {
  return tabs.some((tab) => tab.kind === "session");
}

/** 从会话列表里按编号找回完整会话——右键菜单与外链都需要它。 */
export function findSession(sessions: AiSession[], id: string): AiSession | null {
  return sessions.find((item) => item.id === id) ?? null;
}

/** 目标与编号一起用来还原一条会话的入口。 */
export type ResumeRequest = { target: AiTarget; sessionId: string };
