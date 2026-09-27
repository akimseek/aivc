import { useEffect, useMemo, useRef, useState } from "react";

/**
 * 命令面板（⌘K / Ctrl+K）。
 *
 * # 为什么值得有
 *
 * 界面上的动作会越来越多（新建终端、切视图、打开配置的某个页签、刷新、导出…）。
 * 全都做成按钮会让工具栏挤爆，而全都藏进菜单又难找。命令面板把「找动作」变成
 * 「打字」——这也是它值得有一个**键盘入口**的原因。
 *
 * # 三个必须做对的交互
 *
 * 1. **打开就聚焦输入框**：不聚焦的话用户要先点一下，那就失去键盘操作的意义了；
 * 2. **方向键选择而不是只靠鼠标**：命令面板是键盘驱动的，鼠标只是补充；
 * 3. **高亮项在列表滚动范围内**：用方向键移出可视区时列表要跟着滚，否则「选中了
 *    但看不见」。
 */

export type Command = {
  id: string;
  label: string;
  /** 分组名，例如「视图」「终端」。 */
  group?: string;
  run: () => void;
};

/**
 * 按关键词筛命令。
 *
 * 抽成纯函数是为了**可测**：匹配规则（大小写、分组名也参与匹配）是这段代码里唯一
 * 有分支的地方，而「搜 `终端` 能不能搜到 `新建系统终端`」这种事只靠点界面很难覆盖全。
 */
export function filterCommands(commands: Command[], query: string): Command[] {
  const keyword = query.trim().toLowerCase();
  if (!keyword) return commands;
  return commands.filter((command) =>
    `${command.group ?? ""} ${command.label}`.toLowerCase().includes(keyword)
  );
}

export function CommandPalette({
  commands,
  onClose
}: {
  commands: Command[];
  onClose: () => void;
}) {
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  const filtered = useMemo(() => filterCommands(commands, query), [commands, query]);

  // 过滤结果变了就把选中项收回第一项——否则高亮可能停在一个不存在的下标上。
  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  // 高亮项要滚进可视区。
  useEffect(() => {
    const node = listRef.current?.children[activeIndex];
    if (node instanceof HTMLElement) node.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  const runAt = (index: number) => {
    const command = filtered[index];
    if (!command) return;
    // 先关闭再执行：命令自己可能打开别的东西（例如配置覆盖层），
    // 顺序反了会让两个弹层叠在一起。
    onClose();
    command.run();
  };

  return (
    <div className="overlay" role="dialog" aria-modal="true" aria-label="命令面板" onClick={onClose}>
      <div className="palette" onClick={(event) => event.stopPropagation()}>
        <input
          ref={inputRef}
          className="palette-input"
          placeholder="输入命令…"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") onClose();
            if (event.key === "ArrowDown") {
              event.preventDefault();
              setActiveIndex((current) => Math.min(current + 1, filtered.length - 1));
            }
            if (event.key === "ArrowUp") {
              event.preventDefault();
              setActiveIndex((current) => Math.max(current - 1, 0));
            }
            if (event.key === "Enter") {
              event.preventDefault();
              runAt(activeIndex);
            }
          }}
        />
        <div className="palette-list" ref={listRef}>
          {filtered.map((command, index) => (
            <button
              key={command.id}
              type="button"
              className={`palette-item${index === activeIndex ? " on" : ""}`}
              onMouseEnter={() => setActiveIndex(index)}
              onClick={() => runAt(index)}
            >
              {command.group && <span className="palette-group">{command.group}</span>}
              <span className="palette-label">{command.label}</span>
            </button>
          ))}
          {filtered.length === 0 && <div className="palette-empty">没有匹配的命令。</div>}
        </div>
      </div>
    </div>
  );
}
