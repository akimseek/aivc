import { ListRow, MetaItem } from "../components";
import type { AiMessage, AiSession, AiTarget, SessionQuickAccessItem } from "../types";
import { displayTitle, formatWhen, pickPreview } from "./session-display";

/**
 * 会话行。出现在**四个**地方：会话列表、收藏、最近打开、全局搜索结果。
 *
 * # 为什么把它抽出来
 *
 * 这四个地方的行此前各写了一遍，于是「有几处显示了预览、有几处没显示」这类
 * 不一致就出现了。行本身只有两种形态，用 `variant` 区分：
 *
 * | 形态 | 有复选框/收藏星 | 元信息 |
 * | --- | --- | --- |
 * | `session` | 有 | 时间 + 模型 |
 * | `reference` | 无 | 它属于哪个目标 + 时间 |
 *
 * 参考形态（收藏/最近/搜索）**必须标出目标**：这些列表是跨目标的，
 * 不标出来同名会话分不清。
 */

/** 会话列表里的一行。 */
export function SessionRow({
  session,
  active,
  checked,
  onOpen,
  onToggle,
  onFavorite,
  onMenu
}: {
  session: AiSession;
  active: boolean;
  checked: boolean;
  onOpen: () => void;
  onToggle: () => void;
  onFavorite: () => void;
  onMenu: (x: number, y: number) => void;
}) {
  const preview = pickPreview(session);
  const favorite = session.metadata?.favorite === true;
  // 标题退化成了编号时显示「目录 · 时间」——列表要能区分会话，UUID 不能。
  const title = displayTitle(session);

  return (
    <ListRow
      active={active}
      titleAttr={title}
      onClick={onOpen}
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY);
      }}
      leading={
        <input
          type="checkbox"
          checked={checked}
          aria-label="选择会话"
          // 勾选不该顺带把会话也打开——那是两件事。
          onClick={(event) => event.stopPropagation()}
          onChange={onToggle}
        />
      }
      title={title}
      trailing={
        <button
          type="button"
          className={`star${favorite ? " on" : ""}`}
          aria-label={favorite ? "取消收藏" : "收藏"}
          onClick={(event) => {
            event.stopPropagation();
            onFavorite();
          }}
        >
          {favorite ? "★" : "☆"}
        </button>
      }
      preview={preview}
      meta={
        <>
          <MetaItem>{formatWhen(session.updatedAt ?? session.createdAt)}</MetaItem>
          {session.model && <MetaItem>{session.model}</MetaItem>}
        </>
      }
    />
  );
}

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
    <div className="quick-group">
      <div className="group-title">{title}</div>
      {items.length === 0 && <div className="group-empty">{empty}</div>}
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

/** 全局搜索的一条结果。**必须标出它属于哪个目标**。 */
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
    <ListRow
      titleAttr={displayTitle(session)}
      onClick={onOpen}
      title={displayTitle(session)}
      meta={<MetaItem>{target.label}</MetaItem>}
    />
  );
}

/** 正文里的一条消息。`onBranch` 只有带原始行号的消息才有。 */
export function Message({ message, onBranch }: { message: AiMessage; onBranch?: () => void }) {
  const who =
    message.role === "user"
      ? "你"
      : message.role === "assistant"
        ? "助手"
        : message.role === "tool"
          ? "工具"
          : message.role === "system"
            ? "系统"
            : "记录";
  return (
    <div className={`message ${message.role}`}>
      <div className="who">
        <span>{who}</span>
        {onBranch && (
          <button type="button" className="branch-action" onClick={onBranch}>
            从此处分支
          </button>
        )}
      </div>
      <div className="text">{message.text}</div>
    </div>
  );
}
