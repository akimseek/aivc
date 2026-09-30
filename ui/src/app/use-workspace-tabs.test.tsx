// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiSession, TerminalAttention } from "../types";
import { useWorkspaceTabs } from "./use-workspace-tabs";
import type { TerminalSession } from "./use-terminals";
import type { useTerminals } from "./use-terminals";

/**
 * `useWorkspaceTabs` 的装配测试（统一模型）。
 *
 * # 为什么这里需要它
 *
 * 标签的**规则**已经在 `workspace-tabs.test.ts` 里逐条钉住了，但这一层是另一种东西：
 * 后端事件与 React 状态之间的接线。而这块恰恰是「更新状态」最容易写错的地方——
 * 在 `setState` 的更新函数里取值，React 不保证同步调用它，读到的永远是旧值
 * （「关标签时终端没被终止」那个历史缺陷的形态）。那类错误类型、纯逻辑全绿，
 * 只有把 hook 真的跑起来才看得见。
 *
 * 假终端层只提供这一层真正用到的三个字段（`terminals` / `attention` /
 * `closeTerminal`）——多造的部分只会让测试与实现一起漂移。
 */

/** 造一个「终端层」的假值，并交出 `closeTerminal` 供断言。 */
function fakeTerminals(
  terminals: TerminalSession[] = [],
  attention: Record<string, TerminalAttention> = {}
) {
  const closeTerminal = vi.fn(async (id: string) => {
    // `closeTerminal` 会把终端从登记表里摘掉（真实实现如此，`useTerminals` 里
    // 先 `filter` 再发 IPC）。**就地**删掉才是忠实的模拟：摘干净之后，「干净退出」
    // 那条判据不会再命中残留的标签——那正是它「不自愈」的原因，静态的假值会把
    // 这一幕掩盖成「下一轮把残留标签又收了一遍」。
    const index = terminals.findIndex((item) => item.id === id);
    if (index >= 0) terminals.splice(index, 1);
  });
  return {
    fake: { terminals, attention, closeTerminal } as unknown as ReturnType<typeof useTerminals>,
    closeTerminal
  };
}

function terminal(id: string, overrides: Partial<TerminalSession> = {}): TerminalSession {
  return { id, title: id, running: true, providerId: "codex", ...overrides };
}

function session(id: string, title = "会话甲"): AiSession {
  return { id, title, filePath: `/sessions/${id}.jsonl`, messageCount: 3, preview: [] };
}

describe("useWorkspaceTabs 的接线", () => {
  it("选中会话只**打开标签**（历史视图），不再起终端", () => {
    // 这是本批次的一处行为变化：单击会话行曾经等于「恢复并进入终端」。
    const { fake } = fakeTerminals();
    const view = renderHook(({ selected }) => useWorkspaceTabs(fake, selected), {
      initialProps: { selected: null as AiSession | null }
    });

    view.rerender({ selected: session("sess-1") });

    const tab = view.result.current.tabs[0]!;
    expect(view.result.current.activeKey).toBe("sess-1");
    expect(tab).toMatchObject({ key: "sess-1", sessionId: "sess-1", title: "会话甲" });
    // 关键：标签上没有终端——画布因此只会显示历史。
    expect(tab.terminalId).toBeUndefined();
  });

  it("列表刷新给出「同 id、新对象」时不抢走活动标签", () => {
    /*
      `console.selectedSession` 是 `sessions.find(...) ?? null` 的结果——每次列表
      加载（搜索框 250ms 防抖、点「刷新」）都会换一个新对象。effect 若按**对象身份**
      触发，就会重跑一次 `openSessionTab`，而它对「标签已存在」的处理是**激活**那张
      标签：用户在搜索框打字，画布就被拽回更早选中的会话上。

      R12 允许的状态正是这样：正在跑的新会话终端没有 sessionId，选中项仍指向更早的
      会话。判据只能是**会话编号**变没变。
    */
    const { fake } = fakeTerminals();
    const view = renderHook(({ selected }) => useWorkspaceTabs(fake, selected), {
      initialProps: { selected: null as AiSession | null }
    });

    view.rerender({ selected: session("sess-1") });
    expect(view.result.current.activeKey).toBe("sess-1");

    // 起一路新会话终端（还没有 sessionId），用户切到它上面看输出。
    act(() => view.result.current.attachTerminalTab("term-new", undefined, "新会话"));
    expect(view.result.current.activeKey).toBe("term-new");

    // 列表刷新：同一条选中会话换了一个新对象（内容一致，引用不同）。
    view.rerender({ selected: session("sess-1") });

    expect(view.result.current.tabs).toHaveLength(2);
    expect(view.result.current.activeKey).toBe("term-new");
  });

  it("活动标签是没有会话编号的新会话终端时，点**已选中的**那条会话仍能切到它的标签", () => {
    /*
      把 effect 的依赖收窄成会话编号（R31「选中项抢标签」）漏掉了这一角：
      活动标签 = 一路没有会话编号的新会话终端，而选中项已经指着 X（X 的标签在、
      但非激活）。此时点 X 的卡片只写一次**同值**的 `setSelectedId(X)`——依赖没变、
      effect 不跑，标签因此不被激活，用户看到的是「点了没反应」。

      所以点击处必须有一条**显式**激活入口，不能只靠「编号变了」这条副作用。
    */
    const { fake } = fakeTerminals();
    const view = renderHook(({ selected }) => useWorkspaceTabs(fake, selected), {
      initialProps: { selected: null as AiSession | null }
    });

    // X 先被选中：它的标签就此存在。
    view.rerender({ selected: session("sess-x") });
    expect(view.result.current.activeKey).toBe("sess-x");

    // 起一路新会话终端（还没有会话编号）并切到它上面——这就是那个合法状态。
    act(() => view.result.current.attachTerminalTab("term-new", undefined, "新会话"));
    expect(view.result.current.activeKey).toBe("term-new");

    // 选中项仍是 X（同一条会话，编号没变）：effect 不跑，画布停在终端上。
    view.rerender({ selected: session("sess-x") });
    expect(view.result.current.activeKey).toBe("term-new");

    // 点 X 的卡片 = 显式激活：不新建第二张标签，激活键切回 X 那张。
    act(() => view.result.current.selectSession(session("sess-x")));
    expect(view.result.current.activeKey).toBe("sess-x");
    expect(view.result.current.tabs.map((tab) => tab.key)).toEqual(["sess-x", "term-new"]);
  });

  it("同一会话再挂终端：复用同一张标签，并收掉被替换的旧终端", () => {
    // 不收旧终端就是「再次进入会话被锁住」：标签没了，进程与 xterm 还在。
    const { fake, closeTerminal } = fakeTerminals();
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲"));
    expect(view.result.current.tabs).toHaveLength(1);
    expect(view.result.current.tabs[0]).toMatchObject({
      key: "sess-1",
      sessionId: "sess-1",
      terminalId: "term-1"
    });
    expect(view.result.current.activeKey).toBe("sess-1");

    act(() => view.result.current.attachTerminalTab("term-2", "sess-1", "会话甲"));
    expect(view.result.current.tabs).toHaveLength(1);
    expect(view.result.current.tabs[0]!.terminalId).toBe("term-2");
    expect(closeTerminal).toHaveBeenCalledTimes(1);
    expect(closeTerminal).toHaveBeenCalledWith("term-1");
  });

  it("全新会话以终端编号为 key，拿不到会话编号前不挂会话", () => {
    const { fake } = fakeTerminals();
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => view.result.current.attachTerminalTab("term-3", undefined, "新会话"));

    expect(view.result.current.tabs).toEqual([
      { key: "term-3", terminalId: "term-3", title: "新会话" }
    ]);
    // 标签上**没有**会话编号：画布据此走「空态」与终端信息条（那条会话还不存在）。
    expect(view.result.current.tabs[0]!.sessionId).toBeUndefined();
  });

  it("干净退出的终端：标签自动收掉，并通知终端层关闭", () => {
    const { fake } = fakeTerminals([terminal("term-1")]);
    const view = renderHook(({ terminals }) => useWorkspaceTabs(terminals, null), {
      initialProps: { terminals: fake }
    });
    act(() => view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲"));
    expect(view.result.current.tabs).toHaveLength(1);

    // 后端推来「退出码 0」，终端层把这一路标成干净退出。
    const exited = fakeTerminals([
      terminal("term-1", { running: false, exitedCleanly: true, exitCode: 0 })
    ]);
    view.rerender({ terminals: exited.fake });

    expect(view.result.current.tabs).toHaveLength(0);
    expect(exited.closeTerminal).toHaveBeenCalledWith("term-1");
  });

  it("异常退出的终端：标签留着（要看报错），后缀带退出码", () => {
    // 只有干净退出才收。`1`（自己报错）与 `137`（被 kill）指向完全不同的原因。
    const { fake } = fakeTerminals([
      terminal("term-9", { running: false, exitedCleanly: false, exitCode: 137 })
    ]);
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => view.result.current.attachTerminalTab("term-9", "sess-1", "会话甲"));

    expect(view.result.current.tabs).toHaveLength(1);
    expect(view.result.current.tabItems[0]).toMatchObject({
      key: "sess-1",
      label: "会话甲",
      suffix: "（退出码 137）",
      state: "exited",
      tone: "normal"
    });
  });

  it("提醒点按终端编号查：标签 key 是会话编号时也要对得上", () => {
    // 旧模型里标签 key 恰好就是终端编号，所以按 key 查碰巧能work；
    // 统一模型里 key 优先是会话编号，必须改成按 terminalId 查。
    const { fake } = fakeTerminals([terminal("term-2")], {
      "term-2": { kind: "awaiting-confirmation", title: "等确认" }
    });
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => view.result.current.attachTerminalTab("term-2", "sess-1", "会话甲"));

    expect(view.result.current.tabItems[0]).toMatchObject({
      key: "sess-1",
      suffix: "",
      tone: "attention",
      state: "running",
      providerId: "codex"
    });
  });

  /*
   * 下面五条钉的是**同一批次内的并发写入**。
   *
   * 两次 `attachTerminalTab` 可能发生在同一次渲染之后（两次 `await openTerminal`
   * 并发返回、或一次点击与一个挂起的事件回调撞在一起），那时两个调用点持有的是
   * **同一份渲染期快照**。凡是「用快照算好绝对值再覆写」的实现，后一次写都会把前
   * 一次的结果整个盖掉——标签丢了，而被替换的终端因为没人看见它，永远不会被
   * `closeTerminal`（进程与 xterm 泄漏，正是本任务要消灭的东西）。
   */

  it("同一批次内挂两路终端到同一会话：标签不丢，旧终端恰好被收一次", () => {
    const { fake, closeTerminal } = fakeTerminals();
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => {
      view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲");
      view.result.current.attachTerminalTab("term-2", "sess-1", "会话甲");
    });

    expect(view.result.current.tabs).toHaveLength(1);
    expect(view.result.current.tabs[0]).toMatchObject({ key: "sess-1", terminalId: "term-2" });
    // 这一条是 Important 1 的判据：快照写入时 `replacedTerminalId` 恒为 undefined。
    expect(closeTerminal).toHaveBeenCalledTimes(1);
    expect(closeTerminal).toHaveBeenCalledWith("term-1");
  });

  it("同一批次内挂两条不同会话的终端：两张标签都在", () => {
    const { fake } = fakeTerminals();
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => {
      view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲");
      view.result.current.attachTerminalTab("term-2", "sess-2", "会话乙");
    });

    expect(view.result.current.tabs.map((tab) => tab.key)).toEqual(["sess-1", "sess-2"]);
    expect(view.result.current.activeKey).toBe("sess-2");
  });

  it("同一批次内两张标签的终端都干净退出：两张都被收掉", () => {
    const { fake } = fakeTerminals([terminal("term-1"), terminal("term-2")]);
    const view = renderHook(({ terminals }) => useWorkspaceTabs(terminals, null), {
      initialProps: { terminals: fake }
    });
    // 分两批挂（每次一批一张），让这一步与退出无关：这条测试要单独钉「退出收标签」。
    act(() => view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲"));
    act(() => view.result.current.attachTerminalTab("term-2", "sess-2", "会话乙"));
    expect(view.result.current.tabs).toHaveLength(2);

    // 两路终端在同一次渲染里到达「干净退出」。
    const exited = fakeTerminals([
      terminal("term-1", { running: false, exitedCleanly: true, exitCode: 0 }),
      terminal("term-2", { running: false, exitedCleanly: true, exitCode: 0 })
    ]);
    view.rerender({ terminals: exited.fake });

    // 逐个 `closeTab` 会各自基于同一份快照重算 ⇒ 先关的那张被后一次写「复活」，
    // 而且 `closeTerminal` 已把终端移出登记表，判据不再命中、永不自愈。
    expect(view.result.current.tabs).toHaveLength(0);
    expect(exited.closeTerminal).toHaveBeenCalledTimes(2);
    expect(exited.closeTerminal).toHaveBeenCalledWith("term-1");
    expect(exited.closeTerminal).toHaveBeenCalledWith("term-2");
  });

  it("手动关一张带终端的标签：终端也被收掉", () => {
    const { fake, closeTerminal } = fakeTerminals();
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    act(() => view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲"));
    act(() => view.result.current.closeTab("sess-1"));

    expect(view.result.current.tabs).toHaveLength(0);
    expect(closeTerminal).toHaveBeenCalledTimes(1);
    expect(closeTerminal).toHaveBeenCalledWith("term-1");
  });

  it("closeOtherTabs：保留下来的那张成为激活标签", () => {
    // 从**非激活**标签上右键「关闭其他」：不修 activeKey 就会留下一个指向已关标签的
    // 激活键——标签还在，画布却落到空状态。
    const { fake, closeTerminal } = fakeTerminals();
    const view = renderHook(() => useWorkspaceTabs(fake, null));

    // 分两批挂：让这一步与并发写入无关，这条测试单独钉 activeKey 的修正。
    act(() => view.result.current.attachTerminalTab("term-1", "sess-1", "会话甲"));
    act(() => view.result.current.attachTerminalTab("term-2", "sess-2", "会话乙"));
    expect(view.result.current.activeKey).toBe("sess-2");

    act(() => view.result.current.closeOtherTabs("sess-1"));

    expect(view.result.current.tabs.map((tab) => tab.key)).toEqual(["sess-1"]);
    expect(view.result.current.activeKey).toBe("sess-1");
    expect(closeTerminal).toHaveBeenCalledWith("term-2");
  });
});
