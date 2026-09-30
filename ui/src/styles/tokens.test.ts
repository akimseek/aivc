// @vitest-environment node
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

/**
 * 令牌契约：`var(--…)` 引用必须都能在 `tokens.css` 找到定义。
 *
 * # 为什么这条守卫非有不可
 *
 * 一个拼错或已删除的令牌名**不会报任何错**：CSS 只是把那一条声明整条丢掉。
 * 表现是「某个按钮突然没有底色 / 某个文字变成继承色」——在明亮主题下往往还看得
 * 过去，暗色下才显形，于是它经常活到用户来报。这类缺陷已经在批次一里出现过一次
 * （B2：引用了一个不存在的令牌名）。
 *
 * 反向（tokens.css 里定义了却没人用）**不检查**：那是留白，不是缺陷，
 * 而且会逼着人删掉「这一批还没接上的令牌」。
 */

const STYLES_DIR = fileURLToPath(new URL(".", import.meta.url));
const SRC_DIR = fileURLToPath(new URL("..", import.meta.url));
const TOKENS_CSS = readFileSync(join(STYLES_DIR, "tokens.css"), "utf8");

/** `ui/src/**` 下的全部 CSS（令牌真源自己也在里面——它同样不许引用没定义的名字）。 */
function cssFiles(dir: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) found.push(...cssFiles(full));
    else if (entry.name.endsWith(".css")) found.push(full);
  }
  return found;
}

/** `--name:` 形式的定义（值里的 `--` 不会命中，因为要求后面紧跟冒号）。 */
const DEFINED = new Set(
  [...TOKENS_CSS.matchAll(/^\s*(--[\w-]+)\s*:/gm)].map((match) => match[1]!)
);

/** `var(--name)` 与 `var(--name, fallback)` 都算引用，取名字即可。 */
const REFERENCE = /var\(\s*(--[\w-]+)/g;

describe("tokens.css 的契约", () => {
  it("tokens.css 自己解析出了一批定义（哨兵：解析器坏掉时下面那条会假绿）", () => {
    expect(DEFINED.size).toBeGreaterThan(50);
    expect(DEFINED.has("--ui-accent")).toBe(true);
  });

  it("ui/src 里每个 `var(--…)` 引用都有定义", () => {
    const files = cssFiles(SRC_DIR);
    expect(files.length).toBeGreaterThan(0);

    let references = 0;
    const missing: string[] = [];
    for (const file of files) {
      const css = readFileSync(file, "utf8");
      for (const match of css.matchAll(REFERENCE)) {
        references += 1;
        const name = match[1]!;
        if (!DEFINED.has(name)) {
          missing.push(`${file.slice(SRC_DIR.length)} 引用了未定义的 ${name}`);
        }
      }
    }
    // 一个引用都没扫到时，上面那句 missing 为空是假的「通过」。
    expect(references).toBeGreaterThan(100);
    expect(missing).toEqual([]);
  });
});
