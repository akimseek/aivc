import type { KeyboardEvent, MouseEvent, ReactNode } from "react";

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
  /**
   * 主行左端：复选框或图标。
   *
   * 它被包在 `.row-slot` 里——那一层只做一件事：点击不冒泡到行（见下文的「鼠标那一半」）。
   */
  leading?: ReactNode;
  title: ReactNode;
  /** 主行右端：收藏星、状态标记。与 `leading` 一样包在 `.row-slot` 里。 */
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
 *
 * 有 `onClick` 才渲染成可交互元素（`role="button"` + `tabIndex` + Enter/Space）：
 * 行本身是 `<div>`，只挂 `onClick` 的话键盘用户**完全够不着**它——快捷访问与全局
 * 搜索的结果行都会因此变成鼠标专属。没有 `onClick` 的行保持纯展示，不该进 Tab 序。
 *
 * 键处理开头那条 `event.target !== event.currentTarget` 是**让位**：焦点落在
 * `leading` / `trailing` 插槽里的控件上时，那个控件自己就认 Enter / Space。今天
 * 没有调用方往插槽里放可聚焦元素，但少了这一句，将来一放就会复现会话卡片那类劫持
 * （点勾选/按钮顺带触发行点击，而且 `preventDefault()` 还会让内层控件按不动）。
 *
 * # 鼠标那一半：插槽自己吃掉点击
 *
 * 键处理那一条只管键盘。鼠标点插槽里的 `<button>` 时，点击会**冒泡到行**上，
 * 于是行也跟着开一次——一次点击做了两件事。挡法放在**插槽容器**上
 * （`.row-slot` 的 `onClick` 里 `stopPropagation`），而不是在行的 `onClick` 里
 * 列举「哪些元素算交互元素」：插槽里能放什么由调用方决定（按钮、勾选框、
 * 自绘控件），列举法一定会漏，而漏掉的那一个就是「点它顺带开一条会话」。
 *
 * 这是**契约**（`list.test.tsx` 逐条钉着）：调用方可以放心把按钮、勾选框放进
 * `leading` / `trailing`，那一下点击只属于控件自己，不会顺带跳到行上；反过来，
 * 行级的动作（打开会话、切换选中）别塞进插槽——它按定义不触发行点击。
 *
 * 只挡 `click`：`contextmenu` 照旧冒泡（行右键菜单讲的就是这一行，插槽里的控件
 * 没有自己的菜单要开）。
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
  /** 插槽容器的点击处理器：吃掉冒泡，行不再跟着响应（理由见文件头）。 */
  const stop = (event: MouseEvent<HTMLElement>) => event.stopPropagation();

  return (
    <div
      className={`row${active ? " on" : ""}`}
      {...(onClick
        ? {
            role: "button",
            tabIndex: 0,
            onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => {
              if (event.target !== event.currentTarget) return;
              if (event.key !== "Enter" && event.key !== " ") return;
              // Space 默认滚动页面；Enter 的默认行为在个别嵌入元素里也可能触发。
              event.preventDefault();
              onClick?.();
            }
          }
        : {})}
      onClick={onClick}
      onContextMenu={onContextMenu}
      title={titleAttr}
    >
      <div className="row-line">
        {leading !== undefined && <span className="row-slot" onClick={stop}>{leading}</span>}
        <span className="row-title">{title}</span>
        {trailing !== undefined && <span className="row-slot" onClick={stop}>{trailing}</span>}
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
