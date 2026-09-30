import { describe, expect, it } from "vitest";

import { resolveStageContent } from "./stage-content";

/**
 * 画布内容判定：终端优先于历史（同一张标签里两者都存在时，用户要看的是活的那一个）。
 * 终端不在了就回落到历史——这正是「异常退出的标签保留现场」之外的另一半：
 * 用户关掉终端后不该看到一块空白。
 */
describe("resolveStageContent", () => {
  const alive = (id: string) => id === "term-1";

  it("没有标签 → 空", () => {
    expect(resolveStageContent(null, alive)).toEqual({ kind: "empty" });
  });

  it("有存活的终端 → 终端", () => {
    expect(resolveStageContent({ terminalId: "term-1", sessionId: "s1" }, alive)).toEqual({
      kind: "terminal",
      terminalId: "term-1"
    });
  });

  it("终端不在（未启动 / 已关闭）但有会话 → 历史", () => {
    expect(resolveStageContent({ terminalId: "term-x", sessionId: "s1" }, alive)).toEqual({
      kind: "history",
      sessionId: "s1"
    });
  });

  it("没有会话也没有终端 → 空", () => {
    expect(resolveStageContent({}, alive)).toEqual({ kind: "empty" });
  });
});
