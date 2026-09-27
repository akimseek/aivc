// 用**现网的 TypeScript 解析器**跑对拍场景，把结果写成 golden 文件。
//
// 用法（需要 Node 22.7+）：
//
//   node --experimental-transform-types \
//     crates/aivc-provider/tests/generate-parser-golden.mjs
//
// 现网源码 `src/shared/session-parser.ts` 只依赖 `node:path` 与类型导入，类型擦除后
// 没有本地运行时依赖，因此不需要打包步骤。

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..", "..");
const parserPath = join(repoRoot, "src", "shared", "session-parser.ts");

const fixtures = join(here, "fixtures");
const cases = JSON.parse(
  readFileSync(join(fixtures, "parser-scenarios.json"), "utf8"),
).cases;

const { parseSessionContent, parseSessionListContent } = await import(
  pathToFileURL(parserPath).href
);

/// 固定文件元信息。两侧都用同一个值，因此 updatedAt 可比。
const FIXED_MTIME_MS = 1_758_794_400_000;

/// 把现网返回的会话压成可比较的形状。
///
/// 只保留**契约字段**：编号、标题、工作目录、时间戳、模型、消息数、预览文本。
/// 文件元信息（`fileMtimeMs` 等）由调用方填充，与本模块无关，因此不比对。
function normalize(session) {
  if (!session) return null;
  return {
    id: session.id ?? "",
    title: session.title ?? "",
    cwd: session.cwd ?? null,
    createdAt: session.createdAt ?? null,
    updatedAt: session.updatedAt ?? null,
    model: session.model ?? null,
    cliVersion: session.cliVersion ?? null,
    messageCount: session.messageCount ?? 0,
    previewCount: Array.isArray(session.preview) ? session.preview.length : 0,
    // 预览只比对角色与文本，编号与时间戳在两边可能由不同来源填充。
    preview: (session.preview ?? []).map((message) => ({
      role: message.role ?? null,
      text: message.text ?? "",
    })),
    // 原样透传，不做字段挑选：两侧用各自的类型序列化，形状不一致会直接暴露，
    // 而我手工挑字段只会在「挑错了」时静默通过。
    usage: session.usage ?? null,
  };
}

const golden = {
  _comment:
    "由 generate-parser-golden.mjs 用现网 TypeScript 实现生成。不要手工编辑——改场景后重新生成。",
  generatedBy: "src/shared/session-parser.ts",
  // 每个用例跑**两种模式**：列表（截断预览、全量计数）与详情（保留全部消息）。
  // 两者的判定逻辑不同，必须分开对拍。
  cases: cases.map((item) => ({
    name: item.name,
    session: normalize(
      parseSessionListContent(
        // 固定 mtime：现网会用它推导 updatedAt，缺了会抛 Invalid time value。
        { filePath: item.filePath, mtimeMs: FIXED_MTIME_MS, size: item.content.length },
        item.content,
      ),
    ),
    fullSession: normalize(parseSessionContent(item.filePath, item.content)),
  })),
};

const target = join(fixtures, "parser-golden.json");
writeFileSync(target, `${JSON.stringify(golden, null, 2)}\n`, "utf8");
const parsed = golden.cases.filter((item) => item.session !== null).length;
console.log(
  `已生成 ${target}（${golden.cases.length} 个用例，其中 ${parsed} 个解析出会话）`,
);
