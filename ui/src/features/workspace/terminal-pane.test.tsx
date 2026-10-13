// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

import { TerminalPane } from "./terminal-pane";

/** 假 xterm 的 `focus()` 调用次数——焦点落点只有它能作证。 */
const focusSpy = vi.hoisted(() => vi.fn());

/** `term.open(el)` 收到的挂载点——它必须是**不带 padding** 的内层（见修复用例）。 */
const openSpy = vi.hoisted(() => vi.fn());

/** 假 xterm 的选区状态。右键快照的相关用例靠它注入「选中了什么」。 */
const termState = vi.hoisted(() => ({ hasSelection: false, selection: "" }));

/** 复制最终写进剪贴板的内容（`clipboard:copy-text` 的载荷）。 */
const copyTextSpy = vi.hoisted(() => vi.fn(async () => null));

// 只把「复制到剪贴板」换成间谍，其余 IPC 保持真实导出（它们在这些用例里不会被调到，
// 但别的组件可能从同一模块具名导入——全量替换成假模块会伤到那条链）。
vi.mock("../../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/commands")>();
  return { ...actual, copyText: copyTextSpy };
});

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
    buffer = {
      active: { type: "normal", baseY: 0, length: 0, getLine: () => null },
      onBufferChange() { return { dispose() {} }; }
    };
    loadAddon() {}
    attachCustomKeyEventHandler() {}
    open(element: unknown) {
      openSpy(element);
    }
    focus() {
      focusSpy();
    }
    write() {}
    hasSelection() {
      return termState.hasSelection;
    }
    getSelection() {
      return termState.selection;
    }
    select() {}
    scrollToLine() {}
    paste() {}
    dispose() {}
    onData() {
      return { dispose() {} };
    }
    onWriteParsed() { return { dispose() {} }; }
    onResize() { return { dispose() {} }; }
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
 * 挂载点的结构（2026-10-02 真机缺陷的回归）。
 *
 * `FitAddon` 按 `term.open(el)` 那个元素的**父元素**量尺寸，且只减 xterm 自身的
 * padding；全站 `box-sizing: border-box` 下父元素的 computed height 是**含 padding
 * 的框高**。挂载点若直接带 `padding: 8px 10px 10px`（原来的 `.terminal-host`），
 * 可用高度被多算 18px → 行数多一档 → 最后一行落在宿主外被 `overflow: hidden` 裁掉
 * ——真机上表现为「终端底部被自管输入框挡住」。
 *
 * 结构固定为：`.terminal-host`（带留白）＞ `.terminal-mount`（无 padding，交给 xterm）。
 * 真实几何由真机探针 `scripts/probes/terminal-layout.js` 钉（jsdom 没有排版）。
 */
describe("终端挂载点结构", () => {
  beforeEach(() => openSpy.mockClear());

  it("xterm 挂在不带 padding 的内层挂载点上", () => {
    render(<TerminalPane {...props()} />);
    expect(openSpy).toHaveBeenCalledTimes(1);
    const mount = openSpy.mock.calls[0]![0] as HTMLElement;
    expect(mount.className).toContain("terminal-mount");
    // 留白那一层在**外面**：挂载点的父元素才是 `.terminal-host`。
    expect(mount.parentElement?.className).toContain("terminal-host");
  });
});

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

describe("TerminalPane 的自管输入草稿", () => {
  it("切换到其他标签时仍保留当前终端的输入内容", () => {
    const view = render(<TerminalPane {...props({ composerVisible: true })} />);
    const area = view.container.querySelector("textarea.composer-text") as HTMLTextAreaElement;
    expect(area).toBeTruthy();

    fireEvent.change(area, { target: { value: "跨标签仍应保留" } });
    view.rerender(<TerminalPane {...props({ active: false, composerVisible: true })} />);

    expect((view.container.querySelector("textarea.composer-text") as HTMLTextAreaElement).value).toBe(
      "跨标签仍应保留"
    );
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

/**
 * 右键复制的快照机制（2026-10-02 实机缺陷的回归）。
 *
 * 真机缺陷：CLI（实测 Codex，鼠标模式 "any"）开着鼠标上报时，右键 mousedown
 * 被 xterm 上报给 PTY，触发它的「用户输入即清选区」——菜单还没弹，选中已经没了，
 * 「复制」因此不可用。完整证据见探针 `scripts/probes/terminal-copy.js`
 * （修复前 V2 复现、修复后转 OK）。这里在 jsdom 里钉住修复的两条：
 *
 * 1. 右键 mousedown 在**捕获阶段**被拦住——宿主内部的监听（模拟 xterm 的）收不到，
 *    既没有上报也就没有清选区；
 * 2. 「复制」写的是**右键那一刻的快照**，实时选区之后被谁清掉都不影响。
 */
describe("TerminalPane 的右键复制（鼠标上报形态的回归）", () => {
  beforeEach(() => {
    // **必须清掉**：`openSpy` 还留着早前用例的挂载点，而那个节点在 `cleanup()` 之后
    // 已经脱离 DOM——往它上面派事件到不了当前组件的处理器（写错一次，红得莫名其妙）。
    openSpy.mockClear();
    termState.hasSelection = false;
    termState.selection = "";
    copyTextSpy.mockClear();
  });

  /** 拿挂载点，并在它内部放一个深层的「事件目标」——真实的右键落在 xterm 的 canvas 上。 */
  const mountWithTarget = () => {
    const mount = openSpy.mock.calls[0]![0] as HTMLElement;
    const deep = document.createElement("div");
    mount.appendChild(deep);
    return { mount, deep };
  };

  const menuCopyButton = (container: HTMLElement) =>
    [...container.querySelectorAll(".context-menu button")].find(
      (item) => item.textContent?.trim() === "复制"
    ) as HTMLButtonElement | undefined;

  it("右键 mousedown/up 在捕获阶段被拦住：宿主内部的监听（xterm 位置的）一个都收不到", () => {
    const view = render(<TerminalPane {...props()} />);
    const { mount, deep } = mountWithTarget();
    let atDeep = 0;
    let bubbleAtMount = 0;
    deep.addEventListener("mousedown", () => {
      atDeep += 1;
    });
    deep.addEventListener("mouseup", () => {
      atDeep += 1;
    });
    mount.addEventListener("mousedown", () => {
      bubbleAtMount += 1;
    });
    mount.addEventListener("mouseup", () => {
      bubbleAtMount += 1;
    });

    fireEvent.mouseDown(deep, { button: 2, clientX: 30, clientY: 40 });
    fireEvent.mouseUp(deep, { button: 2, clientX: 30, clientY: 40 });
    expect(atDeep).toBe(0);
    expect(bubbleAtMount).toBe(0);

    // 菜单照常打开（右键没有被自己人吃掉）。
    fireEvent.contextMenu(deep, { clientX: 30, clientY: 40 });
    expect(view.container.querySelector(".context-menu")).not.toBeNull();
  });

  it("**点击「复制」写的是快照**——哪怕实时选区已被清掉（正是真机缺陷的形态）", async () => {
    termState.hasSelection = true;
    termState.selection = "第一段文本";
    const view = render(<TerminalPane {...props()} />);
    const { deep } = mountWithTarget();

    fireEvent.mouseDown(deep, { button: 2, clientX: 30, clientY: 40 });
    // 模拟鼠标上报的清除效果：mousedown 之后 xterm 的选区已经没了。
    termState.hasSelection = false;
    termState.selection = "";
    fireEvent.contextMenu(deep, { clientX: 30, clientY: 40 });

    const copy = menuCopyButton(view.container)!;
    expect(copy.disabled).toBe(false);
    fireEvent.click(copy);
    await vi.waitFor(() => expect(copyTextSpy).toHaveBeenCalledWith("第一段文本"));
  });

  it("没有选中时「复制」禁用（与快照为空一致）", () => {
    const view = render(<TerminalPane {...props()} />);
    const { deep } = mountWithTarget();
    fireEvent.mouseDown(deep, { button: 2, clientX: 30, clientY: 40 });
    fireEvent.contextMenu(deep, { clientX: 30, clientY: 40 });
    expect(menuCopyButton(view.container)?.disabled).toBe(true);
  });

  it("键盘路径（没有 mousedown）退回读实时选区", async () => {
    termState.hasSelection = true;
    termState.selection = "键盘选中";
    const view = render(<TerminalPane {...props()} />);
    fireEvent.contextMenu(view.container.querySelector(".terminal-mount")!, {
      clientX: 10,
      clientY: 10
    });
    const copy = menuCopyButton(view.container)!;
    expect(copy.disabled).toBe(false);
    fireEvent.click(copy);
    await vi.waitFor(() => expect(copyTextSpy).toHaveBeenCalledWith("键盘选中"));
  });
});
