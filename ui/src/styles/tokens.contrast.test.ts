// @vitest-environment node
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * 对比度契约：明/暗两套主题下关键「文字 on 底」配对的 WCAG 对比度必须达标。
 *
 * # 为什么这条守卫非有不可
 *
 * 对比度是**看不见的回归**：它不报错、不动布局，只是「某天开始有点看不清」，
 * 只有截图走查才可能发现——而走查不是每天都做。批次三 T5a 抓到的两处
 * （暗色主按钮白字 2.54:1、暗色 `--ui-faint` 3.59:1）都是「token 没跟上主题」，
 * 改一处就全局生效；反过来，**改坏一处也是全局变差**。把阈值钉进测试，
 * 动 tokens.css 的当天就会红，而不是等下一次走查。
 *
 * # 判据
 *
 * WCAG 2.1：正文级 AA 4.5、AAA 7；大字号 3。这里正文类直接要求 7（现状本就有
 * 16:1 以上的余量，不存在需要放宽的理由）。亮色主按钮是与 demo 对齐、走查时
 * 记录的**已知接受项**（3.77:1），按例外断言 ≥ 3.5，防它继续往下掉。
 */

const STYLES_DIR = fileURLToPath(new URL(".", import.meta.url));
const TOKENS_CSS = readFileSync(join(STYLES_DIR, "tokens.css"), "utf8");

type Theme = "light" | "dark";

/** 去注释：注释里会**逐字引述** CSS 片段，不剥掉会把说明当声明。 */
function withoutComments(css: string): string {
  return css.replace(/\/\*[\s\S]*?\*\//g, "");
}

interface Rule {
  selectors: string[];
  body: string;
}

/** 顶层规则块：按花括号深度切（本文件没有嵌套规则，但检查器不假设这一点）。 */
function topLevelRules(css: string): Rule[] {
  const rules: Rule[] = [];
  let depth = 0;
  let selectorStart = 0;
  let selectorText = "";
  let bodyStart = -1;
  for (let i = 0; i < css.length; i += 1) {
    const ch = css[i];
    if (ch === "{") {
      if (depth === 0) {
        selectorText = css.slice(selectorStart, i);
        bodyStart = i + 1;
      }
      depth += 1;
    } else if (ch === "}") {
      depth -= 1;
      if (depth === 0) {
        rules.push({
          selectors: selectorText.split(",").map((s) => s.trim()),
          body: css.slice(bodyStart, i)
        });
        selectorStart = i + 1;
      }
    }
  }
  return rules;
}

/** 一条令牌声明（值里没有分号）。 */
const DECLARATION = /(--[\w-]+)\s*:\s*([^;]+)/g;

/**
 * 某一主题下的最终令牌表，层叠顺序与浏览器一致：裸 `:root` 块是底色（对两主题
 * 都生效——`--ui-on-accent` 的明亮值就写在那里），`:root[data-theme="…"]` 块覆盖它。
 */
function themeValues(css: string, theme: Theme): Map<string, string> {
  const base = new Map<string, string>();
  const themed = new Map<string, string>();
  for (const rule of topLevelRules(withoutComments(css))) {
    const isBase = rule.selectors.includes(":root");
    const isThemed = rule.selectors.includes(`:root[data-theme="${theme}"]`);
    if (!isBase && !isThemed) continue;
    const target = isThemed ? themed : base;
    for (const match of rule.body.matchAll(DECLARATION)) {
      target.set(match[1], match[2].trim());
    }
  }
  return new Map([...base, ...themed]);
}

interface Rgb {
  r: number;
  g: number;
  b: number;
}

interface Rgba extends Rgb {
  a: number; // 0–1
}

/**
 * 解析 hex（`#rgb` / `#rgba` / `#rrggbb` / `#rrggbbaa`）与 `rgb()` / `rgba()`
 * （逗号或空格写法，alpha 支持小数与百分比）。解析不了就抛错并把「位置 + 原值」
 * 带上——静默跳过等于守卫上有个洞，色值改成认不出的那天不会有人知道。
 */
function parseColor(value: string, where: string): Rgba {
  const text = value.trim().toLowerCase();

  const hex = /^#([0-9a-f]{3,8})$/.exec(text);
  if (hex && [3, 4, 6, 8].includes(hex[1].length)) {
    const digits = hex[1];
    if (digits.length <= 4) {
      const [r, g, b, a] = digits.split("").map((d) => parseInt(d + d, 16));
      return { r, g, b, a: digits.length === 4 ? a / 255 : 1 };
    }
    return {
      r: parseInt(digits.slice(0, 2), 16),
      g: parseInt(digits.slice(2, 4), 16),
      b: parseInt(digits.slice(4, 6), 16),
      a: digits.length === 8 ? parseInt(digits.slice(6, 8), 16) / 255 : 1
    };
  }

  const fn = /^rgba?\(([^)]*)\)$/.exec(text);
  if (fn) {
    const parts = fn[1].split(/[\s,/]+/).filter((part) => part.length > 0);
    if (parts.length === 3 || parts.length === 4) {
      const channel = (raw: string): number => {
        const n = raw.endsWith("%") ? (parseFloat(raw) / 100) * 255 : parseFloat(raw);
        if (!Number.isFinite(n)) throw new Error(`无法解析的颜色（${where}）：${value}`);
        return n;
      };
      const alphaRaw = parts.length === 4 ? parts[3] : undefined;
      const alpha =
        alphaRaw === undefined
          ? 1
          : alphaRaw.endsWith("%")
            ? parseFloat(alphaRaw) / 100
            : parseFloat(alphaRaw);
      if (!Number.isFinite(alpha)) throw new Error(`无法解析的颜色（${where}）：${value}`);
      return { r: channel(parts[0]), g: channel(parts[1]), b: channel(parts[2]), a: alpha };
    }
  }

  throw new Error(`无法解析的颜色（${where}）：${value}`);
}

/** 半透明色压在另一色上的实际颜色（source-over）。 */
function composite(fg: Rgba, bg: Rgb): Rgb {
  return {
    r: fg.r * fg.a + bg.r * (1 - fg.a),
    g: fg.g * fg.a + bg.g * (1 - fg.a),
    b: fg.b * fg.a + bg.b * (1 - fg.a)
  };
}

/** WCAG 2.1 相对亮度（sRGB 线性化后按 0.2126 / 0.7152 / 0.0722 加权）。 */
function relativeLuminance({ r, g, b }: Rgb): number {
  const linear = (v: number): number => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b);
}

function contrastRatio(fg: Rgb, bg: Rgb): number {
  const a = relativeLuminance(fg);
  const b = relativeLuminance(bg);
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

interface Palette {
  theme: Theme;
  /** 取令牌的原始色值；令牌缺失时抛错（带主题与名字），不静默跳过。 */
  color(token: string): Rgba;
}

function palette(theme: Theme): Palette {
  const values = themeValues(TOKENS_CSS, theme);
  return {
    theme,
    color(token: string): Rgba {
      const value = values.get(token);
      if (value === undefined) throw new Error(`[${theme}] 关键令牌未定义：${token}`);
      return parseColor(value, `[${theme}] ${token}`);
    }
  };
}

/**
 * 令牌对的对比度。rgba 必须先合成再算：半透明的底直接与文字比会得出一个
 * 根本不在页面上成立的比值；底半透明时压到页面底色 `--ui-bg`，字半透明时
 * 压到已算好的底上。
 */
function ratioOn(p: Palette, fgToken: string, bgToken: string): number {
  const bgRaw = p.color(bgToken);
  let bg: Rgb = bgRaw;
  if (bgRaw.a !== 1) {
    const page = p.color("--ui-bg");
    if (page.a !== 1) throw new Error(`[${p.theme}] --ui-bg 不是不透明色，无法作为合成底`);
    bg = composite(bgRaw, page);
  }
  const fgRaw = p.color(fgToken);
  const fg: Rgb = fgRaw.a === 1 ? fgRaw : composite(fgRaw, bg);
  return contrastRatio(fg, bg);
}

/** 关键配对与阈值。正文类直接上 AAA 7：现状 16:1 以上，没有放宽的理由。 */
const PAIRS: Array<{ fg: string; bg: string; min: number; why: string }> = [
  { fg: "--ui-text", bg: "--ui-bg", min: 7, why: "正文" },
  { fg: "--ui-text", bg: "--ui-bg-card", min: 7, why: "正文" },
  { fg: "--ui-muted", bg: "--ui-bg", min: 4.5, why: "次要" },
  { fg: "--ui-muted", bg: "--ui-bg-card", min: 4.5, why: "次要" },
  { fg: "--ui-faint", bg: "--ui-bg", min: 4.5, why: "弱次要（预览/时间/hint）" },
  { fg: "--ui-faint", bg: "--ui-bg-card", min: 4.5, why: "弱次要（预览/时间/hint）" },
  { fg: "--ui-danger", bg: "--ui-bg-card", min: 4.5, why: "语义·错误" },
  { fg: "--ui-warn", bg: "--ui-bg-card", min: 4.5, why: "语义·警告" }
];

describe("tokens.css 的对比度契约", () => {
  for (const theme of ["light", "dark"] as const) {
    it(`[${theme}] 关键配对达到 WCAG 阈值`, () => {
      const p = palette(theme);
      const failures = PAIRS.flatMap(({ fg, bg, min, why }) => {
        const ratio = ratioOn(p, fg, bg);
        return ratio >= min ? [] : [`${why}：${fg} on ${bg} = ${ratio.toFixed(2)}:1 < ${min}:1`];
      });
      expect(failures).toEqual([]);
    });
  }

  it("[dark] 强调底用深墨字：`--ui-on-accent` on `--ui-accent` ≥ 4.5（T5a-P1 的回归钉）", () => {
    expect(ratioOn(palette("dark"), "--ui-on-accent", "--ui-accent")).toBeGreaterThanOrEqual(4.5);
  });

  it("[light] 强调底白字 3.77:1 是与 demo 对齐的已知接受项，仍须 ≥ 3.5", () => {
    // 这条**必须**是例外而不是漏配：明亮 `--ui-accent`（#059669）与 demo 同色，
    // 走查时已记录接受 3.77:1；断言 3.5 只是防它继续往下掉（比如有人去调深强调色）。
    expect(ratioOn(palette("light"), "--ui-on-accent", "--ui-accent")).toBeGreaterThanOrEqual(3.5);
  });

  it("[dark] `--ui-faint` 仍弱于 `--ui-muted`：提亮后层次不能被抬平", () => {
    // 两者都是深底上的浅字，「更弱」= 与底色的对比更低。若 faint 被提到和 muted
    // 一样亮，预览/时间与标题/标签就分不开——T5a-P2 的修复必须保留这个台阶。
    const p = palette("dark");
    expect(ratioOn(p, "--ui-faint", "--ui-bg")).toBeLessThan(ratioOn(p, "--ui-muted", "--ui-bg"));
    expect(ratioOn(p, "--ui-faint", "--ui-bg-card")).toBeLessThan(
      ratioOn(p, "--ui-muted", "--ui-bg-card")
    );
  });

  it("哨兵：两套主题各解析到完整令牌，且值确实不同（分块器坏掉时上面全是假绿）", () => {
    const light = themeValues(TOKENS_CSS, "light");
    const dark = themeValues(TOKENS_CSS, "dark");
    expect(light.size).toBeGreaterThan(40);
    expect(dark.size).toBeGreaterThan(40);
    // 两套必须是**不同**的：分块器若把 `[data-theme]` 块并进同一主题，上面那些
    // 断言会在同一套值上做两遍，另一个主题的回归就永远看不见。
    expect(dark.get("--ui-bg")).not.toBe(light.get("--ui-bg"));
    expect(dark.get("--ui-faint")).not.toBe(light.get("--ui-faint"));
  });

  it("哨兵：解析失败 / 令牌缺失都会带上名字抛错，而不是静默跳过", () => {
    expect(() => parseColor("hsl(160 84% 39%)", "[dark] --ui-demo")).toThrow(/--ui-demo/);
    expect(() => parseColor("", "[light] --ui-demo")).toThrow(/--ui-demo/);
    const p = palette("dark");
    expect(() => ratioOn(p, "--ui-not-a-token", "--ui-bg")).toThrow(/--ui-not-a-token/);
    expect(() => ratioOn(p, "--ui-text", "--ui-not-a-token")).toThrow(/--ui-not-a-token/);
  });
});
