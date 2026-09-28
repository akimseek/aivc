import { useEffect, useLayoutEffect, useRef, useState } from "react";

import type { AiSession } from "../types";

/**
 * 会话右键菜单。
 *
 * # 从 `app.tsx` 里搬出来的两件事
 *
 * 1. **菜单项的列表**（打开目录 / 收藏 / 复制编号 / 复制路径 / 移入回收站或彻底删除）。
 *    它此前是 55 行内联 JSX，而每一项都是一个「调 API + 关菜单」的闭包；
 * 2. **视口内定位**。菜单是按鼠标位置开在 `position: fixed` 上的，靠近右/下边缘时
 *    会被切掉一半。旧平台的 `useViewportMenuPosition` 就是干这个的
 *    （`context-menus.tsx`），这里把同一件事实现一遍。
 *
 * # 「点别处关闭」用一层透明背景，不用全局 click 监听
 *
 * 全局监听会把**点菜单自身**也当成「别处」，于是点任何一项之前菜单先关了。
 */

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
      const maxX = Math.max(margin, window.innerWidth - rect.width - margin);
      const maxY = Math.max(margin, window.innerHeight - rect.height - margin);
      setPosition({
        x: Math.min(Math.max(x, margin), maxX),
        y: Math.min(Math.max(y, margin), maxY)
      });
    };
    clamp();
    window.addEventListener("resize", clamp);
    return () => window.removeEventListener("resize", clamp);
  }, [x, y]);

  return { menuRef, position };
}

export type SessionContextMenuProps = {
  x: number;
  y: number;
  session: AiSession;
  /** 在回收站里时，删除项变成「彻底删除」。 */
  trash: boolean;
  /** 该目标的会话支不支持回收站；不支持时隐藏删除项，而不是给一个必然失败的动作。 */
  supportsTrash?: boolean;
  onClose: () => void;
  onOpenFolder: () => void;
  onToggleFavorite: () => void;
  onCopyId: () => void;
  onCopyPath: () => void;
  onRemove: () => void;
};

/*
 * `session` 是**必填**的 `AiSession`，不是 `null`。
 *
 * 这一点是刻意的：菜单要显示「取消收藏 / 收藏」，需要知道当前状态；
 * 而调用方手里可能是 `null`（会话刚被别处删掉）。那种情况下**不该渲染这个菜单**，
 * 而不是渲染一个所有项都不知道自己在操作什么的菜单。类型上直接要求非空，
 * 调用方的分支就没法漏。
 */

export function SessionContextMenu({
  x,
  y,
  session,
  trash,
  supportsTrash = true,
  onClose,
  onOpenFolder,
  onToggleFavorite,
  onCopyId,
  onCopyPath,
  onRemove
}: SessionContextMenuProps) {
  const { menuRef, position } = useViewportPosition(x, y);

  // Escape 关闭。鼠标用户有「点别处」，键盘用户只有这一条路。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  /** 每一项都是「做一件事然后关掉」；抽出来省得五行写五遍。 */
  const run = (action: () => void) => () => {
    action();
    onClose();
  };

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
        style={{ left: position.x, top: position.y }}
        ref={menuRef}
        onClick={(event) => event.stopPropagation()}
      >
        <button type="button" role="menuitem" onClick={run(onOpenFolder)}>
          打开所在目录
        </button>
        <button type="button" role="menuitem" onClick={run(onToggleFavorite)}>
          {session.metadata?.favorite ? "取消收藏" : "收藏"}
        </button>
        {/* 复制**编号**而不是标题：编号是稳定的标识，粘到别处能用来定位；
            标题可能重复，也常常很长。 */}
        <button type="button" role="menuitem" onClick={run(onCopyId)}>
          复制会话编号
        </button>
        <button type="button" role="menuitem" onClick={run(onCopyPath)}>
          复制文件路径
        </button>
        {supportsTrash && (
          <button type="button" role="menuitem" className="danger" onClick={run(onRemove)}>
            {trash ? "彻底删除" : "移入回收站"}
          </button>
        )}
      </div>
    </div>
  );
}
