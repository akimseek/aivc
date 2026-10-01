import { createContext, useContext, useEffect, type ReactNode } from "react";

import { hasLayerAbove } from "./overlay-stack";
import { useViewportMenuPosition } from "./viewport-menu";

/**
 * 右键菜单外壳：遮罩 + 定位 + 「做一件事然后关掉」+ Escape。
 *
 * # 为什么共用一份
 *
 * 会话菜单、标签菜单、终端文本菜单此前各抄了一遍同一套骨架：透明遮罩
 * （点别处关闭）、视口内定位（`useViewportMenuPosition`）、菜单自身吞掉点击、
 * 右键时 `preventDefault`、每一项「先做事再关菜单」、以及 Escape 的
 * [`hasLayerAbove("transient")`](./overlay-stack.ts) 让位判定。三份手写就有三处
 * 走样的机会——漏一个让位判定，一次 Escape 就会同时收掉菜单与它上面那层对话框。
 *
 * # 「点别处关闭」用一层透明背景，不用全局 click 监听
 *
 * 全局监听会把**点菜单自身**也当成「别处」，于是点任何一项之前菜单先关了。
 *
 * # 关菜单的手柄为什么走 context
 *
 * 每一项都是「动作 + 关掉」两个动作。手柄若由调用方逐项传入（或每个项自己写
 * `onClick={run(...)}`），等于把同一段样板再抄 N 遍——这正是要消掉的重复。
 * 通道只在本文件内部（`createContext` 不导出），调用方只见「文案 + 动作」。
 */

/** 「点完关掉」通道。只有 `ContextMenu` 提供、`ContextMenuItem` 消费。 */
const closeChannel = createContext<(() => void) | null>(null);

export function ContextMenu({
  x,
  y,
  ariaLabel,
  onClose,
  children
}: {
  /** 鼠标位置（视口坐标）；贴边时被夹回视口内。 */
  x: number;
  y: number;
  /** 无障碍名称：这个菜单是给什么对象开的（读屏只报一串菜单项时靠它分辨）。 */
  ariaLabel: string;
  onClose: () => void;
  children: ReactNode;
}) {
  const { menuRef, position } = useViewportMenuPosition(x, y);

  // Escape 关闭。鼠标用户有「点别处」（遮罩），键盘用户只有这一条路。
  // 有模态/浮层压在上面时（退出确认框随时可能弹出来）让给它——两边都在
  // `document` 上监听，不判断的话一次按键会同时收掉菜单与那一层。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !hasLayerAbove("transient")) onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div
      className="menu-backdrop"
      onClick={onClose}
      onContextMenu={(event) => event.preventDefault()}
      role="presentation"
    >
      <div
        className="context-menu"
        role="menu"
        aria-label={ariaLabel}
        style={{ left: position.x, top: position.y }}
        ref={menuRef}
        onClick={(event) => event.stopPropagation()}
      >
        <closeChannel.Provider value={onClose}>{children}</closeChannel.Provider>
      </div>
    </div>
  );
}

/**
 * 一项菜单动作。
 *
 * 「先做事、再关菜单」的顺序收在这里：先把动作交出去（同步），再收菜单——
 * 反过来会在动作依赖「菜单里的状态」时读到已经作的废值。
 */
export function ContextMenuItem({
  onClick,
  danger = false,
  disabled = false,
  children
}: {
  onClick: () => void;
  /** 危险动作（一次销毁许多东西的那种）在菜单里也要能认出来。 */
  danger?: boolean;
  disabled?: boolean;
  children: ReactNode;
}) {
  const close = useContext(closeChannel);
  return (
    <button
      type="button"
      role="menuitem"
      className={danger ? "danger" : undefined}
      disabled={disabled}
      onClick={() => {
        onClick();
        close?.();
      }}
    >
      {children}
    </button>
  );
}
