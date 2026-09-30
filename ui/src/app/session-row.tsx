import { ListRow, MetaItem } from "../components";
import type { AiSession, AiTarget, SessionQuickAccessItem } from "../types";
import { displayTitle, formatWhen } from "./session-display";

/**
 * 会话的**参考形态**行：收藏、最近打开、全局搜索结果三处共用。
 *
 * # 为什么主列表那一行不在这里了
 *
 * 会话列表现在用 `features/sessions/session-card.tsx`：它三行结构、带状态点与
 * 悬停动作，与这里的「引用一行」是两种东西。此前两者挤在同一个文件里靠
 * `variant` 区分，结果是「有哪些按钮」和「元信息写什么」两件不相关的事混在一个参数里。
 * 这里剩下的是**只回答「这是哪条会话、它在哪」**的行——它们都跨目标，
 * 必须标出目标，否则同名会话分不清。
 */

/**
 * 快捷访问里的一组（收藏 / 最近）。
 *
 * 每组**各自封顶、独立滚动**。不包这一层时，收藏与最近两组共用外层的同一个滚动区：
 * 翻收藏的时候最近区会跟着一起跑，用户没法在某一组里连续浏览。旧平台是每组单独封顶的。
 */
export function QuickGroup({
  title,
  items,
  empty,
  onOpen
}: {
  title: string;
  items: SessionQuickAccessItem[];
  empty: string;
  onOpen: (target: AiTarget, session: AiSession) => void;
}) {
  return (
    <div className="session-group">
      <div className="session-group-title">{title}</div>
      {items.length === 0 && <div className="session-group-empty">{empty}</div>}
      {items.map((item) => (
        <ListRow
          key={`${item.target.id}:${item.session.id}`}
          titleAttr={displayTitle(item.session)}
          onClick={() => onOpen(item.target, item.session)}
          title={
            <>
              {item.favorite ? "★ " : ""}
              {displayTitle(item.session)}
            </>
          }
          meta={
            <>
              <MetaItem>{item.target.label}</MetaItem>
              <MetaItem>
                {formatWhen(item.lastOpenedAt ?? item.session.updatedAt)}
              </MetaItem>
            </>
          }
        />
      ))}
    </div>
  );
}

/**
 * 全局搜索的一条结果。
 *
 * 外面这层 `.session-result` 是给新侧栏的样式留的钩子：它只影响搜索结果这一处，
 * 不必去改公共的 `ListRow`（那是所有行共用的构件的类名）。
 */
export function SearchResultRow({
  target,
  session,
  onOpen
}: {
  target: AiTarget;
  session: AiSession;
  onOpen: () => void;
}) {
  return (
    <div className="session-result">
      <ListRow
        titleAttr={displayTitle(session)}
        onClick={onOpen}
        title={displayTitle(session)}
        meta={<MetaItem>{target.label}</MetaItem>}
      />
    </div>
  );
}
