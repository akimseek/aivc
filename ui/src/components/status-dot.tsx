export type StatusDotTone = "ok" | "warn" | "danger" | "idle";

/** 状态点。`pulse` 表示「进行中」（运行中的终端、在线网关）。 */
export function StatusDot({
  tone = "idle",
  pulse = false,
  size = 8,
  title
}: {
  tone?: StatusDotTone;
  pulse?: boolean;
  size?: number;
  title?: string;
}) {
  return (
    <span
      className={`status-dot-pin dot-${tone}${pulse ? " pulse" : ""}`}
      style={{ width: size, height: size }}
      title={title}
      aria-hidden="true"
    />
  );
}
