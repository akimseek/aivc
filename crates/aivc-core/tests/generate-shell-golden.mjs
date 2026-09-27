// 用**现网的 TypeScript 实现**跑 shell 与目标编号的对拍输入，写出 golden 文件。
//
// 用法：
//
//   node --experimental-transform-types \
//     crates/aivc-core/tests/generate-shell-golden.mjs
//
// 现网的 `src/shared/shell-args.ts`、`target-ids.ts`、`wsl-paths.ts` 都只依赖
// `node:path` 与类型导入，因此无需打包即可直接跑。

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(here, "..", "..", "..", "..");
const shared = join(repoRoot, "src", "shared");

const fixtures = join(here, "fixtures");
const input = JSON.parse(
  readFileSync(join(fixtures, "shell-scenarios.json"), "utf8"),
);

const shellArgs = await import(pathToFileURL(join(shared, "shell-args.ts")).href);
const targetIds = await import(pathToFileURL(join(shared, "target-ids.ts")).href);
const wslPaths = await import(pathToFileURL(join(shared, "wsl-paths.ts")).href);

/// 安全求值：某些输入会让现网实现抛错，把「抛错」本身也当成可比对的结果。
///
/// 否则生成脚本会在第一个异常处中断，而「两侧是否都不接受该输入」正是需要验证的行为。
function attempt(action) {
  try {
    return { ok: true, value: action() };
  } catch (error) {
    return { ok: false, error: String(error?.message ?? error) };
  }
}

const golden = {
  _comment:
    "由 generate-shell-golden.mjs 用现网 TypeScript 实现生成。不要手工编辑。",
  generatedBy: [
    "src/shared/shell-args.ts",
    "src/shared/target-ids.ts",
    "src/shared/wsl-paths.ts",
  ],
  quoteValues: input.quoteValues.map((value) => ({
    value,
    posix: attempt(() => shellArgs.posixShellQuote(value)),
    cmd: attempt(() => shellArgs.cmdQuote(value)),
    wslShell: attempt(() => wslPaths.shellQuote(value)),
  })),
  commandCases: input.commandCases.map((item) => ({
    command: item.command,
    args: item.args,
    posix: attempt(() => shellArgs.buildPosixShellCommand(item.command, item.args)),
    cmd: attempt(() => shellArgs.buildCmdCommand(item.command, item.args)),
  })),
  parseInputs: input.parseInputs.map((value) => ({
    value,
    args: attempt(() => shellArgs.parseCliArgs(value)),
  })),
  targetIds: input.targetIds.map((targetId) => ({
    targetId,
    provider: attempt(() => targetIds.getProviderIdFromTargetId(targetId)),
    distro: attempt(() => targetIds.getWslDistroFromTargetId(targetId)),
    codexDistro: attempt(() =>
      targetIds.getWslDistroFromProviderTarget("codex", targetId),
    ),
  })),
  // 路径围栏：每一对（文件 × 目录）都要比对。POSIX 侧用 posix 规则，本机侧用
  // path.relative（Windows 上即为 win32 规则）—— 两条规则不同，必须分开对拍。
  fences: input.fenceFiles.flatMap((filePath) =>
    input.fenceDirs.map((dirPath) => ({
      filePath,
      dirPath,
      posix: attempt(() => wslPaths.isInsidePosixDir(filePath, dirPath)),
      local: attempt(() => wslPaths.isInsidePath(filePath, dirPath)),
    })),
  ),
  mountPaths: input.mountPaths.map((filePath) => ({
    filePath,
    converted: attempt(() => wslPaths.wslMountPathToWindowsPath(filePath)),
  })),
  distroNames: input.distroNames.map((distro) => ({
    distro,
    sanitized: attempt(() => wslPaths.sanitizeWslDistro(distro)),
    builtCodex: attempt(() => targetIds.buildWslProviderTargetId("codex", distro)),
    builtClaude: attempt(() => targetIds.buildWslProviderTargetId("claude", distro)),
  })),
};

const target = join(fixtures, "shell-golden.json");
writeFileSync(target, `${JSON.stringify(golden, null, 2)}\n`, "utf8");
console.log(
  `已生成 ${target}（引号 ${golden.quoteValues.length}、命令 ${golden.commandCases.length}、` +
    `解析 ${golden.parseInputs.length}、目标 ${golden.targetIds.length}、发行版 ${golden.distroNames.length}）`,
);
