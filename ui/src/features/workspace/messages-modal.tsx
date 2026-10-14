import { useEffect, useRef, useState } from "react";

import { Badge, Button, EmptyState, Field, FoldableText, Icon, IconButton, LoadingBlock, Pager, Popover, Select, hasLayerAbove } from "../../components";
import type { AiMessage, AiSession, AiTarget, ExportFormat } from "../../types";
import { BranchPanel, shortId, type BranchPanelProps } from "./branch-panel";
import { displayTitle, formatWhen, roleLabel } from "../../app/session-display";
import { useSessionDetail } from "../../app/use-session-detail";
import { flattenConversationTurns } from "./conversation";

/**
 * 消息与分支弹框：把一条会话**停下来看完**。
 *
 * # 它与主区（历史视图）的分工
 *
 * 主区那条线程是**随会话列表走的**：点一条会话，正文就换成它，用户一边浏览列表
 * 一边读，而且只读得到最新的一屏。这个弹框是**研究一条**：全部消息铺开、可以翻页、
 * 带血缘条与「从此处分支」，**关掉就回到原来的视图**（不改变选中状态）。
 *
 * 两者并存而不是二选一：「浏览」与「研究」是两个动作。旧平台就是这么分的
 * （`session-detail-modal.tsx`）。
 *
 * # 分页与竞态
 *
 * 取数、换页时的旧响应保护、换会话回到第 1 页，全部在
 * [`useSessionDetail`](../../app/use-session-detail.ts) 里——**这里不重写一份**，
 * 只把它的返回值接到控件上。
 *
 * # 为什么自绘外壳而不套 `Dialog`
 *
 * 它的结构是「头部 + 血缘条 + 工具行 + 消息区 + 底部」五段，尺寸也比通用弹层宽得多
 * （读会话用的，560px 一行放不下几句中文）。硬套 `Dialog` 会把通用弹层撑成
 * 「什么都能装」的上帝组件。遮罩、层级、圆角仍与 `Dialog`、设置中心用同一批 token。
 */

export function MessagesModal({
  session,
  messages,
  total,
  totalKnown = true,
  branchAnchors = [],
  page,
  pageSize,
  loading,
  error,
  onPageChange,
  onPageSizeChange,
  onReload,
  onBranch,
  canBranch = true,
  provider = "codex",
  messageOffset = (page - 1) * pageSize,
  pageHasMore = false,
  nextMessageRole,
  branching = false,
  onOpenSession,
  branchParent,
  branchChildren,
  onExport,
  onClose
}: {
  session: AiSession;
  /** 当前页的消息。 */
  messages: AiMessage[];
  /** 已知消息数；totalKnown 为 false 时是下界。 */
  total: number;
  totalKnown?: boolean;
  branchAnchors?: number[];
  /** 页码从 1 起。 */
  page: number;
  pageSize: number;
  loading: boolean;
  /** 取数失败的一句话（空串表示没有失败）。 */
  error?: string;
  onPageChange: (page: number) => void;
  onPageSizeChange: (size: number) => void;
  onReload: () => void;
  onBranch: (sourceLine: number) => void;
  /**
   * 平台支不支持「从此处分支」。
   *
   * Qoder 不支持（`ProviderCapabilities::branch == false`）。不门控的话每条带
   * 行号的消息下面都挂一个点下去必然报错的按钮。
   */
  canBranch?: boolean;
  provider?: AiTarget["provider"];
  messageOffset?: number;
  pageHasMore?: boolean;
  nextMessageRole?: AiMessage["role"];
  branching?: boolean;
  /** 血缘条里点父 / 子会话时打开它（由上层决定怎么打开）。 */
  onOpenSession: (session: AiSession) => void;
  branchParent: BranchPanelProps["resolveParent"];
  branchChildren: BranchPanelProps["listChildren"];
  onExport: (format: ExportFormat) => void;
  onClose: () => void;
}) {
  // The session list may only know a lower-bound count. Keep the next page
  // reachable while the current page proves that more data exists.
  const pageCount = Math.max(1, Math.ceil(total / pageSize), pageHasMore ? page + 1 : page);
  const exportRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [exportOpen, setExportOpen] = useState(false);

  // Escape 关闭。有更内层的东西开着（导出浮层、退出确认框）时让给它——不判断的话
  // 一次 Escape 会同时关掉浮层与整个弹框；退出确认框按层序在弹框**之上**，
  // 该由它先处理那一次按键。判定与设置中心共用一份（`hasLayerAbove`）。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      if (hasLayerAbove("modal")) return;
      onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  // 打开时焦点进面板：不做这件事，键盘用户按下 Tab 会落在背后的界面上。
  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = 0;
  }, [session.id, page, pageSize]);

  return (
    <div
      className="messages-overlay"
      role="dialog"
      aria-modal="true"
      aria-label="会话消息历史与分支演进"
    >
      <div className="messages-panel" ref={panelRef} tabIndex={-1}>
        <header className="messages-head">
          <span className="messages-head-icon">
            <Icon name="branch" size={16} />
          </span>
          <div className="messages-head-text">
            <h2>会话消息历史与分支演进</h2>
            <p title={displayTitle(session)}>
              {displayTitle(session)} · {session.cwd ?? "（无目录）"}
            </p>
          </div>
          <Badge mono>ID: {shortId(session.id)}</Badge>
          <IconButton label="关闭" onClick={onClose}>
            <Icon name="close" size={16} />
          </IconButton>
        </header>

        {/* 血缘：根 → 当前 → 子。没有分支时它整块不渲染。 */}
        <BranchPanel
          session={session}
          resolveParent={branchParent}
          listChildren={branchChildren}
          onOpen={onOpenSession}
        />

        {error && (
          // 读不到时**说一声**再继续显示空列表：静默失败会让用户以为会话真的没有消息。
          <div className="messages-error" role="status">
            {error}
          </div>
        )}

        {/* 分页控件。**每页数量**是一等公民：详情是给「研究一条会话」用的，
            有人想看 20 条一屏，有人想一次铺 200 条。
            翻页按钮与边界禁用走共用的 `Pager`（与请求日志、异常诊断同一份实现）；
            它的差异点（每页数量、加载中额外禁用、最后一页、页码文字的字号与
            播报）全部由 props 表达。 */}
        <div className="messages-pagination" role="group" aria-label="会话消息分页">
          <Pager
            page={page}
            pageCount={pageCount}
            onPage={onPageChange}
            disabled={loading || branching}
            extra={
              <>
                <Field label="每页数量">
                  <Select
                    value={String(pageSize)}
                    disabled={loading || branching}
                    onChange={(event) =>
                      onPageSizeChange(Number.parseInt(event.target.value, 10))
                    }
                  >
                    {[20, 50, 100, 200].map((size) => (
                      <option key={size} value={size}>
                        {size}
                      </option>
                    ))}
                  </Select>
                </Field>

                <span className="spacer" />
              </>
            }
            withLast={totalKnown}
            pageCountKnown={totalKnown}
            labelClassName="messages-range"
            ariaLive
          />
        </div>

        <div className="messages-scroll" ref={scrollRef} aria-label="会话时间线" aria-busy={loading || branching}>
          {loading ? (
            <LoadingBlock label="正在加载本页消息…" />
          ) : messages.length === 0 ? (
            <EmptyState
              message="这一页没有可显示的消息。"
              hint="文件可能是空的，或消息都被解析规则过滤掉了。"
            />
          ) : (
            flattenConversationTurns(
              messages,
              messageOffset,
              provider,
              nextMessageRole === undefined ? !pageHasMore : nextMessageRole === "user",
              branchAnchors
            ).map(({ entry, branchAnchor: anchor }) => (
              <MessageCard
                key={`${entry.message.sourceLine ?? entry.index}`}
                message={entry.message}
                branchDisabled={branching}
                branchHint="保留这一轮会话的全部上下文，创建独立分支"
                onBranch={
                  !canBranch || anchor === undefined
                    ? undefined
                    : () => onBranch(anchor)
                }
              />
            ))
          )}
        </div>

        <footer className="messages-foot">
          <span className="messages-count" role="status">{branching ? "正在创建分支…" : loading ? "正在加载消息…" : totalKnown ? `共 ${total} 条历史消息` : `至少 ${total} 条历史消息`}</span>
          <Button small onClick={onReload} disabled={loading || branching} title="重新加载">
            <Icon name="refresh" size={13} />
            重新加载
          </Button>
          <span className="spacer" />
          <button
            type="button"
            className="btn btn-sm"
            ref={exportRef}
            aria-haspopup="menu"
            aria-expanded={exportOpen}
            onClick={() => setExportOpen((open) => !open)}
          >
            导出
            <Icon name="chevron-down" size={12} />
          </button>
          <Popover
            open={exportOpen}
            onClose={() => setExportOpen(false)}
            anchorRef={exportRef}
            className="messages-export-menu"
            ariaLabel="导出格式"
          >
            {/* 三种格式与右键菜单、被移除的菜单栏那份**逐字一致**（旧平台就是这样三项）。 */}
            {(
              [
                ["导出 Markdown", "markdown"],
                ["导出 JSON", "json"],
                ["导出 HTML", "html"]
              ] as [string, ExportFormat][]
            ).map(([label, format]) => (
              <button
                key={format}
                type="button"
                role="menuitem"
                className="messages-export-item"
                onClick={() => {
                  setExportOpen(false);
                  onExport(format);
                }}
              >
                {label}
              </button>
            ))}
          </Popover>
          <Button variant="primary" onClick={onClose}>
            关闭
          </Button>
        </footer>
      </div>
    </div>
  );
}

/**
 * 一条消息：头像 + 角色时间 + 内容卡。
 *
 * **不导出**：只有这个文件里的 `MessagesModal` 用得到它（外部的消息渲染走
 * `HistoryView` 的 `HistoryMessage`）。留一个 open 的出口只会让人以为别处在用它。
 *
 * # 角色规则沿用主区
 *
 * 措辞取自 [`roleLabel`](../../app/session-display.ts)——正文与弹框看的是同一批消息，
 * 两处各写一遍会漂移成两个称呼（同一句话在弹框里叫「你」、在正文里叫「我」）。
 *
 * # 头像
 *
 * 我 = 蓝底、AI = 强调色底：同一条时间线里两个头像必须**一眼可辨**。
 * 工具 / 系统不是对话，用中性底。
 */
function MessageCard({ message, onBranch, branchHint, branchDisabled }: { message: AiMessage; onBranch?: () => void; branchHint?: string; branchDisabled?: boolean }) {
  const mine = message.role === "user";
  const avatar = mine ? "我" : message.role === "assistant" ? "AI" : roleLabel(message.role).slice(0, 1);

  return (
    <article className={`messages-item ${message.role}`}>
      <span className="messages-avatar" aria-hidden="true">
        {avatar}
      </span>
      <div className="messages-main">
        <div className="messages-meta">
          <span className="messages-role">{roleLabel(message.role)}</span>
          {message.timestamp && (
            <time className="messages-time" dateTime={message.timestamp}>
              {formatWhen(message.timestamp)}
            </time>
          )}
          {onBranch && (
            <button type="button" className="messages-branch" disabled={branchDisabled} onClick={onBranch} title={branchHint}>
              从此处分支
            </button>
          )}
        </div>
        <MessageText text={message.text} />
      </div>
    </article>
  );
}

/** 正文卡。长文折叠（共用 `FoldableText`），**一个字都不丢**：可展开看全文。 */
function MessageText({ text }: { text: string }) {
  return (
    <div className="messages-card">
      <FoldableText text={text} className="messages-text" />
    </div>
  );
}

/**
 * 弹框的**自主容器**：分页状态与取数都在这里。
 *
 * # 为什么分页状态不在 `app.tsx`
 *
 * `app.tsx` 已经要管标签、弹层、导出、命令面板…再往里塞一个「详情开着哪一条、
 * 第几页、每页多少条、加载中」会让它继续膨胀。这里只暴露「要看哪一条」（受控，
 * 因为关掉之后要回到原来的视图）与几个动作。
 */
function SessionMessagesModalContainer({
  session,
  target,
  canBranch = true,
  resolveParent,
  listChildren,
  onClose,
  onOpenSession,
  onBranch,
  onExport
}: {
  /** 要看的会话；`null` 表示不打开。 */
  session: AiSession | null;
  /** 取消息要有目标（IPC 用它的编号定位会话文件）。 */
  target: AiTarget | null;
  canBranch?: boolean;
  resolveParent: (sessionId: string) => AiSession | undefined;
  listChildren: (parentSessionId: string) => Promise<AiSession[]>;
  onClose: () => void;
  onOpenSession: (session: AiSession) => void;
  /**
   * 从某条消息处分支。
   *
   * 带 `sessionId`：分支必须分到**弹框里这条**会话上。弹框开着的未必是列表里
   * 选中的那一条（侧栏卡片就能为非选中会话开弹框），上层只读选中值会分错。
   */
  onBranch: (anchor: number, sessionId: string, targetId?: string) => void | Promise<boolean>;
  onExport: (format: ExportFormat) => void;
}) {
  const [pageSize, setPageSize] = useState(50);
  const detail = useSessionDetail(target, session, pageSize);
  const [branching, setBranching] = useState(false);
  const branchPending = useRef(false);
  const [branchError, setBranchError] = useState("");
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  if (!session) return null;

  return (
    <MessagesModal
      session={session}
      messages={detail.messages}
      // 未到 EOF 时，只显示本次分页实际确认的消息数下界。
      total={detail.totalKnown ? detail.total : detail.offset + detail.messages.length + (detail.hasMore ? 1 : 0)}
      totalKnown={detail.totalKnown}
      branchAnchors={detail.branchAnchors}
      page={detail.page}
      pageSize={pageSize}
      loading={detail.loading}
      error={branchError || (detail.error ? `会话消息读取失败：${detail.error}` : "")}
      provider={target?.provider}
      messageOffset={detail.offset}
      pageHasMore={detail.hasMore}
      nextMessageRole={detail.nextMessageRole}
      branching={branching}
      onPageChange={detail.setPage}
      onPageSizeChange={(size) => {
        setPageSize(size);
        // 换每页数量回第 1 页：否则「第 4 页 × 200 条」会停在一个不存在的页上，
        // 界面空白，用户以为没数据。规则与旧容器一致。
        detail.setPage(1);
      }}
      onReload={detail.reload}
      onBranch={async (anchor) => {
        if (branchPending.current) return;
        branchPending.current = true;
        setBranching(true);
        setBranchError("");
        try {
          const succeeded = await onBranch(anchor, session.id, target?.id);
          if (!mounted.current) return;
          if (succeeded !== false) onClose();
          else setBranchError("分支未创建，详情请查看右上角提示。");
        } catch (error) {
          if (mounted.current) setBranchError(error instanceof Error ? error.message : String(error));
        } finally {
          branchPending.current = false;
          if (mounted.current) setBranching(false);
        }
      }}
      canBranch={canBranch}
      onOpenSession={(next) => {
        onClose();
        onOpenSession(next);
      }}
      branchParent={resolveParent}
      branchChildren={listChildren}
      onExport={onExport}
      onClose={onClose}
    />
  );
}

/** Closing or changing the session disposes of all page/branch UI state. */
export function MessagesModalContainer(props: Parameters<typeof SessionMessagesModalContainer>[0]) {
  if (!props.session) return null;
  return <SessionMessagesModalContainer key={JSON.stringify([props.target?.id, props.session.id])} {...props} />;
}
