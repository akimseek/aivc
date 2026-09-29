import { useState, type ReactNode } from "react";

import { Button, Dialog, EmptyState, LoadingBlock, ToolbarSpacer } from "../components";
import type { AiMessage, AiSession } from "../types";
import { messageCountLabel } from "./session-display";
import { Message } from "./session-row";

/**
 * 会话详情弹层。
 *
 * # 它与主区正文的分工
 *
 * 主区那条线程是**随会话列表走的**：点一条会话，正文就换成它，用户一边浏览列表
 * 一边读。详情弹层是**停下来看这一条**：它把这条会话的全部消息铺开、可以前后翻页、
 * 带分支面板，而且**关掉就回到原来的视图**，不改变选中状态。
 *
 * 旧平台就是这么分的（`session-detail-modal.tsx`）。两者并存而不是二选一：
 * 「浏览」与「研究」是两个动作。
 *
 * # 分页
 *
 * 旧平台的详情弹层带一个分页控件：`每页数量` / `第一页` / `上一页` / `下一页` /
 * `最后一页`。这里照抄，因为**详情路径是全量解析**——一条几百条的会话一次性铺开
 * 会很慢，也不好看。
 */
export function SessionDetailDialog({
  session,
  messages,
  total,
  page,
  pageSize,
  loading,
  onPageChange,
  onPageSizeChange,
  onReload,
  onBranch,
  canBranch = true,
  onClose,
  branchPanel
}: {
  session: AiSession;
  /** 当前页的消息。 */
  messages: AiMessage[];
  /** 精确总数（详情路径全量解析得来）。 */
  total: number;
  /** 页码从 1 起。 */
  page: number;
  pageSize: number;
  loading: boolean;
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
  onClose: () => void;
  /** 分支关系面板（由调用方注入，它要拉子会话）。 */
  branchPanel?: ReactNode;
}) {
  const pageCount = Math.max(1, Math.ceil(total / pageSize));

  return (
    <Dialog
      title="会话详情"
      panelClassName="session-detail-dialog"
      ariaLabel="会话详情"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onReload} disabled={loading}>
            重新加载
          </Button>
          <ToolbarSpacer />
          <Button variant="primary" onClick={onClose}>
            关闭
          </Button>
        </>
      }
    >
      <div className="detail-meta">
        <strong title={session.title}>{session.title?.trim() || session.id.slice(0, 8)}</strong>
        <span className="row-meta">
          {session.id} · {messageCountLabel(total)} · {session.cwd ?? "（无目录）"}
        </span>
      </div>

      {branchPanel}

      {/* 分页控件。**每页数量**是一等公民：详情是给「研究一条会话」用的，
          有人想看 20 条一屏，有人想一次铺 200 条。 */}
      <div className="pagination" role="group" aria-label="会话详情分页">
        <label className="field">
          <span className="field-label">每页数量</span>
          <select
            className="control control-select"
            value={String(pageSize)}
            onChange={(event) => onPageSizeChange(Number.parseInt(event.target.value, 10))}
          >
            {[20, 50, 100, 200].map((size) => (
              <option key={size} value={size}>
                {size}
              </option>
            ))}
          </select>
        </label>

        <ToolbarSpacer />

        <Button small disabled={page <= 1 || loading} onClick={() => onPageChange(1)} title="第一页">
          第一页
        </Button>
        <Button
          small
          disabled={page <= 1 || loading}
          onClick={() => onPageChange(page - 1)}
          title="上一页"
        >
          上一页
        </Button>
        <span className="row-meta" aria-live="polite">
          第 {page} / {pageCount} 页
        </span>
        <Button
          small
          disabled={page >= pageCount || loading}
          onClick={() => onPageChange(page + 1)}
          title="下一页"
        >
          下一页
        </Button>
        <Button
          small
          disabled={page >= pageCount || loading}
          onClick={() => onPageChange(pageCount)}
          title="最后一页"
        >
          最后一页
        </Button>
      </div>

      <div className="detail-thread" aria-label="会话时间线">
        {loading ? (
          <LoadingBlock label="正在加载完整会话…" />
        ) : messages.length === 0 ? (
          <EmptyState
            message="这一页没有可显示的消息。"
            hint="文件可能是空的，或消息都被解析规则过滤掉了。"
          />
        ) : (
          messages.map((message, index) => (
            <Message
              key={`${message.sourceLine ?? index}`}
              message={message}
              onBranch={
                // 两个条件都要：平台支持分支，且这条消息有原始行号
                // （后端要靠行号定位原始记录）。
                !canBranch || message.sourceLine === undefined
                  ? undefined
                  : () => onBranch(message.sourceLine!)
              }
            />
          ))
        )}
      </div>
    </Dialog>
  );
}

/** 详情弹层的分页状态。抽出来是因为「换页/换每页数量时页码怎么变」有明确规则。 */
export function useDetailPaging(initialSize = 50) {
  const [pageSize, setPageSize] = useState(initialSize);
  const [page, setPage] = useState(1);
  return {
    page,
    pageSize,
    setPage,
    /**
     * 换每页数量时**回到第 1 页**。
     *
     * 不回到第 1 页会出现一个很难解释的现象：第 4 页、每页 200 条时总数只有 300 条，
     * 界面空白，用户以为「没数据了」，其实是那一页不存在。
     */
    changePageSize: (size: number) => {
      setPageSize(size);
      setPage(1);
    }
  };
}
