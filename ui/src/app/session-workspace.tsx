import { lazy, Suspense, type ReactNode } from "react";

import { Button, EmptyState, Tabs, type TabItem } from "../components";
import type { AiMessage, AiSession, AiTarget } from "../types";
import { BranchPanel } from "./branch-panel";
import { messageCountLabel, formatTokens } from "./session-display";
import { Message } from "./session-row";
import { SessionDetailButton } from "./session-actions";
import type { StageContent } from "./stage-content";

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
 * | 画布状态（`stage`） | 正文显示 |
 * | --- | --- |
 * | 终端 | 终端栈（会话正文整块不渲染） |
 * | 历史 | 会话头 + 分支面板 + 消息线程 |
 * | 空 | 空状态（说明该怎么开始） |
 *
 * 判据由 [`resolveStageContent`](./stage-content.ts) 算好传进来：**终端优先**——
 * 一张标签同时挂着会话与终端（统一模型），两者都在时显示终端；终端不在了
 * （还没起、或已被关掉）才回落到历史。这一步不在渲染里判，否则「终端或历史」
 * 就会在几个组件里各写一遍。
 */
export function SessionWorkspace({
  tabItems,
  activeKey,
  stage,
  terminalItems,
  registerSink,
  unregisterSink,
  onError,
  onActivate,
  onClose,
  onTabContextMenu,
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
  branchParent
}: {
  tabItems: TabItem[];
  activeKey: string;
  /** 画布当前该显示什么（由 `resolveStageContent` 算出）。 */
  stage: StageContent;
  /** 当前挂着的终端（全部，不只是前台那一个）。`sessionId` 决定自管输入的初始可见性。 */
  terminalItems: { id: string; running: boolean; sessionId?: string }[];
  registerSink: (id: string, sink: (data: Uint8Array) => void) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  onActivate: (key: string) => void;
  onClose: (key: string) => void;
  /** 标签右键菜单（关闭其他 / 关闭全部）。 */
  onTabContextMenu?: (key: string, x: number, y: number) => void;
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
}) {
  return (
    <section className="workspace">
      <Tabs
        items={tabItems}
        activeKey={activeKey}
        onActivate={onActivate}
        onClose={onClose}
        onContextMenu={onTabContextMenu}
        trailing={trailing}
      />

      <div className="workspace-body">
        {/* 所有终端**同时挂载**、叠在同一个位置上；非前台的只隐藏不卸载——
            卸载会丢掉 xterm 的缓冲，切回来时历史就没了。 */}
        {terminalItems.length > 0 && (
          <div className={`terminal-stack${stage.kind === "terminal" ? "" : " hidden"}`}>
            <Suspense fallback={<div className="terminal-loading">正在加载终端…</div>}>
              {terminalItems.map((item) => (
                <TerminalPane
                  key={item.id}
                  terminalId={item.id}
                  active={stage.kind === "terminal" && stage.terminalId === item.id}
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

        {stage.kind === "history" ? (
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
          />
        ) : stage.kind === "empty" ? (
          <EmptyState
            message="没有打开的会话。"
            hint="从左侧选择一个会话，或新建一个终端。"
          />
        ) : null}
      </div>
    </section>
  );
}

/**
 * 会话正文。
 *
 * 没有会话时给空状态而不是空白：标签还在、而那条会话已经不在当前列表里
 * （刚被别处删掉，或目标刚切过去还没加载完）时，至少要有一句说明。
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
  branchChildren
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
}) {
  if (!session) {
    return (
      <EmptyState
        message="这条会话已经不在当前列表里。"
        hint="刷新列表，或从左侧重新选择一条会话。"
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

/** 工作区标签条的尾部按钮：详情 + 继续会话 + 新建 Codex。 */
export function WorkspaceTrailing({
  canStartCodex,
  onNewCodex,
  detailSession,
  onOpenDetail,
  onResume
}: {
  canStartCodex: boolean;
  onNewCodex: () => void;
  /**
   * 「详情」要看的会话：**当前标签那条会话优先**，否则当前选中的会话。
   *
   * 旧平台就是这个来源（`activeSession && <button>详情</button>`，而 `activeSession`
   * 是活动标签的会话）。用「当前选中」会在进入终端后失灵——那时选中项可能不是
   * 这条终端所属的会话。
   */
  detailSession: AiSession | null;
  onOpenDetail: (session: AiSession) => void;
  /**
   * 「继续会话」：在活动标签的会话上起一路终端（带 `--resume`）。
   *
   * 传 `undefined` 表示这一刻没有可恢复的东西（标签没有会话编号，或那一路终端
   * 正在跑）。放在**标签条这一行**而不是正文里：终端异常退出时标签保留现场
   * （画布仍是终端），按钮放进正文就够不着了——那正是「退出后被锁住」的形态。
   */
  onResume?: () => void;
}) {
  return (
    <>
      <SessionDetailButton session={detailSession} onOpenDetail={onOpenDetail} />
      {onResume && (
        <Button small onClick={onResume} title="带 --resume 起一路终端，接着这条会话">
          继续会话
        </Button>
      )}
      <Button small disabled={!canStartCodex} onClick={onNewCodex} title="在目标里启动 Codex">
        + Codex
      </Button>
    </>
  );
}

/** 目标上确实有可用的 Codex 会话入口时才算「能开」。 */
export function canStartCodex(target: AiTarget | null): boolean {
  return Boolean(target && target.available);
}
