import type { ReactNode } from "react";

export type BadgeTone = "accent" | "neutral" | "warn" | "danger";

/** 徽章：消息条数、候选池状态、平台标识等小面积信息。mono 用于数字/编号。 */
export function Badge({
  tone = "neutral",
  mono = false,
  children
}: {
  tone?: BadgeTone;
  mono?: boolean;
  children: ReactNode;
}) {
  return <span className={`badge badge-${tone}${mono ? " ui-num" : ""}`}>{children}</span>;
}
