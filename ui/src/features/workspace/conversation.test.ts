import { describe, expect, it } from "vitest";
import type { AiMessage } from "../../types";
import { branchAnchorForTurn, buildConversationTurns, flattenConversationTurns, isVisibleConversationMessage } from "./conversation";

const message = (role: AiMessage["role"], text: string, sourceLine?: number): AiMessage => ({
  role,
  text,
  sourceLine
});

describe("conversation", () => {
  it("过滤合成内容并按用户消息切分轮次", () => {
    const turns = buildConversationTurns([
      message("user", "<environment_context>internal"),
      message("user", "问题"),
      message("assistant", "回答"),
      message("user", "下一轮")
    ]);
    expect(turns).toHaveLength(2);
    expect(turns[0]?.user?.message.text).toBe("问题");
    expect(turns[0]?.replies[0]?.message.text).toBe("回答");
    expect(turns[1]?.replies).toHaveLength(0);
    expect(isVisibleConversationMessage(message("user", "<permissions instructions>"))).toBe(false);
  });

  it("每轮只取最后一条助手回复作为分支锚点", () => {
    const turns = buildConversationTurns([
      message("user", "问题", 10),
      message("assistant", "第一次", 11),
      message("tool", "工具", 12),
      message("assistant", "最终回答", 13)
    ]);
    expect(branchAnchorForTurn("codex", turns[0]!)).toBe(13);
    const rows = flattenConversationTurns(
      [message("user", "问题", 10), message("assistant", "回答", 11), message("tool", "工具", 12)],
      0,
      "codex",
      true,
      [11]
    );
    expect(rows.filter((row) => row.branchAnchor !== undefined)).toHaveLength(1);
    expect(rows[1]?.branchAnchor).toBe(11);
    expect(rows[2]?.branchAnchor).toBeUndefined();
  });

  it("没有助手回复的未完成轮次不提供分支入口", () => {
    const rows = flattenConversationTurns([message("user", "尚未完成", 10)], 0, "codex");
    expect(rows[0]?.branchAnchor).toBeUndefined();
  });
});
