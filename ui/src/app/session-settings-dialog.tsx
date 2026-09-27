import { useEffect, useRef } from "react";

/**
 * 「设置会话」对话框：**手动指定 WSL 里的 Codex 会话目录**。
 *
 * 为什么需要它：WSL 目标上 Codex 的家目录由自动探测决定，而探测会受发行版、用户、
 * 以及 nvm/home 布局影响。探测不到时用户只能自己填一个 —— 没有这个入口，
 * 那台目标就等于用不了。
 *
 * # 只有 WSL 目标能用
 *
 * 本机目标不需要手动设置（它的目录就是用户目录）。因此当目标不是 WSL 时，
 * **输入框与两个按钮全部禁用**，并把这一点写在占位符里
 * （旧平台就是这么做的：`disabled = !supportsSessionSettings || !isWsl`）。
 *
 * 文案照抄旧平台 `session-settings-dialog.tsx`。
 */
export type SessionSettingsDialogProps = {
  wslPath: string;
  isWsl: boolean;
  supportsSessionSettings: boolean;
  onChange: (value: string) => void;
  onClose: () => void;
  /** 清掉手动设置，回到自动探测。 */
  onRestore: () => void;
  onSave: () => void;
};

export function SessionSettingsDialog({
  wslPath,
  isWsl,
  supportsSessionSettings,
  onChange,
  onClose,
  onRestore,
  onSave
}: SessionSettingsDialogProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const disabled = !supportsSessionSettings || !isWsl;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="dialog-overlay">
      <div
        className="dialog-panel session-settings-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="设置会话"
        tabIndex={-1}
        ref={panelRef}
      >
        <header className="dialog-header">
          <strong>设置会话</strong>
        </header>

        <label className="session-path-field">
          <span>会话目录</span>
          <input
            value={wslPath}
            onChange={(event) => onChange(event.target.value)}
            placeholder={isWsl ? "WSL 内 Codex 目录，例如 ~/.codex" : "当前目标不需要手动设置"}
            disabled={disabled}
            // eslint-disable-next-line jsx-a11y/no-autofocus -- 这是弹层的唯一输入，打开即可输入
            autoFocus
          />
        </label>

        <footer className="dialog-footer">
          <button type="button" onClick={onRestore} disabled={disabled}>
            恢复自动探测
          </button>
          <button type="button" className="primary" onClick={onSave} disabled={disabled}>
            保存
          </button>
        </footer>
      </div>
    </div>
  );
}
