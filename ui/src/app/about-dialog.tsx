import { useEffect } from "react";

/**
 * 「关于」对话框。
 *
 * 文案照抄旧平台 `components/about-dialog.tsx`——包括**版本读取中的兜底**
 * （`version || "正在读取"`）。这不是装饰：版本是异步取的，先弹出来的那一帧
 * 如果什么都不显示，看起来像「读不到」。
 *
 * 与旧平台的差异只有一处：那边用 lucide 的 `SquareTerminal` 图标，
 * 这边用一个字符标记，避免为了一处装饰再引一个图标依赖。
 */
export type AboutDialogProps = {
  /** 版本号；还没读到就传空串，会显示 `正在读取`。 */
  version: string;
  onClose: () => void;
};

export function AboutDialog({ version, onClose }: AboutDialogProps) {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [onClose]);

  return (
    <div className="dialog-overlay">
      <div className="dialog-panel about-dialog" role="dialog" aria-modal="true" aria-label="关于">
        <header className="dialog-header">
          <strong>关于</strong>
        </header>

        <div className="about-product">
          <div className="about-product-icon" aria-hidden="true">
            ▮
          </div>
          <div>
            <strong>AI 可视化控制台</strong>
            <span>AI CLI 会话与本地 Gateway 管理</span>
          </div>
        </div>

        <dl className="about-details">
          <div>
            <dt>版本</dt>
            <dd>{version || "正在读取"}</dd>
          </div>
        </dl>

        <p className="about-description">
          统一管理 Codex、Claude Code、Gemini 与 Qoder 会话、供应商配置和请求路由。
        </p>

        <footer className="dialog-footer">
          <button type="button" className="primary" onClick={onClose}>
            关闭
          </button>
        </footer>
      </div>
    </div>
  );
}
