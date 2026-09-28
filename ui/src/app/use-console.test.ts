import { describe, expect, it } from "vitest";

import { pickTarget, planTargetSwitch, toRefs } from "./use-console";
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

/**
 * 「点击进入终端会话后，列表内容就消失了」的根因就在这一组断言里。
 *
 * `selectTarget` 会清空会话列表，而重载 effect 只在 `targetId` 变化时重跑。
 * 所以在**当前目标**下点会话时绝不能走 `selectTarget`——否则清了没人填。
 */
describe("打开会话时的目标切换", () => {
  it("在当前目标下打开会话：**什么都不换**（否则列表被清空且不会重载）", () => {
    expect(planTargetSwitch("codex", "codex:local", { id: "codex:local", provider: "codex" })).toEqual(
      { provider: null, target: null }
    );
  });

  it("同一个平台下换了目标：只换目标，不动平台", () => {
    expect(
      planTargetSwitch("codex", "codex:local", { id: "codex:wsl:ubuntu", provider: "codex" })
    ).toEqual({ provider: null, target: "codex:wsl:ubuntu" });
  });

  it("跨平台打开会话：平台与目标一起换", () => {
    // 平台变了但目标没跟着换，就会拿新平台去查旧目标——列表必然是空的。
    expect(
      planTargetSwitch("codex", "codex:local", { id: "claude:local", provider: "claude" })
    ).toEqual({ provider: "claude", target: "claude:local" });
  });

  it("目标编号相同但平台变了，仍必须换目标", () => {
    // 「编号相同」在跨平台时是个陷阱：`local` 在两家平台上都叫 `local`，
    // 但它们是两个不同的目标，会话也完全不同。
    expect(planTargetSwitch("codex", "local", { id: "local", provider: "claude" })).toEqual({
      provider: "claude",
      target: "local"
    });
  });
});
