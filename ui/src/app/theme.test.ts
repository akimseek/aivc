// @vitest-environment node
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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

/**
 * 首帧引导脚本（`index.html` 里那段内联 JS）。
 *
 * # 为什么这条测试非有不可
 *
 * 首帧必须在 React 起来之前就把 `data-theme` 写好，暗色用户才不会先看到一屏白。
 * 那段脚本因此**手抄了一份** `parseThemePreference` 的判据（文件里还写着
 * 「改一处要改两处」）。抄写是会漂移的：少认识一个合法值、忘了 `system` 的回退，
 * 界面都不会报错——只是某类用户的偏好**静默失效**。
 *
 * 因此这里不比对文本，而是**把脚本真的跑起来**：抽出内联脚本，喂一组假的
 * localStorage / matchMedia / document，对同一组输入断言它与
 * `parseThemePreference` + `resolveTheme` **同解**。任何一侧改了判据，这条就红。
 */
describe("index.html 的首帧引导脚本", () => {
  const HTML = readFileSync(fileURLToPath(new URL("../../index.html", import.meta.url)), "utf8");

  /** 抽内联脚本的正文。没有属性过滤会先匹配到 `<script type="module" src=…>`。 */
  const INLINE_SCRIPT = /<script>([\s\S]*?)<\/script>/.exec(HTML)?.[1];

  /** 在一个最小环境里执行脚本，返回它写在 `<html>` 上的两个 dataset 值。 */
  function runBootstrap(
    stored: string | null,
    prefersDark: boolean,
    { storageThrows = false }: { storageThrows?: boolean } = {}
  ): { theme?: string; preference?: string } {
    if (!INLINE_SCRIPT) throw new Error("index.html 里找不到内联引导脚本");
    const dataset: Record<string, string> = {};
    const localStorage = {
      getItem: () => {
        if (storageThrows) throw new Error("localStorage 不可用");
        return stored;
      }
    };
    const window = { matchMedia: () => ({ matches: prefersDark }) };
    const document = { documentElement: { dataset } };
    new Function("localStorage", "window", "document", INLINE_SCRIPT)(
      localStorage,
      window,
      document
    );
    return { theme: dataset.theme, preference: dataset.themePreference };
  }

  it("三个合法值 + 非法值回退 system：与 parseThemePreference 逐例同解", () => {
    const cases: Array<string | null> = [
      "system",
      "light",
      "dark",
      null,
      "",
      "Dark",
      "ｄａｒｋ",
      "blue",
      "system "
    ];
    for (const raw of cases) {
      for (const prefersDark of [true, false]) {
        const out = runBootstrap(raw, prefersDark);
        expect(out.preference, `pref(${JSON.stringify(raw)})`).toBe(parseThemePreference(raw));
        expect(out.theme, `theme(${JSON.stringify(raw)}, dark=${prefersDark})`).toBe(
          resolveTheme(parseThemePreference(raw), prefersDark)
        );
      }
    }
  });

  it("localStorage 抛错时退回「跟随系统」，且不把界面留在 undefined 上", () => {
    // 隐私模式下 localStorage 可能直接抛；脚本必须还能给出一个主题。
    const out = runBootstrap("dark", true, { storageThrows: true });
    expect(out.preference).toBe("system");
    expect(out.theme).toBe("light");
  });
});
