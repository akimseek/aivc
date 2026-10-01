import { Button, Dialog, Field } from "../../components";

/**
 * 新会话 / 恢复历史会话的对话框。
 *
 * # 为什么它非有不可
 *
 * 用户报的 H2 就是这一步缺失：「**默认不选择平台、目标**，选中后直接进入终端会话」。
 * 旧平台的流程是**先在这个对话框里定下工作目录、标题、启动提示词与 CLI 参数，再打开**。
 *
 * 文案、占位符、长度上限、按钮文字都照抄旧平台
 * （`src/renderer/src/features/sessions/new-session-dialog.tsx`），因为「流程一致」
 * 包含用户看到的字面。
 *
 * # 两种形态
 *
 * | 形态 | 标题 | 字段 | 主按钮 |
 * | --- | --- | --- | --- |
 * | 新会话 | `新会话` | 工作目录 / 会话标题 / 启动提示词 / CLI 参数 | `打开` |
 * | 恢复历史（原目录不存在） | `设置工作目录` | 只有工作目录 + 一段说明 | `继续打开` |
 *
 * # 点遮罩**不关闭**
 *
 * 旧平台传的是 `closeOnOverlay={false}`。这不是小事：这个对话框里可能有用户已经敲了
 * 一半的提示词，点偏一下就全丢了。[`Dialog`] 的默认值正是它。
 */
export type NewSessionDialogProps = {
  /** 恢复历史会话、但原工作目录不存在时传入；`null` 表示新建。 */
  pendingResume: { missingCwd: string } | null;
  /** 当前目标是否支持自定义工作目录。 */
  supportsCustomCwd: boolean;
  cwd: string;
  title: string;
  prompt: string;
  cliArgs: string;
  onChooseDirectory: () => void;
  onTitleChange: (value: string) => void;
  onPromptChange: (value: string) => void;
  onCliArgsChange: (value: string) => void;
  onClose: () => void;
  onConfirm: () => void;
};

export function NewSessionDialog({
  pendingResume,
  supportsCustomCwd,
  cwd,
  title,
  prompt,
  cliArgs,
  onChooseDirectory,
  onTitleChange,
  onPromptChange,
  onCliArgsChange,
  onClose,
  onConfirm
}: NewSessionDialogProps) {
  return (
    <Dialog
      title={pendingResume ? "设置工作目录" : "新会话"}
      panelClassName="new-session-dialog"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>取消</Button>
          <Button variant="primary" onClick={onConfirm}>
            {pendingResume ? "继续打开" : "打开"}
          </Button>
        </>
      }
    >
      {pendingResume && (
        <div className="missing-directory-notice">
          <strong>原工作目录不存在</strong>
          <code title={pendingResume.missingCwd}>{pendingResume.missingCwd}</code>
          <span>请选择新的工作目录后继续打开此历史会话。</span>
        </div>
      )}

      {supportsCustomCwd && (
        <div className="directory-field">
          <span>工作目录</span>
          <code title={cwd}>{cwd}</code>
          <Button small onClick={onChooseDirectory}>
            设置工作目录
          </Button>
        </div>
      )}

      {!pendingResume && (
        <>
          <Field label="会话标题">
            <input
              value={title}
              maxLength={120}
              onChange={(event) => onTitleChange(event.target.value)}
              placeholder="可选，便于在历史列表中识别"
            />
          </Field>
          <Field label="启动提示词">
            <textarea
              value={prompt}
              onChange={(event) => onPromptChange(event.target.value)}
              placeholder="打开新会话后自动发送，可留空"
              rows={4}
            />
          </Field>
          <Field label="CLI 参数">
            <input
              value={cliArgs}
              onChange={(event) => onCliArgsChange(event.target.value)}
              placeholder="例如：--model gpt-5-codex --sandbox workspace-write"
            />
          </Field>
        </>
      )}
    </Dialog>
  );
}
