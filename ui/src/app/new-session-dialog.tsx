import { useEffect, useRef } from "react";

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
 * 一半的提示词，点偏一下就全丢了。
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
  const panelRef = useRef<HTMLDivElement | null>(null);

  // Escape 关闭（与旧平台一致）；遮罩点击**不**关闭。
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  // 打开时把焦点放进面板，否则键盘用户会落在背后的界面上。
  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  return (
    <div className="dialog-overlay">
      <div
        className="dialog-panel new-session-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={pendingResume ? "设置工作目录" : "新会话"}
        tabIndex={-1}
        ref={panelRef}
      >
        <header className="dialog-header">
          <strong>{pendingResume ? "设置工作目录" : "新会话"}</strong>
        </header>

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
            <button type="button" onClick={onChooseDirectory}>
              设置工作目录
            </button>
          </div>
        )}

        {!pendingResume && (
          <>
            <label className="session-template-field compact">
              <span>会话标题</span>
              <input
                value={title}
                maxLength={120}
                onChange={(event) => onTitleChange(event.target.value)}
                placeholder="可选，便于在历史列表中识别"
              />
            </label>
            <label className="session-template-field">
              <span>启动提示词</span>
              <textarea
                value={prompt}
                onChange={(event) => onPromptChange(event.target.value)}
                placeholder="打开新会话后自动发送，可留空"
                rows={4}
              />
            </label>
            <label className="session-template-field compact">
              <span>CLI 参数</span>
              <input
                value={cliArgs}
                onChange={(event) => onCliArgsChange(event.target.value)}
                placeholder="例如：--model gpt-5-codex --sandbox workspace-write"
              />
            </label>
          </>
        )}

        <footer className="dialog-footer">
          <button type="button" onClick={onClose}>
            取消
          </button>
          <button type="button" className="primary" onClick={onConfirm}>
            {pendingResume ? "继续打开" : "打开"}
          </button>
        </footer>
      </div>
    </div>
  );
}
