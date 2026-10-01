import { useCallback, useEffect, useState } from "react";

import {
  parseThemePreference,
  resolveTheme,
  THEME_STORAGE_KEY,
  type ResolvedTheme,
  type ThemePreference
} from "./theme";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * 主题状态：偏好（持久化）+ 解析结果（写到 `<html data-theme>`）。
 *
 * `dataset.theme` 由这里与 `index.html` 的引导脚本共同写：引导脚本负责**首帧**
 * （避免暗色用户闪白），这里负责**之后**每次变化。两处都调 `parseThemePreference`
 * 的判据保持一致，改一处必须改另一处。
 */
export function useTheme(): {
  preference: ThemePreference;
  resolved: ResolvedTheme;
  setPreference: (preference: ThemePreference) => void;
} {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => {
    // 读的是引导脚本已经写好的值：能在首帧就与画面一致，不会闪一下。
    const fromAttr = document.documentElement.dataset.themePreference;
    if (fromAttr !== undefined) return parseThemePreference(fromAttr);
    try {
      return parseThemePreference(window.localStorage.getItem(THEME_STORAGE_KEY));
    } catch {
      // 存储被禁用（隐私模式等）时回退 `system`，与 `index.html` 引导脚本的 catch 同一判据：
      // 引导脚本对「读不到偏好」已经按 system 画好了首帧，这里再抛一次只会把首帧整个带掉。
      return "system";
    }
  });
  const [systemDark, setSystemDark] = useState(() => window.matchMedia(DARK_QUERY).matches);

  useEffect(() => {
    const media = window.matchMedia(DARK_QUERY);
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    media.addEventListener("change", onChange);
    return () => media.removeEventListener("change", onChange);
  }, []);

  const resolved = resolveTheme(preference, systemDark);

  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const setPreference = useCallback((next: ThemePreference) => {
    setPreferenceState(next);
    // 同时写 dataset.themePreference：引导脚本下次启动时用它，省一次 localStorage 读。
    document.documentElement.dataset.themePreference = next;
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      // 存储被禁用（隐私模式等）不该让切换失效——这一次会话内仍然生效。
    }
  }, []);

  return { preference, resolved, setPreference };
}
