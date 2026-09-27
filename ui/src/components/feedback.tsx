import type { ReactNode } from "react";

/**
 * 反馈类组件：空状态与告警条。
 *
 * 两者是**不同性质**的东西，因此分成两个组件而不是一个带 `tone` 的：
 *
 * - [`EmptyState`]：这里**没有内容**。用户需要知道「是没有，还是坏了」。
 * - [`Notice`]：这里**有问题**，而且**有出口**。必须带动作。
 */

/**
 * 空状态。
 *
 * `message` 必须说清**是哪一种空**。只说「没有数据」等于什么都没说——
 * 「没选目标」「目标进不去」「确实没有会话」是三种需要做不同事情的处境。
 */
export function EmptyState({
  message,
  hint,
  action
}: {
  message: ReactNode;
  hint?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="empty-state">
      <span className="empty-message">{message}</span>
      {hint !== undefined && <span className="empty-hint">{hint}</span>}
      {action !== undefined && <div className="empty-action">{action}</div>}
    </div>
  );
}

/**
 * 告警条：说明原因，并给出**下一步动作**。
 *
 * 没有动作的告警等于把问题丢回给用户——他看到一句失败，却不知道能做什么。
 * `actions` 因此是必需的，而不是可选的。
 */
export function Notice({
  message,
  actions
}: {
  message: ReactNode;
  actions: ReactNode;
}) {
  return (
    <div className="notice" role="status">
      <p className="notice-message">{message}</p>
      <div className="notice-actions">{actions}</div>
    </div>
  );
}
