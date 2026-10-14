import { EmptyState } from "../../components";
import type { AiMessage, AiSession } from "../../types";
import type { StageContent } from "../../app/stage-content";
import type { TerminalSession, TerminalUiState } from "../../app/use-terminals";
import { HistoryView } from "./history-view";
import { TerminalView } from "./terminal-view";

/**
 * 画布：按 [`resolveStageContent`](../../app/stage-content.ts) 算出的三态渲染。
 *
 * | stage | 显示 |
 * | --- | --- |
 * | `terminal` | 终端视图（终端栈） |
 * | `history` | 历史视图（会话头 + 血缘条 + 消息线程） |
 * | `empty` | 引导态（说清该怎么开始） |
 *
 * # 终端栈在历史阶段**仍然挂载**
 *
 * 这里没有把 `TerminalView` 写成「只有 `terminal` 态才渲染」：终端一旦卸载，
 * xterm 的缓冲就没了、PTY 的输出也无人接收（切回来中间断一截）。因此它**总是**
 * 渲染（只要还有终端），只是 `hidden` 时整块收起来。判据由 `resolveStageContent`
 * 给出（终端优先），这一层不重算。
 *
 * # 会话只有一个来源
 *
 * 历史视图（含消息列表）吃列表的**选中项**（`session`）——消息也是按选中项取的
 * （`use-console` 按 `[targetId, selectedId]` 载），标题与正文必须同源，否则会出现
 * 「S 的标题 + R 的消息」。会话信息条（曾经吃标签自己挂的那条）2026-10-02 已按
 * 用户裁定移除；终端视图本身不再需要会话。
 */
export function WorkspaceStage({
  stage,
  terminals,
  uiStates,
  onOutput,
  onInputFocus,
  registerSink,
  unregisterSink,
  onError,
  resolvedTheme,
  findSignal,
  session,
  messageTotal,
  messages,
  provider = "codex",
  messageOffset = 0,
  branchAnchors = [],
  canBranch = true,
  hasEarlier,
  loadingMore,
  onLoadEarlier,
  onBranch,
  onOpenSession,
  branchParent,
  branchChildren,
  onResumeSession
}: {
  stage: StageContent;
  /** 当前挂着的终端（全部，不只是前台那一个）。 */
  terminals: TerminalSession[];
  uiStates: Record<string, TerminalUiState>;
  onOutput: (terminalId: string) => void;
  onInputFocus?: (terminalId: string, mode: "composer" | "terminal") => void;
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  resolvedTheme: "light" | "dark";
  /** 递增即「打开当前终端的查找栏」。 */
  findSignal: number;
  /**
   * **历史视图**要的会话：当前列表的**选中项**。
   *
   * 消息列表（`messages` / `messageTotal` / `onLoadEarlier`）在 `app.tsx` 里也是
   * 选中态派生的，两者必须同源——只换标题不换正文就会出「S 的标题 + R 的消息」。
   */
  session: AiSession | null;
  messageTotal: number | null;
  messages: AiMessage[];
  provider?: "codex" | "claude" | "gemini" | "qoder";
  messageOffset?: number;
  branchAnchors?: number[];
  canBranch?: boolean;
  hasEarlier: boolean;
  loadingMore: boolean;
  onLoadEarlier: () => void;
  /** 从某条消息处分支；带会话编号（分支必须分到视图里这条会话上）。 */
  onBranch: (sourceLine: number, sessionId: string) => void;
  onOpenSession: (session: AiSession) => void;
  branchParent: (sessionId: string) => AiSession | undefined;
  branchChildren: (parentSessionId: string) => Promise<AiSession[]>;
  onResumeSession?: () => void;
}) {
  return (
    <div className="stage">
      {terminals.length > 0 && (
        <TerminalView
          terminals={terminals}
          activeTerminalId={stage.kind === "terminal" ? stage.terminalId : ""}
          hidden={stage.kind !== "terminal"}
          uiStates={uiStates}
          onOutput={onOutput}
          onInputFocus={onInputFocus}
          registerSink={registerSink}
          unregisterSink={unregisterSink}
          onError={onError}
          resolvedTheme={resolvedTheme}
          findSignal={findSignal}
        />
      )}

      {stage.kind === "history" && (
        <HistoryView
          session={session}
          messageTotal={messageTotal}
          messages={messages}
          provider={provider}
          messageOffset={messageOffset}
          branchAnchors={branchAnchors}
          canBranch={canBranch}
          hasEarlier={hasEarlier}
          loadingMore={loadingMore}
          onLoadEarlier={onLoadEarlier}
          onBranch={onBranch}
          onOpenSession={onOpenSession}
          branchParent={branchParent}
          branchChildren={branchChildren}
          onResume={onResumeSession}
        />
      )}

      {stage.kind === "empty" && (
        <EmptyState
          message="没有打开的会话。"
          hint="从左侧选择一个会话，或新建一个会话。"
        />
      )}
    </div>
  );
}
