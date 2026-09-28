/**
 * 终端内的文本查找。
 *
 * # 为什么自己实现，而不是用 `@xterm/addon-search`
 *
 * 那个 addon 要新增一个依赖（本项目当前离线，拿不到它）。而终端查找需要的全部能力
 * xterm 的 **buffer API 已经提供**：逐行读文本、滚动到某行、选中一段。
 * 自己要写的只有「在行数组里找匹配」这一步——它恰好是**最该有测试**的那一步。
 *
 * 代价要如实说：这个实现只搜**当前缓冲区**里的可见历史（xterm 默认 1000 行），
 * 不搜已经滚出缓冲区的内容。addon 也一样受这个限制。
 */

/** 一次查找的选项。 */
export type SearchOptions = {
  /** 区分大小写。 */
  caseSensitive: boolean;
  /** 全词匹配。 */
  wholeWord: boolean;
};

/** 一处匹配。`row` 是**缓冲区里的绝对行号**，不是屏幕上第几行。 */
export type SearchMatch = {
  row: number;
  /** 列偏移（以字符计）。 */
  column: number;
  /** 匹配长度。 */
  length: number;
};

/** 在**一行**里找出所有匹配。 */
export function findInLine(line: string, query: string, options: SearchOptions): number[] {
  if (!query) return [];
  const haystack = options.caseSensitive ? line : line.toLowerCase();
  const needle = options.caseSensitive ? query : query.toLowerCase();
  const columns: number[] = [];
  let from = 0;
  for (;;) {
    const at = haystack.indexOf(needle, from);
    if (at < 0) break;
    if (!options.wholeWord || isWholeWord(line, at, needle.length)) {
      columns.push(at);
    }
    // 从匹配的下一个字符继续：跳过整个匹配会漏掉重叠的情况（`aa` 在 `aaa` 里有两处）。
    from = at + 1;
  }
  return columns;
}

/**
 * 判断这一处是不是「一个完整的词」。
 *
 * # 边界规则
 *
 * 词的字符是**字母、数字、下划线**（与 `\w` 一致）。中文字符既不是字母也不是数字
 * （在 `\w` 的意义上），所以「中文全词匹配」实际等价于「前后不是英文单词字符」——
 * 那正是用户想要的行为。
 *
 * 下划线算词内字符：`foo_bar` 里搜 `foo` 不该算全词匹配。
 */
export function isWholeWord(line: string, at: number, length: number): boolean {
  const isWordChar = (char: string | undefined): boolean =>
    char !== undefined && /[A-Za-z0-9_]/.test(char);
  const before = at > 0 ? line[at - 1] : undefined;
  const after = at + length < line.length ? line[at + length] : undefined;
  return !isWordChar(before) && !isWordChar(after);
}

/**
 * 在整段缓冲区里找匹配。
 *
 * `lines` 是**按缓冲区行号顺序**排好的文本（调用方从 xterm 逐行取）。
 * 上限 `limit` 是防止一次搜索把几万处匹配全存下来——界面只需要知道
 * 「有几处、当前在第几处」，以及前后各一个能跳过去的位置。
 */
export function findMatches(
  lines: string[],
  query: string,
  options: SearchOptions,
  limit = 10_000
): SearchMatch[] {
  if (!query.trim()) return [];
  const matches: SearchMatch[] = [];
  for (let row = 0; row < lines.length; row += 1) {
    for (const column of findInLine(lines[row] ?? "", query, options)) {
      matches.push({ row, column, length: query.length });
      if (matches.length >= limit) return matches;
    }
  }
  return matches;
}

/**
 * 从一个位置往前/往后找**下一个**匹配。
 *
 * `wrap` 为真时绕回：用户按「下一个」按到底之后应该回到第一个，
 * 而不是停住不动——停住会让人以为没有匹配了。
 */
export function nextMatch(
  matches: SearchMatch[],
  current: number,
  direction: "next" | "previous",
  wrap = true
): number {
  if (matches.length === 0) return -1;
  const step = direction === "next" ? 1 : -1;
  const candidate = current + step;
  if (candidate >= 0 && candidate < matches.length) return candidate;
  if (!wrap) return current;
  return direction === "next" ? 0 : matches.length - 1;
}

/** 查找结果的说明文字。**必须说清「第几处 / 共几处」**，否则转圈找不到时无从判断。 */
export function matchLabel(total: number, current: number): string {
  if (total === 0) return "无匹配";
  return `${current + 1} / ${total}`;
}
