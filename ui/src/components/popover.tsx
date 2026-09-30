import { useEffect, useLayoutEffect, useRef, useState, type ReactNode, type RefObject } from "react";

import { hasLayerAbove } from "./overlay-stack";

/**
 * 锚定浮层（供应商锁定、附件菜单）。
 *
 * 用自绘而不是 `dialog` 元素：它是**贴在触发按钮旁边**的，不是模态——
 * 模态会抢走焦点并遮住整屏。
 *
 * 定位策略：先按「锚的下方、左对齐」放，超出视口就翻到上方（或右对齐），
 * 与现有右键菜单的 `viewport-menu.ts` 同一套「不越界」原则。
 *
 * # Escape 也要让位
 *
 * 它此前是唯一一个「只要开着就吃掉 Escape」的浮层：退出确认框从后端推来时
 * （可能与浮层同时存在），一次按键会**同时**收掉浮层与那个框——框收掉等于替用户
 * 回答了「取消」，而后端本来在等一个明确的回答。判定与菜单、关注中心同档。
 */
export function Popover({
  open,
  onClose,
  anchorRef,
  children,
  className,
  ariaLabel
}: {
  open: boolean;
  onClose: () => void;
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
  className?: string;
  /**
   * 无障碍名称。
   *
   * 面板是 `role="dialog"`，没有名字时读屏只会念「对话框」——浮层里装的是哪件事
   * （切供应商 / 加附件 / 导出格式）只有视觉上的位置能说明，因此由调用方按需传。
   */
  ariaLabel?: string;
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
      if (event.key !== "Escape") return;
      // 上面还有别的东西时让位。`.popover` 是我自己这一类，必须剔掉（名单里两档
      // 都有它），并跳过**包含我**的那一层：消息弹框里的导出浮层是嵌套不是叠放，
      // 不跳过的话两者会互相让位，Escape 谁都不响应。
      if (hasLayerAbove("transient", ".popover", panelRef.current)) return;
      onClose();
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
      aria-label={ariaLabel}
    >
      {children}
    </div>
  );
}
