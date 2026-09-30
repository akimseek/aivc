import type { ReactNode } from "react";

/**
 * 应用骨架网格：标题栏 /〔活动栏 + 侧栏 + 主区〕/ 状态栏。
 *
 * # 它只管布局
 *
 * 不持有任何状态，也不认识任何 hook——状态全在 `app.tsx`。这样骨架本身可以
 * 单独渲染与断言，而「谁该显示什么」只有一处（`app.tsx` 的接线）。
 *
 * 行高与列宽写死在 CSS 里（40 / 50 / 310 / 28），因为它们是**这个壳的契约**：
 * 三级降级与窄窗规则都按这些数字算。
 */
export function AppShell({
  titleBar,
  activityBar,
  sidebar,
  main,
  statusBar
}: {
  titleBar: ReactNode;
  activityBar: ReactNode;
  sidebar: ReactNode;
  main: ReactNode;
  statusBar: ReactNode;
}) {
  return (
    <div className="app-shell">
      {titleBar}
      <div className="app-body">
        {activityBar}
        {sidebar}
        <div className="app-main">{main}</div>
      </div>
      {statusBar}
    </div>
  );
}
