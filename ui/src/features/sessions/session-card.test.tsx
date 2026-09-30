// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiSession } from "../../types";
import { SessionCard } from "./session-card";

/**
 * 会话卡片。
 *
 * # 这组测试为什么值得写
 *
 * 卡片是**一整个可点区域**，里面还嵌着勾选框与三个动作按钮。卡片最难受的错是
 * 「点勾选框顺手把会话打开了」——它在界面上表现为「我明明只是想勾一条」，
 * 而那条会话莫名其妙变成了当前会话。因此这里钉住的第一条是**冒泡**：
 * 勾选框与动作按钮的点击都不许触发 `onOpen`。
 */

function session(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: "s1",
    title: "重构会话侧栏",
    filePath: "C:/x/s1.jsonl",
    messageCount: 12,
    messageCountIsLowerBound: true,
    updatedAt: "2026-09-30T08:30:00",
    preview: [{ role: "assistant", text: "最后一条可显示的消息" }],
    ...overrides
  };
}

function setup(overrides: Partial<Parameters<typeof SessionCard>[0]> = {}) {
  const handlers = {
    onOpen: vi.fn(),
    onToggle: vi.fn(),
    onFavorite: vi.fn(),
    onBranch: vi.fn(),
    onMenu: vi.fn(),
    onResume: vi.fn()
  };
  const props = {
    session: session(),
    active: false,
    checked: false,
    selectable: true,
    running: false,
    ...handlers,
    ...overrides
  };
  const view = render(<SessionCard {...props} />);
  return { view, handlers };
}

describe("会话卡片内容", () => {
  it("三行：标题 / 预览 / 条数，时间在标题行右端", () => {
    const { view } = setup();
    const card = view.container.querySelector(".session-card")!;
    expect(card.querySelector(".session-card-title")!.textContent).toBe("重构会话侧栏");
    expect(card.querySelector(".session-card-preview")!.textContent).toBe("最后一条可显示的消息");
    // 条数是**下界**（`messageCountIsLowerBound`），不能显示成精确值。
    expect(card.querySelector(".session-card-meta")!.textContent).toContain("≥12 条");
    expect(card.querySelector(".session-card-time")!.textContent).not.toBe("");
  });

  it("标题退化成编号时显示「目录 · 时间」（复用 `displayTitle`）", () => {
    const { view } = setup({ session: session({ id: "98895b66", title: "98895b66", cwd: "/w/项目" }) });
    expect(view.container.querySelector(".session-card-title")!.textContent).toContain("项目");
  });

  it("是分支会话时标出「分支」，不是就省略", () => {
    const { view } = setup({
      session: session({ metadata: { branch: { parentSessionId: "parent" } } })
    });
    expect(view.container.querySelector(".session-card-meta")!.textContent).toContain("分支");

    const plain = setup();
    expect(plain.view.container.querySelector(".session-card-meta")!.textContent).not.toContain(
      "分支"
    );
  });

  it("当前会话带 `on` 类；不勾选 `selectable` 时不渲染勾选框（平台能力门控）", () => {
    const { view } = setup({ active: true });
    expect(view.container.querySelector(".session-card")!.className).toContain("on");
    expect(view.container.querySelector(".session-card-check")).not.toBeNull();

    const locked = setup({ selectable: false });
    expect(locked.view.container.querySelector(".session-card-check")).toBeNull();
  });
});

describe("点击的归属", () => {
  it("点卡片本体 → 打开会话", () => {
    const { view, handlers } = setup();
    act(() => (view.container.querySelector(".session-card") as HTMLElement).click());
    expect(handlers.onOpen).toHaveBeenCalledTimes(1);
  });

  it("**点勾选框不会打开会话**（否则「我只是想勾一条」会把当前会话换掉）", () => {
    const { view, handlers } = setup({ checked: false });
    const box = view.container.querySelector(".session-card-check") as HTMLInputElement;
    act(() => box.click());
    expect(handlers.onToggle).toHaveBeenCalledTimes(1);
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("三个动作按钮分别回传，且都不触发 `onOpen`", () => {
    const { view, handlers } = setup();
    const buttons = [...view.container.querySelectorAll<HTMLButtonElement>(".session-card-actions button")];
    expect(buttons).toHaveLength(3);
    act(() => buttons[0]!.click());
    act(() => buttons[1]!.click());
    act(() => buttons[2]!.click());
    expect(handlers.onFavorite).toHaveBeenCalledTimes(1);
    expect(handlers.onBranch).toHaveBeenCalledTimes(1);
    expect(handlers.onMenu).toHaveBeenCalledTimes(1);
    expect(handlers.onOpen).not.toHaveBeenCalled();
  });

  it("「更多」回传的是**点击坐标**——右键菜单按它就着的位置开", () => {
    const { view, handlers } = setup();
    const more = [...view.container.querySelectorAll<HTMLButtonElement>(".session-card-actions button")][2]!;
    act(() => {
      more.dispatchEvent(new MouseEvent("click", { bubbles: true, clientX: 123, clientY: 45 }));
    });
    expect(handlers.onMenu).toHaveBeenCalledWith(123, 45);
  });

  it("双击卡片 → 在终端中恢复（显式手势，不是单击顺带起终端）", () => {
    const { view, handlers } = setup();
    const card = view.container.querySelector(".session-card") as HTMLElement;
    act(() => {
      card.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));
    });
    expect(handlers.onResume).toHaveBeenCalledTimes(1);
  });

  it("右键回传坐标，并阻止浏览器默认菜单", () => {
    const { view, handlers } = setup();
    const card = view.container.querySelector(".session-card") as HTMLElement;
    const event = new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      clientX: 210,
      clientY: 90
    });
    act(() => {
      card.dispatchEvent(event);
    });
    expect(handlers.onMenu).toHaveBeenCalledWith(210, 90);
    expect(event.defaultPrevented).toBe(true);
  });
});
