import type { ReactNode } from "react";

/** 面板头：标题 + 一句说明 + 右侧操作区。十个面板共用，保证「每个面板第一眼长得一样」。 */
export function PanelHeader({
  title,
  description,
  actions
}: {
  title: string;
  description?: string;
  actions?: ReactNode;
}) {
  return (
    <div className="panel-header">
      <div className="panel-header-text">
        <h3>{title}</h3>
        {description && <p>{description}</p>}
      </div>
      {actions && <div className="panel-header-actions">{actions}</div>}
    </div>
  );
}
