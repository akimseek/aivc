import { describe, expect, it } from "vitest";

import {
  baseName,
  displayTitle,
  isTitleFallback,
  messageCountLabel,
  pickPreview,
  previewLine,
  stripMarkup
} from "./session-display";

describe("messageCountLabel", () => {
  it("精确计数直接显示条数", () => {
    // 详情路径是全量解析，数字就是总数。
    expect(messageCountLabel(80)).toBe("80 条");
  });

  it("下界显示成「≥N 条」", () => {
    // 列表模式只读了头部窗口：数到 2 条，这个会话就**至少**有 2 条。
    // 「≥2 条」是真话，「2 条」是假话——这是这一整块逻辑存在的理由。
    expect(messageCountLabel(2, true)).toBe("≥2 条");
  });

  it("一条也没数到时显示「—」而不是「0 条」", () => {
    // 0 有两种可能：真的空会话，或头部窗口里什么都没读到。
    // 两者都给不出一个能说出口的数字。
    expect(messageCountLabel(0)).toBe("—");
    expect(messageCountLabel(0, true)).toBe("—");
  });

  it("计数不是数字时不显示负数或 NaN", () => {
    // 防御性：真出现负值说明上游算错了，不该把它渲染成「-3 条」。
    expect(messageCountLabel(-1)).toBe("—");
  });
});

describe("stripMarkup", () => {
  it("removes the injected Codex blocks", () => {
    expect(stripMarkup("<multi_agent_mode>Any earlier instruction</multi_agent_mode>")).toBe(
      " Any earlier instruction "
    );
  });

  it("keeps uppercase generic parameters", () => {
    // `<T>` 在预览里撞见的概率比 HTML 标签高，因此大写开头的不动。
    expect(stripMarkup("Vec<T> 保留")).toBe("Vec<T> 保留");
  });

  it("does strip lowercase tags even when they look like HTML", () => {
    // 这是**有意的**：预览是非权威短文本，清掉 `<div>` 用户看不出损失，
    // 而留着 `<multi_agent_mode>` 整行就没法读。正文不受影响。
    expect(stripMarkup("<div>正文</div>")).toBe(" 正文 ");
  });
});

describe("previewLine", () => {
  it("collapses whitespace left behind by stripped tags", () => {
    expect(previewLine("<multi_agent_mode>\n\n  Any  earlier ")).toBe("Any earlier");
  });

  it("truncates with an ellipsis", () => {
    const line = previewLine("x".repeat(200), 10);
    expect(line).toBe(`${"x".repeat(10)}…`);
  });

  it("returns an empty string when there is nothing to show", () => {
    // 调用方据此跳过这一条去找下一条，而不是显示一片空白。
    expect(previewLine("<multi_agent_mode></multi_agent_mode>")).toBe("");
  });
});

describe("pickPreview", () => {
  it("prefers the last displayable message", () => {
    const session = {
      preview: [
        { role: "user" as const, text: "第一条" },
        { role: "user" as const, text: "<multi_agent_mode>x</multi_agent_mode>" },
        { role: "assistant" as const, text: "最后一条" }
      ]
    };
    expect(pickPreview(session)).toBe("最后一条");
  });

  it("skips unusable messages and searches backwards", () => {
    const session = {
      preview: [
        { role: "user" as const, text: "唯一可用" },
        { role: "user" as const, text: "<a></a>" },
        { role: "user" as const, text: "<b></b>" }
      ]
    };
    expect(pickPreview(session)).toBe("唯一可用");
  });

  it("returns an empty string when nothing is displayable", () => {
    expect(pickPreview({ preview: [{ role: "user" as const, text: "<x></x>" }] })).toBe("");
  });
});

describe("baseName", () => {
  it("handles both separators", () => {
    expect(baseName("C:\\code\\ai")).toBe("ai");
    expect(baseName("/home/akim/code/ai")).toBe("ai");
    expect(baseName("C:\\code\\ai\\")).toBe("ai");
  });

  it("returns an empty string for missing paths", () => {
    expect(baseName(undefined)).toBe("");
    expect(baseName("")).toBe("");
  });
});

describe("displayTitle", () => {
  const id = "98895b66-4bbd-4f62-9062-6a2cb5740668";

  it("keeps a real title untouched", () => {
    expect(displayTitle({ id, title: "把登录改成 SSO" })).toBe("把登录改成 SSO");
  });

  it("replaces a title that fell back to the id", () => {
    // 这是线上实际发生的情况：列表 5 条标题全是 UUID，无法区分。
    const shown = displayTitle({ id, title: id, cwd: "/home/akim/code/rewrite", createdAt: "2026-09-23T10:00:00.000Z" });
    expect(shown).not.toContain(id);
    expect(shown).toContain("rewrite");
  });

  it("falls back to a short id when there is nothing else", () => {
    // 连目录和时间都没有：至少短一点，完整 UUID 会把整行撑满。
    const shown = displayTitle({ id, title: id });
    expect(shown).toContain("98895b66");
    expect(shown).not.toContain("4bbd");
  });
});

describe("isTitleFallback", () => {
  it("detects both an empty title and the id fallback", () => {
    const id = "abc";
    expect(isTitleFallback({ id, title: id })).toBe(true);
    expect(isTitleFallback({ id, title: "  " })).toBe(true);
    expect(isTitleFallback({ id, title: "真标题" })).toBe(false);
  });
});
