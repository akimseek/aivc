import type { ITheme } from "@xterm/xterm";

/**
 * xterm 的明暗主题。
 *
 * # 为什么值要在这里再抄一份
 *
 * xterm 用 **canvas 画字符**，它读不到 CSS 变量——`--term-bg` 之类对它完全不存在。
 * 因此同一个底色必须存在两处：`tokens.css`（终端区那些普通元素用它）与这里
 * （xterm 自己的 `theme`）。**两处都改才是完整的改动**，只改一处会出现
 * 「终端底色变了、外面的留白没变」这种一眼就看得出的错位。
 *
 * `terminal-theme.test.ts` 把这件事钉住：它**从 `tokens.css` 里读出值**再和这里
 * 逐字比对，谁忘了改另一处，测试就红。
 *
 * 值的来源：明亮取 `:root` 的 `--term-bg` / `--term-text` / `--term-cursor`，
 * 暗黑取 `[data-theme="dark"]` 的同名三项（见 `tokens.css`）。
 */
/**
 * 终端字体栈。
 *
 * 与主题同理：xterm 读不到 CSS 变量，字体也得在这里抄一份 `--ui-font-mono`
 * （tokens.css 的排版块）的值——**两处都改才是完整的改动**。
 * `terminal-theme.test.ts` 从 tokens.css 读出来逐字比对，改漏一处就红。
 */
export const TERMINAL_FONT_FAMILY =
  '"Cascadia Mono", "Cascadia Code", Consolas, "Courier New", monospace';

export function terminalThemeFor(resolved: "light" | "dark"): ITheme {
  if (resolved === "dark") {
    return {
      background: "#0d0f17",
      foreground: "#e2e8f0",
      cursor: "#10b981",
      selectionBackground: "rgba(16, 185, 129, 0.28)",
      scrollbarSliderBackground: "rgba(148, 163, 184, 0.55)",
      scrollbarSliderHoverBackground: "rgba(148, 163, 184, 0.75)",
      scrollbarSliderActiveBackground: "rgba(148, 163, 184, 0.9)"
    };
  }
  return {
    background: "#f8fafc",
    foreground: "#1e293b",
    cursor: "#0f172a",
    selectionBackground: "rgba(5, 150, 105, 0.22)",
    scrollbarSliderBackground: "rgba(71, 85, 105, 0.5)",
    scrollbarSliderHoverBackground: "rgba(71, 85, 105, 0.7)",
    scrollbarSliderActiveBackground: "rgba(71, 85, 105, 0.85)"
  };
}
