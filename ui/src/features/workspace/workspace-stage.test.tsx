// @vitest-environment jsdom
import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AiSession } from "../../types";
import type { StageContent } from "../../app/stage-content";
import { WorkspaceStage } from "./workspace-stage";

/**
 * 画布三态。
 *
 * 这里**不放终端**（`terminals: []`）：渲染 `TerminalPane` 会拉起 xterm，
 * 而这一层要钉的是「按 stage 渲染哪一块」，不是终端本身。
 * 终端在历史阶段**仍然挂载**这件事由 `workspace-stage.tsx` 的结构保证
 * （`TerminalView` 永远渲染、只是 `hidden`），它的另一半在终端区截图里。
 */

const session: AiSession = {
  id: "abcdef12-3456-7890-abcd-ef1234567890",
  title: "会话甲",
  cwd: "D:\\work",
  filePath: "D:\\work\\rollout.jsonl",
  messageCount: 0,
  preview: []
};

function renderStage(stage: StageContent) {
  return render(
    <WorkspaceStage
      stage={stage}
      terminals={[]}
      uiStates={{}}
      onInputModeChange={vi.fn()}
      onOutput={vi.fn()}
      registerSink={vi.fn()}
      unregisterSink={vi.fn()}
      onError={vi.fn()}
      resolvedTheme="light"
      findSignal={0}
      session={session}
      target={null}
      gateway={null}
      vendors={[]}
      messageTotal={null}
      messages={[{ role: "assistant", text: "正文" }]}
      hasEarlier={false}
      loadingMore={false}
      onLoadEarlier={vi.fn()}
      onBranch={vi.fn()}
      onOpenSession={vi.fn()}
      branchParent={() => undefined}
      branchChildren={async () => []}
    />
  );
}

afterEach(cleanup);

describe("WorkspaceStage", () => {
  it("历史态渲染历史视图", () => {
    const view = renderStage({ kind: "history", sessionId: session.id });
    expect(view.container.querySelector(".history-view")).not.toBeNull();
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
