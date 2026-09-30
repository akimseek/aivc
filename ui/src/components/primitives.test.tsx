// @vitest-environment jsdom
import { readFileSync } from "node:fs";
import { URL as NodeUrl, fileURLToPath } from "node:url";

import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Button } from "./primitives";

/**
 * 样式表原文。
 *
 * 两个坑，绕法都写在这里：
 *
 * - 不能用 `?raw`：Vitest 默认（`css: false`）把 CSS 导入替换成**空字符串**，
 *   断言会以「一条规则都没扫到」的假绿通过；
 * - 也不能用全局 `URL`：jsdom 环境里它是 jsdom 自己的实现，`fileURLToPath`
 *   的类型检查过不去（`The URL must be of scheme file`）。用 `node:url` 的
 *   `URL` 构造，路径就正常了。
 */
const componentsCss = readFileSync(
  fileURLToPath(new NodeUrl("./components.css", import.meta.url)),
  "utf8"
);

/**
 * 禁用态的**可读性契约**。
 *
 * # 为什么非有这一条
 *
 * 截图核对发现：预设页「新增预设」、供应商表单「创建」禁用时是**绿底 + 灰字**——
 * `.btn:disabled` 只改了字色，`.btn-primary` 的绿底留着；实测对比度约 1.26:1，
 * 字几乎读不出来。
 *
 * jsdom 量不了对比度，能做的是把「读不清」翻译成**级联的形状**，逐条钉住：
 *
 * 1. 形态：禁用只是多了 `:disabled`，类名不变（`btn btn-primary`）——不靠摘掉
 *    `btn-primary` 换一副样子；
 * 2. 令牌：承接禁用态的规则要把**底**也换成中性色，且整个样式表里没有任何
 *    `:disabled` 规则引用 `--ui-accent`（强调色只属于可用的主操作）；
 * 3. 级联：该规则的权重（类 + 伪类）严格大于 `.btn-primary`，
 *    因此它盖住绿底不依赖书写顺序。
 */

type CssRule = { selectors: string[]; body: string };

/** 拆出「选择器 → 声明块」；这份样式表没有嵌套规则，正则够用（`@media` 里的会被拆平，不影响）。 */
function cssRules(css: string): CssRule[] {
  const plain = css.replace(/\/\*[\s\S]*?\*\//g, "");
  return [...plain.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((match) => ({
    selectors: match[1]!.split(",").map((item) => item.trim().replace(/\s+/g, " ")),
    body: match[2]!
  }));
}

/** 权重只数类名与伪类：这份样式表里没有 id / 元素选择器参与禁用态这场竞争。 */
function weight(selector: string): number {
  return (selector.match(/[.:][\w-]+/g) ?? []).length;
}

const RULES = cssRules(componentsCss);

/** 声明了 `selector` 的那些块（原本写在逗号列表里的也算）。 */
function blocksOf(selector: string): CssRule[] {
  return RULES.filter((rule) => rule.selectors.includes(selector));
}

/** 一条块里最重的那个选择器——盖不盖得住别人，看的是它。 */
function strongest(rule: CssRule): number {
  return Math.max(...rule.selectors.map(weight));
}

/** 真的会命中禁用态的规则才算：`:not(:disabled)`（悬停态排除禁用）说的是相反的事。 */
function targetsDisabled(selector: string): boolean {
  return /:disabled/.test(selector.replace(/:not\([^)]*\)/g, ""));
}

describe("禁用的主按钮：底与字都必须是中性的", () => {
  it("渲染出来仍是 `btn btn-primary`，靠 `disabled` 变灰（不摘类名换样子）", () => {
    const { container } = render(
      <Button variant="primary" disabled>
        新增预设
      </Button>
    );
    const button = container.querySelector("button")!;
    expect(button.className).toBe("btn btn-primary");
    expect(button.disabled).toBe(true);
  });

  it("`.btn:disabled` 把底一起换掉（只改字色正是这次的缺陷）", () => {
    const rules = blocksOf(".btn:disabled");
    expect(rules).toHaveLength(1);
    const body = rules[0]!.body;
    expect(body).toContain("background: var(--ui-bg-badge)");
    expect(body).toContain("color: var(--ui-faint)");
    expect(body).toContain("border-color: var(--ui-border)");
    expect(body).toContain("cursor: not-allowed");
  });

  it("所有**命中禁用态**的规则都不引用 `--ui-accent`（强调色只属于可用的主操作）", () => {
    const offenders = RULES.filter(
      (rule) =>
        rule.selectors.some(targetsDisabled) &&
        (rule.body + rule.selectors.join(" ")).includes("--ui-accent")
    );
    expect(offenders).toEqual([]);
  });

  it("禁用态的权重严格大于 `.btn-primary`：盖住绿底不依赖书写顺序", () => {
    const disabled = blocksOf(".btn:disabled")[0]!;
    const primary = blocksOf(".btn-primary")[0]!;
    expect(strongest(disabled)).toBeGreaterThan(strongest(primary));
  });
});

describe("chips 不在自己身上折行", () => {
  it("`.chip` 声明了 `white-space: nowrap`，`.chip-group` 允许在组内折行", () => {
    // 折行会把同一排的 chip 撑成两倍高，选中态那个圆角胶囊变成竖直大椭圆。
    expect(blocksOf(".chip")[0]!.body).toContain("white-space: nowrap");
    // 只给 chip 加 nowrap，一排放不下时会**溢出所在的列**，所以组要能折行。
    expect(blocksOf(".chip-group")[0]!.body).toContain("flex-wrap: wrap");
  });
});
