// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TabContextMenu } from "./tab-context-menu";

/**
 * 标签右键菜单。
 *
 * 钉住的是**条目的出现条件**：只剩一张标签时给「关闭其他」是有害的——
 * 它关掉 0 张，用户以为没生效。同理，没有可关的标签时不该出现任何关闭项。
 */

function setup(overrides: Partial<Parameters<typeof TabContextMenu>[0]> = {}) {
  const props = {
    x: 100,
    y: 100,
    tabKey: "t1",
    tabLabel: "终端 1",
    closableCount: 3,
    onCloseThis: vi.fn(),
    onCloseOthers: vi.fn(),
    onCloseAll: vi.fn(),
    onCloseMenu: vi.fn(),
    ...overrides
  };
  const view = render(<TabContextMenu {...props} />);
  const byText = (includes: string) => {
    const found = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes(includes)
    );
    return found as HTMLButtonElement | undefined;
  };
  return { view, props, byText };
}

describe("TabContextMenu", () => {
  it("三个条目都在，且带上会关掉几张", () => {
    const { byText } = setup({ closableCount: 3 });
    expect(byText("关闭这张")).toBeTruthy();
    expect(byText("关闭其他")?.textContent).toContain("2");
    expect(byText("关闭全部")?.textContent).toContain("3");
  });

  it("**只剩一张时不出现「关闭其他」**——它会关掉 0 张，用户以为没生效", () => {
    const { byText } = setup({ closableCount: 1 });
    expect(byText("关闭其他")).toBeUndefined();
    expect(byText("关闭这张")).toBeTruthy();
  });

  it("没有可关的标签时不出现任何关闭项", () => {
    // 给一个点了会失败的动作比不给更糟——用户会以为是自己点错了。
    const { byText, view } = setup({ closableCount: 0 });
    expect(byText("关闭这张")).toBeUndefined();
    expect(byText("关闭全部")).toBeUndefined();
    // 但菜单本身还在（用户点了右键，什么都不显示会更困惑）。
    expect(view.container.querySelector(".context-menu")).toBeTruthy();
  });

  it("点「关闭这张」把这张的 key 递给上层，并关掉菜单", async () => {
    const { byText, props } = setup();
    await act(async () => {
      byText("关闭这张")!.click();
    });
    expect(props.onCloseThis).toHaveBeenCalledWith("t1");
    expect(props.onCloseMenu).toHaveBeenCalled();
  });

  it("点「关闭其他」把这张的 key 递给上层", async () => {
    const { byText, props } = setup();
    await act(async () => {
      byText("关闭其他")!.click();
    });
    expect(props.onCloseOthers).toHaveBeenCalledWith("t1");
  });

  it("「关闭全部」标成危险动作", () => {
    // 它一次关掉所有终端，与「关一张」不是同一量级的操作。
    const { byText } = setup();
    expect(byText("关闭全部")!.className).toContain("danger");
  });

  it("点别处关闭", async () => {
    const { view, props } = setup();
    await act(async () => {
      view.container
        .querySelector(".menu-backdrop")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(props.onCloseMenu).toHaveBeenCalled();
  });

  it("点菜单自身**不**关闭——否则点任何一项之前菜单先关了", async () => {
    const { view, props } = setup();
    await act(async () => {
      view.container
        .querySelector(".context-menu")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(props.onCloseMenu).not.toHaveBeenCalled();
  });

  it("Escape 关闭", async () => {
    const { props } = setup();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(props.onCloseMenu).toHaveBeenCalled();
  });

  it("菜单有可访问名称，说明它是给哪张标签的", () => {
    const { view } = setup({ tabLabel: "重构会话" });
    expect(view.container.querySelector(".context-menu")!.getAttribute("aria-label")).toBe(
      "重构会话 的标签操作"
    );
  });
});
