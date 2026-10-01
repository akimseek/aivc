import { ContextMenu, ContextMenuItem } from "../../components";

import type { AiSession } from "../../types";

/**
 * 会话右键菜单。
 *
 * # 这里是**菜单项的列表**
 *
 * 打开目录 / 重命名 / 复制 / 收藏 / 复制编号 / 复制路径 / 导出三种格式 /
 * 移入回收站或彻底删除。它此前是 `app.tsx` 里 55 行内联 JSX，而每一项都是一个
 * 「调 API + 关菜单」的闭包。
 *
 * # 骨架不在这里
 *
 * 遮罩（点别处关闭）、视口内定位、右键 `preventDefault`、Escape 让位与
 * 「先做事再关菜单」都在共用的 [`ContextMenu`](../../components/context-menu.tsx)
 * 外壳里——三个右键菜单（会话 / 标签 / 终端文本）同一份实现，Escape 的判定
 * 因此只有一个地方需要维护。
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
  return (
    <ContextMenu x={x} y={y} ariaLabel="会话操作" onClose={onClose}>
      <ContextMenuItem onClick={onOpenFolder}>打开所在目录</ContextMenuItem>
      {/* 重命名与复制放在这里：会话头上虽然也有，但那要**先选中它**才点得到；
          在列表里右键直接改名/复制才是用户手上的动作。 */}
      <ContextMenuItem onClick={onRename}>重命名</ContextMenuItem>
      {canDuplicate && <ContextMenuItem onClick={onDuplicate}>复制会话</ContextMenuItem>}
      <ContextMenuItem onClick={onToggleFavorite}>
        {session.metadata?.favorite ? "取消收藏" : "收藏"}
      </ContextMenuItem>
      {/* 复制**编号**而不是标题：编号是稳定的标识，粘到别处能用来定位；
          标题可能重复，也常常很长。 */}
      <ContextMenuItem onClick={onCopyId}>复制会话编号</ContextMenuItem>
      <ContextMenuItem onClick={onCopyPath}>复制文件路径</ContextMenuItem>
      {/* 导出分区。三种格式与菜单栏那份**逐字一致**（旧平台就是这样三项），
          它们在这里是分开的三项，不再套一层子菜单——右键菜单里再展开一层，
          手要移两次。分区标题不可点：它把这一组说出来，否则三项「导出 X」挤在
          「复制文件路径」下面会被读成第四条复制项。 */}
      {onExport && (
        <>
          <div className="context-menu-label">导出</div>
          <ContextMenuItem onClick={() => onExport("markdown")}>导出 Markdown</ContextMenuItem>
          <ContextMenuItem onClick={() => onExport("json")}>导出 JSON</ContextMenuItem>
          <ContextMenuItem onClick={() => onExport("html")}>导出 HTML</ContextMenuItem>
        </>
      )}
      {/* 在回收站里时，「恢复」必须排在「彻底删除」前面：它是这一步更常用的选择，
          而销毁是不可撤销的。顺序本身就在防止误点。 */}
      {supportsTrash && trash && <ContextMenuItem onClick={onRestore}>恢复</ContextMenuItem>}
      {supportsTrash && (
        <ContextMenuItem danger onClick={onRemove}>
          {trash ? "彻底删除" : "移入回收站"}
        </ContextMenuItem>
      )}
    </ContextMenu>
  );
}
