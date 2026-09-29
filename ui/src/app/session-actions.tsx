import { Button } from "../components";
import type { AiSession } from "../types";

/**
 * 「详情」按钮：打开会话详情弹层。
 *
 * # 为什么只有这一个按钮
 *
 * 这里原来是一条**六按钮的动作条**：详情、重命名、复制、导出 MD、导出 JSON、另存到…
 * ——那是我自己造的，旧平台没有。
 *
 * 旧平台的终端工作区头部**只有两个**按钮（`terminal-workspace.tsx`）：
 *
 * ```jsx
 * {activeSession && <button onClick={() => onOpenDetail(activeSession)}>详情</button>}
 * {providerId && targetId && <button onClick={onNewSession}>新会话</button>}
 * ```
 *
 * 其余动作在别处，而且**别处我都已经有了**：
 *
 * | 动作 | 旧平台放哪 | 本工程 |
 * | --- | --- | --- |
 * | 重命名 / 复制 | 会话行右键菜单 | 已有（`session-context-menu.tsx`） |
 * | 导出 Markdown / JSON / HTML | 菜单栏「文件 / 导出」 | 已有（`app-menus.ts`） |
 *
 * 所以那四个按钮是纯多余，而且它们**看起来像是这条会话的操作**，
 * 实际作用在「当前选中的那条」——选中项又曾经是自动挑的第一条，
 * 于是点下去改的往往不是用户以为的那条。
 *
 * # 为什么它必须放在标签栏那一行，而不是会话正文的头部
 *
 * 放在正文头部时，**点历史会话进入终端后它就消失了**（那时不渲染正文），
 * 于是「进了终端就再也看不到详情、也就到不了『从此处分支』」。
 * 旧平台把它放在**标签之上**那一行，两种视图下都在。
 */
export function SessionDetailButton({
  session,
  onOpenDetail
}: {
  /** 要查看的会话；没有就不渲染。 */
  session: AiSession | null;
  onOpenDetail: (session: AiSession) => void;
}) {
  if (!session) return null;
  return (
    <Button small onClick={() => onOpenDetail(session)} title="打开会话详情（可从此处分支）">
      详情
    </Button>
  );
}
