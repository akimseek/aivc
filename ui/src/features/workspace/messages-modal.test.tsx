// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiMessage, AiSession } from "../../types";
import { MessagesModal, MessagesModalContainer } from "./messages-modal";

/**
 * 消息与分支弹框。
 *
 * # 五条最要紧的规则
 *
 * 1. **头部件件齐**：`branch` 图标 + 「会话消息历史与分支演进」+ `ID:` 短编号；
 * 2. **血缘条只讲有内容的血缘**：没有父、也没有子时整块不渲染（不留一格空的
 *    「当前会话」）；根会话没有父，照样去拉子分支——根恰恰是最可能有分支的那一条；
 * 3. **分支动作带原始行号回传**——后端要靠它定位原始记录，可见序号会截错位置；
 * 4. **导出三格式都在**（Markdown / JSON / HTML，与右键菜单逐字一致）；
 * 5. **分页与竞态保护沿用 `use-session-detail`**：这里只验「控件把页码/页大小交回上层」，
 *    取数与竞态在 `use-session-detail.test.tsx` 里逐条钉过。
 */

function session(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: "98895b66-4bbd-4d1e-9b3a-000000000001",
    title: "重构会话列表",
    cwd: "D:\\code\\ai",
    filePath: "/sessions/a.jsonl",
    messageCount: 120,
    preview: [],
    ...overrides
  };
}

const messages: AiMessage[] = [
  { role: "user", text: "第一句", sourceLine: 3, timestamp: "2026-09-27T10:00:00Z" },
  { role: "assistant", text: "回应", sourceLine: 4 }
];

function setup(overrides: Partial<Parameters<typeof MessagesModal>[0]> = {}) {
  const props: Parameters<typeof MessagesModal>[0] = {
    session: session(),
    messages,
    branchAnchors: [4],
    total: 120,
    page: 1,
    pageSize: 50,
    loading: false,
    onPageChange: vi.fn(),
    onPageSizeChange: vi.fn(),
    onReload: vi.fn(),
    onBranch: vi.fn(),
    onOpenSession: vi.fn(),
    branchParent: () => undefined,
    branchChildren: async () => [] as AiSession[],
    onExport: vi.fn(),
    onClose: vi.fn(),
    ...overrides
  };
  const view = render(<MessagesModal {...props} />);
  /** 按**恰好等于**这段文字找按钮（分页那五个按钮与「关闭」靠它定位）。 */
  const byText = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  return { view, props, byText };
}

describe("MessagesModal", () => {
  it("列表消息数只是下界且当前页还有数据时，仍可继续翻页并隐藏不准确的末页入口", () => {
    const { view, byText } = setup({ total: 8, totalKnown: false, page: 1, pageHasMore: true });
    expect(view.container.textContent).toContain("第 1 页");
    expect(byText("下一页").disabled).toBe(false);
    expect(view.container.querySelector('button[title="最后一页"]')).toBeNull();
  });

  it("头部有分支图标、标题与 ID 短编号", () => {
    const { view } = setup();
    const head = view.container.querySelector(".messages-head")!;
    expect(head).toBeTruthy();
    const text = head.textContent ?? "";
    expect(text).toContain("会话消息历史与分支演进");
    // 会话自己的标题也在（否则看不出这是哪条会话的历史）。
    expect(text).toContain("重构会话列表");
    expect(text).toContain("ID: 98895b66");
    // 图标是 SVG，不再是字符画（旧实现这里是 emoji 的位置）。
    expect(head.querySelector("svg")).toBeTruthy();
  });

  it("血缘条渲染「根会话 → 当前会话」；无父无子时整块不渲染", async () => {
    const parent = session({ id: "parent-1234567890", title: "父会话" });
    const withParent = setup({
      session: session({
        metadata: {
          branch: {
            parentSessionId: parent.id,
            parentMessageIndex: 7,
            parentMessageLine: 42
          }
        }
      }),
      branchParent: () => parent
    });
    await waitFor(() => {
      expect(withParent.view.container.textContent).toContain("根会话");
    });
    const text = withParent.view.container.querySelector(".lineage-bar")!.textContent ?? "";
    expect(text).toContain("父会话");
    expect(text).toContain("当前会话");
    expect(text).toContain("重构会话列表");
    // 锚点：从父会话的哪一条消息分出来的。
    expect(text).toContain("消息 7");
    expect(text).toContain("JSONL 行 42");

    const alone = setup();
    await waitFor(() => {
      const bar = alone.view.container.querySelector(".lineage-bar");
      expect(bar).toBeNull();
    });
  });

  it("根会话（没有父）也显示子分支", async () => {
    // 一条会话没有父不代表没有分出去过——根恰恰是最可能有子分支的那一条。
    const child = session({ id: "child-1", title: "从根分出来的" });
    const { view } = setup({ branchChildren: async () => [child] });
    await waitFor(() => {
      expect(view.container.textContent).toContain("从根分出来的");
    });
    const bar = view.container.querySelector(".lineage-bar")!;
    expect(bar.querySelector(".lineage-node.root")).toBeNull();
    expect(bar.querySelector(".lineage-node.child")).toBeTruthy();
  });

  it("点子分支回传那一条会话（由上层去打开）", async () => {
    const child = session({ id: "child-1", title: "分出去的那个" });
    const { view, props } = setup({
      session: session({ metadata: { branch: { parentSessionId: "p1" } } }),
      branchChildren: async () => [child]
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("分出去的那个");
    });
    await act(async () => {
      [...view.container.querySelectorAll("button")]
        .find((item) => item.textContent?.includes("分出去的那个"))!
        .click();
    });
    expect(props.onOpenSession).toHaveBeenCalledWith(child);
  });

  it("每轮只在最后一个节点显示「从此处分支」，锚点取最后一条助手回复", async () => {
    const { view, props } = setup({ total: messages.length });
    const branchButtons = [...view.container.querySelectorAll("button")].filter(
      (item) => item.textContent?.trim() === "从此处分支"
    );
    expect(branchButtons).toHaveLength(1);
    await act(async () => {
      branchButtons[0]!.click();
    });
    expect(props.onBranch).toHaveBeenCalledWith(4);
  });

  it("过滤非会话合成节点，并保留每轮的最后节点", () => {
    const { view } = setup({
      total: 4,
      branchAnchors: [8],
      messages: [
        { role: "user", text: "<turn_aborted>" },
        { role: "user", text: "真实问题", sourceLine: 7 },
        { role: "assistant", text: "回答", sourceLine: 8 },
        { role: "tool", text: "工具记录", sourceLine: 9 }
      ]
    });
    expect(view.container.textContent).not.toContain("<turn_aborted>");
    expect(view.container.textContent).toContain("真实问题");
    expect(view.container.textContent).toContain("工具记录");
    expect([...view.container.querySelectorAll("button")].filter((item) => item.textContent?.trim() === "从此处分支")).toHaveLength(1);
  });

  it("没行号的消息不给分支入口", () => {
    // 后端要靠行号定位原始记录，拿可见序号去截会截错位置。
    const { view } = setup({
      messages: [
        { role: "user", text: "有行号", sourceLine: 7 },
        { role: "assistant", text: "有回复" },
        { role: "user", text: "无助手回复", sourceLine: 9 }
      ]
    });
    const branchButtons = [...view.container.querySelectorAll("button")].filter(
      (item) => item.textContent?.trim() === "从此处分支"
    );
    expect(branchButtons).toHaveLength(0);
  });

  it("平台不支持分支时整条消息都不给入口", () => {
    const { view } = setup({ canBranch: false });
    const branchButtons = [...view.container.querySelectorAll("button")].filter(
      (item) => item.textContent?.trim() === "从此处分支"
    );
    expect(branchButtons).toHaveLength(0);
  });

  it("「导出 ▾」的三种格式各自回传", async () => {
    const { view, props, byText } = setup();
    const labels: [string, string][] = [
      ["导出 Markdown", "markdown"],
      ["导出 JSON", "json"],
      ["导出 HTML", "html"]
    ];
    for (const [label, format] of labels) {
      await act(async () => {
        byText("导出").click();
      });
      const menu = view.container.querySelector(".popover")!;
      const button = [...menu.querySelectorAll("button")].find(
        (entry) => entry.textContent?.trim() === label
      )!;
      expect(button, `缺少「${label}」`).toBeTruthy();
      await act(async () => {
        button.click();
      });
      expect(props.onExport).toHaveBeenCalledWith(format);
    }
    // 三种格式都点过；导出**不关**弹框（用户多半还要接着看）。
    expect(props.onExport).toHaveBeenCalledTimes(3);
    expect(props.onClose).not.toHaveBeenCalled();
  });

  it("五个分页部件都在，页码显示「第几 / 共几页」", () => {
    const { view, byText } = setup({ page: 2, total: 120, pageSize: 50 });
    expect(view.container.textContent).toContain("每页数量");
    expect(byText("第一页")).toBeTruthy();
    expect(byText("上一页")).toBeTruthy();
    expect(byText("下一页")).toBeTruthy();
    expect(byText("最后一页")).toBeTruthy();
    expect(view.container.textContent).toContain("第 2 / 3 页");
  });

  it("点分页按钮把页码交给上层", async () => {
    const { props, byText } = setup({ page: 2 });
    await act(async () => {
      byText("下一页").click();
    });
    expect(props.onPageChange).toHaveBeenCalledWith(3);

    await act(async () => {
      byText("第一页").click();
    });
    expect(props.onPageChange).toHaveBeenCalledWith(1);

    await act(async () => {
      byText("最后一页").click();
    });
    expect(props.onPageChange).toHaveBeenCalledWith(3);
  });

  it("第 1 页时「第一页 / 上一页」禁用；最后一页时「下一页 / 最后一页」禁用", () => {
    // 点了没反应会让人以为点错了。
    const first = setup({ page: 1 });
    expect(first.byText("第一页").disabled).toBe(true);
    expect(first.byText("上一页").disabled).toBe(true);
    expect(first.byText("下一页").disabled).toBe(false);

    const last = setup({ page: 3, total: 120, pageSize: 50 });
    expect(last.byText("下一页").disabled).toBe(true);
    expect(last.byText("最后一页").disabled).toBe(true);
    expect(last.byText("上一页").disabled).toBe(false);
  });

  it("每页数量是可选的下拉，改它会回传页大小", async () => {
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
    expect(view.container.textContent).toContain("正在加载本页消息");
    expect(byText("下一页").disabled).toBe(true);
  });

  it("这一页没有消息时说清是「这一页」为空，而不是「会话为空」", () => {
    const { view } = setup({ messages: [] });
    expect(view.container.textContent).toContain("这一页没有可显示的消息");
  });

  it("底部有「共 N 条历史消息」、重新加载与关闭", async () => {
    const { view, props, byText } = setup();
    expect(view.container.textContent).toContain("共 120 条历史消息");

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
    const scroll = view.container.querySelector(".messages-scroll");
    expect(scroll).toBeTruthy();
    expect(scroll!.contains(view.container.querySelector(".messages-pagination"))).toBe(false);
  });

  it("超过 4000 字的消息折叠起来，可展开；正好 4000 字不折叠", async () => {
    // 几万字的日志糊满弹框没法读，但也不能把它丢掉。
    const long = `${"长".repeat(4000)}尾巴`;
    const { view } = setup({ messages: [{ role: "assistant", text: long }] });
    const card = view.container.querySelector(".messages-text")!;
    expect(card.textContent!.endsWith("…")).toBe(true);
    // 折叠是**截断显示**，不是丢掉后半段——展开就能看全。
    expect(card.textContent).not.toContain("尾巴");

    const expand = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("展开全文")
    )!;
    expect(expand).toBeTruthy();
    await act(async () => {
      expand.click();
    });
    expect(view.container.querySelector(".messages-text")!.textContent).toBe(long);

    const exact = setup({ messages: [{ role: "assistant", text: "长".repeat(4000) }] });
    expect(
      [...exact.view.container.querySelectorAll("button")].some((item) =>
        item.textContent?.includes("展开全文")
      )
    ).toBe(false);
  });

  it("Escape 关闭", async () => {
    const { props } = setup();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => expect(props.onClose).toHaveBeenCalled());
  });

  it("有内层浮层（退出确认框）开着时，Escape 让给它", async () => {
    // 退出确认框盖在弹框之上；不判断的话一次 Escape 会把它和整个弹框一起关掉，
    // 用户还没来得及回答「最小化还是退出」。它现在是 `Dialog` 家族的
    // `.dialog-overlay`——自绘那套 `.overlay` 外壳随批次二删除，判定里也不再留它。
    const { props } = setup();
    for (const layer of [".dialog-overlay"]) {
      const confirm = document.createElement("div");
      confirm.className = layer.slice(1);
      document.body.appendChild(confirm);
      try {
        await act(async () => {
          document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
        });
        expect(props.onClose, `${layer} 在场时不该关弹框`).not.toHaveBeenCalled();
      } finally {
        confirm.remove();
      }
    }

    // 内层收掉之后，Escape 仍然是这个弹框的关闭键。
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => expect(props.onClose).toHaveBeenCalled());
  });

  it("角色与时间都在：我 / 助手 + 时间", () => {
    const { view } = setup();
    const items = [...view.container.querySelectorAll(".messages-item")];
    expect(items).toHaveLength(2);
    expect(items[0]!.textContent).toContain("你");
    expect(items[0]!.querySelector(".messages-avatar")!.textContent).toBe("我");
    expect(items[1]!.textContent).toContain("助手");
    expect(items[1]!.querySelector(".messages-avatar")!.textContent).toBe("AI");
    // 时间来自消息自己的 `timestamp`（旧实现没有这一段）。
    expect(items[0]!.querySelector("time")).toBeTruthy();
  });
});

/**
 * 容器那一段接线。
 *
 * `MessagesModalContainer` 此前零测试，而 R30 的会话错配恰恰出在这一段：分支必须
 * 带上**弹框里这条**会话的编号，否则上层只会去读列表的选中项——弹框开在非选中
 * 会话上时（侧栏卡片就能开），分支会分到别的会话上。
 */
describe("MessagesModalContainer 的分支接线", () => {
  it("分支回传的是**弹框里这条**会话的编号，而不是选中项", async () => {
    // 假运行时：详情那一页由它给。这里不碰真的 IPC（也只有这一条命令会被调）。
    window.__TAURI__ = {
      core: {
        invoke: async () => ({
          offset: 0,
          hasMore: false,
          messageCount: 1,
          branchAnchors: [10],
          messages: [
            { role: "user", text: "第一句", sourceLine: 9 },
            { role: "assistant", text: "回答", sourceLine: 10 }
          ]
        })
      }
    };
    const onBranch = vi.fn();
    const modalSession = session({ id: "modal-session", title: "弹框这条" });
    const view = render(
      <MessagesModalContainer
        session={modalSession}
        target={{ id: "codex:local", provider: "codex", label: "本机", kind: "local", available: true }}
        resolveParent={() => undefined}
        listChildren={async () => []}
        onClose={vi.fn()}
        onOpenSession={vi.fn()}
        onBranch={onBranch}
        onExport={vi.fn()}
      />
    );

    await waitFor(() => {
      expect(view.container.textContent).toContain("第一句");
    });
    await act(async () => {
      [...view.container.querySelectorAll("button")]
        .find((item) => item.textContent?.trim() === "从此处分支")!
        .click();
    });
    expect(onBranch).toHaveBeenCalledWith(10, "modal-session", "codex:local");
  });

  it.each(["claude", "gemini"] as const)("%s 翻页后按后端绝对偏移分支，失败时保留弹框", async (provider) => {
    window.__TAURI__ = { core: { invoke: async () => ({
      offset: 100, hasMore: false, messageCount: 102,
      messages: [{ role: "user", text: "第101条" }, { role: "assistant", text: "第102条" }]
    }) } };
    const onBranch = vi.fn().mockResolvedValue(false);
    const onClose = vi.fn();
    const view = render(<MessagesModalContainer session={session({ id: "source" })}
      target={{ id: `${provider}:local`, provider, label: "本机", kind: "local", available: true }}
      resolveParent={() => undefined} listChildren={async () => []}
      onClose={onClose} onOpenSession={vi.fn()} onBranch={onBranch} onExport={vi.fn()} />);
    await waitFor(() => expect(view.container.textContent).toContain("第102条"));
    await act(async () => {
      const buttons = [...view.container.querySelectorAll("button")].filter((button) => button.textContent === "从此处分支");
      buttons[0]!.click();
      buttons[0]!.click();
    });
    expect(onBranch).toHaveBeenCalledTimes(1);
    expect(onBranch).toHaveBeenCalledWith(102, "source", `${provider}:local`);
    expect(onClose).not.toHaveBeenCalled();
    expect(view.container.textContent).toContain("分支未创建");
  });
});
