// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { TerminalPane } from "./terminal-pane";

/** 假 xterm 的 `focus()` 调用次数——焦点落点只有它能作证。 */
const focusSpy = vi.hoisted(() => vi.fn());

/**
 * `TerminalPane` 的两条**消费式信号**：查找（`findSignal`）与焦点（`focusRequest`）。
 *
 * # 为什么这里要 mock 掉 xterm
 *
 * xterm 要一块真实画布（`getContext("2d")`）与 `ResizeObserver`，jsdom 两样都没有，
 * 直接渲染就抛。而这里要钉的**不是终端本身**，是那两条信号通路：
 * 「计数变化 → 前台那一路开查找栏」「计数变化 → 焦点交给终端」。把 xterm 换成假实例
 * 之后，这两条通路（含 `TerminalSearchBar` 的真实渲染与消失、`document.activeElement`
 * 的落点）都是真跑的。
 *
 * # 它防的是什么
 *
 * 查找那一条曾经是 `if (findSignal <= 0) return;`——**挂载**时那次也执行。
 * 于是用户在点过一次查找（计数 ≥ 1）之后新建的每一路终端都会一挂到前台
 * 就自己弹出查找栏；而查找栏打开即 `focus()` + `select()`，用户在新终端里
 * 敲的第一串字进了搜索框。这几条用例就是那次回归的护栏。焦点请求同理：
 * 挂载时拿到的计数不算一次请求，否则新终端一挂上去就把焦点抢走。
 */

vi.mock("@xterm/xterm", () => {
  class FakeTerminal {
    /** 组件会在主题变化时原地替换它（不重建终端）。 */
    options: { theme?: unknown } = {};
    buffer = { active: { length: 0, getLine: () => null } };
    loadAddon() {}
    open() {}
    focus() {
      focusSpy();
    }
    write() {}
    hasSelection() {
      return false;
    }
    getSelection() {
      return "";
    }
    select() {}
    scrollToLine() {}
    paste() {}
    dispose() {}
    onData() {
      return { dispose() {} };
    }
  }
  return { Terminal: FakeTerminal };
});

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit() {}
  }
}));

interface PaneProps {
  terminalId: string;
  active: boolean;
  running: boolean;
  composerVisible: boolean;
  onOutput: () => void;
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  resolvedTheme: "light" | "dark";
  findSignal?: number;
  focusRequest?: number;
  providerId: string;
  vendorId?: string;
  targetId?: string;
}

function props(overrides: Partial<PaneProps> = {}): PaneProps {
  return {
    terminalId: "t1",
    active: true,
    running: false,
    composerVisible: false,
    onOutput: vi.fn(),
    registerSink: vi.fn(),
    unregisterSink: vi.fn(),
    onError: vi.fn(),
    resolvedTheme: "dark",
    findSignal: 0,
    providerId: "codex",
    ...overrides
  };
}

const searchBar = (container: HTMLElement) => container.querySelector(".terminal-search");

beforeAll(() => {
  // jsdom 没有 ResizeObserver，而终端挂载时要观察宿主尺寸。
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      disconnect() {}
    }
  );
});

afterEach(cleanup);

/**
 * 焦点交给终端的那条通路（状态栏胶囊切到「终端原生输入」时发）。
 *
 * 这里统一用 `composerVisible: true` 把组件**另一条**抢焦点的规则排除掉
 * （`active` 且没有自管输入时才聚焦终端）——不加这个前提，`focus()` 的次数里
 * 会混进挂载那一次，断言就不精确了。
 */
describe("TerminalPane 的焦点请求", () => {
  beforeEach(() => focusSpy.mockClear());

  it("挂载时拿到的请求值不算一次请求", () => {
    render(<TerminalPane {...props({ focusRequest: 3, composerVisible: true })} />);
    expect(focusSpy).not.toHaveBeenCalled();
  });

  it("请求变化时聚焦一次；同一个计数重复渲染不再抢焦点", () => {
    const view = render(<TerminalPane {...props({ focusRequest: 0, composerVisible: true })} />);
    expect(focusSpy).not.toHaveBeenCalled();

    view.rerender(<TerminalPane {...props({ focusRequest: 1, composerVisible: true })} />);
    expect(focusSpy).toHaveBeenCalledTimes(1);

    // 计数没变（父组件因别的原因重渲）不该再抢一次。
    view.rerender(<TerminalPane {...props({ focusRequest: 1, composerVisible: true })} />);
    expect(focusSpy).toHaveBeenCalledTimes(1);
  });

  it("非前台的终端收到请求也不抢焦点", () => {
    // 它仍然挂着（不能卸载，会丢输出），但绝不能把焦点从用户正在用的那一路抢走。
    const view = render(
      <TerminalPane {...props({ active: false, focusRequest: 0, composerVisible: true })} />
    );
    view.rerender(
      <TerminalPane {...props({ active: false, focusRequest: 1, composerVisible: true })} />
    );
    expect(focusSpy).not.toHaveBeenCalled();
  });
});

describe("TerminalPane 的查找信号", () => {
  it("挂载时计数已经 ≥ 1 也不弹（新建终端不该自己开查找栏抢焦点）", () => {
    const view = render(<TerminalPane {...props({ findSignal: 3 })} />);
    expect(searchBar(view.container)).toBeNull();
  });

  it("计数变化时才弹（点一次查找开一次），且焦点落进搜索框", () => {
    const view = render(<TerminalPane {...props({ findSignal: 3 })} />);
    expect(searchBar(view.container)).toBeNull();

    view.rerender(<TerminalPane {...props({ findSignal: 4 })} />);
    expect(searchBar(view.container)).not.toBeNull();
    // 查找栏打开即 `focus()` + `select()`：键盘用户按了 Ctrl+F，下一步一定是打字。
    // 这一条此前只写在注释里（注释声称覆盖「焦点落点」，但没有任何断言）。
    expect(document.activeElement).toBe(
      searchBar(view.container)!.querySelector("input")
    );
  });

  it("非前台收到信号不弹；之后切到前台也不弹，直到再来一次真正的点击", () => {
    const view = render(<TerminalPane {...props({ active: false, findSignal: 1 })} />);

    // 信号变了，但它不在前台：看不见的查找栏没有意义。
    view.rerender(<TerminalPane {...props({ active: false, findSignal: 2 })} />);
    expect(searchBar(view.container)).toBeNull();

    // 切到前台：那一次信号已经消费掉了，不该顺手弹出来。
    view.rerender(<TerminalPane {...props({ active: true, findSignal: 2 })} />);
    expect(searchBar(view.container)).toBeNull();

    // 前台下的下一次点击（计数再变）才算数。
    view.rerender(<TerminalPane {...props({ active: true, findSignal: 3 })} />);
    expect(searchBar(view.container)).not.toBeNull();
  });

  it("手工关掉之后，同一个计数不会把它再弹回来", () => {
    const view = render(<TerminalPane {...props({ findSignal: 0 })} />);
    view.rerender(<TerminalPane {...props({ findSignal: 1 })} />);
    expect(searchBar(view.container)).not.toBeNull();

    fireEvent.click(
      [...view.container.querySelectorAll("button")].find(
        (item) => item.getAttribute("aria-label") === "关闭搜索"
      )!
    );
    expect(searchBar(view.container)).toBeNull();

    // 计数没变，重新渲染（例如父组件因别的原因重渲）不该把它弹回来。
    view.rerender(<TerminalPane {...props({ findSignal: 1 })} />);
    expect(searchBar(view.container)).toBeNull();
  });
});
