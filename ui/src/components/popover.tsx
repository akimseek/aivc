import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

/**
 * 锚定浮层（供应商锁定、附件菜单）。
 *
 * 用自绘而不是 `dialog` 元素：它是**贴在触发按钮旁边**的，不是模态——
 * 模态会抢走焦点并遮住整屏。
 *
 * 定位策略：先按「锚的下方、左对齐」放，超出视口就翻到上方（或右对齐），
 * 与现有右键菜单的 `viewport-menu.ts` 同一套「不越界」原则。
 */
export function Popover({
  open,
  onClose,
  anchorRef,
  children,
  className
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
  className?: string;
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const [style, setStyle] = useState<{ top: number; left: number }>({ top: 0, left: 0 });

  useLayoutEffect(() => {
    if (!open) return;
    const anchor = anchorRef.current;
    const panel = panelRef.current;
    if (!anchor || !panel) return;
    const a = anchor.getBoundingClientRect();
    const p = panel.getBoundingClientRect();
    const below = a.bottom + 6;
    const top = below + p.height > window.innerHeight ? Math.max(8, a.top - p.height - 6) : below;
    const left = Math.min(Math.max(8, a.left), Math.max(8, window.innerWidth - p.width - 8));
    setStyle({ top, left });
  }, [open, anchorRef]);

  useEffect(() => {
    if (!open) return;
    const onDown = (event: MouseEvent) => {
      const target = event.target as Node;
      if (panelRef.current?.contains(target) || anchorRef.current?.contains(target)) return;
      onClose();
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, onClose, anchorRef]);

  if (!open) return null;
  return (
    <div
      ref={panelRef}
      className={`popover${className ? ` ${className}` : ""}`}
      style={{ top: style.top, left: style.left }}
      role="dialog"
    >
      {children}
    </div>
  );
}
