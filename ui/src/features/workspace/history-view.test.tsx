// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AiMessage, AiSession } from "../../types";
import { HISTORY_MESSAGE_LIMIT } from "../../app/use-console";
import { HistoryView } from "./history-view";

/**
 * 历史视图。
 *
 * 钉住四件事：
 *
 * 1. **三种角色三种形态**（用户引述块 / 助手正文 / 工具等宽块）——它们此前都是
 *    「浅色底 + 边框」的方块，在白色工作区上分不出谁说的；
 * 2. **「继续会话」在头部**（这是显式恢复终端唯一的常驻入口）；
 * 3. **只有带原始行号的消息能分支**——后端靠行号定位，拿可见序号去截会截错；
 * 4. 没有可显示的消息时给一句说明，而不是一片空白。
 */

const session: AiSession = {
  id: "abcdef12-3456-7890-abcd-ef1234567890",
  title: "会话甲",
  cwd: "D:\\work",
  filePath: "D:\\work\\rollout.jsonl",
  messageCount: 3,
  preview: []
};

function message(patch: Partial<AiMessage>): AiMessage {
  return { role: "assistant", text: "内容", ...patch };
}

function setup(overrides: {
  messages?: AiMessage[];
  session?: AiSession | null;
  hasEarlier?: boolean;
  loadingMore?: boolean;
  onResume?: () => void;
  messageTotal?: number | null;
} = {}) {
  const handlers = {
    onLoadEarlier: vi.fn(),
    onBranch: vi.fn(),
    onOpenSession: vi.fn(),
    onResume: overrides.onResume
  };
  const view = render(
    <HistoryView
      session={overrides.session === undefined ? session : overrides.session}
      messageTotal={overrides.messageTotal ?? null}
      messages={
        overrides.messages ?? [
          message({ role: "user", text: "帮我看看这个报错" }),
          message({ role: "assistant", text: "先跑一下测试" }),
          message({ role: "tool", text: "$ npm test", sourceLine: 12 })
        ]
      }
      hasEarlier={overrides.hasEarlier ?? false}
      loadingMore={overrides.loadingMore ?? false}
      {...handlers}
      branchParent={() => undefined}
      branchChildren={async () => []}
    />
  );
  return { view, ...handlers };
}

afterEach(cleanup);

describe("HistoryView", () => {
  it("按角色给三种形态：用户引述块、助手正文、工具等宽块", () => {
    const { view } = setup();
    const blocks = [...view.container.querySelectorAll(".history-message")];
    expect(blocks.map((item) => item.className)).toEqual([
      "history-message user",
      "history-message assistant",
      "history-message tool"
    ]);
    expect(blocks[0]!.querySelector(".history-text")!.textContent).toBe("帮我看看这个报错");
  });

  it("头部显示标题与条数，并带「继续会话」", () => {
    const onResume = vi.fn();
    const { view } = setup({ onResume });
    const head = view.container.querySelector(".history-head")!;
    expect(head.textContent).toContain("会话甲");
    expect(head.textContent).toContain("3 条");

    const resume = [...head.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("继续会话")
    )!;
    expect(resume.getAttribute("title")).toBe("在终端中恢复这个会话");
    fireEvent.click(resume);
    expect(onResume).toHaveBeenCalled();
  });

  it("没有可恢复的东西时**不渲染**继续会话（而不是渲染一个点不动的）", () => {
    const { view } = setup();
    expect(view.container.querySelector(".history-head")!.textContent).not.toContain("继续会话");
  });

  it("只有带原始行号的消息能分支", () => {
    const { view, onBranch } = setup();
    const blocks = [...view.container.querySelectorAll(".history-message")];
    // 前两条没有行号：不渲染分支动作。
    expect(blocks[0]!.querySelector(".history-branch")).toBeNull();
    expect(blocks[1]!.querySelector(".history-branch")).toBeNull();

    fireEvent.click(blocks[2]!.querySelector(".history-branch")!);
    // 行号 + **这条视图的会话编号**：分支要分到正在看的这条上。
    expect(onBranch).toHaveBeenCalledWith(12, session.id);
  });

  it("有更早的消息时才给加载入口，加载中则禁用", () => {
    const first = setup();
    expect(first.view.container.textContent).not.toContain("加载更早的消息");
    cleanup();

    const second = setup({ hasEarlier: true, loadingMore: true });
    const button = [...second.view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("加载中")
    ) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
  });

  it("到累计上限后不给加载入口，改为指路「消息与分支」（P1-J）", () => {
    // 窗口封顶后继续提供按钮只有两种结局：点不动，或把刚加载的旧消息又丢掉。
    // 所以到顶就换成一句说明——完整记录在弹框里（那边有完整分页）。
    const atLimit = Array.from({ length: HISTORY_MESSAGE_LIMIT }, (_, index) =>
      message({ text: `第 ${index} 条` })
    );
    const capped = setup({ messages: atLimit, hasEarlier: true });
    expect(capped.view.container.textContent).not.toContain("加载更早的消息");
    // 文案与常量绑在一条断言里：上限改成别的数，「400 条」这句话必须跟着改。
    expect(capped.view.container.textContent).toContain(
      `最多保留最近 ${HISTORY_MESSAGE_LIMIT} 条`
    );
    expect(capped.view.container.textContent).toContain("消息与分支");
    cleanup();

    // 没到顶时仍然是按钮，提示不出现。
    const below = setup({ hasEarlier: true });
    expect(below.view.container.textContent).toContain("加载更早的消息");
    expect(below.view.container.textContent).not.toContain("最多保留");
  });

  it("没有消息 / 没有会话时各给一句说明", () => {
    const empty = setup({ messages: [] });
    expect(empty.view.container.textContent).toContain("这个会话没有可显示的消息。");
    cleanup();

    const missing = setup({ session: null });
    expect(missing.view.container.textContent).toContain("这条会话已经不在当前列表里。");
  });
});
