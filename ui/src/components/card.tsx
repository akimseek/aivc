import type { ReactNode } from "react";

/**
 * 卡片 / 区块。
 *
 * # 它与 `.field` 的分工
 *
 * `Field` 是**一个字段**（标签 + 一个控件）；`Card` 是**一组内容**（标题 + 若干字段或表格
 * + 右上角动作）。配置面里到处是后者：外部 API 接入、共享访问令牌、用量统计都是
 * 「一块有标题的区域」，此前每一处都在手抄 `section-title` 加一圈 `<div>`。
 */
export function Card({
  title,
  description,
  actions,
  children,
  className
}: {
  /** 卡片标题。**可以不给**：面板头已经写了这一页的标题时，再写一遍就是重复。 */
  title?: ReactNode;
  /** 标题下的一句说明。有些开关的后果不看说明根本猜不到。 */
  description?: ReactNode;
  /** 右上角动作（按钮、状态胶囊）。 */
  actions?: ReactNode;
  children?: ReactNode;
  className?: string;
}) {
  // 三样都没有就不渲染卡片头：一个空 `<header>` 会白占一段间距。
  const hasHead = title !== undefined || description !== undefined || actions !== undefined;
  return (
    <section className={["card", className ?? ""].filter(Boolean).join(" ")}>
      {hasHead && (
        <header className="card-head">
          <div className="card-heading">
            {title !== undefined && <h3 className="card-title">{title}</h3>}
            {description !== undefined && <p className="card-description">{description}</p>}
          </div>
          {actions !== undefined && <div className="card-actions">{actions}</div>}
        </header>
      )}
      {children !== undefined && <div className="card-body">{children}</div>}
    </section>
  );
}

/**
 * 键值对列表。
 *
 * 状态类信息（版本、路径、端口）用 `<dl>` 而不是一堆 `<div>`：屏幕阅读器能读出
 * 「项目—值」的对应关系，而 `<div>` 堆出来的一列在无障碍树里是一串孤立的文本。
 */
export function DescriptionList({
  items
}: {
  items: { label: string; value: ReactNode; tone?: "ok" | "warn" }[];
}) {
  return (
    <dl className="description-list">
      {items.map((item) => (
        <div key={item.label} className={item.tone ? `tone-${item.tone}` : undefined}>
          <dt>{item.label}</dt>
          <dd>{item.value}</dd>
        </div>
      ))}
    </dl>
  );
}
