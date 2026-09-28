import { Button, Dialog, Field, TextInput } from "../components";

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
 * 这里比旧平台多一项「已有会话目录」的只读显示：旧平台只能盲填，填错了没有反馈。
 * 该值的来源与目标列表同源（`AiTarget.codexHome`），因此不会出现「界面说 A、
 * 后端用 B」。
 */
export type SessionSettingsDialogProps = {
  /** 当前生效的会话目录（可能来自自动探测）。 */
  currentPath?: string;
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
  currentPath,
  wslPath,
  isWsl,
  supportsSessionSettings,
  onChange,
  onClose,
  onRestore,
  onSave
}: SessionSettingsDialogProps) {
  const disabled = !supportsSessionSettings || !isWsl;

  return (
    <Dialog
      title="设置会话"
      panelClassName="session-settings-dialog"
      ariaLabel="设置会话"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onRestore} disabled={disabled}>
            恢复自动探测
          </Button>
          <Button variant="primary" onClick={onSave} disabled={disabled}>
            保存
          </Button>
        </>
      }
    >
      <Field label="会话目录" hint={isWsl ? undefined : "当前目标不需要手动设置"}>
        <TextInput
          value={wslPath}
          onChange={(event) => onChange(event.target.value)}
          placeholder={isWsl ? "WSL 内 Codex 目录，例如 ~/.codex" : "当前目标不需要手动设置"}
          disabled={disabled}
          // 这是弹层的唯一输入，打开即可输入。
          autoFocus
        />
      </Field>

      {currentPath !== undefined && (
        <Field label="当前生效">
          <code title={currentPath}>{currentPath || "（未探测到）"}</code>
        </Field>
      )}
    </Dialog>
  );
}
