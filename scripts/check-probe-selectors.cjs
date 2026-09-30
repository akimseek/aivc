/**
 * 探针选择器守卫：**脚本里引用的类名必须真的出现在界面源码里**。
 *
 * # 为什么要有它
 *
 * `scripts/probes/*.js` 与 `scripts/ui-baseline.ps1` 是「人工验收程序」：它们坏掉时
 * 不会报错，只会**静默地**查不到东西——探针输出 `NO-ROW` / `{}`，截图依旧是张 PNG，
 * 看的人却以为「界面里就是没有这个元素」。批次一/二把侧栏改成活动栏、把弹层换成共享
 * `Dialog` 之后，`gap-closeout.js` 就这样背了三条 FAIL、基线 01–03 步拍到的是空列表。
 *
 * 类名对不上是这类腐坏里**唯一能被机械查出来**的一种，因此钉在这里：源码里改了类名
 * 就必须同步改脚本，否则门禁直接失败。文案与 aria-label 对不上仍然只能靠人看，
 * 但那是另一类（更慢、更明显）的腐坏。
 *
 * # 级别：失败（exit 1），不是警告
 *
 * 「警告」在这里等于没有：这三条 FAIL 曾经就那么挂着一个版本没人管，因为验收人
 * 看到「警告」会先假定是误报。规则本身收得很窄（见下），假阳性主要由**白名单**兜住，
 * 而白名单每一条都要写理由、走 review——代价远低于一次误判。
 *
 * # 规则（收窄到「几乎不可能是别的东西」）
 *
 * 扫 `scripts/**` 里的 `\.([a-z][a-z0-9-]{2,})`，排除四类必然的假阳性：
 *
 * 1. **属性访问/链式调用**：`.` 前面是标识符、`)`、`]`、`?`、另一个 `.`（展开语法
 *    `...document`）或 `*`（glob `*.ps1`）；
 * 2. **方法调用**：`.` 后面紧跟 `(`（`.map(`、`.sleep(`）；
 * 3. **路径与扩展名**：`.` 前面是 `\` 或 `/`（`ui/dist/index.html`），或命中已知
 *    扩展名表（`png` / `ps1` / `cjs` / `json` …）；
 * 4. **注释**：`#`（PowerShell）与行注释、块注释（JS）之后的内容不执行，里面写的
 *    类名是**历史说明**（例如 ui-baseline 注释里的「`.overlay-panel` → `.settings-panel`」），
 *    不算锚点。
 *
 * 剩下的再与源码类名比对，仍不存在的才报——最后还有一层白名单（见 `ALLOW`）。
 *
 * 类名来源：`ui/src/**` 里 `className="…"`、`className={…}`（含模板与三元的字符串
 * 字面量）、`panelClassName="…"` 与 `.css` 里的选择器。
 *
 * 用法：
 *   node scripts/check-probe-selectors.cjs        # 有命中即 exit 1
 *   node scripts/check-probe-selectors.cjs -v     # 顺带打印白名单与被扫文件数
 */

const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");

/**
 * 白名单：页面上**真实存在**、但不由 `ui/src` 声明的类名。
 *
 * 每一条都必须写「为什么不在源码里」——写不出来就说明它是失效锚点，别往里加。
 */
const ALLOW = new Map([
  ["xterm", "xterm.js 自己渲染的根节点（第三方库，不在 ui/src）"],
  ["xterm-rows", "xterm.js 的屏幕行容器（第三方库）"],
  ["xterm-helper-textarea", "xterm.js 的隐藏输入框（第三方库，聚焦与按键都靠它）"]
]);

function walk(dir, exts, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === "node_modules" || entry.name === "dist" || entry.name === ".git") continue;
      walk(full, exts, out);
    } else if (exts.some((suffix) => entry.name.endsWith(suffix))) {
      out.push(full);
    }
  }
  return out;
}

const rel = (file) => path.relative(root, file).replace(/\\/g, "/");

/** `className=` 之后的值（引号串或花括号块，花括号按字符串状态配平）。 */
function readClassValue(text, from) {
  let i = from;
  while (i < text.length && /\s/.test(text[i])) i += 1;
  if (text[i] === '"') {
    const end = text.indexOf('"', i + 1);
    return text.slice(i + 1, end < 0 ? text.length : end);
  }
  if (text[i] === "{") {
    let depth = 0;
    let quote = null;
    for (let j = i; j < text.length; j += 1) {
      const ch = text[j];
      if (quote) {
        if (ch === "\\") j += 1;
        else if (ch === quote) quote = null;
        continue;
      }
      if (ch === '"' || ch === "'" || ch === "`") {
        quote = ch;
        continue;
      }
      if (ch === "{") depth += 1;
      else if (ch === "}") {
        depth -= 1;
        if (depth === 0) return text.slice(i + 1, j);
      }
    }
    return text.slice(i + 1);
  }
  return "";
}

/** 把 `className` 的值拆成类名候选（丢掉变量名与插值表达式）。 */
function classTokens(value) {
  const chunks = [];
  for (const match of value.matchAll(/"([^"]*)"|`([^`]*)`/g)) {
    chunks.push(match[1] ?? match[2] ?? "");
  }
  const tokens = [];
  for (const chunk of chunks) {
    const text = chunk.replace(/\$\{[^}]*\}/g, " ");
    for (const token of text.split(/\s+/)) {
      if (/^[a-z][a-z0-9-]*$/.test(token)) tokens.push(token);
    }
  }
  // 引号风格之外（例如 `className="a b"` 的直给形态）再兜一次。
  if (chunks.length === 0) {
    for (const token of value.replace(/\$\{[^}]*\}/g, " ").split(/\s+/)) {
      const clean = token.replace(/[^a-z0-9-]/g, "");
      if (/^[a-z][a-z0-9-]*$/.test(clean)) tokens.push(clean);
    }
  }
  return tokens;
}

/** 收集 ui/src 声明的全部类名。 */
function collectSourceClasses() {
  const found = new Map();
  const add = (name, where) => {
    if (!found.has(name)) found.set(name, where);
  };

  for (const file of walk(path.join(root, "ui", "src"), [".tsx", ".ts", ".css"])) {
    const text = fs.readFileSync(file, "utf8");
    const where = rel(file);

    if (file.endsWith(".css")) {
      // CSS 里 `.foo` 就是类选择器；数字（0.5）与属性值不会匹配这个形态。
      for (const match of text.matchAll(/\.([a-z][a-z0-9-]{1,})/g)) add(match[1], where);
      continue;
    }

    for (const match of text.matchAll(/(?:panel)?[Cc]lassName\s*=/g)) {
      const value = readClassValue(text, match.index + match[0].length);
      for (const token of classTokens(value)) add(token, where);
    }
  }
  return found;
}

const PREV_SKIP = /[A-Za-z0-9_$)\]?.*]/;

/** 文件扩展名不是类名（`*.ps1`、`shots/x.png` 里的那一截会被正则捞到）。 */
const EXT_SKIP = new Set([
  "ps1", "js", "cjs", "mjs", "ts", "tsx", "json", "png", "exe", "html",
  "css", "md", "txt", "rs", "toml", "yml", "yaml", "lock"
]);

/**
 * 把注释抹成空白，保留长度（这样行号仍然准确）。
 *
 * 只认「行首或空白之后的 `//` / `#`」：`https://…` 里的 `//` 前面是 `:`，不会被误判。
 */
function stripComments(text, kind) {
  const chars = [...text];
  const blank = (from, to) => {
    for (let i = from; i < to && i < chars.length; i += 1) {
      if (chars[i] !== "\n") chars[i] = " ";
    }
  };
  let inBlock = false;
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (inBlock) {
      if (ch === "*" && text[i + 1] === "/") {
        blank(i, i + 2);
        inBlock = false;
        i += 1;
      } else {
        blank(i, i + 1);
      }
      continue;
    }
    const atLineStart = i === 0 || text[i - 1] === "\n";
    const afterSpace = i > 0 && /\s/.test(text[i - 1]);
    if ((atLineStart || afterSpace) && ch === "/" && text[i + 1] === "*") {
      blank(i, i + 2);
      inBlock = true;
      i += 1;
      continue;
    }
    if ((atLineStart || afterSpace) && ch === "/" && text[i + 1] === "/" && kind === "js") {
      const end = text.indexOf("\n", i);
      blank(i, end < 0 ? text.length : end);
      i = (end < 0 ? text.length : end) - 1;
      continue;
    }
    if ((atLineStart || afterSpace) && ch === "#" && kind === "ps1") {
      const end = text.indexOf("\n", i);
      blank(i, end < 0 ? text.length : end);
      i = (end < 0 ? text.length : end) - 1;
    }
  }
  return chars.join("");
}

/** 收集脚本里引用的类名（含出处，便于照着改）。 */
function collectScriptRefs() {
  const refs = new Map();
  const files = [
    ...walk(path.join(root, "scripts"), [".ps1", ".js", ".cjs"])
  ].filter((file) => path.resolve(file) !== path.resolve(__filename));
  // 仓库根下真实存在的条目（`.toolchain`）：写成 `.foo` 时那是路径，不是类名。
  const pathTokens = new Set(
    fs.readdirSync(root, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() || entry.isFile())
      .map((entry) => entry.name.replace(/^\./, ""))
  );

  for (const file of files) {
    const raw = fs.readFileSync(file, "utf8");
    const text = stripComments(raw, file.endsWith(".ps1") ? "ps1" : "js");
    const where = rel(file);
    for (const match of text.matchAll(/\.([a-z][a-z0-9-]{2,})/g)) {
      const name = match[1];
      const start = match.index;
      const prev = text[start - 1] ?? "";
      const next = text[start + 1 + name.length] ?? "";
      // 属性访问 / 链式调用 / 展开语法
      if (PREV_SKIP.test(prev)) continue;
      // 方法调用：紧跟 `(`，或后面还接着标识符（`.getBoundingClientRect` 的 `get`）
      if (next === "(" || /[A-Z_$]/.test(next)) continue;
      // 路径（`ui/dist/index.html`、`scripts\cargo.ps1`）与扩展名
      if (prev === "/" || prev === "\\") continue;
      if (EXT_SKIP.has(name)) continue;
      // 仓库里真实存在的目录/文件名（`.toolchain`）是路径，不是类名。
      if (pathTokens.has(name)) continue;
      // 行号：拿 `\n` 出现次数，够用且不必再扫一遍。
      const line = text.slice(0, start).split("\n").length;
      const entry = `${where}:${line}`;
      const list = refs.get(name) ?? [];
      if (!list.includes(entry)) list.push(entry);
      refs.set(name, list);
    }
  }
  return refs;
}

const source = collectSourceClasses();
const refs = collectScriptRefs();

const missing = [];
for (const [name, places] of [...refs].sort((a, b) => a[0].localeCompare(b[0]))) {
  if (source.has(name) || ALLOW.has(name)) continue;
  missing.push({ name, places });
}

const verbose = process.argv.includes("-v") || process.argv.includes("--verbose");
if (verbose) {
  console.log(`[守卫] 源码类名 ${source.size} 个；脚本引用 ${refs.size} 个；白名单 ${ALLOW.size} 条`);
  for (const [name, reason] of ALLOW) console.log(`[白名单] .${name} —— ${reason}`);
}

if (missing.length === 0) {
  console.log(`[守卫] 探针选择器 OK：脚本引用的类名全部存在于 ui/src（白名单 ${ALLOW.size} 条）。`);
  process.exit(0);
}

console.log(`[守卫] 以下类名被脚本引用，但 ui/src 里不存在（改了类名就要同步改脚本）：`);
for (const { name, places } of missing) {
  console.log(`  .${name}  <- ${places.join(", ")}`);
}
process.exit(1);
