import type { ReactNode } from "react";

export type BadgeTone = "accent" | "neutral" | "warn" | "danger";

/**
 * 徽章：消息条数、候选池状态、平台标识、表格里的结果标记等小面积信息。
 *
 * `mono` 用于数字/编号；`small` 是给密集表格的紧凑变体（此前那份靠
 * `.data-table .tag` 上下文选择器压紧的写法已删除——上下文选择器让
 * 「同样的标记移出表格就变胖」，紧凑与否现在由调用点显式声明）。
 */
export function Badge({
  tone = "neutral",
  mono = false,
  small = false,
  children
}: {
  tone?: BadgeTone;
  mono?: boolean;
  /** 紧凑变体：密集表格行里用。 */
  small?: boolean;
  children: ReactNode;
}) {
  return (
    <span className={`badge badge-${tone}${small ? " badge-sm" : ""}${mono ? " ui-num" : ""}`}>
      {children}
    </span>
  );
}
