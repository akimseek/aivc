/**
 * 命令名守卫：前端 `invoke` 出的「域:动作」必须登记在后端 `COMMANDS` 表里。
 *
 * # 为什么要有它
 *
 * 命令名是**跨进程契约**，写错不会在编译期暴露：两边都编译得过，点下去才收到一句
 * 「未知命令」。`crates/aivc-ipc/src/commands.rs` 的文件头把这条表存在的理由之一定为
 * 「前端期望的名字与本表的差异**可以用一条脚本比对出来**」——本脚本就是那条脚本。
 * 此前这份承诺只落在人脑里：前端几百行命令面只有后端测试里 7 条手写断言兜底，
 * B1（前端写 `codex:open-session-folder`、后端注册的却是 `shell:open-session-folder`）
 * 是真实发生过的这类缺陷。
 *
 * # 级别
 *
 * - **前端有、后端无：失败（exit 1）**——B1 类，点下去才炸的未知命令；
 * - **后端有、前端无：提示（不退出）**——可能是有意的平台命令或已实现未接线，
 *   列出来供人判断，不当缺陷判据。
 *
 * # 抽取规则与排除项
 *
 * 前端（`ui/src/ipc/commands.ts`）：只认 `invoke(...)` 调用中**第一个参数是字符串
 * 字面量**的调用点，`invoke<…>` 泛型与「泛型后换行再写字符串」的形态都覆盖。
 * 两个推论：
 *
 * 1. **事件通道常量天然被排除**：`CHANNEL_TERMINAL_DATA = "terminal:data"` 这类是
 *    const 声明、`listen(...)` 的实参，不是 `invoke` 调用点，不在扫描面上；将来若有人
 *    把通道值误当命令来 invoke，它就会被当普通名字比对——这正是我们要的。
 * 2. **第一个参数不是字面量的调用点是失败**：`invoke(getName())` 会让本规则失明，
 *    守卫宁可报红也不装作看不见（fail-closed）。
 *
 * 后端（`crates/aivc-ipc/src/commands.rs`）：`COMMANDS` 表区间里的 `("域:动作", …)`
 * 字面量，单行与多行形态都认；抽出的条数必须等于类型标注里的 `; N]`——抽取失效时
 * 宁可报红，不能让守卫**静默地全绿**（静默正是它要防的病）。
 *
 * 两侧都在**先抹掉注释**的文本上扫描：注释里引用命令名是历史说明、不是调用
 * （例如 commands.ts 注释里的 `vendor:list`）。
 *
 * # 自检
 *
 * `--self-test`：在内存里分别改坏一个前端名字、删掉一个后端名字（不动源码），
 * 要求比对恰好报出它们——直接回答「改一个名字真的会红吗」。
 * `verify.ps1` 先跑自检、再跑真实比对。
 *
 * 用法：
 *   node scripts/check-command-names.cjs               # 有失败级命中即 exit 1
 *   node scripts/check-command-names.cjs --self-test   # 只跑注入式自检
 */

const fs = require("node:fs");
const path = require("node:path");

const root = path.resolve(__dirname, "..");

const FRONTEND_REL = "ui/src/ipc/commands.ts";
const BACKEND_REL = "crates/aivc-ipc/src/commands.rs";

/** 命令形状：「域:动作」，两段都是小写 kebab-case（与 commands.rs 的命名规律一致）。 */
const COMMAND_SHAPE = /^[a-z][a-z0-9-]*:[a-z][a-z0-9-]+$/;

const lineOf = (text, index) => text.slice(0, index).split("\n").length;

/**
 * 把注释抹成空白、保留长度（行号仍然准确）。
 *
 * 与 `check-probe-selectors.cjs` 里的同名助手同构，但**故意各留一份**：守卫要能
 * 单独读懂、单独跑，不为共享几十行助手引一层模块耦合。
 */
function stripComments(text) {
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
    if ((atLineStart || afterSpace) && ch === "/" && text[i + 1] === "/") {
      const end = text.indexOf("\n", i);
      blank(i, end < 0 ? text.length : end);
      i = (end < 0 ? text.length : end) - 1;
    }
  }
  return chars.join("");
}

/**
 * 从 `invoke` 泛型的 `<` 跳到配平的 `>` 之后，找不到返回 -1。
 *
 * 必须真配平：`invoke<Array<{ id: string }>>(…)` 有两个 `>`；类型里还可能出现
 * 字符串常量类型（如 `"a" | "b"`），引号内的尖括号不算数。
 */
function skipGeneric(text, start) {
  let depth = 0;
  for (let i = start; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === '"' || ch === "'" || ch === "`") {
      const quote = ch;
      i += 1;
      while (i < text.length && text[i] !== quote) {
        if (text[i] === "\\") i += 1;
        i += 1;
      }
      continue;
    }
    if (ch === "=" && text[i + 1] === ">") {
      i += 1; // 函数类型的 `=>`：它的 `>` 不是泛型收尾
      continue;
    }
    if (ch === "<") depth += 1;
    else if (ch === ">") {
      depth -= 1;
      if (depth === 0) return i + 1;
    }
  }
  return -1;
}

/** 抽前端的 invoke 调用点；返回 { commands: Map<名字, 行号>, problems }。 */
function extractFrontend(text) {
  const commands = new Map();
  const problems = [];

  for (const match of text.matchAll(/\binvoke\b/g)) {
    let i = match.index + match[0].length;
    while (i < text.length && /\s/.test(text[i])) i += 1;

    if (text[i] === "<") {
      i = skipGeneric(text, i);
      if (i < 0) {
        problems.push({
          line: lineOf(text, match.index),
          note: "invoke 泛型的尖括号配不平，抽取器看不懂——需同步更新本脚本"
        });
        continue;
      }
      while (i < text.length && /\s/.test(text[i])) i += 1;
    }

    // `import { invoke }` 这类出现不是调用点，静默跳过。
    if (text[i] !== "(") continue;
    i += 1;
    while (i < text.length && /\s/.test(text[i])) i += 1;

    if (text[i] !== '"') {
      problems.push({
        line: lineOf(text, match.index),
        note: "invoke 的第一个参数不是字符串字面量，无法机械比对（命令名必须写字面量）"
      });
      continue;
    }

    const end = text.indexOf('"', i + 1);
    const name = text.slice(i + 1, end < 0 ? text.length : end);
    if (!COMMAND_SHAPE.test(name)) {
      problems.push({
        line: lineOf(text, i),
        note: `invoke 的第一个参数 ${JSON.stringify(name)} 不是「域:动作」形态`
      });
      continue;
    }
    if (!commands.has(name)) commands.set(name, lineOf(text, i));
  }

  if (commands.size === 0 && problems.length === 0) {
    problems.push({ line: 1, note: "一条 invoke 命令字面量都没抽到——抽取规则已失效，不能静默放行" });
  }
  return { commands, problems };
}

/** 抽后端 COMMANDS 表的条目；返回 { commands: Set, declaredCount, problems }。 */
function extractBackend(text) {
  const entries = [];
  const problems = [];

  const start = text.indexOf("pub const COMMANDS");
  if (start < 0) {
    problems.push({ line: 1, note: "找不到 `pub const COMMANDS`——表被改名/挪走后本脚本必须同步更新" });
    return { commands: new Set(), declaredCount: null, problems };
  }
  const end = text.indexOf("];", start);
  if (end < 0) {
    problems.push({ line: lineOf(text, start), note: "COMMANDS 表找不到收尾的 `];`" });
    return { commands: new Set(), declaredCount: null, problems };
  }
  const region = text.slice(start, end);

  // 长度标注 `[(&str, CommandDomain); N]`：抽出的条数必须与它一致，
  // 否则说明抽取正则漏了条目——守卫不能在看不清的情况下放行。
  const declared = /\[\s*\(&str\s*,\s*CommandDomain\s*\)\s*;\s*(\d+)\s*\]/.exec(region);
  const declaredCount = declared ? Number(declared[1]) : null;
  if (declaredCount === null) {
    problems.push({
      line: lineOf(text, start),
      note: "解析不出数组长度标注 `[(&str, CommandDomain); N]`——抽取计数失去自检"
    });
  }

  for (const match of region.matchAll(/\(\s*"([a-z][a-z0-9-]*:[a-z][a-z0-9-]+)"\s*,/g)) {
    entries.push(match[1]);
  }

  if (entries.length === 0) {
    problems.push({
      line: lineOf(text, start),
      note: "COMMANDS 表里一条命令字面量都没抽到——抽取规则已失效，不能静默放行"
    });
  }
  if (declaredCount !== null && entries.length !== declaredCount) {
    problems.push({
      line: lineOf(text, start),
      note: `表声明 ${declaredCount} 条、实际抽出 ${entries.length} 条——抽取正则漏了条目，守卫不能放行`
    });
  }
  return { commands: new Set(entries), declaredCount, problems };
}

/** 双向比对：前端有后端无（失败级）；后端有前端无（提示级）。 */
function diff(frontend, backend) {
  const frontendOnly = [...frontend].filter(([name]) => !backend.has(name));
  const backendOnly = [...backend].filter((name) => !frontend.has(name));
  return { frontendOnly, backendOnly };
}

/**
 * 注入式自检：在内存里改坏名字，要求比对恰好报出——直接回答「这个守卫真的会红吗」。
 *
 * 注入以真实抽取结果为底，且只断言「相对基线的增量恰好是注入的那一个」，
 * 因此即使仓库当前本身有问题（真实比对会另行报出），自检结论仍然成立。
 */
function runSelfTest(frontend, backend) {
  const problems = [];
  if (frontend.size === 0 || backend.size === 0) {
    return ["抽取结果为空，注入式自检无从构造——先查抽取规则"];
  }
  const base = diff(frontend, backend);
  const sample = frontend.keys().next().value; // 第一条前端命令（Map 保序）

  // 注入 1：改坏前端一个名字的前缀（`codex:x` → `codexx:x`），失败级必须恰好多出它。
  const [domain, action] = sample.split(":");
  const mutated = `${domain}x:${action}`;
  if (backend.has(mutated) || base.frontendOnly.some(([name]) => name === mutated)) {
    problems.push(`注入名 ${mutated} 与既有名字撞车，自检构造无效`);
  } else {
    const injected = new Map([[mutated, 1], ...frontend]);
    const hit = diff(injected, backend).frontendOnly.map(([name]) => name);
    if (!(hit.includes(mutated) && hit.length === base.frontendOnly.length + 1)) {
      problems.push(`改坏前端名字为 ${mutated} 后失败级应恰好 +1 条并含它，实际 ${JSON.stringify(hit)}`);
    }
  }

  // 注入 2：从后端删掉前端在用的一个名字，失败级必须恰好报出它。
  if (!backend.has(sample)) {
    problems.push(`前端在用的 ${sample} 不在后端表里，无法构造「后端缺名字」注入`);
  } else {
    const reduced = new Set(backend);
    reduced.delete(sample);
    const hit = diff(frontend, reduced).frontendOnly.map(([name]) => name);
    if (!(hit.includes(sample) && hit.length === base.frontendOnly.length + 1)) {
      problems.push(`后端删掉 ${sample} 后失败级应恰好 +1 条并含它，实际 ${JSON.stringify(hit)}`);
    }
  }

  return problems;
}

function readStripped(relPath) {
  const file = path.join(root, relPath);
  if (!fs.existsSync(file)) {
    console.log(`[守卫] 读不到 ${relPath}——文件被挪走/改名了，先同步本脚本再跑门禁。`);
    process.exit(1);
  }
  return stripComments(fs.readFileSync(file, "utf8"));
}

const args = process.argv.slice(2);
const unknown = args.filter((arg) => arg !== "--self-test");
if (unknown.length > 0) {
  console.log("用法：node scripts/check-command-names.cjs [--self-test]");
  process.exit(1);
}

const frontendResult = extractFrontend(readStripped(FRONTEND_REL));
const backendResult = extractBackend(readStripped(BACKEND_REL));

const extractProblems = [
  ...frontendResult.problems.map((item) => ({ where: FRONTEND_REL, ...item })),
  ...backendResult.problems.map((item) => ({ where: BACKEND_REL, ...item }))
];
if (extractProblems.length > 0) {
  console.log("[守卫] 抽取阶段报错（守卫不能在看不清的情况下放行）：");
  for (const { where, line, note } of extractProblems) {
    console.log(`  ${where}:${line}  ${note}`);
  }
  process.exit(1);
}

if (args.includes("--self-test")) {
  const problems = runSelfTest(frontendResult.commands, backendResult.commands);
  if (problems.length === 0) {
    console.log("[守卫] 自检通过：改坏前端一个名字 / 后端删掉一个名字，比对都会恰好报出（注入在内存里，不动源码）。");
    process.exit(0);
  }
  console.log("[守卫] 注入式自检失败——守卫自己不可信了：");
  for (const problem of problems) console.log(`  ${problem}`);
  process.exit(1);
}

const { frontendOnly, backendOnly } = diff(frontendResult.commands, backendResult.commands);

if (frontendOnly.length > 0) {
  console.log("[守卫] 前端 invoke 了后端 COMMANDS 表里没有的命令名（B1 类：两边都编译得过，点下去才报「未知命令」）：");
  for (const [name, line] of frontendOnly) {
    console.log(`  ${name}  <- ${FRONTEND_REL}:${line}`);
  }
  process.exit(1);
}

console.log(
  `[守卫] 命令名 OK：前端 invoke ${frontendResult.commands.size} 条、后端 COMMANDS 表 ${backendResult.commands.size} 条，失败级差异 0。`
);
if (backendOnly.length > 0) {
  console.log(`[守卫] 后端有、前端无 ${backendOnly.length} 条（提示级：平台命令或暂未接线，列出仅供参考）：`);
  for (const name of backendOnly) console.log(`  ${name}`);
}
