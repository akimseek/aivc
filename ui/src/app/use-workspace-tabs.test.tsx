// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiSession, TerminalAttention } from "../types";
import { hasSessionTab, useWorkspaceTabs } from "./use-workspace-tabs";
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
  const closeTerminal = vi.fn(async () => undefined);
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
    // 导出动作的启用条件看的就是这个（转出的纯函数与标签用的是同一份规则）。
    expect(hasSessionTab(view.result.current.tabs)).toBe(false);
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
});
