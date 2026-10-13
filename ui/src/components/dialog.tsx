import { useEffect, useRef, type ReactNode } from "react";

import { hasLayerAbove } from "./overlay-stack";
import { Icon } from "./icon";

/**
 * 同类 `Dialog` 之间的实例栈。
 *
 * # 为什么类名单在这里不够用
 *
 * `.dialog-overlay` 是**每一个** `Dialog` 的类名。拿类名问「上面还有没有 `.dialog-overlay`」
 * 只会得到「有」——那是我自己。同类之间谁在最上面，只有**挂载顺序**说得清：
 * 退出确认框由后端推来（`app:request-exit`），永远比用户先打开的新会话弹框挂得晚，
 * 所以那一次 Escape 该归它；反过来，新会话弹框盖在退出确认框上时（用户先点了窗口
 * 关闭、又点了「＋」）也按同一份顺序判定。
 *
 * # 与 `overlay-stack.ts` 的分工
 *
 * **同类之间用这个栈，跨类仍用类名单**：`.popover` / 右键菜单之类压在上面时，
 * 判定仍交给 `hasLayerAbove`（见那里的说明）。两者缺一不可——只有类名单，
 * 一次 Escape 会把叠着的两层 `Dialog` 一起收掉（用户刚填了一半的表单跟着没）；
 * 只有实例栈，压在上面的浮层会被无视。
 */
const dialogStack: string[] = [];

/** 实例编号的自增源。编号只用来在栈里认出自己，不参与渲染。 */
let dialogSeq = 0;

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
  showCloseButton = false,
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
  showCloseButton?: boolean;
  closeOnOverlay?: boolean;
  focusPanel?: boolean;
  /** 面板的附加类名（用于个别对话框的专属布局）。 */
  panelClassName?: string;
  /** 宽度档：确认框用 `sm`（360–440），设置这类大面板用 `lg`（≤1024/92vw）；`md` 就是基础档。 */
  size?: "sm" | "md" | "lg";
}) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  /** 本实例在栈里的编号。惰性生成一次，卸载前不变。 */
  const idRef = useRef("");
  if (idRef.current === "") idRef.current = `dialog-${++dialogSeq}`;

  // 挂载进栈、卸载出栈。放在键处理之前：同一次提交里先有位置，再接听按键。
  useEffect(() => {
    const id = idRef.current;
    dialogStack.push(id);
    return () => {
      // 从**后**往前找：同一个编号只会在栈里出现一次，`lastIndexOf` 与 `indexOf`
      // 等价，但它对「重复挂载」这种不该发生的情形也不会误删别人的位置。
      const index = dialogStack.lastIndexOf(id);
      if (index >= 0) dialogStack.splice(index, 1);
    };
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      // 同类：只有栈顶（挂得最晚的那一层）才回答。栈为空时（不该发生）不拦，
      // 保持这个组件在测试与单独使用时的原行为。
      const top = dialogStack[dialogStack.length - 1];
      if (top !== undefined && top !== idRef.current) return;
      // 跨类：`.popover` 之类压在我上面时让给它。`.dialog-overlay` 是我自己那一类，
      // 必须剔掉——否则每个 Dialog 都会看见「上面有 Dialog」，Escape 谁也收不掉。
      if (hasLayerAbove("modal", ".dialog-overlay")) return;
      onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  useEffect(() => {
    if (focusPanel) panelRef.current?.focus();
  }, [focusPanel]);

  // `md` 不加类名：它就是基础档 `.dialog-panel`，多一个空动作的 `dialog-panel-md`
  // 只会让人以为尺寸另有来源。
  const sizeClass = size === "md" ? "" : `dialog-panel-${size}`;
  const classes = ["dialog-panel", sizeClass, panelClassName ?? ""].filter(Boolean).join(" ");

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
          {showCloseButton && (
            <button type="button" className="dialog-close" aria-label="关闭" title="关闭" onClick={onClose}>
              <Icon name="close" size={15} />
            </button>
          )}
        </header>

        {children}

        <footer className="dialog-footer">{footer}</footer>
      </div>
    </div>
  );
}
