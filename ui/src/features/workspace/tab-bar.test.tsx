// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { WorkspaceTabItem } from "../../app/use-workspace-tabs";
import { TabBar } from "./tab-bar";

/**
 * 标签栏。
 *
 * 钉住四件最容易漏的事：
 *
 * 1. **激活态**由 `on` 类表达（样式靠它，测试也靠它认「哪一张在前台」）；
 * 2. **关闭不会顺带切换标签**——两件事混在一起时用户每次关标签都会被弹到另一张；
 * 3. **消息计数**进的是「消息与分支」按钮，且没有数据时不编数字；
 * 4. **`＋` 在目标不可用时是禁用的**（点得动却必然失败的按钮最糟）。
 */

const items: WorkspaceTabItem[] = [
  {
    key: "session-1",
    label: "会话甲",
    suffix: "",
    tone: "normal",
    providerId: "codex",
    state: "running"
  },
  {
    key: "session-2",
    label: "会话乙",
    suffix: "（退出码 137）",
    tone: "attention",
    state: "exited"
  }
];

function setup(overrides: { canCreate?: boolean; messageCount?: number | null } = {}) {
  const handlers = {
    onActivate: vi.fn(),
    onClose: vi.fn(),
    onContextMenu: vi.fn(),
    onNewSession: vi.fn(),
    onFind: vi.fn(),
    onOpenMessages: vi.fn()
  };
  const view = render(
    <TabBar
      items={items}
      activeKey="session-1"
      messageCount={overrides.messageCount === undefined ? 42 : overrides.messageCount}
      canCreate={overrides.canCreate ?? true}
      providerLabel={(id) => (id === "codex" ? "Claude Code" : id)}
      {...handlers}
    />
  );
  const pills = () => [...view.container.querySelectorAll<HTMLElement>('[role="tab"]')];
  return { view, pills, ...handlers };
}

afterEach(cleanup);

describe("TabBar", () => {
  it("激活的标签带 on 类，其余不带", () => {
    const { pills } = setup();
    expect(pills()[0]!.className).toContain("on");
    expect(pills()[1]!.className).not.toContain("on");
    expect(pills()[0]!.getAttribute("aria-selected")).toBe("true");
  });

  it("点标签切换、点关闭只关闭（不切换）", () => {
    const { pills, onActivate, onClose } = setup();
    fireEvent.click(pills()[1]!);
    expect(onActivate).toHaveBeenCalledWith("session-2");

    const close = pills()[1]!.querySelector<HTMLButtonElement>(".tab-pill-close")!;
    fireEvent.click(close);
    expect(onClose).toHaveBeenCalledWith("session-2");
    // 关闭那一下不该再触发一次激活。
    expect(onActivate).toHaveBeenCalledTimes(1);
  });

  it("键盘可达：Enter/Space 激活，←/→ 在标签间移动焦点", () => {
    const { pills, onActivate } = setup();
    // 只有激活的那张在 Tab 序列里（roving tabindex），否则整条标签栏键盘够不着。
    expect(pills()[0]!.tabIndex).toBe(0);
    expect(pills()[1]!.tabIndex).toBe(-1);
    fireEvent.keyDown(pills()[0]!, { key: "Enter" });
    expect(onActivate).toHaveBeenCalledWith("session-1");

    // 方向键只移动焦点，不激活：切标签会换掉画布，不该在找路时顺手发生。
    pills()[0]!.focus();
    fireEvent.keyDown(pills()[0]!, { key: "ArrowRight" });
    expect(document.activeElement).toBe(pills()[1]);
    expect(onActivate).toHaveBeenCalledTimes(1);

    // 到头绕回另一端。
    fireEvent.keyDown(pills()[1]!, { key: "ArrowRight" });
    expect(document.activeElement).toBe(pills()[0]);
    fireEvent.keyDown(pills()[0]!, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(pills()[1]);

    // Space 与 Enter 等价（键盘上的「点一下」）。
    fireEvent.keyDown(pills()[1]!, { key: " " });
    expect(onActivate).toHaveBeenCalledWith("session-2");
  });

  it("关闭按钮上的键盘事件不被标签抢走", () => {
    // `Enter` 落在关闭按钮上时该关掉这张标签，而不是顺带把它切到前台。
    const { pills, onActivate, onClose } = setup();
    const close = pills()[1]!.querySelector<HTMLButtonElement>(".tab-pill-close")!;
    fireEvent.keyDown(close, { key: "Enter" });
    expect(onActivate).not.toHaveBeenCalled();
    // 关闭按钮自己的点击不受影响。
    fireEvent.click(close);
    expect(onClose).toHaveBeenCalledWith("session-2");
  });

  it("只有激活标签的关闭按钮在 Tab 序里", () => {
    // 否则 Tab 会依次停在每一张标签的 ✕ 上：标签一多，整条标签栏要按十几次才穿得过去。
    const { pills } = setup();
    const closeOf = (index: number) =>
      pills()[index]!.querySelector<HTMLButtonElement>(".tab-pill-close")!;
    expect(closeOf(0).tabIndex).toBe(0);
    expect(closeOf(1).tabIndex).toBe(-1);
  });

  it("状态点区分「跑着」「等确认」与「异常退出」", () => {
    const { pills } = setup();
    const first = pills()[0]!.querySelector(".status-dot-pin")!;
    expect(first.className).toContain("dot-ok");
    expect(first.className).toContain("pulse");

    const second = pills()[1]!.querySelector(".status-dot-pin")!;
    expect(second.className).toContain("dot-warn");
    expect(second.className).not.toContain("pulse");
    // 等确认是读屏看不到的状态点，因此写进标签标题。
    expect(pills()[1]!.getAttribute("title")).toContain("等待确认");
  });

  it("平台徽章用 label 的首段；后缀原样显示", () => {
    const { pills } = setup();
    expect(pills()[0]!.querySelector(".badge")?.textContent).toBe("Claude");
    expect(pills()[1]!.textContent).toContain("（退出码 137）");
  });

  it("消息与分支按钮带上条数，点了打开详情", () => {
    const { view, onOpenMessages } = setup();
    const button = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("消息与分支")
    )!;
    expect(button.textContent).toContain("42");
    fireEvent.click(button);
    expect(onOpenMessages).toHaveBeenCalled();
  });

  it("没有条数时不渲染徽章", () => {
    const { view } = setup({ messageCount: null });
    const button = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("消息与分支")
    )!;
    expect(button.querySelector(".badge")).toBeNull();
  });

  it("＋ 新建会话；目标不可用时禁用", () => {
    const first = setup();
    const plus = () => first.view.container.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')!;
    expect(plus().disabled).toBe(false);
    // 不承诺未实现的快捷键：全仓没有 Ctrl+T 的处理器，写了用户就会以为功能坏了。
    expect(plus().getAttribute("title")).toBe("新建会话");
    fireEvent.click(plus());
    expect(first.onNewSession).toHaveBeenCalled();

    cleanup();
    const second = setup({ canCreate: false });
    const disabledPlus = second.view.container.querySelector<HTMLButtonElement>(
      '[aria-label="新建会话"]'
    )!;
    expect(disabledPlus.disabled).toBe(true);
    fireEvent.click(disabledPlus);
    expect(second.onNewSession).not.toHaveBeenCalled();
  });

  it("查找按钮把「打开搜索」交给调用方；右键带坐标", () => {
    const { view, onFind, onContextMenu, pills } = setup();
    fireEvent.click(view.container.querySelector('[aria-label="查找"]')!);
    expect(onFind).toHaveBeenCalled();

    fireEvent.contextMenu(pills()[0]!, { clientX: 30, clientY: 40 });
    expect(onContextMenu).toHaveBeenCalledWith("session-1", 30, 40);
  });

  it("没有终端可查找时，查找按钮禁用（而不是点了没反应）", () => {
    cleanup();
    const view = render(
      <TabBar
        items={items}
        activeKey="session-1"
        messageCount={null}
        onActivate={vi.fn()}
        onNewSession={vi.fn()}
      />
    );
    const find = view.container.querySelector<HTMLButtonElement>('[aria-label="查找"]')!;
    expect(find.disabled).toBe(true);
    expect(find.getAttribute("title")).toBe("当前标签没有终端");
  });
});
