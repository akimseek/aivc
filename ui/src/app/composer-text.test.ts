// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  PASTE_COMPACT_THRESHOLD,
  composeSubmission,
  expandBlocks,
  formatAttachments,
  initialComposerVisible,
  insertAtCursor,
  shouldCompact,
  shouldRevealComposer
} from "./composer-text";

/**
 * 自管输入的文本处理。
 *
 * 这些函数的错**看不见**：写进 PTY 的字节不对，用户只会觉得「它把我的话吞了」，
 * 没有任何报错。所以每一步都单独钉住。
 */

describe("粘贴收成标记", () => {
  it("超过阈值就收起来——几万字的日志糊满输入框没法用", () => {
    expect(shouldCompact("a".repeat(PASTE_COMPACT_THRESHOLD))).toBe(true);
  });

  it("**行数多也要收**，哪怕字符数不多", () => {
    // 40 行的报错可能只有 800 字符，但它同样会糊满输入框——而那正是最常被粘的东西。
    const short = Array.from({ length: 20 }, () => "err").join("\n");
    expect(short.length).toBeLessThan(PASTE_COMPACT_THRESHOLD);
    expect(shouldCompact(short)).toBe(true);
  });

  it("短文本原样插入", () => {
    expect(shouldCompact("帮我看看这个 bug")).toBe(false);
  });
});

describe("insertAtCursor", () => {
  it("插在光标处，并把光标移到插入内容之后", () => {
    // 只返回文本、把光标丢到末尾，用户接着打字就会跑到整段的最后。
    const result = insertAtCursor("前后", 1, 1, "中间");
    expect(result.value).toBe("前中间后");
    expect(result.cursor).toBe(3);
  });

  it("有选区时替换选区", () => {
    const result = insertAtCursor("abcdef", 1, 4, "X");
    expect(result.value).toBe("aXef");
  });
});

describe("expandBlocks", () => {
  it("提交前必须把标记展开成原文", () => {
    // 忘了展开的后果很隐蔽：CLI 收到的是**字面的**「[已粘贴内容 #1]」。
    const blocks = [{ marker: "[已粘贴内容 #1]", text: "很长的一段" }];
    expect(expandBlocks("看这个 [已粘贴内容 #1] 谢谢", blocks)).toBe("看这个 很长的一段 谢谢");
  });

  it("同一段粘两次也会全部展开", () => {
    const blocks = [{ marker: "[已粘贴内容 #1]", text: "X" }];
    expect(expandBlocks("[已粘贴内容 #1] 与 [已粘贴内容 #1]", blocks)).toBe("X 与 X");
  });

  it("没有标记时原样返回", () => {
    expect(expandBlocks("普通文本", [])).toBe("普通文本");
  });
});

describe("formatAttachments", () => {
  it("用完整路径加 `@` 前缀——只写文件名 CLI 找不到", () => {
    expect(formatAttachments(["D:\\work\\a.ts"])).toBe("@D:\\work\\a.ts");
  });

  it("含空格的路径加引号，否则 CLI 会把一个路径当成两个", () => {
    expect(formatAttachments(["C:\\my dir\\a.ts"])).toBe('@"C:\\my dir\\a.ts"');
  });

  it("没有附件时返回空串", () => {
    expect(formatAttachments([])).toBe("");
  });
});

describe("composeSubmission", () => {
  it("括号粘贴标记与回车在**同一次写入**里", () => {
    // 分开写的话，Windows 输入法组合态下回车可能只表现为换行而未提交。
    const bytes = composeSubmission("你好", [], []);
    expect(bytes).toBe("\x1b[200~你好\x1b[201~\r");
  });

  it("清掉文本里的转义符——粘贴终端输出时很常见", () => {
    // 不清的话它会被当成转义序列执行，可能改变终端显示状态。
    const bytes = composeSubmission("a\x1b[31mb", [], []);
    expect(bytes).not.toContain("\x1b[31m");
    expect(bytes).toContain("a[31mb");
  });

  it("**空内容返回 null**——空提交会往 CLI 里塞一个多余的回车", () => {
    expect(composeSubmission("   ", [], [])).toBeNull();
    expect(composeSubmission("", [], [])).toBeNull();
  });

  it("只有附件、没有正文时也能提交", () => {
    const bytes = composeSubmission("", [], ["D:\\a.ts"]);
    expect(bytes).toBe("\x1b[200~@D:\\a.ts\x1b[201~\r");
  });

  it("附件排在正文前面", () => {
    const bytes = composeSubmission("看这个", [], ["D:\\a.ts"])!;
    expect(bytes.indexOf("@D:\\a.ts")).toBeLessThan(bytes.indexOf("看这个"));
  });

  it("标记会先展开再写入 PTY", () => {
    const blocks = [{ marker: "[已粘贴内容 #1]", text: "原文" }];
    const bytes = composeSubmission("[已粘贴内容 #1]", blocks, [])!;
    expect(bytes).toContain("原文");
    expect(bytes).not.toContain("[已粘贴内容 #1]");
  });
});

describe("自管输入的显示规则", () => {
  it("新会话一进去就显示（`sessionId` 为空）", () => {
    expect(initialComposerVisible(undefined)).toBe(true);
    expect(initialComposerVisible("")).toBe(true);
  });

  it("恢复历史会话时先藏着——那时终端正在重放历史，输入框会挡住输出", () => {
    expect(initialComposerVisible("sess-1")).toBe(false);
  });

  it("**第一段输出到达时自动显示**——这是唯一的入口", () => {
    // 少了这条，恢复的会话里用户永远看不到自管输入：
    // 旧平台的 `revealComposer()` 是唯一显示入口，没有任何按钮能调出来。
    expect(shouldRevealComposer(false, true)).toBe(true);
  });

  it("已经显示时不再重复触发（每一段输出都触发会白跑一遍 fit）", () => {
    expect(shouldRevealComposer(true, true)).toBe(false);
  });

  it("没有输出时不显示", () => {
    expect(shouldRevealComposer(false, false)).toBe(false);
  });
});
