/**
 * 「功能是否已经对齐旧平台」的一次可量化检查。
 *
 * # 做法
 *
 * 把旧平台前端（`src/renderer/src`）里**用户可见的中文串**抽出来——JSX 文本、
 * `label=` / `title=` / `placeholder=` 的值——再逐个到新平台的源码里找。
 * 找不到的就是**候选缺口**：用户能在旧平台看到这一句话，而在新平台上它不存在。
 *
 * # 它测不了什么（必须说清）
 *
 * - **只看字符串**，不看行为。同一个词可能在新平台上做的是另一件事。
 * - 找不到**不等于**缺功能：文案可能被改写得更清楚了（这一轮我改过不少）。
 *   因此结果是**候选清单**，要逐条人看。
 * - 反过来，字符串在、功能不在的情况它**测不出来**（例如终端类型选择：
 *   新平台压根没有那几个词）。
 *
 * 用法：node scripts/audit-legacy-strings.cjs
 */
const fs = require("fs");
const path = require("path");

const LEGACY = "D:/code/ai/src/renderer/src";
const NEXT = "D:/code/ai/aivc-rs/ui/src";

function walk(dir, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "node_modules") continue;
      walk(full, out);
    } else if (/\.(tsx?|jsx?)$/.test(entry.name)) {
      out.push(full);
    }
  }
  return out;
}

/** 抽出「用户可见的中文串」。 */
function visibleStrings(source) {
  const found = new Set();
  const push = (value) => {
    let text = value.replace(/\s+/g, " ").trim();
    // 去掉 JSX 解析残留：正则跨元素时会带出 `>` `<` `</` 这类东西。
    text = text.replace(/^[>)\]}]+/, "").replace(/[<(=[{]+$/, "").trim();
    if (!/[\u4e00-\u9fa5]/.test(text)) return;
    if (text.length < 2 || text.length > 40) return;
    if (!/[\u4e00-\u9fa5]{2}/.test(text)) return;
    // 含代码符号的一律当噪声丢掉。真正给用户看的文案里不会有 `<` `{` `;` 之类。
    if (/[<>{}()[\];/*"'`\\|]/.test(text)) return;
    // 模板串的插值残留。
    if (/\$\{/.test(text)) return;
    found.add(text);
  };

  // JSX 文本：>文本<（**不跨行**，跨行会把多个元素粘成一条）
  for (const match of source.matchAll(/>([^<>{}\n]{2,60})</g)) push(match[1]);
  // 属性值：label="…" / title="…" / placeholder="…" / aria-label="…"
  for (const match of source.matchAll(
    /(?:label|title|placeholder|aria-label)\s*=\s*["']([^"']{2,60})["']/g
  )) {
    push(match[1]);
  }
  // 字符串字面量（"中文"）
  for (const match of source.matchAll(/["']([^"'\n]*[\u4e00-\u9fa5][^"'\n]*)["']/g)) push(match[1]);
  return found;
}

const legacyFiles = walk(LEGACY).filter((file) => !/\.test\./.test(file));
const nextFiles = walk(NEXT).filter((file) => !/\.test\./.test(file));

/** 文案 → 它在旧平台出现在哪个文件（分类时靠这个拿上下文）。 */
const origin = new Map();
for (const file of legacyFiles) {
  for (const value of visibleStrings(fs.readFileSync(file, "utf8"))) {
    if (!origin.has(value)) origin.set(value, []);
    origin.get(value).push(path.basename(file));
  }
}

const nextText = nextFiles.map((file) => fs.readFileSync(file, "utf8")).join("\n");

const missing = [...origin.keys()].filter((value) => !nextText.includes(value)).sort();

console.log(`旧平台文件 ${legacyFiles.length} 个，抽出可见中文串 ${origin.size} 条`);
console.log(`新平台源码里找不到的：${missing.length} 条`);

// 按来源文件分组输出：分类时要靠上下文判断「真缺功能」还是「只是措辞」。
const byFile = new Map();
for (const value of missing) {
  for (const file of origin.get(value)) {
    if (!byFile.has(file)) byFile.set(file, []);
    byFile.get(file).push(value);
  }
}

const lines = [];
for (const [file, values] of [...byFile.entries()].sort((a, b) => b[1].length - a[1].length)) {
  lines.push(`\n## ${file}（${values.length}）`);
  for (const value of [...new Set(values)].sort()) lines.push(`- ${value}`);
}
const report = lines.join("\n");
fs.writeFileSync("D:/code/ai/aivc-rs/scripts/legacy-strings-missing.txt", report, "utf8");
console.log(`\n按来源文件分组已写入 scripts/legacy-strings-missing.txt`);
console.log(report.slice(0, 2000));
