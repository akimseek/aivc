import { describe, expect, it } from "vitest";

import { pickTarget, toRefs } from "./use-console";
import type { AiSession, AiTarget } from "../types";

const target = (id: string, available: boolean): AiTarget => ({
  id,
  provider: "codex",
  label: id,
  kind: id.includes("wsl") ? "wsl" : "local",
  available
});

const session = (id: string): AiSession => ({
  id,
  title: id,
  filePath: `/sessions/${id}.jsonl`,
  messageCount: 1,
  preview: []
});

describe("target selection", () => {
  it("restores the remembered target when it still exists", () => {
    const items = [target("a", true), target("b", true)];
    expect(pickTarget(items, ["b", "a"])).toBe("b");
  });

  it("falls back to the current one when the remembered target is gone", () => {
    // 发行版被移除、目录被删——记住的目标会失效。这时**不能**回到空选中，
    // 否则界面停在「请选择目标」，用户每次都要重新点一遍。
    const items = [target("a", true), target("b", true)];
    expect(pickTarget(items, ["gone", "a"])).toBe("a");
  });

  it("prefers an available target over an unavailable first one", () => {
    // 这是实测踩过的形状：本机有 Codex，另有一台进不去的发行版。
    // 自动选中一个**用不了**的目标，用户会以为「点了却没好」。
    const items = [target("codex:wsl:broken", false), target("codex:local", true)];
    expect(pickTarget(items, [])).toBe("codex:local");
  });

  it("still selects something when nothing is available", () => {
    // 全都不可用时也要选一个——用户至少能看到它的 `detail` 说明为什么。
    const items = [target("codex:wsl:broken", false)];
    expect(pickTarget(items, [])).toBe("codex:wsl:broken");
  });

  it("returns an empty string for an empty list", () => {
    expect(pickTarget([], [])).toBe("");
  });
});

describe("mutation references", () => {
  it("carries the file path as a hint for the backend's directory fence", () => {
    expect(toRefs([session("s1")], ["s1"])).toEqual([
      { id: "s1", filePath: "/sessions/s1.jsonl" }
    ]);
  });

  it("ignores selected ids that are no longer in the list", () => {
    // 列表可能已经刷新过：选中的条目被别处删掉了。按 id 过滤，不能把不存在的条目
    // 送进批量操作——那会让后端收到无意义的引用。
    expect(toRefs([session("s1")], ["s1", "gone"])).toEqual([
      { id: "s1", filePath: "/sessions/s1.jsonl" }
    ]);
  });

  it("returns nothing when nothing is selected", () => {
    expect(toRefs([session("s1")], [])).toEqual([]);
  });
});
