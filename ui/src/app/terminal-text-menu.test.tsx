// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TerminalPasteDialog, TerminalTextContextMenu } from "./terminal-text-menu";

/**
 * 终端里的文本右键菜单与粘贴确认框。
 *
 * # 为什么终端需要自己的菜单
 *
 * 终端是 `canvas`，浏览器默认菜单对它没有意义，因此必须拦掉；
 * 而一旦拦掉，**复制一段终端输出就只剩「选中 + Ctrl+C」这一条路**——鼠标用户没有出口。
 *
 * 因此这里钉住两件事：**可用性判定**（没选中就不能复制）与
 * **粘贴必须先确认**（终端里的换行等于替用户按回车）。
 */

describe("TerminalTextContextMenu", () => {
  const setup = (canCopy: boolean, canPaste = true) => {
    const handlers = { onCopy: vi.fn(), onPaste: vi.fn(), onDismiss: vi.fn() };
    const view = render(
      <TerminalTextContextMenu
        x={10}
        y={10}
        canCopy={canCopy}
        canPaste={canPaste}
        {...handlers}
      />
    );
    const button = (label: string) =>
      [...view.container.querySelectorAll("button")].find(
        (item) => (item.textContent ?? "").trim() === label
      ) as HTMLButtonElement | undefined;
    return { view, handlers, button };
  };

  it("只有「复制」与「粘贴」两项", () => {
    const { view } = setup(true);
    const labels = [...view.container.querySelectorAll("button")].map((item) =>
      (item.textContent ?? "").trim()
    );
    expect(labels).toEqual(["复制", "粘贴"]);
  });

  it("**没有选中时「复制」不可点**——否则会退化成复制整屏", () => {
    const { button } = setup(false);
    expect(button("复制")!.disabled).toBe(true);
  });

  it("有选中时「复制」可点", () => {
    const { button } = setup(true);
    expect(button("复制")!.disabled).toBe(false);
  });

  it("点了「复制」只调复制，不调粘贴", () => {
    const { button, handlers } = setup(true);
    act(() => {
      button("复制")!.click();
    });
    expect(handlers.onCopy).toHaveBeenCalledTimes(1);
    expect(handlers.onPaste).not.toHaveBeenCalled();
  });

  it("点完之后菜单自己收掉", () => {
    const { button, handlers } = setup(true);
    act(() => {
      button("粘贴")!.click();
    });
    expect(handlers.onDismiss).toHaveBeenCalledTimes(1);
  });

  it("没有可写目标时「粘贴」不可点", () => {
    const { button } = setup(true, false);
    expect(button("粘贴")!.disabled).toBe(true);
  });

  it("按 Escape 收掉菜单", () => {
    const { handlers } = setup(true);
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(handlers.onDismiss).toHaveBeenCalled();
  });
});

describe("TerminalPasteDialog", () => {
  const setup = (text: string) => {
    const handlers = { onCancel: vi.fn(), onConfirm: vi.fn() };
    const view = render(<TerminalPasteDialog text={text} {...handlers} />);
    const button = (label: string) =>
      [...view.container.querySelectorAll("button")].find(
        (item) => (item.textContent ?? "").trim() === label
      ) as HTMLButtonElement;
    return { view, handlers, button };
  };

  it("文案写清「不会自动提交」——这是这个框存在的理由", () => {
    const { view } = setup("a\nb");
    expect(view.container.textContent).toContain("确认粘贴");
    expect(view.container.textContent).toContain("不会自动提交");
  });

  it("把待粘内容**原样**显示出来，让人先看清", () => {
    const { view } = setup("rm -rf /\nnext");
    const textarea = view.container.querySelector("textarea")!;
    expect(textarea.value).toBe("rm -rf /\nnext");
    // 只读：这个框是给人看的，不是给人改的（改了就与实际要写进去的内容不符）。
    expect(textarea.readOnly).toBe(true);
  });

  it("「取消」不粘贴", () => {
    const { button, handlers } = setup("x");
    act(() => {
      button("取消").click();
    });
    expect(handlers.onCancel).toHaveBeenCalledTimes(1);
    expect(handlers.onConfirm).not.toHaveBeenCalled();
  });

  it("「确认粘贴」才真的粘", () => {
    const { button, handlers } = setup("x");
    act(() => {
      button("确认粘贴").click();
    });
    expect(handlers.onConfirm).toHaveBeenCalledTimes(1);
  });

  it("按 Escape 等于取消", () => {
    const { handlers } = setup("x");
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(handlers.onCancel).toHaveBeenCalled();
  });
});
