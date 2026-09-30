// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AttentionCenter, type AttentionCenterProps, type AttentionEntry } from "./attention-center";

/**
 * 会话提醒中心。
 *
 * 最要紧的三条：**没有提醒时整个不渲染**、**点条目是「跳过去 + 关掉」**、
 * **未读超过 99 要收成 `99+`**（不然徽标会把布局撑歪）。
 */

function entry(overrides: Partial<AttentionEntry> = {}): AttentionEntry {
  return {
    terminalId: "t1",
    title: "终端 · 甲",
    provider: "codex",
    kind: "awaiting-confirmation",
    detail: "是否继续？",
    ...overrides
  };
}

function setup(overrides: Partial<AttentionCenterProps> = {}) {
  const onSelect = vi.fn();
  const onDismiss = vi.fn();
  const props: AttentionCenterProps = { entries: [], onSelect, onDismiss, ...overrides };
  const view = render(<AttentionCenter {...props} />);
  return { view, onSelect, onDismiss };
}

const open = async (view: ReturnType<typeof render>) => {
  const trigger = view.container.querySelector<HTMLButtonElement>(".attention-trigger")!;
  await act(async () => {
    trigger.click();
  });
};

describe("AttentionCenter", () => {
  it("没有提醒时整个组件不渲染", async () => {
    // 留一个空铃铛只是噪音。
    const { view } = setup();
    expect(view.container.querySelector(".attention-center")).toBeNull();
  });

  it("有提醒时显示未读数，展开后逐条列出", async () => {
    const { view } = setup({ entries: [entry(), entry({ terminalId: "t2", title: "终端 · 乙" })] });
    expect(view.container.querySelector(".attention-count")?.textContent).toBe("2");

    await open(view);
    const text = view.container.textContent ?? "";
    expect(text).toContain("会话提醒");
    expect(text).toContain("2 项待处理");
    expect(text).toContain("终端 · 甲");
    // 每条要说清是哪一家、发生了什么，而不只是一句「有提醒」。
    expect(text).toContain("codex · 等确认：是否继续？");
  });

  it("点条目跳过去并关闭弹层", async () => {
    const { view, onSelect } = setup({ entries: [entry()] });
    await open(view);

    const item = view.container.querySelector<HTMLButtonElement>(".attention-open")!;
    await act(async () => {
      item.click();
    });

    expect(onSelect).toHaveBeenCalledWith("t1");
    await waitFor(() => {
      expect(view.container.querySelector(".attention-popover")).toBeNull();
    });
  });

  it("移除提醒**不**跳转", async () => {
    // 两者是不同的意图：移除是「我知道了，别烦我」，跳转是「带我去看看」。
    const { view, onSelect, onDismiss } = setup({ entries: [entry()] });
    await open(view);

    await act(async () => {
      view.container.querySelector<HTMLButtonElement>(".attention-dismiss")!.click();
    });

    expect(onDismiss).toHaveBeenCalledWith("t1");
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("未读超过 99 收成 99+", async () => {
    const many = Array.from({ length: 120 }, (_, index) =>
      entry({ terminalId: `t${index}`, title: `终端 ${index}` })
    );
    const { view } = setup({ entries: many });
    expect(view.container.querySelector(".attention-count")?.textContent).toBe("99+");
  });

  it("最新的排在前面", async () => {
    // 用户要找的是「刚刚发生了什么」。
    const { view } = setup({
      entries: [
        entry({ terminalId: "old", title: "旧的提醒", updatedAt: 1000 }),
        entry({ terminalId: "new", title: "新的提醒", updatedAt: 2000 })
      ]
    });
    await open(view);

    const titles = [...view.container.querySelectorAll(".attention-open strong")].map(
      (item) => item.textContent
    );
    expect(titles).toEqual(["新的提醒", "旧的提醒"]);
  });

  it("Escape 关闭，点外面关闭", async () => {
    const { view } = setup({ entries: [entry()] });
    await open(view);

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => {
      expect(view.container.querySelector(".attention-popover")).toBeNull();
    });

    await open(view);
    await act(async () => {
      document.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    await waitFor(() => {
      expect(view.container.querySelector(".attention-popover")).toBeNull();
    });
  });

  it("有模态/浮层压在上面时，Escape 让给它——一次按键只收掉最上面那层", async () => {
    // 退出确认框（`.overlay` / `Dialog` 的 `.dialog-overlay`）随时可能从后端推来，
    // 而它盖在铃铛之上。两边都在 `document` 上监听，不判断的话一次 Escape 会
    // 同时收掉提醒弹层与那一次退出询问。
    const { view } = setup({ entries: [entry()] });
    await open(view);

    for (const layer of [".overlay", ".dialog-overlay"]) {
      const modal = document.createElement("div");
      modal.className = layer.slice(1);
      document.body.appendChild(modal);
      try {
        await act(async () => {
          document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
        });
        expect(
          view.container.querySelector(".attention-popover"),
          `${layer} 在场时不该收掉提醒弹层`
        ).toBeTruthy();
      } finally {
        modal.remove();
      }
    }

    // 那一层收掉之后，Escape 仍然是提醒弹层的关闭键。
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => {
      expect(view.container.querySelector(".attention-popover")).toBeNull();
    });
  });
});
