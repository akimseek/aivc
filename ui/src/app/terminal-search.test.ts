// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  findInLine,
  findMatches,
  isWholeWord,
  matchLabel,
  nextMatch
} from "./terminal-search";

/**
 * 终端内的文本查找。
 *
 * 这些函数的错**看起来只是「没找到」**：大小写判断错了、全词边界算错了，
 * 用户看到的都是「无匹配」，然后以为自己记错了内容。所以逐条钉住。
 */

const both = { caseSensitive: false, wholeWord: false };
const exact = { caseSensitive: true, wholeWord: false };
const word = { caseSensitive: false, wholeWord: true };

describe("findInLine", () => {
  it("默认不区分大小写", () => {
    expect(findInLine("Error: FAILED", "error", both)).toEqual([0]);
  });

  it("开了区分大小写就真的区分", () => {
    expect(findInLine("Error: FAILED", "error", exact)).toEqual([]);
    expect(findInLine("Error: FAILED", "Error", exact)).toEqual([0]);
  });

  it("一行里的多处匹配全找出来", () => {
    expect(findInLine("aXaXa", "a", both)).toEqual([0, 2, 4]);
  });

  it("**重叠的匹配也要找出来**", () => {
    // `aaa` 里搜 `aa` 有两处。从匹配之后继续找只能找到一处。
    expect(findInLine("aaa", "aa", both)).toEqual([0, 1]);
  });

  it("空查询返回空，而不是匹配每一处", () => {
    expect(findInLine("abc", "", both)).toEqual([]);
  });
});

describe("isWholeWord", () => {
  it("前后是空格或标点时算全词", () => {
    expect(isWholeWord("a foo b", 2, 3)).toBe(true);
  });

  it("前面是字母时不算", () => {
    expect(isWholeWord("xfoo", 1, 3)).toBe(false);
  });

  it("后面是字母时不算", () => {
    expect(isWholeWord("foox", 0, 3)).toBe(false);
  });

  it("**下划线算词内字符**——`foo_bar` 里搜 `foo` 不是全词", () => {
    expect(isWholeWord("foo_bar", 0, 3)).toBe(false);
  });

  it("数字也算词内字符", () => {
    expect(isWholeWord("foo1", 0, 3)).toBe(false);
  });

  it("行首与行尾算边界", () => {
    expect(isWholeWord("foo", 0, 3)).toBe(true);
  });

  it("中文全词匹配等价于「前后不是英文单词字符」", () => {
    // 用户想搜的是这个词，而中文本身不是 `\w`；这正好是想要的行为。
    expect(isWholeWord("这是 error 信息", 3, 5)).toBe(true);
  });
});

describe("findMatches", () => {
  it("逐行找，并带上**缓冲区绝对行号**", () => {
    const matches = findMatches(["first", "second", "first again"], "first", both);
    expect(matches).toEqual([
      { row: 0, column: 0, length: 5 },
      { row: 2, column: 0, length: 5 }
    ]);
  });

  it("全词匹配会过滤掉一部分", () => {
    const lines = ["foo foobar foo"];
    expect(findMatches(lines, "foo", both)).toHaveLength(3);
    expect(findMatches(lines, "foo", word)).toHaveLength(2);
  });

  it("空白查询返回空——空搜索不该把整个缓冲区当成匹配", () => {
    expect(findMatches(["a", "b"], "   ", both)).toEqual([]);
  });

  it("上限生效，避免几万处匹配把内存占满", () => {
    const lines = Array.from({ length: 100 }, () => "a");
    expect(findMatches(lines, "a", both, 10)).toHaveLength(10);
  });

  it("行缺失时跳过而不是抛异常", () => {
    // 缓冲区在读取过程中可能被裁剪，`lines` 里出现空洞是正常的。
    const sparse = ["a", undefined as unknown as string, "a"];
    expect(findMatches(sparse, "a", both)).toHaveLength(2);
  });
});

describe("nextMatch", () => {
  const matches = [
    { row: 0, column: 0, length: 1 },
    { row: 1, column: 0, length: 1 },
    { row: 2, column: 0, length: 1 }
  ];

  it("往后走一格", () => {
    expect(nextMatch(matches, 0, "next")).toBe(1);
  });

  it("往前走一格", () => {
    expect(nextMatch(matches, 2, "previous")).toBe(1);
  });

  it("**走到头会绕回**，而不是停住不动", () => {
    // 停住会让人以为「没有更多匹配了」。
    expect(nextMatch(matches, 2, "next")).toBe(0);
    expect(nextMatch(matches, 0, "previous")).toBe(2);
  });

  it("没有匹配时返回 -1", () => {
    expect(nextMatch([], 0, "next")).toBe(-1);
  });

  it("可以关掉绕回", () => {
    expect(nextMatch(matches, 2, "next", false)).toBe(2);
  });
});

describe("matchLabel", () => {
  it("没有匹配时说「无匹配」", () => {
    expect(matchLabel(0, -1)).toBe("无匹配");
  });

  it("**必须显示「第几处 / 共几处」**", () => {
    // 只说「找到了」的话，用户按了十次下一个也不知道到头没有。
    expect(matchLabel(7, 2)).toBe("3 / 7");
  });
});
