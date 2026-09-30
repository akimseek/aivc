import { useEffect } from "react";

import { hasLayerAbove } from "../components";
import { useViewportMenuPosition } from "./viewport-menu";

import type { AiSession } from "../types";

/**
 * 会话右键菜单。
 *
 * # 从 `app.tsx` 里搬出来的两件事
 *
 * 1. **菜单项的列表**（打开目录 / 收藏 / 复制编号 / 复制路径 / 导出三种格式 /
 *    移入回收站或彻底删除）。它此前是 55 行内联 JSX，而每一项都是一个
 *    「调 API + 关菜单」的闭包；
 * 2. **视口内定位**。菜单是按鼠标位置开在 `position: fixed` 上的，靠近右/下边缘时
 *    会被切掉一半。旧平台的 `useViewportMenuPosition` 就是干这个的
 *    （`context-menus.tsx`），这里把同一件事实现一遍。
 *
 * # 「点别处关闭」用一层透明背景，不用全局 click 监听
 *
 * 全局监听会把**点菜单自身**也当成「别处」，于是点任何一项之前菜单先关了。
 */

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
  /** 重命名。会话头也有这个按钮，但右键是**就着这条会话**改名的自然位置。 */
  onRename: () => void;
  /** 复制出一条新会话。 */
  onDuplicate: () => void;
  onToggleFavorite: () => void;
  onCopyId: () => void;
  onCopyPath: () => void;
  /**
   * 导出这一条会话。
   *
   * 菜单栏被移除之后这里是导出的入口之一（spec §11 的入口迁移表）。它承接的是
   * 「**就着右键的这条**导出」——菜单栏那个导出只能导出当前选中项，而右键一条
   * 别的会话时，用户的意图显然是它。
   *
   * 不传就**整组不渲染**（沿用被移除的菜单栏那条约定的精神：宁可没有这一组，
   * 也不要一组点了没反应的项）。
   */
  onExport?: (format: "markdown" | "json" | "html") => void;
  onRemove: () => void;
  /**
   * 从回收站恢复。
   *
   * 只在 `trash` 为真时会用到。此前没有这一项：恢复只能先勾选那一行、
   * 再到批量条上点「恢复」——**右键一条会话却恢复不了它**，
   * 而右键正是「就着这一条做事」的位置。旧平台的回收站菜单里就有「恢复」。
   */
  onRestore: () => void;
  /**
   * 平台支不支持复制会话。
   *
   * Qoder 不支持（`ProviderCapabilities::duplicate == false`）。不门控的话
   * 它的用户会看到一个**点下去必然报错**的项——旧平台用 `canDuplicate` 挡过这一条。
   */
  canDuplicate?: boolean;
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
  canDuplicate = true,
  onClose,
  onOpenFolder,
  onRename,
  onDuplicate,
  onToggleFavorite,
  onCopyId,
  onCopyPath,
  onExport,
  onRemove,
  onRestore
}: SessionContextMenuProps) {
  const { menuRef, position } = useViewportMenuPosition(x, y);

  // Escape 关闭。鼠标用户有「点别处」，键盘用户只有这一条路。
  // 有模态/浮层压在上面时（退出确认框随时可能弹出来）让给它——两边都在 `document`
  // 上监听，不判断的话一次按键会同时收掉菜单与那一层。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !hasLayerAbove("transient")) onClose();
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
        aria-label="会话操作"
        style={{ left: position.x, top: position.y }}
        ref={menuRef}
        onClick={(event) => event.stopPropagation()}
      >
        <button type="button" role="menuitem" onClick={run(onOpenFolder)}>
          打开所在目录
        </button>
        {/* 重命名与复制放在这里：会话头上虽然也有，但那要**先选中它**才点得到；
            在列表里右键直接改名/复制才是用户手上的动作。 */}
        <button type="button" role="menuitem" onClick={run(onRename)}>
          重命名
        </button>
        {canDuplicate && (
          <button type="button" role="menuitem" onClick={run(onDuplicate)}>
            复制会话
          </button>
        )}
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
        {/* 导出分区。三种格式与菜单栏那份**逐字一致**（旧平台就是这样三项），
            它们在这里是分开的三项，不再套一层子菜单——右键菜单里再展开一层，
            手要移两次。 */}
        {onExport && (
          <>
            <div className="context-menu-label">导出</div>
            <button type="button" role="menuitem" onClick={run(() => onExport("markdown"))}>
              导出 Markdown
            </button>
            <button type="button" role="menuitem" onClick={run(() => onExport("json"))}>
              导出 JSON
            </button>
            <button type="button" role="menuitem" onClick={run(() => onExport("html"))}>
              导出 HTML
            </button>
          </>
        )}
        {/* 在回收站里时，「恢复」必须排在「彻底删除」前面：它是这一步更常用的选择，
            而销毁是不可撤销的。顺序本身就在防止误点。 */}
        {supportsTrash && trash && (
          <button type="button" role="menuitem" onClick={run(onRestore)}>
            恢复
          </button>
        )}
        {supportsTrash && (
          <button type="button" role="menuitem" className="danger" onClick={run(onRemove)}>
            {trash ? "彻底删除" : "移入回收站"}
          </button>
        )}
      </div>
    </div>
  );
}
