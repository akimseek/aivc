import { Button, EmptyState } from "../../components";
import type { AiMessage, AiSession } from "../../types";
import { BranchPanel } from "../../app/branch-panel";
import { displayTitle, formatTokens, messageCountLabel, roleLabel } from "../../app/session-display";

/**
 * 历史视图：会话头 + 血缘条 + 消息线程。
 *
 * # 三种角色三种形态（不是「深浅」）
 *
 * | 角色 | 形态 | 为什么 |
 * | --- | --- | --- |
 * | 用户 | 强调色引述块（左竖线 + 浅底） | 「哪句是我说的」一眼就能看出来 |
 * | 助手 | 无框正文，直接落在工作区上 | 它是主体内容，加框反而像卡片列表 |
 * | 工具 / 系统 | 等宽小字块 | 它们是**过程记录**，不是对话 |
 *
 * 此前用户与助手都是「浅色底 + 边框」的方块，在白色工作区上几乎一样，
 * 整段对话读起来是一堆同样的方块。
 *
 * # 「继续会话」为什么在这里
 *
 * 恢复终端是**显式动作**（点会话行只看历史，不起 CLI 进程）。它必须在历史视图里
 * 也有一个入口：用户看着这条会话的正文，最自然的下一步就是「接着它聊」。
 * 放在这里而不是标签栏尾部，是因为那是标签栏的位置、说的是标签的事。
 */
export function HistoryView({
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
  onResume
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
  /**
   * 「继续会话」：在终端中恢复这条会话。
   *
   * 传 `undefined` 表示这一刻没有可恢复的东西（标签没有会话编号，或那一路终端
   * 正在跑）——那时不渲染按钮，而不是渲染一个点不动的。
   */
  onResume?: () => void;
}) {
  if (!session) {
    return (
      <EmptyState
        message="这条会话已经不在当前列表里。"
        hint="刷新列表，或从左侧重新选择一条会话。"
      />
    );
  }

  const title = displayTitle(session);

  return (
    <div className="history-view">
      <div className="history-head">
        <h2 title={title}>{title}</h2>
        <span className="meta">
          {/* 优先用**详情路径**带回来的精确总数；它还没到时回退到列表的条数，
              并如实标注那是下界。 */}
          {messageCountLabel(messageTotal ?? session.messageCount, messageTotal === null)}
        </span>
        {/* 工作台要点：上下文占用、累计 token、模型。全都在会话自带的数据里。 */}
        {session.usage && <UsagePills usage={session.usage} model={session.model} />}
        {onResume && (
          <Button
            small
            className="history-resume"
            onClick={onResume}
            title="在终端中恢复这个会话"
          >
            ▶ 继续会话
          </Button>
        )}
      </div>

      {/* 分支关系。放在正文**上方**、会话头下方：
          它讲的是「这条会话从哪来、分出去了哪些」，属于这条会话的元信息。 */}
      <BranchPanel
        session={session}
        resolveParent={branchParent}
        listChildren={branchChildren}
        onOpen={onOpenSession}
      />

      <div className="history-scroll">
        {/* 往前翻的入口放在**顶部**：更早的消息在视觉上就在上面。 */}
        {hasEarlier && (
          <div className="load-earlier">
            <Button small disabled={loadingMore} onClick={onLoadEarlier}>
              {loadingMore ? "加载中…" : "加载更早的消息"}
            </Button>
          </div>
        )}

        {messages.map((message, index) => (
          <HistoryMessage
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
    </div>
  );
}

/** 一条消息。分支动作**悬停才显形**：它是低频动作，常驻会干扰阅读。 */
export function HistoryMessage({
  message,
  onBranch
}: {
  message: AiMessage;
  onBranch?: () => void;
}) {
  return (
    <div className={`history-message ${message.role}`}>
      <div className="history-who">
        <span>{roleLabel(message.role)}</span>
        {onBranch && (
          <button type="button" className="history-branch" onClick={onBranch}>
            从此处分支
          </button>
        )}
      </div>
      <div className="history-text">{message.text}</div>
    </div>
  );
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
