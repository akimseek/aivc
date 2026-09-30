/** 开关。受控组件——界面不允许再出现「本地状态与后端不一致」的开关。 */
export function Toggle({
  checked,
  onChange,
  disabled = false,
  label,
  title
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  label: string;
  title?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      title={title ?? label}
      disabled={disabled}
      className={`toggle${checked ? " on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="toggle-knob" />
    </button>
  );
}
