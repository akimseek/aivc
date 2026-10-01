import { useState } from "react";

/**
 * 超过这个字数就折叠。
 *
 * 几万字的日志糊满一页会让整块内容没法读；但**丢掉也不行**——折叠只是显示层，
 * 展开看全文，一个字都不丢。
 */
export const FOLD_CHARS = 4000;

/**
 * 长文折叠：超过 `FOLD_CHARS` 只显示前一段 +「展开全文 / 收起」。
 *
 * # 为什么是共享件
 *
 * 消息弹框与历史正文讲的是同一批消息。此前只有弹框折叠，历史正文把几万字直接
 * 糊在页面上——同一条消息在两处长得完全不一样。判定阈值与展开交互只该有一份。
 */
export function FoldableText({ text, className }: { text: string; className?: string }) {
  const [expanded, setExpanded] = useState(false);
  const folded = text.length > FOLD_CHARS;
  return (
    <>
      <div className={className}>{folded && !expanded ? `${text.slice(0, FOLD_CHARS)}…` : text}</div>
      {folded && (
        <button
          type="button"
          className="fold-toggle"
          onClick={() => setExpanded((open) => !open)}
        >
          {expanded ? "收起" : `展开全文（共 ${text.length} 字）`}
        </button>
      )}
    </>
  );
}
