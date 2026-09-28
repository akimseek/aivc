import type { ReactNode } from "react";

/**
 * 列表与行。
 *
 * # 为什么要有它
 *
 * 这个应用里**四种东西**长得几乎一样却各写了一遍：会话列表、快捷访问的两组、
 * 全局搜索结果、配置面里的表格。它们共享的其实只有三件事：一行怎么排版、
 * 行内元信息怎么排、悬停/选中怎么标。这里把这三件事固定下来，
 * 各处的差别（要不要复选框、要不要右键菜单）由具名插槽决定。
 */

export type ListRowProps = {
  /** 主行左端：复选框或图标。 */
  leading?: ReactNode;
  title: ReactNode;
  /** 主行右端：收藏星、状态标记。 */
  trailing?: ReactNode;
  /** 标题下的一行次要信息（预览文本）。 */
  preview?: ReactNode;
  /** 行底的元信息（时间、目标、模型）。 */
  meta?: ReactNode;
  active?: boolean;
  onClick?: () => void;
  onContextMenu?: (event: React.MouseEvent<HTMLDivElement>) => void;
  titleAttr?: string;
};

/**
 * 一行。
 *
 * `titleAttr` 与 `title` 是两件事：后者是**行内容**（可能被截断），
 * 前者是鼠标悬停时应该显示的**完整文本**。内容被 CSS 省略号截掉时，
 * 不给出完整文本用户就永远读不到它。
 */
export function ListRow({
  leading,
  title,
  trailing,
  preview,
  meta,
  active = false,
  onClick,
  onContextMenu,
  titleAttr
}: ListRowProps) {
  return (
    <div
      className={`row${active ? " on" : ""}`}
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={titleAttr}
    >
      <div className="row-line">
        {leading}
        <span className="row-title">{title}</span>
        {trailing}
      </div>
      {preview !== undefined && preview !== "" && <div className="row-preview">{preview}</div>}
      {meta !== undefined && <div className="row-meta">{meta}</div>}
    </div>
  );
}

/**
 * 列表容器。
 *
 * `empty` 是**必填**的：列表空着而不说明原因，用户分不清「没有数据」与「还没加载」。
 * 调用方往往已经有现成的 [`EmptyState`](./feedback.tsx)，直接传进来即可。
 */
export function List({
  children,
  empty,
  className
}: {
  children: ReactNode;
  /** 空列表时显示什么。 */
  empty?: ReactNode;
  className?: string;
}) {
  return (
    <div className={["rows", className ?? ""].filter(Boolean).join(" ")}>
      {children}
      {empty}
    </div>
  );
}

/** 行内元信息的一段：`<span>{children}</span>`，统一间距由 CSS 负责。 */
export function MetaItem({ children, title }: { children: ReactNode; title?: string }) {
  return <span title={title}>{children}</span>;
}
