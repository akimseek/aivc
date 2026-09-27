// 用**现网的 TypeScript 实现**跑对拍场景，把结果写成 golden 文件。
//
// 用法（需要 Node 22.7+，因为它要转译构造函数的参数属性）：
//
//   node --experimental-transform-types \
//     crates/aivc-terminal/tests/generate-status-golden.mjs
//
// 为什么需要它：差分测试只能证明「两边一致」，不能证明「哪边对」。这个脚本是**唯一**
// 让现网实现参与进来的地方——生成一次 golden 之后，Rust 侧的测试就完全离线，
// 既快又不需要 Node。
//
// 注意：现网源码按 `.ts` 直接导入，Node 会就地转译。它的唯一导入是 `import type`，
// 类型擦除后没有运行时依赖，因此不需要打包步骤。

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..", "..");
const detectorPath = join(
  repoRoot,
  "src",
  "electron",
  "terminal",
  "terminal-status-detector.ts",
);

const fixtures = join(here, "fixtures");
const scenarios = JSON.parse(
  readFileSync(join(fixtures, "status-scenarios.json"), "utf8"),
).scenarios;

const { TerminalStatusDetector } = await import(pathToFileURL(detectorPath).href);

/// 对一个场景跑出现网实现发出的事件序列。
///
/// 只保留可比较的字段：状态、提醒内容、轮次编号。`updatedAt` 是墙钟时间，
/// 两边不可能相等，因此不纳入对拍。
function runScenario(scenario) {
  const events = [];
  const detector = new TerminalStatusDetector(
    "t1",
    scenario.providerId,
    (event) => events.push(event),
  );
  for (const step of scenario.steps) {
    if (step.input !== undefined) detector.markInput(step.input);
    else if (step.output !== undefined) detector.inspectOutput(step.output);
    else if (step.exit !== undefined) detector.markExit(step.exit);
  }
  return events.map((event) => ({
    status: event.status,
    attention: event.attention
      ? {
          kind: event.attention.kind,
          title: event.attention.title,
          detail: event.attention.detail ?? null,
        }
      : null,
    turnId: event.turnId ?? null,
  }));
}

const golden = {
  _comment:
    "由 generate-status-golden.mjs 用现网 TypeScript 实现生成。不要手工编辑——改场景后重新生成。",
  generatedBy: "src/electron/terminal/terminal-status-detector.ts",
  scenarios: scenarios.map((scenario) => ({
    name: scenario.name,
    providerId: scenario.providerId,
    events: runScenario(scenario),
  })),
};

const target = join(fixtures, "status-golden.json");
writeFileSync(target, `${JSON.stringify(golden, null, 2)}\n`, "utf8");
console.log(`已生成 ${target}（${golden.scenarios.length} 个场景）`);
