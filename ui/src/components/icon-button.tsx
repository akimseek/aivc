import type { ButtonHTMLAttributes, ReactNode } from "react";

/**
 * 图标按钮：方形的纯图标操作钮（关闭、刷新、折叠、回收站…）。
 *
 * # 为什么收成组件
 *
 * 此前 6 处手写 `<button className="icon-btn">`，每处都要同时记对三件事：
 * `type="button"`（漏了会在表单里意外提交）、`aria-label`（漏了读屏只念「按钮」）、
 * `icon-btn` 类名。收进来之后三件事只在这里写一次；`title` 缺省取 `label`——
 * 悬停说明与无障碍名字默认是同一句话，需要补充说明（如「关闭工作台，回到会话」）时显式传。
 *
 * `active` 只负责视觉上的 `on` 类；需要表达「按下状态」的按钮（回收站视图这类
 * 开关）由调用方透传 `aria-pressed`——普通动作钮不该带这个属性。
 */
export function IconButton({
  label,
  title,
  active = false,
  className,
  children,
  ...rest
}: {
  /** 无障碍名字（`aria-label`），必填。 */
  label: string;
  /** 悬停说明；缺省与 `label` 相同。 */
  title?: string;
  /** 选中态：补 `on` 类（如回收站视图打开时）。 */
  active?: boolean;
  children: ReactNode;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  const classes = ["icon-btn", active ? "on" : "", className ?? ""].filter(Boolean).join(" ");
  return (
    <button type="button" className={classes} title={title ?? label} aria-label={label} {...rest}>
      {children}
    </button>
  );
}
