/** 进度条。value 为 0–100，超界夹取；tone 由调用方按语义给（≥80% 用 warn）。 */
export function ProgressBar({
  value,
  tone = "accent",
  label
}: {
  value: number;
  tone?: "accent" | "warn" | "danger";
  label?: string;
}) {
  const clamped = Math.min(100, Math.max(0, value));
  return (
    <div className={`progress progress-${tone}`} role="progressbar" aria-valuenow={clamped} aria-valuemin={0} aria-valuemax={100} aria-label={label}>
      <div className="progress-fill" style={{ width: `${clamped}%` }} />
    </div>
  );
}
