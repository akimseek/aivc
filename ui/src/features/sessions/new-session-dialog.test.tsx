// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { NewSessionDialog, type NewSessionDialogProps } from "./new-session-dialog";

/**
 * 新会话对话框的测试。
 *
 * 这个面是用户实际撞到的那一步（H2：「**默认不选择平台、目标**就直接进了终端」），
 * 所以除了字段，**它的两种形态与两个按钮文字**也要钉住——
 * 「流程一致」包含用户看到的字面。
 */

function setup(overrides: Partial<NewSessionDialogProps> = {}) {
  const props: NewSessionDialogProps = {
    pendingResume: null,
    supportsCustomCwd: true,
    cwd: "D:\\work",
    title: "",
    prompt: "",
    cliArgs: "",
    onChooseDirectory: vi.fn(),
    onTitleChange: vi.fn(),
    onPromptChange: vi.fn(),
    onCliArgsChange: vi.fn(),
    onClose: vi.fn(),
    onConfirm: vi.fn(),
    ...overrides
  };
  const view = render(<NewSessionDialog {...props} />);
  const byText = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  return { view, props, byText };
}

describe("NewSessionDialog", () => {
  it("新建形态：三个字段与「打开」", () => {
    const { view, byText } = setup();
    const text = view.container.textContent ?? "";
    for (const label of ["工作目录", "会话标题", "启动提示词", "CLI 参数"]) {
      expect(text).toContain(label);
    }
    expect(byText("打开")).toBeTruthy();
    expect(byText("取消")).toBeTruthy();
  });

  it("字段的占位符与长度上限照抄旧平台", () => {
    // 「流程一致」包含用户看到的字面；少了提示，用户不知道该填什么。
    const { view } = setup();
    const title = view.container.querySelector<HTMLInputElement>("input[maxlength='120']");
    expect(title?.placeholder).toBe("可选，便于在历史列表中识别");

    const prompt = view.container.querySelector<HTMLTextAreaElement>("textarea");
    expect(prompt?.placeholder).toBe("打开新会话后自动发送，可留空");
    expect(prompt?.rows).toBe(4);

    const cliArgs = [...view.container.querySelectorAll("input")].find((item) =>
      item.placeholder.includes("--model")
    );
    expect(cliArgs).toBeTruthy();
  });

  it("恢复形态：只问工作目录，主按钮变成「继续打开」", () => {
    const { view, byText } = setup({
      pendingResume: { missingCwd: "D:\\gone" }
    });
    const text = view.container.textContent ?? "";
    expect(text).toContain("原工作目录不存在");
    expect(text).toContain("请选择新的工作目录后继续打开此历史会话。");
    // 原目录要显示出来，用户才知道自己丢的是哪个。
    // 注意：不能写成 `code[title='D:\gone']` —— CSS 属性选择器里的反斜杠要转义，
    // 直接比对属性值更稳。
    const path = [...view.container.querySelectorAll("code")].find(
      (item) => item.getAttribute("title") === "D:\\gone"
    );
    expect(path?.textContent).toBe("D:\\gone");
    expect(byText("继续打开")).toBeTruthy();
    // 恢复历史会话不该再问标题/提示词/参数——那些是新建时才有的。
    expect(text).not.toContain("会话标题");
    expect(text).not.toContain("启动提示词");
    expect(text).not.toContain("CLI 参数");
  });

  it("不支持自定义工作目录时不显示那一行", () => {
    const { view } = setup({ supportsCustomCwd: false });
    expect(view.container.textContent).not.toContain("工作目录");
  });

  it("改字段会回传，但不自己保存", () => {
    // 这个组件是**受控**的：值由上层持有，确认时才落地。
    const onTitleChange = vi.fn();
    const { view } = setup({ onTitleChange });
    const title = view.container.querySelector<HTMLInputElement>("input[maxlength='120']")!;

    act(() => {
      // React 的受控输入要通过原生 setter 触发，否则 value 不会变。
      const setter = Object.getOwnPropertyDescriptor(
        HTMLInputElement.prototype,
        "value"
      )!.set!;
      setter.call(title, "我的会话");
      title.dispatchEvent(new Event("input", { bubbles: true }));
    });

    expect(onTitleChange).toHaveBeenCalledWith("我的会话");
  });

  it("点遮罩**不**关闭", () => {
    // 旧平台是 `closeOnOverlay={false}`：里面可能有敲了一半的提示词，
    // 点偏一下全丢了是不可接受的。
    const onClose = vi.fn();
    const { view } = setup({ onClose });
    const overlay = view.container.querySelector(".dialog-overlay")!;
    act(() => {
      overlay.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("Escape 关闭，取消关闭", () => {
    const onClose = vi.fn();
    const { byText } = setup({ onClose });
    act(() => {
      byText("取消").click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
