import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { AiSession } from "../types";
import { displayTitle } from "./session-display";
import {
  attachTerminal,
  closeTab as closeTabPure,
  openSessionTab,
  tabSuffix,
  type WorkspaceTabLike
} from "./workspace-tabs";
import type { useTerminals } from "./use-terminals";

/**
 * 工作区的标签。
 *
 * # 为什么这段逻辑必须离开 `app.tsx`
 *
 * 它包含三类东西，而它们**都不该出现在渲染代码里**：
 *
 * 1. **标签的增删规则**（已有的就切过去，不新建第二张）——规则本身在
 *    `workspace-tabs.ts` 里且可单测，但「谁在什么时候调用它」有五条路径
 *    （选中会话、起终端（新建 / 恢复）、手动关闭、干净退出自动收、批量关闭）；
 * 2. **一个真实的时间差**：标签的 key 由 `terminal:start` 返回的编号决定，
 *    而 `setState` 是异步的——`activeKey` 必须同时设好，否则新终端起来了却不在前台；
 * 3. **一条容易漏的分支**：终端自己退出（干净退出）时不收标签，用户会看到一张
 *    已经死掉的标签，再次进入会话时被它挡住。这条此前真的漏过。
 *
 * 抽出来之后 `app.tsx` 只需在事件里调一个方法，而这里可以按「给定这些事件，
 * 标签变成什么样」来断言。
 *
 * # 统一模型（spec D3）
 *
 * 一张标签**同时**挂会话与终端：`sessionId` 是它的身份（key 优先用它），终端经
 * `attachTerminal` 挂上去、并**替换**掉这张标签上原来那一路——被替换的旧终端必须
 * `closeTerminal`，否则后端登记项与 xterm 会泄漏（这正是「再次进入会话被锁住」
 * 那个历史缺陷的成因之一）。画布显示什么由 `resolveStageContent` 判定。
 *
 * # 写入一律走更新函数，副作用经队列出队
 *
 * 这一层踩过两次同一类坑：**用渲染期的 `tabs` 快照算出绝对值再写回**。
 * 看起来等价，实际不等价——两次写入完全可能来自同一次渲染（两路 `await
 * openTerminal` 并发返回、一次点击与一个挂起的事件回调撞在一起），那时两个调用点
 * 持有的是同一份快照：后一次写把前一次的结果整个盖掉，而被替换的终端因为没人
 * 看得见它，永远不会被 `closeTerminal`（标签没了，进程与 xterm 还活着）。
 *
 * 所以这里：
 *
 * - 状态是**一个** `{ tabs, activeKey }`，写入只有 `update` 一条路——它把「按当前值
 *   算下一个值」交给 `setState` 的更新函数。React 保证更新函数按顺序作用在上一次
 *   的结果之上，同一批次里的两次写入因此是**叠加**而不是互相覆盖；
 * - `closeTerminal` 是**副作用**，不能写在更新函数里（React 不保证同步调用更新函数，
 *   也不允许在其中派生副作用）：更新函数只把要关的终端编号**幂等地**推进队列，
 *   由下面的 effect 在提交之后逐个排空。
 */

/** 工作区的一个标签：同时挂会话与终端（统一模型）。 */
export type WorkspaceTab = WorkspaceTabLike;

/** 标签条要显示的一项。`suffix` / `state` / `providerId` 供新 `TabBar` 用。 */
export type WorkspaceTabItem = {
  key: string;
  label: string;
  suffix: string;
  tone: "normal" | "attention";
  providerId?: string;
  state: "running" | "idle" | "exited";
};

/**
 * 标签的状态。
 *
 * `tabs` 与 `activeKey` 放进**同一个** state：它们几乎总是一起变（挂终端、关标签、
 * 关其他），分开写就会漏出「标签已经换了、激活键还指着旧的」这类中间态。
 */
type TabsState = { tabs: WorkspaceTab[]; activeKey: string };

export function useWorkspaceTabs(
  terminals: ReturnType<typeof useTerminals>,
  selectedSession: AiSession | null
) {
  const [state, setState] = useState<TabsState>({ tabs: [], activeKey: "" });
  const { tabs, activeKey } = state;

  /**
   * 待关闭的终端编号。
   *
   * 更新函数里只能**入队**（关终端是要发 IPC 的副作用）；提交之后由下面的 effect
   * 逐个排出。用 `Set` 而不是数组：入队必须**幂等**——StrictMode 下更新函数会被
   * 调用两次（并发渲染也可能重放），重复的编号不能变成两次 `closeTerminal`。
   */
  const pendingClose = useRef<Set<string>>(new Set());

  /**
   * 标签的**唯一**写入路径。
   *
   * `mutate` 拿到的是**当前**状态，而不是渲染期的闭包快照——同一批次里的两次写入
   * 因此能依次叠加。返回下一个状态与要关闭的终端编号（副作用留给 effect）。
   */
  const update = useCallback(
    (mutate: (current: TabsState) => { next: TabsState; close?: readonly string[] }) => {
      setState((current) => {
        const { next, close } = mutate(current);
        if (close) {
          for (const id of close) pendingClose.current.add(id);
        }
        return next;
      });
    },
    []
  );

  /** 排空待关闭队列。放在这里而不是更新函数里：关终端是副作用。 */
  useEffect(() => {
    if (pendingClose.current.size === 0) return;
    const closing = [...pendingClose.current];
    pendingClose.current.clear();
    for (const id of closing) void terminals.closeTerminal(id);
  }, [state, terminals]);

  /**
   * 把一路终端挂到标签上（等价于旧模型里的 `addTerminalTab`）。
   *
   * 关键差异：**复用**已有标签而不是新建第二张；同一张标签上被替换掉的旧终端要
   * `closeTerminal` 收掉，否则后端登记项与 xterm 会泄漏。
   *
   * 「被替换的是谁」只有对着**更新函数里的当前值**算才知道：渲染期的闭包在两个
   * 调用点之间是同一份快照，第二次挂载看不见第一次新建的标签。
   */
  const attachTerminalTab = useCallback(
    (terminalId: string, sessionId: string | undefined, title: string) => {
      update((current) => {
        const result = attachTerminal(current.tabs, { terminalId, sessionId, title });
        return {
          next: { tabs: result.tabs, activeKey: result.activeKey },
          close: result.replacedTerminalId ? [result.replacedTerminalId] : undefined
        };
      });
    },
    [update]
  );

  /**
   * 关一张标签。
   *
   * 关掉当前标签时**切到剩下最后一张**，而不是回到空：空工作区看起来像「什么都没了」，
   * 而用户通常只是想回到刚才那条。
   *
   * 终端也要真的停掉——只从列表里去掉会让进程与 xterm 留在原地。
   */
  const closeTab = useCallback(
    (key: string) => {
      update((current) => {
        const tab = current.tabs.find((item) => item.key === key);
        return {
          next: closeTabPure(current.tabs, key, current.activeKey),
          close: tab?.terminalId ? [tab.terminalId] : undefined
        };
      });
    },
    [update]
  );

  /**
   * 关掉除这一张之外的全部标签。
   *
   * 统一模型里**每张标签都可关**（不再有「跟着选中走的会话标签」），因此不需要
   * 按 `kind` 挑「可关的那一部分」。
   *
   * `activeKey` 必须跟着保留的那张走：从**非激活**标签上右键「关闭其他」时，不修
   * 它会留下一个指向已关标签的激活键——标签还在，画布却落到空状态。
   */
  const closeOtherTabs = useCallback(
    (keepKey: string) => {
      update((current) => ({
        next: { tabs: current.tabs.filter((tab) => tab.key === keepKey), activeKey: keepKey },
        close: current.tabs.flatMap((tab) =>
          tab.key !== keepKey && tab.terminalId ? [tab.terminalId] : []
        )
      }));
    },
    [update]
  );

  /** 关掉全部标签，并把终端逐个收掉。 */
  const closeAllTabs = useCallback(() => {
    update((current) => ({
      next: { tabs: [], activeKey: "" },
      close: current.tabs.flatMap((tab) => (tab.terminalId ? [tab.terminalId] : []))
    }));
  }, [update]);

  /**
   * 外部切换激活键的唯一入口（点击标签、点提醒切换）。
   *
   * 走同一份 state：否则「标签换了、激活键还指着旧的」这类中间态就会从这里漏出去。
   */
  const selectTab = useCallback((key: string) => {
    setState((current) => (current.activeKey === key ? current : { ...current, activeKey: key }));
  }, []);

  /**
   * 选中会话 → 打开/激活它的标签（列表点击、搜索跳转、批量后刷新都汇到这一处）。
   *
   * **不再起终端**：单击的意图多半是看历史，拉起一个 CLI 进程既慢又意外；
   * 恢复终端改成显式动作（标签栏的「继续会话」）。
   *
   * 依赖是**会话编号**而不是整个对象：`console.selectedSession` 由
   * `sessions.find(...)` 得来，每次列表刷新（搜索框防抖、点「刷新」）都会给出
   * 一个新对象——按对象身份触发的话，用户在列表里刷新一下就会让 `openSessionTab`
   * 走「已有标签」那条路、把画布从正在跑的新会话终端抢回更早的选中会话上。
   * 编号没变就不是一次「换了会话」。effect 体内仍从**本次渲染**读对象，标题不丢。
   *
   * 不绑 `state`：把 `state` 也绑上，任何一次标签变化（刚起了终端、刚切了标签）
   * 都会让它重跑，把激活键抢回选中会话那张标签——用户手动切走的标签会被弹回去。
   * 最新标签从更新函数的当前值里取，不需要额外的 ref。
   */
  useEffect(() => {
    if (!selectedSession) return;
    const id = selectedSession.id;
    const title = displayTitle(selectedSession);
    update((current) => ({ next: openSessionTab(current.tabs, { id, title }) }));
  }, [selectedSession?.id, update]);

  /**
   * **干净退出的终端：收掉它的标签。**
   *
   * 与旧平台一致的语义（`handleTerminalExit` 第一行 `if (exitCode !== 0) return;`）：
   * 只有干净退出才收；异常退出保留现场，让用户看得到最后几行报错。
   * 退出码的判断在 `use-terminals` 里做（只有它拿得到），这里只负责执行。
   *
   * 要收的可能**不止一张**（两路终端同时退出），所以先把 `finished` 整个算出来，
   * 再用**一次** `update` 提交：逐个 `closeTab` 会各自基于同一份快照重算，先关的
   * 那张被后一次写「复活」；而 `closeTerminal` 已经把这些终端从登记表里摘掉，
   * 残留的标签再也不会被上面的判据命中，也就永远不会自愈。
   */
  useEffect(() => {
    const finished = tabs.filter(
      (tab) =>
        tab.terminalId !== undefined &&
        terminals.terminals.some(
          (item) => item.id === tab.terminalId && item.exitedCleanly === true
        )
    );
    if (finished.length === 0) return;
    update((current) => {
      let next: TabsState = current;
      for (const tab of finished) {
        next = closeTabPure(next.tabs, tab.key, next.activeKey);
      }
      return {
        next,
        close: finished.flatMap((tab) => (tab.terminalId ? [tab.terminalId] : []))
      };
    });
  }, [tabs, terminals.terminals, update]);

  const activeTab = tabs.find((tab) => tab.key === activeKey) ?? null;

  /**
   * 标签条要显示的样子。
   *
   * 三件事在这一层算完，渲染层不必各查一遍：
   * - **后缀**（`（已退出）` / `（退出码 N）`）来自终端的结束状态，规则在 `tabSuffix`
   *   ——`1`（自己报错）与 `137`（被 kill）要查的地方完全不同，后者必须带退出码；
   * - **提醒点**按 `terminalId` 查：提醒是终端的属性，而标签的 key 可能是会话编号；
   * - **状态**给新 `TabBar` 用：跑着 / 空闲（还没起终端）/ 已结束。
   */
  const tabItems: WorkspaceTabItem[] = useMemo(
    () =>
      tabs.map((tab) => {
        const terminal = terminals.terminals.find((item) => item.id === tab.terminalId);
        return {
          key: tab.key,
          label: tab.title,
          suffix: tabSuffix(terminal),
          tone:
            tab.terminalId && terminals.attention[tab.terminalId]
              ? ("attention" as const)
              : ("normal" as const),
          providerId: terminal?.providerId,
          state: terminal ? (terminal.running ? "running" : "exited") : "idle"
        };
      }),
    [tabs, terminals.attention, terminals.terminals]
  );

  return {
    tabs,
    activeKey,
    setActiveKey: selectTab,
    tabItems,
    activeTab,
    attachTerminalTab,
    closeTab,
    closeOtherTabs,
    closeAllTabs
  };
}
