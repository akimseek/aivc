// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  nextPreference,
  parseThemePreference,
  resolveTheme,
  themePreferenceLabel
} from "./theme";

/**
 * 主题偏好的解析规则。
 *
 * 最要紧的一条：**非法值必须回退到 system**，而不是保留坏值——
 * localStorage 是外部输入（用户可手改、旧版本可能写过别的值），
 * 留着一个不认识的字符串会让 `dataset.theme` 变成 undefined，界面配色整块失效。
 */
describe("parseThemePreference", () => {
  it("认识三个合法值", () => {
    expect(parseThemePreference("system")).toBe("system");
    expect(parseThemePreference("light")).toBe("light");
    expect(parseThemePreference("dark")).toBe("dark");
  });

  it("null 与非法值都回退到 system", () => {
    expect(parseThemePreference(null)).toBe("system");
    expect(parseThemePreference("")).toBe("system");
    expect(parseThemePreference("Dark")).toBe("system");
    expect(parseThemePreference("ｄａｒｋ")).toBe("system");
  });
});

describe("resolveTheme", () => {
  it("明确偏好优先于系统", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });

  it("system 跟随系统", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });
});

describe("nextPreference", () => {
  it("按 明亮→暗黑→跟随系统 循环", () => {
    expect(nextPreference("light")).toBe("dark");
    expect(nextPreference("dark")).toBe("system");
    expect(nextPreference("system")).toBe("light");
  });
});

describe("themePreferenceLabel", () => {
  it("每个偏好都有可读文案", () => {
    expect(themePreferenceLabel("light")).toBe("明亮");
    expect(themePreferenceLabel("dark")).toBe("暗黑");
    expect(themePreferenceLabel("system")).toBe("跟随系统");
  });
});
