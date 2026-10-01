import { forwardRef, type ButtonHTMLAttributes, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes } from "react";

/**
 * 基础控件。
 *
 * # 为什么集中在这里
 *
 * 这一层解决的是**风格一致性**：按钮、下拉、搜索框的边框、圆角、字号、悬停与聚焦态
 * 只在一个地方定义。散落在各页面里写的化，第一个页面和第五个页面一定会长得不一样。
 *
 * 样式在 `components.css` 的「基础控件」段；这里只负责结构，不写内联样式。
 */

/* ── 按钮 ────────────────────────────────────────────── */

export type ButtonVariant = "default" | "primary" | "danger" | "ghost";

export function Button({
  variant = "default",
  small = false,
  className,
  children,
  ...rest
}: {
  variant?: ButtonVariant;
  /** 小号：用于工具栏、行内操作。 */
  small?: boolean;
  children: ReactNode;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  const classes = ["btn", `btn-${variant}`, small ? "btn-sm" : "", className ?? ""]
    .filter(Boolean)
    .join(" ");
  // `type` 默认 button：忘了写会让按钮在表单里意外提交。
  return (
    <button type="button" className={classes} {...rest}>
      {children}
    </button>
  );
}

/* ── 分段切换（活跃/回收站、视图切换…）─────────────────── */

export type ChipOption = {
  value: string;
  label: string;
  /**
   * 不可用。
   *
   * 有些项在当前平台上没有意义（例如 Qoder 上的「回收站」）。
   * **禁用而不是隐藏**：隐藏会让那一行在不同平台上长度不一样，
   * 用户会以为功能被删了；禁用配上 `title` 才说得出「为什么不能点」。
   */
  disabled?: boolean;
  /** 鼠标悬停时的说明。禁用时通常写清原因。 */
  title?: string;
};

export function ChipGroup({
  value,
  options,
  onChange,
  ariaLabel
}: {
  value: string;
  options: ChipOption[];
  onChange: (value: string) => void;
  /**
   * 这一组 chips 在无障碍树里的名字。
   *
   * `role="group"` 没有名字时，读屏只能念出一串孤立的按钮，说不出它们是干什么的；
   * 一页上有多组 chips（例如供应商页的厂商筛选与表单里的模型厂商）时尤其分不清。
   * 旧平台的筛选下拉带 `aria-label`，换成 chips 后那条信息不能丢。
   */
  ariaLabel?: string;
}) {
  return (
    <div className="chip-group" role="group" aria-label={ariaLabel}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          className={`chip${option.value === value ? " on" : ""}`}
          aria-pressed={option.value === value}
          disabled={option.disabled}
          title={option.title}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/* ── 表单字段 ────────────────────────────────────────── */

/**
 * 字段外壳：标签在上、控件在下、提示在最后。
 *
 * 用 `<label>` 包住控件，点标签即可聚焦——手写时很容易漏掉。
 */
export function Field({
  label,
  hint,
  children
}: {
  label: string;
  hint?: ReactNode;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span className="field-label">{label}</span>
      {children}
      {hint !== undefined && <span className="field-hint">{hint}</span>}
    </label>
  );
}

export function Select({ className, children, ...rest }: SelectHTMLAttributes<HTMLSelectElement>) {
  return (
    <select className={["control", "control-select", className ?? ""].filter(Boolean).join(" ")} {...rest}>
      {children}
    </select>
  );
}

/**
 * 单行文本输入。
 *
 * # 为什么要 `forwardRef`
 *
 * 「打开查找栏就聚焦输入框」这类动作需要一个真实的 DOM 引用去调 `focus()`。
 * 组件包住 `<input>` 之后，父组件拿不到那个元素——除非把 ref 透下去。
 * 不透的话调用方只能 `querySelector` 去猜，而那在组件改结构时会静默失效。
 */
export const TextInput = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  function TextInput({ className, ...rest }, ref) {
    return (
      <input
        ref={ref}
        className={["control", className ?? ""].filter(Boolean).join(" ")}
        {...rest}
      />
    );
  }
);

/**
 * 搜索框。
 *
 * `busy` 不是装饰：后端的搜索**第二段会扫全文**，慢到需要让用户知道「它在找」，
 * 否则界面看起来像卡住了。
 */
export function SearchInput({
  value,
  onChange,
  placeholder,
  busy = false
}: {
  value: string;
  onChange: (value: string) => void;
  placeholder?: string;
  busy?: boolean;
}) {
  return (
    <div className="search-input">
      <input
        type="search"
        value={value}
        placeholder={placeholder}
        onChange={(event) => onChange(event.target.value)}
      />
      {busy && <span className="search-busy">搜索中…</span>}
    </div>
  );
}

/* ── 工具栏 ──────────────────────────────────────────── */

export function Toolbar({ children }: { children: ReactNode }) {
  return <div className="toolbar">{children}</div>;
}

/** 撑开剩余空间的占位；用它比到处写 `<span className="spacer" />` 更不容易漏。 */
export function ToolbarSpacer() {
  return <span className="spacer" />;
}
