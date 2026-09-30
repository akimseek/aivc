/**
 * 主题偏好的纯逻辑。
 *
 * 与读写的分工：**这里只做纯函数**（解析、解析结果、循环顺序、文案），
 * localStorage 与 matchMedia 的读写都在 [`useTheme`](./use-theme.ts) 与
 * `index.html` 的引导脚本里——纯函数才好逐条断言。
 */

/** 用户偏好。`system` 表示跟随操作系统。 */
export type ThemePreference = "system" | "light" | "dark";

/** 解析后的实际主题。 */
export type ResolvedTheme = "light" | "dark";

export const THEME_STORAGE_KEY = "aivc.theme";

/** 非法值回退 `system`：localStorage 是外部输入，坏值会让 dataset.theme 变成垃圾。 */
export function parseThemePreference(raw: string | null): ThemePreference {
  return raw === "light" || raw === "dark" || raw === "system" ? raw : "system";
}

export function resolveTheme(pref: ThemePreference, systemPrefersDark: boolean): ResolvedTheme {
  if (pref === "system") return systemPrefersDark ? "dark" : "light";
  return pref;
}

/** 切换顺序：明亮 → 暗黑 → 跟随系统 → …（两态切换器表达不了「跟随系统」，故三态循环）。 */
export function nextPreference(current: ThemePreference): ThemePreference {
  if (current === "light") return "dark";
  if (current === "dark") return "system";
  return "light";
}

export function themePreferenceLabel(pref: ThemePreference): string {
  if (pref === "light") return "明亮";
  if (pref === "dark") return "暗黑";
  return "跟随系统";
}
