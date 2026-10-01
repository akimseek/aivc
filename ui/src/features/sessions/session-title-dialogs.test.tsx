// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SessionDuplicateDialog, SessionRenameDialog } from "./session-title-dialogs";

/**
 * 重命名 / 复制会话的两个对话框。
 *
 * 最要紧的一条是**复制要问名字**（旧平台的字段与提示文字在这里逐字照抄）：
 * 不问的话列表里立刻出现两条一模一样的标题，而用户复制它通常正是为了改出个
 * 不同的名字。
 */

const session = {
  id: "s1",
  title: "重构会话列表",
  filePath: "/a.jsonl",
  messageCount: 3,
  preview: []
};

function setupDialog(node: React.ReactElement) {
  const view = render(node);
  const byText = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  const input = () => view.container.querySelector<HTMLInputElement>("input.control")!;
  const type = async (value: string) => {
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input(), value);
      input().dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  return { view, byText, input, type };
}

describe("SessionRenameDialog", () => {
  it("标题与提示照抄旧平台", () => {
    const { view } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={vi.fn()} onClose={vi.fn()} />
    );
    const text = view.container.textContent ?? "";
    expect(text).toContain("重命名会话");
    expect(text).toContain("会话名称");
  });

  it("**一打开就带着原名字**，用户可以直接改", () => {
    const { input } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={vi.fn()} onClose={vi.fn()} />
    );
    expect(input().value).toBe("重构会话列表");
  });

  it("确认时回传新名字", async () => {
    const onConfirm = vi.fn();
    const { byText, type } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={onConfirm} onClose={vi.fn()} />
    );
    await type("新名字");
    await act(async () => {
      byText("保存").click();
    });
    expect(onConfirm).toHaveBeenCalledWith("新名字");
  });

  it("**空白名字时保存按钮禁用**——改名成空等于把它变成无名会话", async () => {
    const { byText, type } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={vi.fn()} onClose={vi.fn()} />
    );
    await type("   ");
    expect(byText("保存").disabled).toBe(true);
  });

  it("名字两端的空白会被去掉", async () => {
    const onConfirm = vi.fn();
    const { byText, type } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={onConfirm} onClose={vi.fn()} />
    );
    await type("  中间有空格  ");
    await act(async () => {
      byText("保存").click();
    });
    expect(onConfirm).toHaveBeenCalledWith("中间有空格");
  });

  it("回车也能保存", async () => {
    const onConfirm = vi.fn();
    const { input, type } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={onConfirm} onClose={vi.fn()} />
    );
    await type("回车保存");
    await act(async () => {
      input().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    expect(onConfirm).toHaveBeenCalledWith("回车保存");
  });

  it("取消与 Escape 都能关", async () => {
    const onClose = vi.fn();
    const { byText } = setupDialog(
      <SessionRenameDialog session={session as never} onConfirm={vi.fn()} onClose={onClose} />
    );
    await act(async () => {
      byText("取消").click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});

describe("SessionDuplicateDialog", () => {
  it("**必须问名字**，且提示写明留空的行为", () => {
    const { view, input } = setupDialog(
      <SessionDuplicateDialog onConfirm={vi.fn()} onClose={vi.fn()} />
    );
    const text = view.container.textContent ?? "";
    expect(text).toContain("复制会话");
    expect(text).toContain("会话名称（可选）");
    expect(text).toContain("留空则使用原会话标题");
    expect(input().placeholder).toBe("留空则使用原会话标题");
  });

  it("不填名字时回传 `undefined`（表示沿用原标题）", async () => {
    const onConfirm = vi.fn();
    const { byText } = setupDialog(<SessionDuplicateDialog onConfirm={onConfirm} onClose={vi.fn()} />);
    await act(async () => {
      byText("复制").click();
    });
    expect(onConfirm).toHaveBeenCalledWith(undefined);
  });

  it("填了名字就回传它", async () => {
    const onConfirm = vi.fn();
    const { byText, type } = setupDialog(
      <SessionDuplicateDialog onConfirm={onConfirm} onClose={vi.fn()} />
    );
    await type("副本 A");
    await act(async () => {
      byText("复制").click();
    });
    expect(onConfirm).toHaveBeenCalledWith("副本 A");
  });

  it("**「复制」按钮在空名字时也可点**——留空是一个合法选择", () => {
    const { byText } = setupDialog(<SessionDuplicateDialog onConfirm={vi.fn()} onClose={vi.fn()} />);
    expect(byText("复制").disabled).toBe(false);
  });

  it("只填空白也按留空处理", async () => {
    const onConfirm = vi.fn();
    const { byText, type } = setupDialog(
      <SessionDuplicateDialog onConfirm={onConfirm} onClose={vi.fn()} />
    );
    await type("   ");
    await act(async () => {
      byText("复制").click();
    });
    expect(onConfirm).toHaveBeenCalledWith(undefined);
  });
});
