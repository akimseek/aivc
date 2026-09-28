import { describe, expect, it } from "vitest";

import { activateSessionTab, deadTabsForSession, type WorkspaceTabLike } from "./workspace-tabs";

/**
 * 标签规则。
 *
 * 最要紧的一条：**已经有标签挂着这个会话时不许再建一个**——
 * 否则点一次历史会话会同时冒出一个会话标签和一个终端标签（这是真实发生过的缺陷）。
 */
describe("activateSessionTab", () => {
  const session = { id: "sess-1", title: "会话甲" };

  it("没有相关标签时建一个会话标签", () => {
    const { tabs, activeKey } = activateSessionTab<WorkspaceTabLike>([], session);
    expect(tabs).toHaveLength(1);
    expect(tabs[0]).toMatchObject({ key: "sess-1", kind: "session", sessionId: "sess-1" });
    expect(activeKey).toBe("sess-1");
  });

  it("**终端标签挂着这个会话时直接切过去，不新建**", () => {
    // 这就是那个缺陷的解药：点历史会话会先起终端（标签 key 是终端编号，
    // 但 `sessionId` 指向这条会话），随后「选中会话」的 effect 不能再建一个。
    const tabs: WorkspaceTabLike[] = [
      { key: "term-9", kind: "terminal", title: "终端", terminalId: "term-9", sessionId: "sess-1" }
    ];
    const result = activateSessionTab(tabs, session);

    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("term-9");
  });

  it("已经有会话标签时切过去，不新建", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "sess-1", kind: "session", title: "会话甲", sessionId: "sess-1" }
    ];
    const result = activateSessionTab(tabs, session);
    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("sess-1");
  });

  it("别的会话的标签不影响：仍然新建自己那条", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "sess-0", kind: "session", title: "会话零", sessionId: "sess-0" }
    ];
    const result = activateSessionTab(tabs, session);
    expect(result.tabs).toHaveLength(2);
    expect(result.activeKey).toBe("sess-1");
  });

  it("用户名下的老标签（只有 key 没有 sessionId）也能认出来", () => {
    // 早期建的会话标签可能只写了 `key === sessionId`，没有 `sessionId` 字段。
    const tabs: WorkspaceTabLike[] = [
      { key: "sess-1", kind: "session", title: "旧标签" }
    ];
    const result = activateSessionTab(tabs, session);
    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("sess-1");
  });
});

/**
 * 「再次进入被锁住」的第二层根因。
 *
 * 干净退出的标签会被收掉，但**异常退出的标签是故意留着的**（要让人看到报错）。
 * 于是再次进入这条会话时，`activateSessionTab` 会切到那一张**已经死掉的**终端标签上，
 * 用户看到的就是不能输入、不再产出的终端。
 *
 * 因此在进入之前要先找出这些死标签——判据是「终端不在了」，不是「标签存在」。
 */
describe("deadTabsForSession", () => {
  const running = (ids: string[]) => (id: string) => ids.includes(id);

  it("终端已经不在跑的标签会被找出来", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "t1", kind: "terminal", title: "死掉的", terminalId: "t1", sessionId: "s1" }
    ];
    expect(deadTabsForSession(tabs, "s1", running([]))).toEqual(["t1"]);
  });

  it("终端还在跑的不算死", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "t1", kind: "terminal", title: "活的", terminalId: "t1", sessionId: "s1" }
    ];
    expect(deadTabsForSession(tabs, "s1", running(["t1"]))).toEqual([]);
  });

  it("只是看正文的会话标签（没有终端）永远不算死", () => {
    // 它本来就没有终端，清掉它会把用户正在看的正文也关掉。
    const tabs: WorkspaceTabLike[] = [
      { key: "s1", kind: "session", title: "会话", sessionId: "s1" }
    ];
    expect(deadTabsForSession(tabs, "s1", running([]))).toEqual([]);
  });

  it("别的会话的死标签不动手", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "t9", kind: "terminal", title: "别人的", terminalId: "t9", sessionId: "s9" }
    ];
    expect(deadTabsForSession(tabs, "s1", running([]))).toEqual([]);
  });

  it("一条会话上多个死标签全都找出来", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "t1", kind: "terminal", title: "死一", terminalId: "t1", sessionId: "s1" },
      { key: "t2", kind: "terminal", title: "活", terminalId: "t2", sessionId: "s1" },
      { key: "t3", kind: "terminal", title: "死二", terminalId: "t3", sessionId: "s1" }
    ];
    expect(deadTabsForSession(tabs, "s1", running(["t2"]))).toEqual(["t1", "t3"]);
  });
});
