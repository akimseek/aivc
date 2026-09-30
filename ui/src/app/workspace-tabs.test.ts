import { describe, expect, it } from "vitest";

import {
  attachTerminal,
  closeTab,
  openSessionTab,
  tabSuffix,
  type WorkspaceTabLike
} from "./workspace-tabs";

/**
 * 统一的会话标签模型（spec D3）。
 *
 * 三条最要紧的：**同一会话不建第二张标签**（点一次历史会话曾同时冒出两张）、
 * **终端挂到已有标签上而不是新建**（恢复会话时复用）、
 * **关闭激活标签后切到剩余最后一张**（不留空工作区）。
 */
describe("openSessionTab", () => {
  const session = { id: "sess-1", title: "会话甲" };

  it("没有相关标签时建一个会话标签（key = 会话编号）", () => {
    const { tabs, activeKey } = openSessionTab<WorkspaceTabLike>([], session);
    expect(tabs).toEqual([{ key: "sess-1", sessionId: "sess-1", title: "会话甲" }]);
    expect(activeKey).toBe("sess-1");
  });

  it("已有挂着该会话的终端标签时直接切过去，不新建", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "term-9", sessionId: "sess-1", terminalId: "term-9", title: "会话甲" }
    ];
    const result = openSessionTab(tabs, session);
    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("term-9");
  });

  it("已有会话标签时切过去，不新建", () => {
    const tabs: WorkspaceTabLike[] = [{ key: "sess-1", sessionId: "sess-1", title: "会话甲" }];
    const result = openSessionTab(tabs, session);
    expect(result.tabs).toBe(tabs);
    expect(result.activeKey).toBe("sess-1");
  });

  it("别的会话的标签不影响，仍然新建自己那条", () => {
    const tabs: WorkspaceTabLike[] = [{ key: "sess-0", sessionId: "sess-0", title: "会话零" }];
    const result = openSessionTab(tabs, session);
    expect(result.tabs).toHaveLength(2);
    expect(result.activeKey).toBe("sess-1");
  });
});

describe("attachTerminal", () => {
  it("会话已有标签时把终端挂上去，key 不变，并报告被替换的旧终端", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "sess-1", sessionId: "sess-1", terminalId: "term-old", title: "会话甲" }
    ];
    const result = attachTerminal(tabs, { terminalId: "term-new", sessionId: "sess-1", title: "会话甲" });
    expect(result.activeKey).toBe("sess-1");
    expect(result.tabs[0].terminalId).toBe("term-new");
    expect(result.replacedTerminalId).toBe("term-old");
  });

  it("会话还没有标签时用会话编号为 key 新建（带终端）", () => {
    const result = attachTerminal<WorkspaceTabLike>([], {
      terminalId: "term-1",
      sessionId: "sess-2",
      title: "会话乙"
    });
    expect(result.tabs).toEqual([
      { key: "sess-2", sessionId: "sess-2", terminalId: "term-1", title: "会话乙" }
    ]);
    expect(result.activeKey).toBe("sess-2");
    expect(result.replacedTerminalId).toBeUndefined();
  });

  it("全新会话（还没有会话编号）以终端编号为 key 新建", () => {
    const result = attachTerminal<WorkspaceTabLike>([], { terminalId: "term-3", title: "新会话" });
    expect(result.tabs).toEqual([{ key: "term-3", terminalId: "term-3", title: "新会话" }]);
    expect(result.activeKey).toBe("term-3");
  });

  it("同一终端重复挂载不产生第二张标签", () => {
    const tabs: WorkspaceTabLike[] = [
      { key: "term-3", terminalId: "term-3", title: "新会话" }
    ];
    const result = attachTerminal(tabs, { terminalId: "term-3", title: "新会话" });
    expect(result.tabs).toHaveLength(1);
    expect(result.activeKey).toBe("term-3");
  });
});

describe("closeTab", () => {
  const tabs: WorkspaceTabLike[] = [
    { key: "sess-1", sessionId: "sess-1", title: "甲" },
    { key: "term-2", terminalId: "term-2", title: "乙" }
  ];

  it("关掉激活标签时切到剩余最后一张", () => {
    const result = closeTab(tabs, "term-2", "term-2");
    expect(result.tabs).toHaveLength(1);
    expect(result.activeKey).toBe("sess-1");
  });

  it("关掉非激活标签时激活键不变", () => {
    const result = closeTab(tabs, "sess-1", "term-2");
    expect(result.activeKey).toBe("term-2");
  });

  it("关掉最后一张时激活键清空", () => {
    const result = closeTab([tabs[0]], "sess-1", "sess-1");
    expect(result.tabs).toEqual([]);
    expect(result.activeKey).toBe("");
  });
});

describe("tabSuffix", () => {
  it("运行中不加后缀", () => {
    expect(tabSuffix({ running: true })).toBe("");
    expect(tabSuffix(undefined)).toBe("");
  });

  it("异常退出带退出码——1 与 137 指向完全不同的原因", () => {
    expect(tabSuffix({ running: false, exitedCleanly: false, exitCode: 137 })).toBe("（退出码 137）");
    expect(tabSuffix({ running: false, exitedCleanly: false })).toBe("（退出码 ?）");
  });

  it("干净退出标「已退出」（正常路径下会被自动收掉，这是兜底）", () => {
    expect(tabSuffix({ running: false, exitedCleanly: true })).toBe("（已退出）");
  });
});
