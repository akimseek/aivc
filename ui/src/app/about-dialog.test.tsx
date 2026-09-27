// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AboutDialog } from "./about-dialog";

/**
 * 「关于」对话框。
 *
 * 最要紧的一条是**版本还没读到时显示 `正在读取`** 而不是空白：版本是异步取回来的，
 * 空白看起来像「读不到」。
 */
describe("AboutDialog", () => {
  const close = (view: ReturnType<typeof render>) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === "关闭"
    );
    if (!found) throw new Error("没有「关闭」按钮");
    return found as HTMLButtonElement;
  };

  it("显示产品名与用途", () => {
    const view = render(<AboutDialog version="0.1.0" onClose={vi.fn()} />);
    const text = view.container.textContent ?? "";
    expect(text).toContain("关于");
    expect(text).toContain("AI 可视化控制台");
    expect(text).toContain("AI CLI 会话与本地 Gateway 管理");
  });

  it("版本还没读到时显示「正在读取」，不是空白", () => {
    const view = render(<AboutDialog version="" onClose={vi.fn()} />);
    expect(view.container.textContent).toContain("正在读取");
  });

  it("读到版本后显示它", () => {
    const view = render(<AboutDialog version="0.1.0" onClose={vi.fn()} />);
    expect(view.container.textContent).toContain("0.1.0");
  });

  it("说明里写明支持哪几个平台", () => {
    // 这段话是用户判断「这个工具管不管我的 CLI」的依据。
    const view = render(<AboutDialog version="0.1.0" onClose={vi.fn()} />);
    const text = view.container.textContent ?? "";
    for (const name of ["Codex", "Claude Code", "Gemini", "Qoder"]) {
      expect(text).toContain(name);
    }
  });

  it("「关闭」与 Escape 都能关掉", () => {
    const onClose = vi.fn();
    const view = render(<AboutDialog version="0.1.0" onClose={onClose} />);

    act(() => {
      close(view).click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
