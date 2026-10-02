// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AiSession } from "../../types";
import type { StageContent } from "../../app/stage-content";
import { WorkspaceStage } from "./workspace-stage";

/**
 * 画布三态。
 *
 * 这里**不放真实的终端组件**：`TerminalPane` 会拉起 xterm，而这一层要钉的是
 * 「按 stage 渲染哪一块」与「历史视图吃列表选中项」，不是终端本身。
 * 终端在历史阶段**仍然挂载**这件事由 `workspace-stage.tsx` 的结构保证
 * （`TerminalView` 永远渲染、只是 `hidden`），它的另一半在终端区截图里。
 *
 * 信息条（吃标签口径的第二条会话）2026-10-02 已按用户裁定移除；原先那条
 * 「两个会话来源各归各的」用例随之失效——分叉只剩历史视图一处消费方。
 */
vi.mock("./terminal-pane", () => ({ TerminalPane: () => null }));

const session: AiSession = {
  id: "abcdef12-3456-7890-abcd-ef1234567890",
  title: "会话甲",
  cwd: "D:\\work",
  filePath: "D:\\work\\rollout.jsonl",
  messageCount: 0,
  preview: []
};

function renderStage(
  stage: StageContent,
  overrides: Partial<Parameters<typeof WorkspaceStage>[0]> = {}
) {
  return render(
    <WorkspaceStage
      stage={stage}
      terminals={[]}
      uiStates={{}}
      onOutput={vi.fn()}
      registerSink={vi.fn()}
      unregisterSink={vi.fn()}
      onError={vi.fn()}
      resolvedTheme="light"
      findSignal={0}
      session={session}
      messageTotal={null}
      messages={[{ role: "assistant", text: "正文" }]}
      hasEarlier={false}
      loadingMore={false}
      onLoadEarlier={vi.fn()}
      onBranch={vi.fn()}
      onOpenSession={vi.fn()}
      branchParent={() => undefined}
      branchChildren={async () => []}
      {...overrides}
    />
  );
}

afterEach(cleanup);

describe("WorkspaceStage", () => {
  it("历史态渲染历史视图", () => {
    const view = renderStage({ kind: "history", sessionId: session.id });
    expect(view.container.querySelector(".history-view")).not.toBeNull();
  });

  it("历史视图吃列表选中项：标题与消息同源", async () => {
    /*
      历史视图的标题与消息列表必须同源（都来自列表选中项）——只换标题不换正文
      就会出「S 的标题 + R 的消息」。
    */
    const view = renderStage({ kind: "history", sessionId: session.id });
    expect(view.container.querySelector(".history-head h2")!.textContent).toBe("会话甲");
    expect(view.container.textContent).toContain("正文");

    // 终端组件是懒加载的（这里换成了占位块）：把那次动态导入的微任务在 act 里排完，
    // 免得它在断言之后才落地，变成一条「不在 act 里」的警告。
    await act(async () => {});
  });

  it("空态说清该怎么开始", () => {
    const view = renderStage({ kind: "empty" });
    expect(view.container.textContent).toContain("没有打开的会话。");
    expect(view.container.textContent).toContain("从左侧选择一个会话，或新建一个会话。");
  });

  it("终端态没有终端时不渲染终端栈（不摆一个空的深色框）", () => {
    const view = renderStage({ kind: "terminal", terminalId: "t1" });
    expect(view.container.querySelector(".terminal-view")).toBeNull();
  });
});
