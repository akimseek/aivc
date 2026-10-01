import { ContextMenu, ContextMenuItem } from "../../components";

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
 * 「可关的」由调用方数出来（`closableCount`）：统一模型里每张标签都可关，
 * 它现在就是标签总数。数量仍然要传——只剩一张时不该出现「关闭其他」
 * （那只会关掉 0 张，用户以为没生效）。
 *
 * # 骨架不在这里
 *
 * 遮罩、视口内定位、右键 `preventDefault`、Escape 让位与「先做事再关菜单」
 * 都在共用的 [`ContextMenu`](../../components/context-menu.tsx) 外壳里——
 * 与会话、终端文本两个右键菜单同一份实现。
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
  return (
    <ContextMenu x={x} y={y} ariaLabel={`${tabLabel} 的标签操作`} onClose={onCloseMenu}>
      {/* 一张都关不掉时（`closableCount` 为 0）**连「关闭这张」都不出现**：
          给一个点了会失败的动作比不给更糟——用户会以为是自己点错了。 */}
      {closableCount > 0 && (
        <ContextMenuItem onClick={() => onCloseThis(tabKey)}>关闭这张</ContextMenuItem>
      )}
      {/* 「关闭其他」在只剩一张时没有意义——它会关掉 0 张，用户以为没生效。 */}
      {closableCount > 1 && (
        <ContextMenuItem onClick={() => onCloseOthers(tabKey)}>
          关闭其他（{closableCount - 1}）
        </ContextMenuItem>
      )}
      {closableCount > 0 && (
        <ContextMenuItem danger onClick={onCloseAll}>
          关闭全部（{closableCount}）
        </ContextMenuItem>
      )}
    </ContextMenu>
  );
}
