import { EmptyState } from "../../components";
import type { AiMessage, AiSession, AiTarget, ApiVendor, GatewayPortStatus } from "../../types";
import type { StageContent } from "../../app/stage-content";
import type { TerminalSession, TerminalUiState } from "../../app/use-terminals";
import { HistoryView } from "./history-view";
import { TerminalView } from "./terminal-view";

/**
 * 画布：按 [`resolveStageContent`](../../app/stage-content.ts) 算出的三态渲染。
 *
 * | stage | 显示 |
 * | --- | --- |
 * | `terminal` | 终端视图（会话信息条 + 终端栈） |
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
 * # 会话有两个来源，不能合成一个
 *
 * | 消费方 | 来源 | 为什么 |
 * | --- | --- | --- |
 * | 历史视图（含消息列表） | 列表的**选中项**（`session`） | 消息也是按选中项取的（`use-console` 按 `[targetId, selectedId]` 载）——标题与正文必须同源，否则会出现「S 的标题 + R 的消息」 |
 * | 终端信息条 | 标签自己挂的那条（`infoSession`） | 信息条讲的是**这一路终端属于谁**；读选中项会让新终端顶着另一条会话的标题与编号 |
 *
 * 两者在「右键菜单的导出 / 打开目录 / 重命名 / 复制」之后会分叉：那几个动作只改
 * 选中项。合成一个 prop 时同一份值必然有一边是错的。
 */
export function WorkspaceStage({
  stage,
  terminals,
  uiStates,
  onOutput,
  registerSink,
  unregisterSink,
  onError,
  resolvedTheme,
  findSignal,
  session,
  infoSession,
  target,
  gateway,
  vendors,
  messageTotal,
  messages,
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
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  resolvedTheme: "light" | "dark";
  /** 递增即「打开当前终端的查找栏」。 */
  findSignal: number;
  /**
   * **历史视图**要的会话：当前列表的**选中项**（恢复改造前的口径）。
   *
   * 消息列表（`messages` / `messageTotal` / `onLoadEarlier`）在 `app.tsx` 里也是
   * 选中态派生的，两者必须同源——只换标题不换正文就会出「S 的标题 + R 的消息」。
   */
  session: AiSession | null;
  /**
   * **终端信息条**要的会话：活动标签真正挂着的那条；新标签还没有编号时是 `null`
   * （那时信息条退回显示这一路终端自己的标题与编号）。
   *
   * 不能用列表的选中项顶替：新终端那条标签会顶着另一条会话的标题与编号
   * （判据与理由在 `app.tsx` 的 `tabSession` 那里）。
   */
  infoSession: AiSession | null;
  target: AiTarget | null;
  gateway: GatewayPortStatus | null;
  vendors: ApiVendor[];
  messageTotal: number | null;
  messages: AiMessage[];
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
          session={infoSession}
          target={target}
          gateway={gateway}
          vendors={vendors}
          uiStates={uiStates}
          onOutput={onOutput}
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
