// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SessionSettingsDialog, type SessionSettingsDialogProps } from "./session-settings-dialog";

/**
 * 「设置会话」对话框。
 *
 * 要紧的是**非 WSL 目标上必须整体禁用**：本机目标不需要手动设置会话目录，
 * 允许输入会让用户以为填了有效。旧平台就是 `disabled = !supportsSessionSettings || !isWsl`。
 */

function setup(overrides: Partial<SessionSettingsDialogProps> = {}) {
  const props: SessionSettingsDialogProps = {
    wslPath: "",
    isWsl: true,
    supportsSessionSettings: true,
    onChange: vi.fn(),
    onClose: vi.fn(),
    onRestore: vi.fn(),
    onSave: vi.fn(),
    ...overrides
  };
  const view = render(<SessionSettingsDialog {...props} />);
  const byText = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  const input = () => view.container.querySelector<HTMLInputElement>("input")!;
  return { view, props, byText, input };
}

describe("SessionSettingsDialog", () => {
  it("WSL 目标：可输入，占位符说明该填什么", () => {
    const { input, byText } = setup({ isWsl: true });
    expect(input().disabled).toBe(false);
    // 占位符直接给一个例子，否则用户不知道这个目录长什么样。
    expect(input().placeholder).toBe("WSL 内 Codex 目录，例如 ~/.codex");
    expect(byText("保存").disabled).toBe(false);
    expect(byText("恢复自动探测").disabled).toBe(false);
  });

  it("非 WSL 目标：输入与两个按钮**全部禁用**，并说明原因", () => {
    const { input, byText, view } = setup({ isWsl: false });
    expect(input().disabled).toBe(true);
    expect(input().placeholder).toBe("当前目标不需要手动设置");
    expect(byText("保存").disabled).toBe(true);
    expect(byText("恢复自动探测").disabled).toBe(true);
    expect(view.container.textContent).toContain("设置会话");
  });

  it("目标本身不支持时同样禁用（即便它是 WSL）", () => {
    // `supportsSessionSettings` 与 `isWsl` 是**两个**条件，缺一不可。
    const { input } = setup({ isWsl: true, supportsSessionSettings: false });
    expect(input().disabled).toBe(true);
  });

  it("改动会回传（受控组件，不自己保存）", () => {
    const onChange = vi.fn();
    const { input } = setup({ onChange });

    act(() => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input(), "~/.codex-alt");
      input().dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(onChange).toHaveBeenCalledWith("~/.codex-alt");
  });

  it("两个按钮各自的意图不混：保存 / 恢复自动探测", () => {
    // 「恢复自动探测」是**清掉手动设置**，不是保存空值——这两件事不能合。
    const onSave = vi.fn();
    const onRestore = vi.fn();
    const { byText } = setup({ onSave, onRestore });

    act(() => {
      byText("保存").click();
    });
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onRestore).not.toHaveBeenCalled();

    act(() => {
      byText("恢复自动探测").click();
    });
    expect(onRestore).toHaveBeenCalledTimes(1);
  });

  it("点遮罩不关闭（可能已经敲了一半路径），Escape 关闭", () => {
    const onClose = vi.fn();
    const { view } = setup({ onClose });

    act(() => {
      view.container
        .querySelector(".dialog-overlay")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
