import { useEffect, useLayoutEffect, useRef, useState } from "react";

/**
 * 标签的右键菜单：**关闭其他 / 关闭全部**。
 *
 * # 为什么需要它
 *
 * 开了七八路终端之后，一个一个点 ✕ 很难受。旧平台的标签条也有这组动作
 * （`context-menus.tsx` 里的终端标签菜单）。
 *
 * # 三条必须写清的规则
 *
 * | 动作 | 会关掉什么 |
 * | --- | --- |
 * | 关闭其他 | 除这一张之外**全部可关的**标签 |
 * | 关闭全部 | 全部可关的标签 |
 * | 关闭这张 | 就是这一张 |
 *
 * 「可关的」= `closable !== false`。会话标签通常标着不可关——把它一起关掉
 * 会让用户正在看的那条会话凭空消失。
 */
export type TabContextMenuProps = {
  x: number;
  y: number;
  /** 右键点中的那张标签。 */
  tabKey: string;
  tabLabel: string;
  /** 一共关得掉几张（含这一张）。为 0 时菜单里不出现任何关闭项。 */
  closableCount: number;
  onCloseThis: (key: string) => void;
  onCloseOthers: (key: string) => void;
  onCloseAll: () => void;
  onCloseMenu: () => void;
};

/** 把菜单夹在视口内。布局完成后才量得到尺寸，因此用 `useLayoutEffect`。 */
function useViewportPosition(x: number, y: number) {
  const menuRef = useRef<HTMLDivElement | null>(null);
  const [position, setPosition] = useState({ x, y });

  useLayoutEffect(() => {
    const clamp = () => {
      const element = menuRef.current;
      if (!element) return;
      const rect = element.getBoundingClientRect();
      const margin = 8;
      setPosition({
        x: Math.min(Math.max(x, margin), Math.max(margin, window.innerWidth - rect.width - margin)),
        y: Math.min(Math.max(y, margin), Math.max(margin, window.innerHeight - rect.height - margin))
      });
    };
    clamp();
    window.addEventListener("resize", clamp);
    return () => window.removeEventListener("resize", clamp);
  }, [x, y]);

  return { menuRef, position };
}

export function TabContextMenu({
  x,
  y,
  tabKey,
  tabLabel,
  closableCount,
  onCloseThis,
  onCloseOthers,
  onCloseAll,
  onCloseMenu
}: TabContextMenuProps) {
  const { menuRef, position } = useViewportPosition(x, y);

  // Escape 关闭。鼠标用户有「点别处」，键盘用户只有这一条路。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCloseMenu();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onCloseMenu]);

  const run = (action: () => void) => () => {
    action();
    onCloseMenu();
  };

  return (
    <div
      className="menu-backdrop"
      onClick={onCloseMenu}
      onContextMenu={(event) => event.preventDefault()}
      role="presentation"
    >
      <div
        className="context-menu"
        role="menu"
        aria-label={`${tabLabel} 的标签操作`}
        style={{ left: position.x, top: position.y }}
        ref={menuRef}
        onClick={(event) => event.stopPropagation()}
      >
        {/* 这张也不可关时**连「关闭这张」都不出现**：给一个点了会失败的动作，
            比不给更糟——用户会以为是自己点错了。 */}
        {closableCount > 0 && (
          <button type="button" role="menuitem" onClick={run(() => onCloseThis(tabKey))}>
            关闭这张
          </button>
        )}
        {/* 「关闭其他」在只剩一张时没有意义——它会关掉 0 张，用户以为没生效。 */}
        {closableCount > 1 && (
          <button type="button" role="menuitem" onClick={run(() => onCloseOthers(tabKey))}>
            关闭其他（{closableCount - 1}）
          </button>
        )}
        {closableCount > 0 && (
          <button type="button" role="menuitem" className="danger" onClick={run(onCloseAll)}>
            关闭全部（{closableCount}）
          </button>
        )}
      </div>
    </div>
  );
}
