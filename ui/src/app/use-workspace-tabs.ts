import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import type { AiSession } from "../types";
import { displayTitle } from "./session-display";
import {
  attachTerminal,
  closeTab as closeTabPure,
  hasSessionTab,
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
 * # 更新状态一律「先用闭包里的 `tabs` 算、再 setState」
 *
 * 这一条踩过：在 `setTabs` 的更新函数里给外层变量赋值，看起来等价，实际不等价——
 * **React 不保证同步调用更新函数**（它可能推迟到渲染阶段），那一行读到的还是旧值，
 * 于是终端从未被 `closeTerminal`（标签消失了、进程与 xterm 还活着，而且每开关一次
 * 就多一个）。所以这里所有地方都先算结果，再 `setTabs` / `setActiveKey`。
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

export function useWorkspaceTabs(
  terminals: ReturnType<typeof useTerminals>,
  selectedSession: AiSession | null
) {
  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeKey, setActiveKey] = useState("");

  /**
   * 最新标签列表的影子。
   *
   * 选中会话那个 effect **只认 `selectedSession` 一个依赖**：把 `tabs` 也绑上，
   * 任何一次标签变化（刚起了终端、刚切了标签）都会让它重跑一遍，把激活键抢回
   * 选中会话那张标签——用户手动切走的标签会被弹回去。所以最新值只能从 ref 读。
   */
  const tabsRef = useRef<WorkspaceTab[]>(tabs);
  useEffect(() => {
    tabsRef.current = tabs;
  }, [tabs]);

  /**
   * 把一路终端挂到标签上（等价于旧模型里的 `addTerminalTab`）。
   *
   * 关键差异：**复用**已有标签而不是新建第二张；同一张标签上被替换掉的旧终端要
   * `closeTerminal` 收掉，否则后端登记项与 xterm 会泄漏。
   */
  const attachTerminalTab = useCallback(
    (terminalId: string, sessionId: string | undefined, title: string) => {
      const result = attachTerminal(tabs, { terminalId, sessionId, title });
      setTabs(result.tabs);
      setActiveKey(result.activeKey);
      if (result.replacedTerminalId) void terminals.closeTerminal(result.replacedTerminalId);
    },
    [tabs, terminals]
  );

  /**
   * 关一张标签。
   *
   * 关掉当前标签时**切到剩下最后一张**，而不是回到空：空工作区看起来像「什么都没了」，
   * 而用户通常只是想回到刚才那条。
   *
   * `terminalId` 必须从**闭包里的 `tabs`** 取（见文件头的说明），终端也要真的停掉：
   * 只从列表里去掉会让进程与 xterm 留在原地。
   */
  const closeTab = useCallback(
    (key: string) => {
      const tab = tabs.find((item) => item.key === key);
      const result = closeTabPure(tabs, key, activeKey);
      setTabs(result.tabs);
      setActiveKey(result.activeKey);
      if (tab?.terminalId) void terminals.closeTerminal(tab.terminalId);
    },
    [tabs, activeKey, terminals]
  );

  /**
   * 关掉除这一张之外的全部标签。
   *
   * 统一模型里**每张标签都可关**（不再有「跟着选中走的会话标签」），因此不需要
   * 按 `kind` 挑「可关的那一部分」。
   *
   * 批量关必须**一次算完再逐个调 `closeTerminal`**：逐个 `setState` 会在闭包里
   * 反复读到同一份旧列表，最后只剩下最后一次的结果——与「关标签时终端没被终止」
   * 是同一类问题。
   */
  const closeOtherTabs = useCallback(
    (keepKey: string) => {
      const closing = tabs.filter((tab) => tab.key !== keepKey);
      setTabs(tabs.filter((tab) => tab.key === keepKey));
      for (const tab of closing) {
        if (tab.terminalId) void terminals.closeTerminal(tab.terminalId);
      }
    },
    [tabs, terminals]
  );

  /** 关掉全部标签，并把终端逐个收掉。 */
  const closeAllTabs = useCallback(() => {
    const closing = [...tabs];
    setTabs([]);
    setActiveKey("");
    for (const tab of closing) {
      if (tab.terminalId) void terminals.closeTerminal(tab.terminalId);
    }
  }, [tabs, terminals]);

  /**
   * 选中会话 → 打开/激活它的标签（列表点击、搜索跳转、批量后刷新都汇到这一处）。
   *
   * **不再起终端**：单击的意图多半是看历史，拉起一个 CLI 进程既慢又意外；
   * 恢复终端改成显式动作（标签栏的「继续会话」）。
   */
  useEffect(() => {
    if (!selectedSession) return;
    const result = openSessionTab(tabsRef.current, {
      id: selectedSession.id,
      title: displayTitle(selectedSession)
    });
    setTabs(result.tabs);
    setActiveKey(result.activeKey);
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
        tab.terminalId !== undefined &&
        terminals.terminals.some(
          (item) => item.id === tab.terminalId && item.exitedCleanly === true
        )
    );
    for (const tab of finished) closeTab(tab.key);
  }, [tabs, terminals.terminals, closeTab]);

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
    setActiveKey,
    tabItems,
    activeTab,
    attachTerminalTab,
    closeTab,
    closeOtherTabs,
    closeAllTabs
  };
}

/**
 * 供上层判断「有没有能导出的活动会话」（导出动作的启用条件）。
 *
 * 规则本体在 `workspace-tabs.ts`（纯函数、有测试）；这里只把它转出去，
 * 让调用方从一个模块就能拿到 hook 与它的判定。
 */
export { hasSessionTab };
