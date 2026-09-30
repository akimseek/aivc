// @vitest-environment jsdom
import { render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiSession } from "../types";
import { BranchPanel, type BranchPanelProps } from "./branch-panel";

/**
 * 血缘条（`根 → 当前 → 子`）。
 *
 * 三条最要紧的：**没有分支时整块不渲染**（大多数会话没有分支，留个空框只占地方）、
 * **父会话解析不出来不能变成错误**（界面没有「按编号取会话」的命令，退回短编号就行）、
 * 以及**子分支点了要打开那个会话**。
 *
 * 形态从纵向改成横向之后，这里锁的是新 DOM：`.lineage-bar` 里三格节点，
 * 锚点（消息序号 / 行号）挂在**当前会话**那一格上。
 */

function session(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: "sess-child",
    title: "子会话",
    filePath: "C:\\sessions\\a.jsonl",
    messageCount: 3,
    preview: [],
    ...overrides
  };
}

function setup(overrides: Partial<BranchPanelProps> = {}) {
  const onOpen = vi.fn();
  const listChildren = vi.fn(async () => [] as AiSession[]);
  const props: BranchPanelProps = { session: session(), listChildren, onOpen, ...overrides };
  const view = render(<BranchPanel {...props} />);
  return { view, onOpen, listChildren };
}

const byText = (view: ReturnType<typeof render>, text: string) => {
  const found = [...view.container.querySelectorAll("button")].find((item) =>
    item.textContent?.includes(text)
  );
  if (!found) throw new Error(`没有按钮 ${text}`);
  return found as HTMLButtonElement;
};

describe("BranchPanel", () => {
  it("没有分支时整块不渲染", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.querySelector(".lineage-bar")).toBeNull();
    });
  });

  it("横着摆成「根会话 → 当前会话」，锚点挂在当前会话上", async () => {
    // 锚点是**一条消息**。行号尤其要紧：后端靠它定位原始记录。
    const { view } = setup({
      session: session({
        metadata: {
          branch: {
            parentSessionId: "parent-1234567890",
            parentMessageIndex: 7,
            parentMessageLine: 42
          }
        }
      })
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("根会话");
    });
    const bar = view.container.querySelector(".lineage-bar")!;
    expect(bar.querySelector(".lineage-node.root")).toBeTruthy();
    const current = bar.querySelector(".lineage-node.current")!;
    expect(current.textContent).toContain("当前会话");
    expect(current.textContent).toContain("消息 7");
    expect(current.textContent).toContain("JSONL 行 42");
  });

  it("父会话解析不出来时退回短编号，而不是报错", async () => {
    const { view } = setup({
      session: session({ metadata: { branch: { parentSessionId: "parent-1234567890" } } })
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("parent-1");
    });
    expect(view.container.querySelector(".lineage-node.root code")).not.toBeNull();
  });

  it("能解析出父会话时给一个可点开的按钮", async () => {
    const parent = session({ id: "parent-1234567890", title: "父会话" });
    const { view, onOpen } = setup({
      session: session({ metadata: { branch: { parentSessionId: parent.id } } }),
      resolveParent: () => parent
    });

    await waitFor(() => {
      expect(view.container.textContent).toContain("父会话");
    });
    await byText(view, "父会话").click();
    expect(onOpen).toHaveBeenCalledWith(parent);
  });

  it("列出子分支，点了打开那个会话并带相对时间", async () => {
    const child = session({ id: "child-1", title: "分出去的那个", updatedAt: "2026-09-27T10:00:00Z" });
    // 注意：要把 spy **传进去**而不是在 `setup` 之后再覆盖——
    // 覆盖掉的那个是普通函数，`toHaveBeenCalledWith` 对它不成立。
    const listChildren = vi.fn(async () => [child]);
    const { view, onOpen } = setup({
      session: session({ metadata: { branch: { parentSessionId: "p1" } } }),
      listChildren
    });

    await waitFor(() => {
      expect(view.container.textContent).toContain("分出去的那个");
    });
    // 拉的是**这条会话的**子分支。
    expect(listChildren).toHaveBeenCalledWith("sess-child");
    // 子分支在当前会话**右边**（DOM 顺序就是阅读顺序）。
    const nodes = [...view.container.querySelectorAll(".lineage-node")];
    expect(nodes.at(-1)!.classList.contains("child")).toBe(true);

    await byText(view, "分出去的那个").click();
    expect(onOpen).toHaveBeenCalledWith(child);
  });

  it("拉子分支失败不把整块变成错误", async () => {
    // 来源那一格仍然有意义，不该因为列不出子分支就什么都不显示。
    const { view } = setup({
      session: session({ metadata: { branch: { parentSessionId: "p1" } } }),
      listChildren: async () => {
        throw new Error("读不出来");
      }
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("当前会话");
    });
    expect(view.container.querySelector(".lineage-bar")).not.toBeNull();
  });
});
