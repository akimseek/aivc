import { Fragment, useEffect, useState } from "react";

import { displayTitle, formatWhen } from "./session-display";
import type { AiSession } from "../types";

/**
 * 血缘条：一条会话**从哪来、分出去了哪些**，横向铺开成 `根 → 当前 → 子`。
 *
 * # 为什么是横的
 *
 * 它此前是纵向的三段（`来源` / `子分支`），因为那时它站在详情弹层的左侧。
 * 现在它同时出现在消息弹框与历史视图里，两处都是**通栏**的位置——纵向摆会白白
 * 吃掉正文的高度，而血缘本身就是一条**线**：根在左、当前在中、分出去的在右，
 * 与「时间从左往右」的直觉一致。
 *
 * # 锚点属于「当前」那一格
 *
 * `消息 {n}` / `JSONL 行 {n}` 讲的是「当前这条是从父会话的第几条分出来的」，
 * 因此挂在当前会话上，而不是父会话上。
 *
 * # 形态
 *
 * - **来源**：父会话（能解析出标题就给按钮，否则退回短编号——旧平台的兜底也是这个）；
 * - **子分支**：每个子会话一个按钮（标题 + 相对时间）；
 * - `正在加载…` 的加载态；
 * - **父与子都没有时整块不渲染**——大多数会话没有分支，留一个空框只会占地方。
 */
export type BranchPanelProps = {
  session: AiSession;
  /**
   * 从已加载的会话列表里找父会话。
   *
   * 界面没有「按编号取一个会话」的命令，因此这里由调用方注入；
   * **解析不出来不是错误**，退回显示短编号即可（旧平台同样如此）。
   */
  resolveParent?: (sessionId: string) => AiSession | undefined;
  /** 拉子分支。注入是为了可测：面板本身不直接碰 IPC。 */
  listChildren: (parentSessionId: string) => Promise<AiSession[]>;
  onOpen: (session: AiSession) => void;
};

/** 取编号的前 8 位——完整编号在界面上是噪音。 */
export function shortId(id: string): string {
  return id.length > 8 ? `${id.slice(0, 8)}…` : id;
}

export function BranchPanel({ session, resolveParent, listChildren, onOpen }: BranchPanelProps) {
  const branch = session.metadata?.branch;
  const parentSessionId = branch?.parentSessionId;
  const [children, setChildren] = useState<AiSession[]>([]);
  const [loading, setLoading] = useState(Boolean(parentSessionId));

  useEffect(() => {
    if (!parentSessionId) {
      setChildren([]);
      setLoading(false);
      return;
    }
    let cancelled = false;
    setLoading(true);
    void listChildren(session.id)
      .then((list) => {
        // 组件已经换了一个会话时不要再写状态，否则会把上一个会话的子分支显示出来。
        if (!cancelled) setChildren(list);
      })
      .catch(() => {
        // 列不出来不是致命错误：来源那一行仍然有意义，不因此整块变成错误。
        if (!cancelled) setChildren([]);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // 会话换了就要重新拉；`listChildren` 由调用方注入，通常是稳定引用。
  }, [session.id, parentSessionId, listChildren]);

  const parent = parentSessionId ? resolveParent?.(parentSessionId) : undefined;
  const hasParent = Boolean(parentSessionId);

  // 父与子都没有：整块不渲染。
  if (!hasParent && children.length === 0 && !loading) return null;

  return (
    <nav className="lineage-bar" aria-label="分支关系">
      <span className="lineage-label">血缘</span>

      {hasParent && (
        <>
          <div className="lineage-node root">
            <span className="lineage-node-label">根会话</span>
            {parent ? (
              <button
                type="button"
                className="lineage-open"
                onClick={() => onOpen(parent)}
                title={displayTitle(parent)}
              >
                {displayTitle(parent)}
              </button>
            ) : (
              // 解析不出父会话时退回短编号：有它总比什么都没有强，
              // 而且锚点（消息序号 / 行号）仍然能说明「从哪分出来的」。
              <code title={parentSessionId}>{shortId(parentSessionId!)}</code>
            )}
          </div>
          <span className="lineage-arrow" aria-hidden="true">
            →
          </span>
        </>
      )}

      <div className="lineage-node current">
        <span className="lineage-node-label">当前会话</span>
        <strong title={displayTitle(session)}>{displayTitle(session)}</strong>
        {(typeof branch?.parentMessageIndex === "number" ||
          typeof branch?.parentMessageLine === "number") && (
          <span className="lineage-anchor">
            {typeof branch?.parentMessageIndex === "number" && (
              <small>消息 {branch.parentMessageIndex}</small>
            )}
            {typeof branch?.parentMessageLine === "number" && (
              <small>JSONL 行 {branch.parentMessageLine}</small>
            )}
          </span>
        )}
      </div>

      {(children.length > 0 || loading) && (
        <>
          {children.map((child) => (
            <Fragment key={child.id}>
              <span className="lineage-arrow" aria-hidden="true">
                →
              </span>
              <div className="lineage-node child">
                <span className="lineage-node-label">子分支</span>
                <button
                  type="button"
                  className="lineage-open"
                  onClick={() => onOpen(child)}
                  title={displayTitle(child)}
                >
                  <span>{displayTitle(child)}</span>
                  <small>{formatWhen(child.updatedAt ?? child.createdAt)}</small>
                </button>
              </div>
            </Fragment>
          ))}
          {loading && <span className="lineage-loading">正在加载…</span>}
        </>
      )}
    </nav>
  );
}
