import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import { TERMINAL_FONT_FAMILY, terminalThemeFor } from "./terminal-theme";

/**
 * 把「xterm 主题」与「tokens.css」钉在一起。
 *
 * # 为什么这条测试非有不可
 *
 * xterm 读不到 CSS 变量（canvas 画字符），底色必须在 `tokens.css` 与
 * `terminal-theme.ts` 各写一份。两份值漂移时的表现是**半截错位**：终端里的底色
 * 换了，终端外那圈留白（`.terminal-stack` / `.terminal-host`）还是旧的，
 * 或者反过来。它不会报错，只会看起来「脏」。
 *
 * 因此这里**不复制常量**：直接读 `src/styles/tokens.css` 的文本，按选择器取出
 * `--term-*` 的值再与 `terminalThemeFor` 比对。改动 CSS 却没改 xterm 时，
 * 这条测试会立刻变红。
 */

const TOKENS_CSS = readFileSync(
  fileURLToPath(new URL("../styles/tokens.css", import.meta.url)),
  "utf8"
);

/** 明亮那一块：`:root,` 与 `:root[data-theme="light"]` 共用一组值。 */
const LIGHT_BLOCK = /:root,\s*\n:root\[data-theme="light"\]\s*\{([\s\S]*?)\n\}/;
/** 暗黑那一块：`data-theme` 由引导脚本写在 `<html>` 上。 */
const DARK_BLOCK = /:root\[data-theme="dark"\]\s*\{([\s\S]*?)\n\}/;

function block(selector: RegExp, css: string): string {
  const match = selector.exec(css);
  if (!match?.[1]) throw new Error(`tokens.css 里找不到 ${selector}`);
  return match[1];
}

function token(name: string, css: string): string {
  const match = new RegExp(`--${name}\\s*:\\s*([^;]+);`).exec(css);
  const value = match?.[1]?.trim();
  if (!value) throw new Error(`tokens.css 里找不到 --${name}`);
  return value;
}

describe("terminalThemeFor", () => {
  it("明亮主题与 tokens.css 的明亮值逐字一致", () => {
    const light = block(LIGHT_BLOCK, TOKENS_CSS);
    const theme = terminalThemeFor("light");
    expect(theme.background).toBe(token("term-bg", light));
    expect(theme.foreground).toBe(token("term-text", light));
    expect(theme.cursor).toBe(token("term-cursor", light));
  });

  it("暗黑主题与 tokens.css 的暗黑值逐字一致", () => {
    const dark = block(DARK_BLOCK, TOKENS_CSS);
    const theme = terminalThemeFor("dark");
    expect(theme.background).toBe(token("term-bg", dark));
    expect(theme.foreground).toBe(token("term-text", dark));
    expect(theme.cursor).toBe(token("term-cursor", dark));
  });

  it("两套主题不是同一份值——否则切换等于没切", () => {
    const light = terminalThemeFor("light");
    const dark = terminalThemeFor("dark");
    expect(light.background).not.toBe(dark.background);
    expect(light.foreground).not.toBe(dark.foreground);
    expect(light.cursor).not.toBe(dark.cursor);
  });

  it("终端字体栈与 tokens.css 的 --ui-font-mono 逐字一致", () => {
    // 字体在独立的 `:root`（排版）块里，不属于明暗任何一块——直接全文取。
    const match = /--ui-font-mono\s*:\s*([^;]+);/.exec(TOKENS_CSS);
    if (!match?.[1]) throw new Error("tokens.css 里找不到 --ui-font-mono");
    expect(TERMINAL_FONT_FAMILY).toBe(match[1].trim());
  });
});
