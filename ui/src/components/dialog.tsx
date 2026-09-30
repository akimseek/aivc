import { useEffect, useRef, type ReactNode } from "react";

/**
 * 弹层外壳。
 *
 * # 为什么必须有它
 *
 * 「关于」「新会话」「设置会话」三个对话框此前各自抄了一遍同样的骨架：遮罩、
 * `role="dialog"`、`aria-modal`、Escape 关闭、焦点进面板、页脚按钮排布。
 * 四次手写就有四处走样的机会——Escape 漏一个、`aria-modal` 少一个。
 *
 * # 两条与旧平台一致的默认行为
 *
 * 1. **点遮罩不关闭**（`closeOnOverlay` 默认 `false`）。
 *    这不是偷懒：新会话对话框里可能有用户敲了一半的提示词，点偏一下就全丢了。
 *    旧平台传的就是 `closeOnOverlay={false}`。
 * 2. **打开时焦点进面板**。不做这件事，键盘用户按下 Tab 会落在背后的界面上。
 *
 * 遮罩与面板的类名沿用既有的 `dialog-overlay` / `dialog-panel`——样式不必搬家。
 */
export function Dialog({
  title,
  ariaLabel,
  children,
  footer,
  onClose,
  closeOnOverlay = false,
  focusPanel = true,
  panelClassName,
  size = "md"
}: {
  /** 标题栏文字。 */
  title: ReactNode;
  /** 无障碍名称；不传就用 `title` 的文本。 */
  ariaLabel?: string;
  children: ReactNode;
  footer: ReactNode;
  onClose: () => void;
  closeOnOverlay?: boolean;
  focusPanel?: boolean;
  /** 面板的附加类名（用于个别对话框的专属布局）。 */
  panelClassName?: string;
  /** 宽度档：确认框用 `sm`，设置这类大面板用 `lg`。 */
  size?: "sm" | "md" | "lg";
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    if (focusPanel) panelRef.current?.focus();
  }, [focusPanel]);

  const classes = ["dialog-panel", `dialog-panel-${size}`, panelClassName ?? ""].filter(Boolean).join(" ");

  return (
    <div
      className="dialog-overlay"
      onClick={closeOnOverlay ? onClose : undefined}
      role="presentation"
    >
      <div
        className={classes}
        role="dialog"
        aria-modal="true"
        aria-label={ariaLabel ?? (typeof title === "string" ? title : undefined)}
        tabIndex={-1}
        ref={panelRef}
        // 遮罩的点击不该穿透到面板上——否则面板内部的点击会被当成「点了外面」。
        onClick={(event) => event.stopPropagation()}
      >
        <header className="dialog-header">
          <strong>{title}</strong>
        </header>

        {children}

        <footer className="dialog-footer">{footer}</footer>
      </div>
    </div>
  );
}
