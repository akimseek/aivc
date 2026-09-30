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
 * 「按 stage 渲染哪一块」与「两个会话来源各归各的消费方」，不是终端本身。
 * 终端在历史阶段**仍然挂载**这件事由 `workspace-stage.tsx` 的结构保证
 * （`TerminalView` 永远渲染、只是 `hidden`），它的另一半在终端区截图里。
 */
vi.mock("../../app/terminal-pane", () => ({ TerminalPane: () => null }));

const session: AiSession = {
  id: "abcdef12-3456-7890-abcd-ef1234567890",
  title: "会话甲",
  cwd: "D:\\work",
  filePath: "D:\\work\\rollout.jsonl",
  messageCount: 0,
  preview: []
};

/** 与 `session` **不同源**的那条：标签口径（信息条）用它。 */
const infoSession: AiSession = {
  id: "11112222-3333-4444-5555-666677778888",
  title: "会话乙",
  cwd: "D:\\other",
  filePath: "D:\\other\\rollout.jsonl",
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
      infoSession={null}
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

  it("空态说清该怎么开始", () => {
    const view = renderStage({ kind: "empty" });
    expect(view.container.textContent).toContain("没有打开的会话。");
    expect(view.container.textContent).toContain("从左侧选择一个会话，或新建一个会话。");
  });

  it("终端态没有终端时不渲染终端栈（不摆一个空的深色框）", () => {
    const view = renderStage({ kind: "terminal", terminalId: "t1" });
    expect(view.container.querySelector(".terminal-view")).toBeNull();
  });

  it("两个会话来源不同源时，历史视图与信息条各拿各自的那条", async () => {
    /*
      `session`（选中口径）与 `infoSession`（标签口径）会分叉：右键菜单里的
      导出 / 打开目录 / 重命名 / 复制只改选中项、不动标签。合成一个值的话，
      历史视图会出现「标题讲这条、正文讲那条」，而信息条也会顶着别人的标题
      （这正是 T9 那条结转项与本次收口各自要防的事）。
    */
    const view = renderStage(
      { kind: "history", sessionId: session.id },
      {
        infoSession,
        terminals: [
          { id: "t1", title: "终端自己记的标题", running: true, providerId: "codex" }
        ]
      }
    );

    // 历史视图吃 `session`：标题与消息都落在选中那条上。
    expect(view.container.querySelector(".history-head h2")!.textContent).toBe("会话甲");
    expect(view.container.textContent).toContain("正文");

    // 信息条吃 `infoSession`：它讲的是这一路终端属于谁，与画布在看哪条无关。
    const strip = view.container.querySelector(".terminal-info-strip")!;
    expect(strip.querySelector(".strip-title")!.textContent).toBe("会话乙");
    expect(strip.querySelector(".strip-id")!.textContent).toBe(infoSession.id.slice(0, 8));

    // 终端组件是懒加载的（这里换成了占位块）：把那次动态导入的微任务在 act 里排完，
    // 免得它在断言之后才落地，变成一条「不在 act 里」的警告。
    await act(async () => {});
  });
});
