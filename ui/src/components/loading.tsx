import type { ReactNode } from "react";

/**
 * 加载态。
 *
 * # 为什么单独一层
 *
 * 「正在加载」此前在项目里散成三种写法：`.terminal-loading` 一段文字、
 * 表格里一句「正在查询…」、按钮上把文字改成「加载中…」。它们的**语义其实不同**，
 * 混成一个组件就只能取其一。这里分三种，各自对应一种真实处境：
 *
 * | 组件 | 用在哪 | 它回答的问题 |
 * | --- | --- | --- |
 * | [`Spinner`] | 按钮、行内 | 「它在动吗」 |
 * | [`LoadingBlock`] | 区域占位 | 「这一块为什么是空的」 |
 * | [`LoadingOverlay`] | 盖住已有内容 | 「旧内容还在，但正在被替换」 |
 */

/** 转圈。`label` 会同时进 `aria-label`，屏幕阅读器才读得出。 */
export function Spinner({ size = 14, label = "加载中" }: { size?: number; label?: string }) {
  return (
    <span
      className="spinner"
      role="status"
      aria-label={label}
      style={{ width: size, height: size }}
    />
  );
}

/**
 * 区域占位。
 *
 * **必须给文字**：只转圈不说话，用户分不清是在加载还是卡住了。
 */
export function LoadingBlock({ label, hint }: { label: string; hint?: ReactNode }) {
  return (
    <div className="loading-block" role="status">
      <Spinner />
      <span className="loading-label">{label}</span>
      {hint !== undefined && <span className="loading-hint">{hint}</span>}
    </div>
  );
}

/** 盖在已有内容之上。用在「刷新表格」这类**旧数据还看得见**的场合。 */
export function LoadingOverlay({ label }: { label: string }) {
  return (
    <div className="loading-overlay" role="status">
      <Spinner size={18} />
      <span className="loading-label">{label}</span>
    </div>
  );
}
