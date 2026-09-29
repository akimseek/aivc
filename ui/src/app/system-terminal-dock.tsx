import { useCallback, useEffect, useRef, useState } from "react";

import { Button, Tabs, Toolbar, type TabItem } from "../components";
import { TerminalPane } from "./terminal-pane";
import type { TerminalSession, TerminalSink } from "./use-terminals";

/**
 * 底部停靠的**系统终端**面板。
 *
 * # 为什么系统终端不该是工作区的一张标签
 *
 * 它是**另一种东西**：Codex 标签是「一条会话」，系统终端是「一个随手敲命令的窗口」。
 * 摆进同一排标签意味着**同一时刻只能看一个**——而真实用法恰恰是
 * 「一边看 Codex 在跑，一边在下面敲 `git status`」。旧平台就是一块独立底板
 * （`system-terminal.tsx`：自带标签、可拖动高度、可最小化、可关面板）。
 *
 * # 高度与最小化都记在本地
 *
 * 高度用 localStorage，键沿用旧平台的 `ai-console.system-terminal.height`——
 * 换个客户端不该丢掉这个偏好。最小化状态**不持久化**：它是「这一会儿先收起来」，
 * 下次启动还收着会让人以为面板没开。
 */

/** 旧平台用的同一个键，别改。 */
const STORAGE_HEIGHT_KEY = "ai-console.system-terminal.height";
const DEFAULT_HEIGHT = 260;
/** 低于这个高度标签条与终端都不可用了。 */
const MIN_HEIGHT = 120;
/** 再高就把上面的工作区挤没了。 */
const MAX_HEIGHT_RATIO = 0.72;

function readStoredHeight(): number {
  try {
    const raw = window.localStorage.getItem(STORAGE_HEIGHT_KEY);
    const value = raw ? Number.parseInt(raw, 10) : Number.NaN;
    if (!Number.isFinite(value)) return DEFAULT_HEIGHT;
    return clampHeight(value);
  } catch {
    // 隐私模式下 localStorage 会抛。拿不到就退回默认值，不影响用。
    return DEFAULT_HEIGHT;
  }
}

/**
 * 夹在可用范围内。
 *
 * # 两处顺序与兜底都是有原因的
 *
 * 1. **先取上界的下限，再夹**。窗口很矮时 `MAX_HEIGHT_RATIO` 算出来可能小于
 *    `MIN_HEIGHT`（视口 100px → 72px）。那时候若先夹下界再夹上界，结果会变成 72 ——
 *    比「最小可用高度」还矮。
 * 2. **`NaN` 要兜住**。localStorage 里的值可能是垃圾（`Number.parseInt` 给出 NaN），
 *    而 `Math.min/max` 会把这个 `NaN` 一路带出去，内联样式 `height: NaNpx` 无效，
 *    面板就没有高度——**表现是「面板不见了」**，而不是「高度不对」。
 */
export function clampHeight(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_HEIGHT;
  const upper = Math.max(MIN_HEIGHT, Math.round(window.innerHeight * MAX_HEIGHT_RATIO));
  return Math.min(Math.max(Math.round(value), MIN_HEIGHT), upper);
}

export function SystemTerminalDock({
  items,
  activeKey,
  terminals,
  registerSink,
  unregisterSink,
  onActivate,
  onClose,
  onClosePanel,
  onNew,
  onError
}: {
  /** 只有系统终端的标签。空数组时整块不渲染。 */
  items: TabItem[];
  activeKey: string;
  terminals: TerminalSession[];
  registerSink: (id: string, sink: TerminalSink) => void;
  unregisterSink: (id: string) => void;
  onActivate: (key: string) => void;
  /** 关掉**某一路**终端（那张标签的 ✕）。 */
  onClose: (key: string) => void;
  /** 收起整个面板（等同于「关闭面板」）。 */
  onClosePanel: () => void;
  /** 再开一路系统终端。 */
  onNew: () => void;
  onError: (message: string) => void;
}) {
  const [minimized, setMinimized] = useState(false);
  const [height, setHeight] = useState(readStoredHeight);
  const panelRef = useRef<HTMLElement | null>(null);
  const dragRef = useRef<{ y: number; height: number } | null>(null);

  // 窗口变小之后原来的高度可能超出上界，重新夹一次。
  useEffect(() => {
    const onResize = () => setHeight((current) => clampHeight(current));
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  /**
   * 拖动上边缘改高度。
   *
   * 用 `pointermove` / `pointerup` 挂在 `window` 上而不是面板上：鼠标拖到面板外面时
   * 事件仍然要跟着走，否则一拖快就「掉了」。
   */
  const startResize = useCallback((event: React.PointerEvent) => {
    event.preventDefault();
    dragRef.current = { y: event.clientY, height: panelRef.current?.offsetHeight ?? DEFAULT_HEIGHT };
    const onMove = (moveEvent: PointerEvent) => {
      const state = dragRef.current;
      if (!state) return;
      // 往上拖是变高，因此用起始 y 减当前 y。
      setHeight(clampHeight(state.height + (state.y - moveEvent.clientY)));
    };
    const onUp = () => {
      dragRef.current = null;
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      // 拖动结束才写盘：过程中的每一帧都写没有意义。
      try {
        const current = panelRef.current?.offsetHeight;
        if (current) window.localStorage.setItem(STORAGE_HEIGHT_KEY, String(current));
      } catch {
        /* 写不进去就算了，下次用默认高度。 */
      }
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  }, []);

  // 没有系统终端时不占任何位置——空面板会一直吃掉工作区的高度。
  if (items.length === 0) return null;

  return (
    <section
      className={`system-dock${minimized ? " minimized" : ""}`}
      style={minimized ? undefined : { height: `${height}px` }}
      aria-label="系统终端"
      ref={panelRef}
    >
      {!minimized && (
        <div
          className="system-dock-resize"
          role="separator"
          aria-orientation="horizontal"
          aria-label="调整系统终端高度"
          onPointerDown={startResize}
        />
      )}

      <Toolbar>
        <Tabs
          items={items}
          activeKey={activeKey}
          onActivate={onActivate}
          onClose={onClose}
        />
        <span className="spacer" />
        <Button small onClick={onNew} title="再开一路系统终端">
          新建
        </Button>
        <Button small onClick={() => setMinimized((value) => !value)}>
          {minimized ? "展开" : "最小化"}
        </Button>
        <Button small onClick={onClosePanel} title="关闭面板（会结束里面的终端）">
          关闭面板
        </Button>
      </Toolbar>

      {/* 最小化时**只留标签条**：终端本体仍在 DOM 里（卸载会丢 PTY 输出），
          只是不显示。`display: none` 会让 xterm 量不到尺寸，展开时靠
          `TerminalPane` 的 `active` 分支重新 fit。 */}
      <div className="system-dock-body">
        {items.map((item) => {
          const terminal = terminals.find((candidate) => candidate.id === item.key);
          if (!terminal) return null;
          return (
            <TerminalPane
              key={item.key}
              terminalId={terminal.id}
              running={terminal.running}
              // 最小化时**也算非前台**：不然隐藏中的终端会一直抢焦点。
              active={!minimized && item.key === activeKey}
              registerSink={registerSink}
              unregisterSink={unregisterSink}
              onError={onError}
            />
          );
        })}
      </div>
    </section>
  );
}
