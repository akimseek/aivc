import { useEffect, useRef, useState } from "react";

import { hasLayerAbove } from "../../components";

/**
 * 会话提醒中心。
 *
 * # 它解决的是什么
 *
 * 标签上那个小点**只在窗口可见时有用**，而「程序在等你确认」恰恰常常发生在
 * 用户切到别的窗口之后。旧平台的形状是：菜单栏右侧一个铃铛 + 未读数，
 * 点开列出每一条，点条目**跳到那路终端**，也可以单条移除。
 *
 * # 与旧平台一致的三条行为
 *
 * 1. **没有提醒时整个组件不渲染**——留一个空铃铛只是噪音；
 * 2. 未读数超过 99 显示 `99+`（不然徽标会把布局撑歪）；
 * 3. 点条目 = **跳过去 + 清掉它**，然后关闭弹层；点外面或 Escape 也关闭。
 *
 * # 「未读」在这个模型里就是「在表里」
 *
 * 后端不发未读标记：状态变回运行中/已完成时 `use-terminals` 会把它删掉。
 * 所以这里不需要额外的已读概念——**在列表里就是要处理的**。
 */
export type AttentionEntry = {
  /** 终端编号，也是跳转目标。 */
  terminalId: string;
  /** 终端标题（标签上那一行）。 */
  title: string;
  /** 平台名，显示成 `{平台} · {标题}：{说明}`。 */
  provider?: string;
  kind: string;
  detail?: string;
  /** 毫秒时间戳，用于排序；缺失的排最后。 */
  updatedAt?: number;
};

export type AttentionCenterProps = {
  entries: AttentionEntry[];
  /** 跳到那路终端（并清掉它的提醒）。 */
  onSelect: (terminalId: string) => void;
  /** 只移除提醒，不跳转。 */
  onDismiss: (terminalId: string) => void;
};

/** 按 kind 给一个标记；三类分别对应「等确认 / 出错 / 已完成」。 */
function markerOf(kind: string): string {
  if (kind === "awaiting-confirmation") return "⏳";
  if (kind === "error") return "⚠";
  return "✓";
}

export function AttentionCenter({ entries, onSelect, onDismiss }: AttentionCenterProps) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement | null>(null);

  // 没有提醒时收起弹层；下面还会整个不渲染。
  useEffect(() => {
    if (entries.length === 0) setOpen(false);
  }, [entries.length]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      // 有模态/浮层压在它之上时（退出确认框随时可能弹出来）让那一次 Escape 给它：
      // 都在 `document` 上监听，不判断的话一次按键会同时收掉两层。
      if (event.key === "Escape" && !hasLayerAbove("transient")) setOpen(false);
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open]);

  if (entries.length === 0) return null;

  // 最新的在最上面：用户要找的是「刚刚发生了什么」。
  const sorted = [...entries].sort((a, b) => (b.updatedAt ?? 0) - (a.updatedAt ?? 0));

  return (
    <div className="attention-center" ref={rootRef}>
      <button
        type="button"
        className={`attention-trigger${open ? " active" : ""}`}
        aria-label="会话提醒"
        aria-haspopup="dialog"
        aria-expanded={open}
        title="会话提醒"
        onClick={() => setOpen((current) => !current)}
      >
        <span aria-hidden="true">🔔</span>
        {entries.length > 0 && (
          <span className="attention-count">{entries.length > 99 ? "99+" : entries.length}</span>
        )}
      </button>

      {open && (
        <aside className="attention-popover" role="dialog" aria-label="会话状态提醒">
          <div className="attention-heading">
            <div>
              <strong>会话提醒</strong>
              <span>{entries.length} 项待处理</span>
            </div>
            <button
              type="button"
              className="attention-close"
              aria-label="关闭会话提醒"
              title="关闭"
              onClick={() => setOpen(false)}
            >
              ✕
            </button>
          </div>

          <div className="attention-list">
            {sorted.map((entry) => (
              <div key={entry.terminalId} className={`attention-item ${entry.kind}`}>
                <span aria-hidden="true">{markerOf(entry.kind)}</span>
                <button
                  type="button"
                  className="attention-open"
                  onClick={() => {
                    onSelect(entry.terminalId);
                    setOpen(false);
                  }}
                >
                  <strong>{entry.title}</strong>
                  <span>
                    {entry.provider ? `${entry.provider} · ` : ""}
                    {entry.kind === "awaiting-confirmation"
                      ? "等确认"
                      : entry.kind === "error"
                        ? "出错了"
                        : "已完成"}
                    {entry.detail ? `：${entry.detail}` : ""}
                  </span>
                </button>
                <button
                  type="button"
                  className="attention-dismiss"
                  title="移除提醒"
                  aria-label={`移除 ${entry.title} 提醒`}
                  onClick={() => onDismiss(entry.terminalId)}
                >
                  ✕
                </button>
              </div>
            ))}
          </div>
        </aside>
      )}
    </div>
  );
}
