import type {
  AiMessage,
  AiSession,
  AiTarget,
  ApiVendor,
  GatewayPortStatus
} from "../types";
import type { StageContent } from "./stage-content";
import type { TerminalSession, TerminalUiState } from "./use-terminals";
import type { WorkspaceTabItem } from "./use-workspace-tabs";
import { TabBar } from "../features/workspace/tab-bar";
import { WorkspaceStage } from "../features/workspace/workspace-stage";

/**
 * 工作区：标签栏 + 画布。
 *
 * # 这一层只负责接线
 *
 * 三块内容各在别处，且都是可以单独读的：
 *
 * | 内容 | 位置 |
 * | --- | --- |
 * | 标签栏（状态点 / 徽章 / 悬停关闭 / 右端动作） | [`TabBar`](../features/workspace/tab-bar.tsx) |
 * | 画布三态（终端 / 历史 / 空） | [`WorkspaceStage`](../features/workspace/workspace-stage.tsx) |
 * | 画布该显示什么（**终端优先**的判据） | [`resolveStageContent`](./stage-content.ts) |
 *
 * 判据不在这里重算：终端优先（两者都在时显示终端）这条规则只写在 `stage-content.ts`
 * 一处，逐条有测试。这一层把 props 原样递下去，不做判断、不持有状态。
 */
export function SessionWorkspace({
  tabItems,
  activeKey,
  stage,
  terminalItems,
  uiStates,
  onOutput,
  registerSink,
  unregisterSink,
  onError,
  onActivate,
  onClose,
  onTabContextMenu,
  onNewSession,
  onFind,
  onOpenMessages,
  canCreate,
  messageCount,
  providerLabel,
  findSignal,
  resolvedTheme,
  target,
  gateway,
  vendors,
  onResumeSession,
  session,
  infoSession,
  messageTotal,
  messages,
  hasEarlier,
  loadingMore,
  onLoadEarlier,
  onBranch,
  onOpenSession,
  branchParent,
  branchChildren
}: {
  tabItems: WorkspaceTabItem[];
  activeKey: string;
  /** 画布当前该显示什么（由 `resolveStageContent` 算出）。 */
  stage: StageContent;
  /** 当前挂着的终端（全部，不只是前台那一个）。 */
  terminalItems: TerminalSession[];
  /**
   * 各终端的输入侧界面状态（输入模式 + 输入框可见性）。
   *
   * 由 [`useTerminals`](./use-terminals.ts) 上收（状态栏也要读它），这里只按编号
   * 透传给对应的 `TerminalPane`。编号缺席时用「自管输入 + 隐藏」，见下面的回退。
   */
  uiStates: Record<string, TerminalUiState>;
  /** 某一路收到了输出（用于把藏着的输入框放出来）。 */
  onOutput: (terminalId: string) => void;
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  onActivate: (key: string) => void;
  onClose: (key: string) => void;
  /** 标签右键菜单（关闭其他 / 关闭全部）。 */
  onTabContextMenu?: (key: string, x: number, y: number) => void;
  /** `＋`：新建会话（走新会话对话框，不是「直接进终端」）。 */
  onNewSession: () => void;
  /** 查找：打开当前终端的查找栏（计数由 `app.tsx` 转发下来）。没有终端时不传。 */
  onFind?: () => void;
  /** 「消息与分支」：打开当前标签那条会话的详情。没有会话时不传。 */
  onOpenMessages?: () => void;
  /** `＋` 能不能点（目标可用性，判据见下面的 `canStartCodex`）。 */
  canCreate: boolean;
  /** 「消息与分支」上的条数；`null` 表示还不知道。 */
  messageCount: number | null;
  /** 平台编号 → 展示名（真源是后端的 `ai:list-providers`）。 */
  providerLabel?: (providerId: string) => string;
  /** 递增即「打开查找栏」。 */
  findSignal: number;
  /** 当前解析出的主题：xterm 的主题跟它走（xterm 读不到 CSS 变量）。 */
  resolvedTheme: "light" | "dark";
  target: AiTarget | null;
  gateway: GatewayPortStatus | null;
  vendors: ApiVendor[];
  /** 历史视图头部的「继续会话」；没有可恢复的东西时不传。 */
  onResumeSession?: () => void;
  /**
   * **历史视图**要的会话：列表的**选中项**（与 `messages` / `messageTotal` 同源）。
   */
  session: AiSession | null;
  /**
   * **终端信息条**要的会话：活动标签真正挂着的那条；新标签还没有编号时是 `null`。
   *
   * 两个来源会分叉（右键菜单的导出 / 打开目录 / 重命名 / 复制只改选中项），
   * 因此分成两个 prop 各自往下递——合成一个必有一边是错的。判据与理由在
   * `app.tsx` 的 `tabSession` 那里。
   */
  infoSession: AiSession | null;
  messageTotal: number | null;
  messages: AiMessage[];
  hasEarlier: boolean;
  loadingMore: boolean;
  onLoadEarlier: () => void;
  /** 从某条消息处分支；带会话编号，见 `HistoryView` 的同名 prop。 */
  onBranch: (sourceLine: number, sessionId: string) => void;
  onOpenSession: (session: AiSession) => void;
  branchParent: (sessionId: string) => AiSession | undefined;
  branchChildren: (parentSessionId: string) => Promise<AiSession[]>;
}) {
  return (
    <section className="workspace">
      <TabBar
        items={tabItems}
        activeKey={activeKey}
        messageCount={messageCount}
        canCreate={canCreate}
        providerLabel={providerLabel}
        onActivate={onActivate}
        onClose={onClose}
        onContextMenu={onTabContextMenu}
        onNewSession={onNewSession}
        onFind={onFind}
        onOpenMessages={onOpenMessages}
      />

      <WorkspaceStage
        stage={stage}
        terminals={terminalItems}
        uiStates={uiStates}
        onOutput={onOutput}
        registerSink={registerSink}
        unregisterSink={unregisterSink}
        onError={onError}
        resolvedTheme={resolvedTheme}
        findSignal={findSignal}
        session={session}
        infoSession={infoSession}
        target={target}
        gateway={gateway}
        vendors={vendors}
        messageTotal={messageTotal}
        messages={messages}
        hasEarlier={hasEarlier}
        loadingMore={loadingMore}
        onLoadEarlier={onLoadEarlier}
        onBranch={onBranch}
        onOpenSession={onOpenSession}
        branchParent={branchParent}
        branchChildren={branchChildren}
        onResumeSession={onResumeSession}
      />
    </section>
  );
}

/** 目标上确实有可用的 Codex 会话入口时才算「能开」。 */
export function canStartCodex(target: AiTarget | null): boolean {
  return Boolean(target && target.available);
}
