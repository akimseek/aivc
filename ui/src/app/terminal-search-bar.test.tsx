// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TerminalSearchBar } from "./terminal-search-bar";

/**
 * 终端内的查找栏。
 *
 * 钉住三件事：
 *
 * 1. **范围提示必须在**——滚出回滚缓冲区的输出搜不到，而用户会以为
 *    「日志里没有这个词」；
 * 2. **跳转会真的把位置交给调用方**——只算出位置不滚过去等于没找到；
 * 3. **走到头会绕回**。
 */

function setup(lines: string[], overrides: { open?: boolean } = {}) {
  const onReveal = vi.fn();
  const onClose = vi.fn();
  const view = render(
    <TerminalSearchBar
      open={overrides.open ?? true}
      readLines={() => lines}
      onReveal={onReveal}
      onClose={onClose}
    />
  );
  const input = () => view.container.querySelector<HTMLInputElement>("input.control")!;
  const button = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.getAttribute("aria-label") === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  const type = async (value: string) => {
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input(), value);
      input().dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  return { view, onReveal, onClose, input, button, type };
}

describe("TerminalSearchBar", () => {
  it("未打开时整块不渲染", () => {
    const { view } = setup(["a"], { open: false });
    expect(view.container.querySelector(".terminal-search")).toBeNull();
  });

  it("**范围提示必须在**——搜不到的原因可能就是被截掉了", () => {
    const { view } = setup(["a"]);
    expect(view.container.textContent).toContain("回滚缓冲区");
  });

  it("打开时聚焦输入框：下一步一定是打字", async () => {
    const { input } = setup(["a"]);
    await waitFor(() => {
      expect(document.activeElement).toBe(input());
    });
  });

  it("输入查询后立刻跳到第一处匹配", async () => {
    const { type, onReveal } = setup(["one", "two", "one again"]);
    await type("one");
    await waitFor(() => {
      expect(onReveal).toHaveBeenCalledWith({ row: 0, column: 0, length: 3 });
    });
  });

  it("显示「第几处 / 共几处」——否则按十次也不知道到头没有", async () => {
    const { view, type } = setup(["a", "a", "a"]);
    await type("a");
    await waitFor(() => {
      expect(view.container.textContent).toContain("1 / 3");
    });
  });

  it("「下一个」跳到第二处，并且**绕回**", async () => {
    const { button, type, onReveal } = setup(["a", "a"]);
    await type("a");
    await waitFor(() => expect(onReveal).toHaveBeenCalledWith({ row: 0, column: 0, length: 1 }));

    await act(async () => {
      button("下一个匹配项").click();
    });
    await waitFor(() => expect(onReveal).toHaveBeenCalledWith({ row: 1, column: 0, length: 1 }));

    await act(async () => {
      button("下一个匹配项").click();
    });
    // 走到头绕回第一处，而不是停住不动。
    await waitFor(() =>
      expect(onReveal).toHaveBeenLastCalledWith({ row: 0, column: 0, length: 1 })
    );
  });

  it("「上一个」从第一处绕到最后一处", async () => {
    const { button, type, onReveal } = setup(["a", "b", "a"]);
    await type("a");
    await act(async () => {
      button("上一个匹配项").click();
    });
    await waitFor(() => expect(onReveal).toHaveBeenLastCalledWith({ row: 2, column: 0, length: 1 }));
  });

  it("没有匹配时说「无匹配」，且上下按钮禁用", async () => {
    const { view, button, type } = setup(["abc"]);
    await type("zzz");
    await waitFor(() => {
      expect(view.container.textContent).toContain("无匹配");
      expect(button("下一个匹配项").disabled).toBe(true);
    });
  });

  it("区分大小写的开关真的生效", async () => {
    const { view, type } = setup(["Error", "error"]);
    await type("error");
    await waitFor(() => expect(view.container.textContent).toContain("1 / 2"));

    await act(async () => {
      const toggle = view.container.querySelector<HTMLInputElement>('input[aria-label="区分大小写"]')!;
      toggle.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("1 / 1"));
  });

  it("全词匹配的开关真的生效", async () => {
    const { view, type } = setup(["foo foobar"]);
    await type("foo");
    await waitFor(() => expect(view.container.textContent).toContain("1 / 2"));

    await act(async () => {
      const toggle = view.container.querySelector<HTMLInputElement>('input[aria-label="全词匹配"]')!;
      toggle.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("1 / 1"));
  });

  it("Enter 下一个、Shift+Enter 上一个", async () => {
    const { input, type, onReveal } = setup(["a", "a"]);
    await type("a");
    await waitFor(() => expect(onReveal).toHaveBeenCalled());

    await act(async () => {
      input().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    await waitFor(() => expect(onReveal).toHaveBeenLastCalledWith({ row: 1, column: 0, length: 1 }));

    await act(async () => {
      input().dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true })
      );
    });
    await waitFor(() => expect(onReveal).toHaveBeenLastCalledWith({ row: 0, column: 0, length: 1 }));
  });

  it("Escape 关闭", async () => {
    const { input, type, onClose } = setup(["a"]);
    await type("a");
    await act(async () => {
      input().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(onClose).toHaveBeenCalled();
  });

  it("关闭按钮能关", async () => {
    const { button, onClose } = setup(["a"]);
    await act(async () => {
      button("关闭搜索").click();
    });
    expect(onClose).toHaveBeenCalled();
  });
});
