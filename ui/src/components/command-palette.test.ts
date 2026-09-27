import { describe, expect, it } from "vitest";

import { filterCommands } from "./command-palette";
import type { Command } from "./command-palette";

const noop = () => undefined;

const commands: Command[] = [
  { id: "term.codex", group: "终端", label: "新建 Codex 终端", run: noop },
  { id: "term.shell", group: "终端", label: "新建系统终端", run: noop },
  { id: "config.gateway", group: "配置", label: "网关", run: noop },
  { id: "export.md", group: "动作", label: "导出为 Markdown", run: noop }
];

describe("command palette filtering", () => {
  it("returns everything for an empty query", () => {
    expect(filterCommands(commands, "")).toHaveLength(commands.length);
    // 只敲空格也不该把列表清空——那看起来像「没有命令可用」。
    expect(filterCommands(commands, "   ")).toHaveLength(commands.length);
  });

  it("matches the label case-insensitively", () => {
    const found = filterCommands(commands, "codex");
    expect(found.map((command) => command.id)).toEqual(["term.codex"]);
  });

  it("matches the group name too", () => {
    // 只按标签匹配的话，搜「终端」会漏掉所有终端命令——用户是按**类别**想的。
    const found = filterCommands(commands, "终端");
    expect(found.map((command) => command.id)).toEqual(["term.codex", "term.shell"]);
  });

  it("returns nothing when nothing matches", () => {
    expect(filterCommands(commands, "definitely-not-a-command")).toEqual([]);
  });
});
