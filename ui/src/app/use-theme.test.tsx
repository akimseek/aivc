// @vitest-environment jsdom
import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { THEME_STORAGE_KEY } from "./theme";
import { useTheme } from "./use-theme";

/**
 * 主题 hook 的读写行为。
 *
 * 这里钉的是**纯逻辑之外的接线**：偏好从哪儿读、写到哪儿、谁能覆盖谁。
 * `parseThemePreference` 自身的规则在 `theme.test.ts` 里逐条断言过了，
 * 所以这里用不到 matchMedia 之外的环境设施。
 */
type ChangeListener = (event: { matches: boolean }) => void;

/** jsdom 没有 matchMedia，只能自己造一个能换系统偏好的。 */
function stubMatchMedia(initial: boolean) {
  const listeners = new Set<ChangeListener>();
  let matches = initial;
  window.matchMedia = ((query: string) => ({
    matches,
    media: query,
    addEventListener: (_type: string, listener: ChangeListener) => listeners.add(listener),
    removeEventListener: (_type: string, listener: ChangeListener) => listeners.delete(listener),
    dispatchEvent: () => false
  })) as unknown as typeof window.matchMedia;
  return {
    set(next: boolean) {
      matches = next;
      for (const listener of listeners) listener({ matches: next });
    }
  };
}

beforeEach(() => {
  window.localStorage.clear();
  delete document.documentElement.dataset.theme;
  delete document.documentElement.dataset.themePreference;
});

describe("useTheme", () => {
  it("首帧采用引导脚本写好的偏好，暗色用户不会先闪一屏白", () => {
    document.documentElement.dataset.themePreference = "dark";
    stubMatchMedia(false); // 系统是明亮，但用户明确选了暗黑。

    const { result } = renderHook(() => useTheme());

    expect(result.current.preference).toBe("dark");
    expect(result.current.resolved).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("没有引导脚本痕迹时读 localStorage，坏值回退 system 并跟随系统", () => {
    window.localStorage.setItem(THEME_STORAGE_KEY, "Dracula");
    stubMatchMedia(true);

    const { result } = renderHook(() => useTheme());

    expect(result.current.preference).toBe("system");
    expect(result.current.resolved).toBe("dark");
  });

  it("setPreference 同时落地到 dataset 与 localStorage，下次启动直接读得到", () => {
    stubMatchMedia(false);

    const { result } = renderHook(() => useTheme());
    act(() => result.current.setPreference("light"));

    expect(result.current.preference).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(document.documentElement.dataset.themePreference).toBe("light");
    expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");
  });

  it("偏好为 system 时，系统换主题要跟着换", () => {
    const media = stubMatchMedia(false);

    const { result } = renderHook(() => useTheme());
    expect(result.current.resolved).toBe("light");

    act(() => media.set(true));

    expect(result.current.resolved).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("明确偏好压过系统：系统换主题不动用户的选择", () => {
    const media = stubMatchMedia(false);

    const { result } = renderHook(() => useTheme());
    act(() => result.current.setPreference("dark"));
    act(() => media.set(true));

    expect(result.current.resolved).toBe("dark");
    act(() => result.current.setPreference("light"));
    act(() => media.set(false));

    expect(result.current.resolved).toBe("light");
  });
});
