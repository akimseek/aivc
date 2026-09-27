import { useEffect, useState } from "react";

import type { AiSession } from "../types";
import { formatWhen } from "./session-display";

/**
 * 会话详情里的「分支关系」面板。
 *
 * # 为什么它在详情里，而不是别处
 *
 * 「从此处分支」的锚点是**一条消息**（后端要的是 `parentMessageLine`）。
 * 所以「这条会话从哪来、分出去了哪些」这份信息只有在能看到会话正文的地方才有意义——
 * 旧平台把它放在详情弹层里，不是藏起来，而是那里才有上下文。
 *
 * # 形态
 *
 * - **来源**：父会话（能解析出标题就给按钮，否则退回显示短编号——旧平台的兜底也是这个）
 *   + `消息 {n}` + `JSONL 行 {n}`；
 * - **子分支**：每个子会话一个按钮（标题 + 相对时间）；
 * - `正在加载...` 的加载态；
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
function shortId(id: string): string {
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
    <section className="branch-panel" aria-label="分支关系">
      <header>
        <h3>分支关系</h3>
        {loading && <span>正在加载...</span>}
      </header>

      {hasParent && (
        <div className="branch-row">
          <span className="branch-label">来源</span>
          {parent ? (
            <button type="button" onClick={() => onOpen(parent)} title={parent.title}>
              {parent.title}
            </button>
          ) : (
            // 解析不出父会话时退回短编号：有它总比什么都没有强，
            // 而且锚点（消息序号 / 行号）仍然能说明「从哪分出来的」。
            <code>{shortId(parentSessionId!)}</code>
          )}
          {typeof branch?.parentMessageIndex === "number" && (
            <small>消息 {branch.parentMessageIndex}</small>
          )}
          {typeof branch?.parentMessageLine === "number" && (
            <small>JSONL 行 {branch.parentMessageLine}</small>
          )}
        </div>
      )}

      {(children.length > 0 || loading) && (
        <div className="branch-children">
          <span className="branch-label">子分支</span>
          <div>
            {children.map((child) => (
              <button key={child.id} type="button" onClick={() => onOpen(child)} title={child.title}>
                <span>{child.title}</span>
                <small>{formatWhen(child.updatedAt ?? child.createdAt)}</small>
              </button>
            ))}
          </div>
        </div>
      )}
    </section>
  );
}
