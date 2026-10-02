// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TerminalSession } from "../../app/use-terminals";
import { TerminalView } from "./terminal-view";

/**
 * 终端视图到终端栈的映射：全部终端同时挂着、非前台只标 inactive、`hidden` 只收不卸。
 *
 * # 为什么这一层只钉「映射」不钉 xterm
 *
 * 栈里是懒加载的 `TerminalPane`（xterm）：它要真实画布与 `ResizeObserver`，jsdom
 * 两样都没有（见 `terminal-pane.test.tsx` 的说明）。这里把那一层换成空组件，记下
 * 它拿到的 props——xterm 的**行为**由 `terminal-pane.test.tsx` 与真机探针负责。
 *
 * 会话信息条（标题/编号/目标/供应商/网关）2026-10-02 按用户裁定移除；随之删掉的
 * 是它那五条「数据不可得就不渲染」的用例（依赖的信息源与 props 都已不存在）。
 */

/** 空组件替身：一渲染就把 props 记下来。 */
const mounted = vi.hoisted(() => [] as Array<{ id: string; active: boolean }>);
vi.mock("./terminal-pane", () => ({
  TerminalPane: (props: { terminalId: string; active: boolean }) => {
    mounted.push({ id: props.terminalId, active: props.active });
    return null;
  }
}));

const TERMINAL_ID = "7f3c1a2b-9d4e-4f5a-8b6c-000000000001";

function terminal(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: TERMINAL_ID,
    title: "终端自己记的标题",
    running: true,
    providerId: "codex",
    ...overrides
  };
}

/** 最小 props。 */
function props(
  overrides: Partial<Parameters<typeof TerminalView>[0]> = {}
): Parameters<typeof TerminalView>[0] {
  return {
    terminals: [],
    activeTerminalId: "",
    hidden: false,
    uiStates: {},
    onOutput: vi.fn(),
    registerSink: vi.fn(),
    unregisterSink: vi.fn(),
    onError: vi.fn(),
    resolvedTheme: "dark",
    findSignal: 0,
    ...overrides
  };
}

/** 懒加载那一次动态导入在 `act` 里排完，免得它在断言之后才落地（同 `workspace-stage.test.tsx`）。 */
const settle = () => act(async () => {});

beforeEach(() => {
  mounted.length = 0;
});
afterEach(cleanup);

describe("终端栈", () => {
  it("把全部终端都挂着：非前台只标 inactive，不卸载", async () => {
    // 卸载会让 PTY 的输出无人接收、xterm 的缓冲也没了；切回来就断一截。
    render(
      <TerminalView
        {...props({
          terminals: [terminal({ id: "t1" }), terminal({ id: "t2" })],
          activeTerminalId: "t2"
        })}
      />
    );
    await settle();

    const final = new Map(mounted.map((item) => [item.id, item.active]));
    expect([...final.keys()]).toEqual(["t1", "t2"]);
    expect(final.get("t2")).toBe(true);
    expect(final.get("t1")).toBe(false);
  });

  it("hidden 只是给容器加个类：终端照旧挂着，切回来的状态不丢", async () => {
    const view = render(
      <TerminalView {...props({ terminals: [terminal()], activeTerminalId: "", hidden: true })} />
    );
    await settle();
    expect(view.container.querySelector(".terminal-view")!.className).toContain("hidden");
    expect(mounted.map((item) => item.id)).toContain(TERMINAL_ID);
  });
});
