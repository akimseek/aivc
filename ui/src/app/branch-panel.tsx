import { Fragment, useEffect, useRef, useState } from "react";

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
 *
 * # 根会话（没有父）也必须拉子分支
 *
 * 曾经这里「没有 `parentSessionId` 就直接 return」：于是**根会话永远看不到自己的
 * 子分支**——而「从一个根分出去好几条」恰恰是最需要看血缘的场景。父会话解析不出来
 * 只是「来源那一格退化成短编号」，与「有没有子分支」是两件毫不相干的事。
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
  /**
   * 子分支**连同它是哪条会话的**一起存。
   *
   * 只存一个数组的话，换会话后的那一次渲染会拿旧数组配新会话（effect 要等提交之后
   * 才跑），屏幕上闪出的是上一条会话的子分支。
   */
  const [loaded, setLoaded] = useState<{ forId: string; children: AiSession[] }>({
    forId: "",
    children: []
  });
  const children = loaded.forId === session.id ? loaded.children : [];
  const loading = loaded.forId !== session.id;

  /**
   * 最新的 `listChildren` 放在 ref 里。
   *
   * 它是调用方**每次渲染新建**的箭头函数（`overlay-layer` 与 `app.tsx` 两处都是
   * 内联写的）。写进依赖数组等于「上层每重渲一次就重拉一次子分支」——终端一有输出
   * 上层一秒能重渲好几次，那些 IPC 全是白发的。判据只有会话编号。
   */
  const listChildrenRef = useRef(listChildren);
  listChildrenRef.current = listChildren;

  useEffect(() => {
    let cancelled = false;
    // **无论有没有父都拉**：没有父的根会话同样可能有分出去的子分支（见文件头）。
    void listChildrenRef
      .current(session.id)
      .then((list) => {
        // 组件已经换了一个会话时不要再写状态，否则会把上一个会话的子分支显示出来。
        if (!cancelled) setLoaded({ forId: session.id, children: list });
      })
      .catch(() => {
        // 列不出来不是致命错误：来源那一行仍然有意义，不因此整块变成错误。
        if (!cancelled) setLoaded({ forId: session.id, children: [] });
      });
    return () => {
      cancelled = true;
    };
    // 只有「换了一条会话」才重拉，理由见 `listChildrenRef`。
  }, [session.id]);

  const parent = parentSessionId ? resolveParent?.(parentSessionId) : undefined;
  const hasParent = Boolean(parentSessionId);

  // 父与子都没有：整块不渲染。
  //
  // 子分支**还在拉的那一刻也算「没有」**：绝大多数会话没有任何分支，先摆一个
  // 「正在加载…」的框再收回去，正文会白白跳一下。反过来，真要显示子分支时，
  // 这条 IPC 通常在一次往返内就回来了。
  if (!hasParent && children.length === 0) return null;

  return (
    // 地标角色（`nav`）对一行条过重：读屏的「跳转到导航」列表里会多出一个只含
    // 一条血缘的入口。`group` 保留标签语义，不冒充地标。
    <div className="lineage-bar" role="group" aria-label="分支关系">
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
    </div>
  );
}
