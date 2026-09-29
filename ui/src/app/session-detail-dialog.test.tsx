// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SessionDetailDialog } from "./session-detail-dialog";

/**
 * 会话详情弹层。
 *
 * # 三条最要紧的规则
 *
 * 1. **关掉不改变选中状态**——它是「停下来看」，不是「切过去」；
 * 2. **分页控件的五个部件都在**（旧平台有「每页数量 / 第一页 / 上一页 / 下一页 / 最后一页」）；
 * 3. **边界页时按钮禁用**——在第 1 页还让点「上一页」，点了没反应会让人以为点错了。
 */

const session = {
  id: "98895b66-4bbd-4d1e-9b3a-000000000001",
  title: "重构会话列表",
  cwd: "D:\\code\\ai",
  filePath: "/sessions/a.jsonl",
  messageCount: 120,
  preview: []
};

function setup(overrides: Partial<Parameters<typeof SessionDetailDialog>[0]> = {}) {
  const props = {
    session: session as never,
    messages: [
      { role: "user" as const, text: "第一句", sourceLine: 3 },
      { role: "assistant" as const, text: "回应", sourceLine: 4 }
    ],
    total: 120,
    page: 1,
    pageSize: 50,
    loading: false,
    onPageChange: vi.fn(),
    onPageSizeChange: vi.fn(),
    onReload: vi.fn(),
    onBranch: vi.fn(),
    onClose: vi.fn(),
    ...overrides
  };
  const view = render(<SessionDetailDialog {...props} />);
  const byText = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  return { view, props, byText };
}

describe("SessionDetailDialog", () => {
  it("显示会话标题、编号与总数", () => {
    const { view } = setup();
    const text = view.container.textContent ?? "";
    expect(text).toContain("重构会话列表");
    expect(text).toContain("98895b66");
    expect(text).toContain("120 条");
  });

  it("五个分页部件都在（旧平台的形状）", () => {
    const { view, byText } = setup();
    expect(view.container.textContent).toContain("每页数量");
    expect(byText("第一页")).toBeTruthy();
    expect(byText("上一页")).toBeTruthy();
    expect(byText("下一页")).toBeTruthy();
    expect(byText("最后一页")).toBeTruthy();
  });

  it("第 1 页时「第一页 / 上一页」禁用——点了没反应会让人以为点错了", () => {
    const { byText } = setup({ page: 1 });
    expect(byText("第一页").disabled).toBe(true);
    expect(byText("上一页").disabled).toBe(true);
    expect(byText("下一页").disabled).toBe(false);
  });

  it("最后一页时「下一页 / 最后一页」禁用", () => {
    const { byText } = setup({ page: 3, total: 120, pageSize: 50 });
    expect(byText("下一页").disabled).toBe(true);
    expect(byText("最后一页").disabled).toBe(true);
    expect(byText("上一页").disabled).toBe(false);
  });

  it("页码显示「第几 / 共几页」", () => {
    const { view } = setup({ page: 2, total: 120, pageSize: 50 });
    expect(view.container.textContent).toContain("第 2 / 3 页");
  });

  it("点「下一页」把页码加一递给上层", async () => {
    const { byText, props } = setup({ page: 2 });
    await act(async () => {
      byText("下一页").click();
    });
    expect(props.onPageChange).toHaveBeenCalledWith(3);
  });

  it("点「最后一页」跳到页数上限", async () => {
    const { byText, props } = setup({ page: 1, total: 120, pageSize: 50 });
    await act(async () => {
      byText("最后一页").click();
    });
    expect(props.onPageChange).toHaveBeenCalledWith(3);
  });

  it("每页数量是可选的下拉，改它会回传", async () => {
    const { view, props } = setup();
    const select = view.container.querySelector("select")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select, "200");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(props.onPageSizeChange).toHaveBeenCalledWith(200);
  });

  it("加载中显示进度，且分页按钮禁用（避免并发翻页）", () => {
    const { view, byText } = setup({ loading: true });
    expect(view.container.textContent).toContain("正在加载完整会话");
    expect(byText("下一页").disabled).toBe(true);
  });

  it("这一页没有消息时说清是「这一页」为空，而不是「会话为空」", () => {
    // 两者的下一步动作不同：一个是往前翻，一个是接受它真的没内容。
    const { view } = setup({ messages: [] });
    expect(view.container.textContent).toContain("这一页没有可显示的消息");
  });

  it("带行号的消息可以「从此处分支」，没行号的不给入口", () => {
    // 后端要靠行号定位原始记录，拿可见序号去截会截错位置。
    const { view } = setup({
      messages: [
        { role: "user" as const, text: "有行号", sourceLine: 7 },
        { role: "user" as const, text: "没行号" }
      ]
    });
    const branchButtons = [...view.container.querySelectorAll("button")].filter(
      (item) => item.textContent?.trim() === "从此处分支"
    );
    expect(branchButtons).toHaveLength(1);
  });

  it("「重新加载」与「关闭」都在，且各自回调", async () => {
    const { byText, props } = setup();
    await act(async () => {
      byText("重新加载").click();
    });
    expect(props.onReload).toHaveBeenCalled();

    await act(async () => {
      byText("关闭").click();
    });
    expect(props.onClose).toHaveBeenCalled();
  });

  it("消息区自己滚，分页控件在它外面", () => {
    // 滚动整块会把分页控件一起滚走，而用户翻页时恰恰要按它。
    const { view } = setup();
    const thread = view.container.querySelector(".detail-thread");
    expect(thread).toBeTruthy();
    expect(thread!.contains(view.container.querySelector(".pagination"))).toBe(false);
  });

  it("Escape 关闭", async () => {
    const { props } = setup();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => expect(props.onClose).toHaveBeenCalled());
  });
});
