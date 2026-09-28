import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { Button, TextInput } from "../components";
import {
  findMatches,
  matchLabel,
  nextMatch,
  type SearchMatch,
  type SearchOptions
} from "./terminal-search";

/**
 * 终端内的查找栏。
 *
 * # 它读的是什么
 *
 * 通过 `readLines` 从**调用方持有的 xterm 实例**逐行取当前缓冲区文本。
 * 查找栏自己不碰 xterm 的内部结构——那样它就没法脱离终端单独测了。
 *
 * # 跳转必须**滚动并选中**，不能只是把行号算出来
 *
 * 用户按「下一个」的期望是「让我看到它」：只算出位置而不滚过去，等于没找到。
 * 因此 `onReveal` 由调用方实现（它拿得到 xterm 的 `scrollToLine` 与 `select`）。
 *
 * # 只搜当前缓冲区
 *
 * xterm 默认保留 1000 行回滚。滚出去的内容搜不到——这一点**必须写进提示**，
 * 否则用户会以为「日志里没有这个词」，而其实只是被截掉了。
 */

/** xterm 回滚缓冲区默认保留的行数。用来提示用户「搜的范围有多大」。 */
export const DEFAULT_SCROLLBACK = 1000;

export function TerminalSearchBar({
  open,
  readLines,
  onReveal,
  onClose
}: {
  open: boolean;
  /** 取当前缓冲区的文本行，**按缓冲区行号顺序**。 */
  readLines: () => string[];
  /** 把某一处匹配滚到可见并选中。 */
  onReveal: (match: SearchMatch) => void;
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [options, setOptions] = useState<SearchOptions>({
    caseSensitive: false,
    wholeWord: false
  });
  const [current, setCurrent] = useState(-1);
  const inputRef = useRef<HTMLInputElement | null>(null);

  /**
   * 匹配结果。
   *
   * 用 `useMemo` 而不是 state：它完全由「查询 + 选项 + 缓冲区内容」决定。
   * 放进 state 就要在缓冲区变化时手动同步，而那正是会漏的地方。
   *
   * 依赖里有一个 `revision`：xterm 的缓冲区**是可变的**，`readLines()` 返回新数组
   * 但内容可能已经变了。这里用一个自增号强制在每次查找时重算。
   */
  const [revision, setRevision] = useState(0);
  const matches = useMemo(() => {
    if (!open) return [];
    return findMatches(readLines(), query, options);
    // `revision` 只用于强制重算——它本身不参与计算。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, query, options, revision]);

  /** 查询或选项变了，把当前匹配重置到第一处并跳过去。 */
  useEffect(() => {
    setCurrent(matches.length > 0 ? 0 : -1);
    if (matches.length > 0) onReveal(matches[0]!);
    // 只在「匹配集合变了」时跳——把 `onReveal` 放进依赖会让每次父组件重渲染都跳一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [matches]);

  /** 打开时聚焦输入框：用户按了 Ctrl+F，下一步一定是打字。 */
  useEffect(() => {
    if (!open) return;
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [open]);

  const go = useCallback(
    (direction: "next" | "previous") => {
      const target = nextMatch(matches, current, direction);
      if (target < 0) return;
      setCurrent(target);
      onReveal(matches[target]!);
    },
    [matches, current, onReveal]
  );

  if (!open) return null;

  return (
    <div className="terminal-search" role="search" aria-label="搜索终端内容">
      <TextInput
        ref={inputRef}
        value={query}
        placeholder="搜索终端内容"
        aria-label="搜索终端内容"
        onChange={(event) => {
          setQuery(event.target.value);
          setRevision((value) => value + 1);
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            go(event.shiftKey ? "previous" : "next");
          }
          if (event.key === "Escape") {
            event.preventDefault();
            onClose();
          }
        }}
      />

      <label className="terminal-search-toggle" title="区分大小写">
        <input
          type="checkbox"
          checked={options.caseSensitive}
          aria-label="区分大小写"
          onChange={(event) => {
            setOptions((current) => ({ ...current, caseSensitive: event.target.checked }));
            setRevision((value) => value + 1);
          }}
        />
        <span>Aa</span>
      </label>

      <label className="terminal-search-toggle" title="全词匹配">
        <input
          type="checkbox"
          checked={options.wholeWord}
          aria-label="全词匹配"
          onChange={(event) => {
            setOptions((current) => ({ ...current, wholeWord: event.target.checked }));
            setRevision((value) => value + 1);
          }}
        />
        <span>全词</span>
      </label>

      <span className="terminal-search-count" aria-live="polite">
        {matchLabel(matches.length, current)}
      </span>

      <Button
        small
        disabled={matches.length === 0}
        aria-label="上一个匹配项"
        title="上一个匹配项"
        onClick={() => go("previous")}
      >
        ↑
      </Button>
      <Button
        small
        disabled={matches.length === 0}
        aria-label="下一个匹配项"
        title="下一个匹配项"
        onClick={() => go("next")}
      >
        ↓
      </Button>
      <Button small aria-label="关闭搜索" title="关闭搜索" onClick={onClose}>
        ✕
      </Button>

      {/* 范围必须说出来：回滚区之外的输出搜不到，而用户会以为「日志里没有」。 */}
      <span className="terminal-search-hint">
        只在当前回滚缓冲区（约 {DEFAULT_SCROLLBACK} 行）内查找
      </span>
    </div>
  );
}
