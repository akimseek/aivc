export type StatusDotTone = "ok" | "warn" | "danger" | "idle";

/** 静态状态点：用颜色表达状态，避免空闲界面持续触发 WebView2 合成绘制。 */
export function StatusDot({
  tone = "idle",
  size = 8,
  title
}: {
  tone?: StatusDotTone;
  size?: number;
  title?: string;
}) {
  return (
    <span
      className={`status-dot-pin dot-${tone}`}
      style={{ width: size, height: size }}
      title={title}
      aria-hidden="true"
    />
  );
}
