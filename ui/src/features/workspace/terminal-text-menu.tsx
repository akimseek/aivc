import { useEffect, useRef } from "react";

import { Button, Dialog, hasLayerAbove } from "../../components";
import { useViewportMenuPosition } from "../../app/viewport-menu";

/**
 * 终端里的文本右键菜单。
 *
 * # 为什么终端需要自己的右键菜单
 *
 * 终端是个 `canvas`，浏览器的默认菜单对它没有意义（「在新标签页打开图片」之类），
 * 因此必须 `preventDefault` 掉。而一旦拦掉，**复制一条终端输出就只剩
 * 「选中 + Ctrl+C」这一条路**——鼠标用户没有出口。
 *
 * 只有两项：复制 / 粘贴。不做「全选」：终端的「全选」在几万行回滚缓冲区上是
 * 一个会把剪贴板塞满的动作，旧平台也没有它。
 *
 * # `canCopy` / `canPaste` 由调用方判定
 *
 * 这里**不去读剪贴板**。`navigator.clipboard.readText()` 需要权限、
 * 而且是异步的——菜单要在同一个事件里立刻弹出来，不能等一个 Promise。
 * 所以可用性由调用方算好传进来（见 `terminal-pane.tsx`）。
 */
export function TerminalTextContextMenu({
  x,
  y,
  canCopy,
  canPaste,
  onCopy,
  onPaste,
  onDismiss
}: {
  x: number;
  y: number;
  /** 终端里有选中的文字。 */
  canCopy: boolean;
  /** 有可写目标的终端。 */
  canPaste: boolean;
  onCopy: () => void;
  onPaste: () => void;
  onDismiss: () => void;
}) {
  const { menuRef, position } = useViewportMenuPosition(x, y);
  const dismissRef = useRef(onDismiss);
  dismissRef.current = onDismiss;

  useEffect(() => {
    // 点别处、按 Escape、滚动——三种都要收掉，否则菜单会浮在那里挡住终端。
    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Element | null;
      if (target?.closest(".context-menu")) return;
      dismissRef.current();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      // 有模态/浮层压在上面时（退出确认框随时可能弹出来）让给那一次 Escape。
      if (event.key === "Escape" && !hasLayerAbove("transient")) dismissRef.current();
    };
    const onScroll = () => dismissRef.current();

    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("scroll", onScroll, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("scroll", onScroll, true);
    };
  }, []);

  const run = (action: () => void) => () => {
    action();
    dismissRef.current();
  };

  return (
    <div
      className="menu-backdrop"
      onClick={onDismiss}
      onContextMenu={(event) => event.preventDefault()}
      role="presentation"
    >
      <div
        className="context-menu"
        role="menu"
        aria-label="终端文本操作"
        style={{ left: position.x, top: position.y }}
        ref={menuRef}
        onClick={(event) => event.stopPropagation()}
      >
        <button type="button" role="menuitem" disabled={!canCopy} onClick={run(onCopy)}>
          复制
        </button>
        <button type="button" role="menuitem" disabled={!canPaste} onClick={run(onPaste)}>
          粘贴
        </button>
      </div>
    </div>
  );
}

/**
 * 粘贴确认框。
 *
 * # 为什么粘贴要先确认
 *
 * 终端里的粘贴是**直接写进 PTY**，也就是等同于点下回车之前的那一下输入。
 * 从网页或文档里拷来的内容常常带换行——一旦直接写进去，
 * 每个换行都可能被 shell 当成一次提交。确认框让人先看清整段内容。
 *
 * 「确认后内容会粘贴到终端输入区，**不会自动提交**」这句不是客套：
 * 本实现确实不带结尾回车（走 xterm 的 `paste`，它自己处理括号粘贴）。
 *
 * 骨架用共享的 `Dialog`，不自己搭遮罩：那套东西（Escape、aria、焦点进面板）
 * 抄第二遍就会走样。
 */
export function TerminalPasteDialog({
  text,
  onCancel,
  onConfirm
}: {
  text: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const ref = useRef<HTMLTextAreaElement | null>(null);

  useEffect(() => {
    ref.current?.focus();
  }, []);

  return (
    <Dialog
      title="确认粘贴"
      // 文本框要拿焦点（用户要先看内容、可能还要滚一下），因此不把焦点给面板。
      focusPanel={false}
      onClose={onCancel}
      panelClassName="terminal-paste-panel"
      footer={
        <>
          <Button small onClick={onCancel}>
            取消
          </Button>
          <Button small variant="primary" onClick={onConfirm}>
            确认粘贴
          </Button>
        </>
      }
    >
      <p className="row-meta">确认后内容会粘贴到终端输入区，不会自动提交。</p>
      <textarea ref={ref} readOnly value={text} spellCheck={false} rows={8} />
    </Dialog>
  );
}
