import { lazy, Suspense, type ReactNode } from "react";

import { Button, EmptyState, Tabs, ToolbarSpacer, type TabItem } from "../components";
import type { AiMessage, AiSession, AiTarget } from "../types";
import { BranchPanel } from "./branch-panel";
import { messageCountLabel, formatTokens } from "./session-display";
import { Message } from "./session-row";
import type { WorkspaceTab } from "./use-workspace-tabs";

/**
 * 终端**按需加载**。
 *
 * xterm 大约 340 KB，而它只在用户真的打开终端时才需要。静态引入会让每一个用户
 * 在首屏就为它付出代价——实测把主包从 235 KB 顶到 573 KB。
 */
const TerminalPane = lazy(() =>
  import("./terminal-pane").then((module) => ({ default: module.TerminalPane }))
);

/**
 * 工作区：标签条 + 正文。
 *
 * # 为什么把「终端」和「会话正文」放在同一个组件里
 *
 * 它们**同时存在**：所有终端都挂着（非前台的只隐藏不卸载），会话正文也在。
 * 分成两个组件的话就要在父层协调「谁该显示」，而那份协调恰恰是最容易出错的地方
 * （此前真的错过一次：进了终端就把整个正文区换掉，用户看不到消息了）。
 *
 * 这里把「三种正文怎么切换」一次说清：
 *
 * | 当前标签 | 正文显示 |
 * | --- | --- |
 * | 终端 | 终端栈（会话正文整块不渲染） |
 * | 会话且没有会话标签 | 空状态（说明该怎么开始） |
 * | 会话 | 会话头 + 分支面板 + 消息线程 |
 */
export function SessionWorkspace({
  tabItems,
  activeKey,
  activeTab,
  terminalItems,
  registerSink,
  unregisterSink,
  onError,
  onActivate,
  onClose,
  trailing,
  session,
  messageTotal,
  messages,
  hasEarlier,
  loadingMore,
  onLoadEarlier,
  onBranch,
  onOpenSession,
  branchChildren,
  branchParent,
  sessionActions
}: {
  tabItems: TabItem[];
  activeKey: string;
  activeTab: WorkspaceTab | null;
  /** 当前挂着的终端（全部，不只是前台那一个）。`sessionId` 决定自管输入的初始可见性。 */
  terminalItems: { id: string; running: boolean; sessionId?: string }[];
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  onActivate: (key: string) => void;
  onClose: (key: string) => void;
  trailing: ReactNode;
  session: AiSession | null;
  messageTotal: number | null;
  messages: AiMessage[];
  hasEarlier: boolean;
  loadingMore: boolean;
  onLoadEarlier: () => void;
  onBranch: (sourceLine: number) => void;
  onOpenSession: (session: AiSession) => void;
  branchParent: (sessionId: string) => AiSession | undefined;
  branchChildren: (parentSessionId: string) => Promise<AiSession[]>;
  /**
   * 会话头右侧的动作（重命名 / 复制 / 导出…）以及重命名时替换标题的输入框。
   *
   * 做成插槽而不是在这里实现：那些动作要调 IPC，属于「接线」，
   * 而本组件只负责**它们显示在哪**。
   */
  sessionActions: ReactNode;
}) {
  return (
    <section className="workspace">
      <Tabs
        items={tabItems}
        activeKey={activeKey}
        onActivate={onActivate}
        onClose={onClose}
        trailing={trailing}
      />

      <div className="workspace-body">
        {/* 所有终端**同时挂载**、叠在同一个位置上；非前台的只隐藏不卸载——
            卸载会丢掉 xterm 的缓冲，切回来时历史就没了。 */}
        {terminalItems.length > 0 && (
          <div className={`terminal-stack${activeTab?.kind === "terminal" ? "" : " hidden"}`}>
            <Suspense fallback={<div className="terminal-loading">正在加载终端…</div>}>
              {terminalItems.map((item) => (
                <TerminalPane
                  key={item.id}
                  terminalId={item.id}
                  active={activeTab?.kind === "terminal" && activeTab.terminalId === item.id}
                  running={item.running}
                  /* 自管输入的初始可见性由「是不是恢复历史会话」决定。 */
                  sessionId={item.sessionId}
                  registerSink={registerSink}
                  unregisterSink={unregisterSink}
                  onError={onError}
                />
              ))}
            </Suspense>
          </div>
        )}

        {activeTab?.kind === "terminal" ? null : (
          <SessionBody
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
            sessionActions={sessionActions}
          />
        )}
      </div>
    </section>
  );
}

/**
 * 会话正文。
 *
 * 没有会话标签时给空状态而不是空白：用户需要知道「从这里怎么开始」。
 * 三个入口（左侧选会话 / 新建终端 / 新建 Codex）都写在提示里。
 */
function SessionBody({
  session,
  messageTotal,
  messages,
  hasEarlier,
  loadingMore,
  onLoadEarlier,
  onBranch,
  onOpenSession,
  branchParent,
  branchChildren,
  sessionActions
}: {
  session: AiSession | null;
  messageTotal: number | null;
  messages: AiMessage[];
  hasEarlier: boolean;
  loadingMore: boolean;
  onLoadEarlier: () => void;
  onBranch: (sourceLine: number) => void;
  onOpenSession: (session: AiSession) => void;
  branchParent: (sessionId: string) => AiSession | undefined;
  branchChildren: (parentSessionId: string) => Promise<AiSession[]>;
  sessionActions: ReactNode;
}) {
  if (!session) {
    return (
      <EmptyState
        message="没有打开的会话。"
        hint="从左侧选择一个会话，或新建一个终端。"
      />
    );
  }

  return (
    <>
      <div className="session-head">
        <h2 title={session.title}>{sessionLabel(session)}</h2>
        <span className="meta">
          {/* 优先用**详情路径**带回来的精确总数；它还没到时回退到列表的条数，
              并如实标注那是下界。 */}
          {messageCountLabel(messageTotal ?? session.messageCount, messageTotal === null)}
        </span>
        {/* 工作台要点：上下文占用、累计 token、模型。全都在会话自带的数据里。 */}
        {session.usage && <UsagePills usage={session.usage} model={session.model} />}
        <ToolbarSpacer />
        {sessionActions}
      </div>

      {/* 分支关系。放在正文**上方**、会话头下方：
          它讲的是「这条会话从哪来、分出去了哪些」，属于这条会话的元信息。 */}
      <BranchPanel
        session={session}
        resolveParent={branchParent}
        listChildren={branchChildren}
        onOpen={onOpenSession}
      />

      <div className="thread">
        {/* 往前翻的入口放在**顶部**：更早的消息在视觉上就在上面。 */}
        {hasEarlier && (
          <div className="load-earlier">
            <Button small disabled={loadingMore} onClick={onLoadEarlier}>
              {loadingMore ? "加载中…" : "加载更早的消息"}
            </Button>
          </div>
        )}

        {messages.map((message, index) => (
          <Message
            key={`${message.sourceLine ?? index}`}
            message={message}
            onBranch={
              // 只有带原始行号的消息才能分支——后端要靠行号定位到原始记录，
              // 拿可见序号去截会截错位置。
              message.sourceLine === undefined ? undefined : () => onBranch(message.sourceLine!)
            }
          />
        ))}

        {messages.length === 0 && (
          <EmptyState
            message="这个会话没有可显示的消息。"
            hint="文件可能是空的，或消息都被解析规则过滤掉了。"
          />
        )}
      </div>
    </>
  );
}

/** 会话头里的标题。标题等于编号时退回「目录 · 时间」——UUID 区分不出会话。 */
function sessionLabel(session: AiSession): string {
  return session.title?.trim() && session.title !== session.id
    ? session.title
    : (session.cwd ?? "未命名会话");
}

/** 上下文占用 / token / 模型。缺哪项就不显示哪项，不写 `undefined`。 */
function UsagePills({ usage, model }: { usage: NonNullable<AiSession["usage"]>; model?: string }) {
  return (
    <span className="workbench">
      {usage.contextPercent !== undefined && (
        <span title="上下文占用">
          上下文 <b>{usage.contextPercent}%</b>
        </span>
      )}
      {usage.total?.totalTokens !== undefined && (
        <span title="累计 token">
          <b>{formatTokens(usage.total.totalTokens)}</b> token
        </span>
      )}
      {model && <span title="模型">{model}</span>}
    </span>
  );
}

/** 工作区标签条的尾部按钮：新建 Codex / 新建系统终端。 */
export function WorkspaceTrailing({
  canStartCodex,
  onNewCodex,
  onNewShell
}: {
  canStartCodex: boolean;
  onNewCodex: () => void;
  onNewShell: () => void;
}) {
  return (
    <>
      <Button small disabled={!canStartCodex} onClick={onNewCodex} title="在目标里启动 Codex">
        + Codex
      </Button>
      <Button small onClick={onNewShell} title="启动系统 shell（WSL 目标会开该发行版的 shell）">
        + Shell
      </Button>
    </>
  );
}

/** 目标上确实有可用的 Codex 会话入口时才算「能开」。 */
export function canStartCodex(target: AiTarget | null): boolean {
  return Boolean(target && target.available);
}
