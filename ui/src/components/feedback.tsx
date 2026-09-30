import type { ReactNode } from "react";

import { Button } from "./primitives";

/**
 * 反馈类组件：空状态、告警条与一句话提示。
 *
 * 三者是**不同性质**的东西，因此分成三个组件而不是一个带 `tone` 的：
 *
 * - [`EmptyState`]：这里**没有内容**。用户需要知道「是没有，还是坏了」。
 * - [`Notice`]：这里**有问题**，而且**有出口**。必须带动作。
 * - [`HintBar`]：这里**刚做完一件事**。说完就该消失，不需要出口。
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

/**
 * 一句话提示条：某个动作**做完了**的瞬时回音（导出到哪、清了 N 条日志、余额刷新结果）。
 *
 * # 为什么要收成一个组件
 *
 * 它此前有三处手写：应用级两条提示（浮层形态）、供应商页与请求日志页（面板内形态）。
 * 三处的类名组合还不一样，而 `hint-float` 为了浮起来把底色、边框又写了一遍——
 * 与 `.export-hint` 的两条规则互相压（浮层的边框被 `border-bottom` 压掉一条）。
 * 现在外观只在 `.export-hint` 一处定义，`floating` 只加「浮起来」的位置。
 *
 * # `floating`
 *
 * 浮层形态是**压在界面之上**的（外壳是 100vh 网格，插进正文会把状态栏挤出去），
 * 面板内形态则占面板顶部一行。
 */
export function HintBar({
  message,
  onDismiss,
  extra,
  floating = false
}: {
  message: ReactNode;
  onDismiss: () => void;
  /** 「打开」这类附加动作。 */
  extra?: ReactNode;
  /** 浮在界面之上（而不是在面板里占一行）。 */
  floating?: boolean;
}) {
  return (
    <div className={floating ? "export-hint hint-float" : "export-hint"} role="status">
      {message}
      {extra}
      <Button small onClick={onDismiss}>
        知道了
      </Button>
    </div>
  );
}
