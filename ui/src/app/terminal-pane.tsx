import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import * as api from "../ipc/commands";
import { ComposerInput } from "./composer-input";
import { initialComposerVisible, shouldRevealComposer } from "./composer-text";
import { TerminalSearchBar } from "./terminal-search-bar";
import type { TerminalSink } from "./terminal-output";

/**
 * 一路终端的宿主。
 *
 * # 三件事必须接对，否则终端会静默卡住
 *
 * 1. **输入要回到 PTY**：`term.onData` → `terminal:write`。交互式程序会发 `ESC[6n`
 *    之类的查询，xterm 会通过同一个回调给出应答；这条链路断了，界面显示正常但完全
 *    没反应。
 * 2. **输出是原始字节**：事件载荷是数字数组，直接喂给 `term.write(Uint8Array)`，
 *    **不要**先转成字符串——光标控制序列不是合法 UTF-8，转一次就废了。
 * 3. **尺寸要告诉后端**：容器变化时 `fit()` 之后必须发 `terminal:resize`，
 *    否则 PTY 里的程序仍按旧尺寸排版，换行位置全错。
 */
export function TerminalPane({
  terminalId,
  active,
  running,
  /**
   * 这条终端属于哪条会话。为空表示「新会话」——自管输入的初始可见性由它决定：
   * 新会话一进去就有输入框，恢复历史会话时先藏着（见 `initialComposerVisible`）。
   */
  sessionId,
  registerSink,
  unregisterSink,
  onError
}: {
  terminalId: string;
  /** 是否在前台。非前台的仍然挂载——见下。 */
  active: boolean;
  /**
   * PTY 是否已经起来了。
   *
   * **启动失败时必须为 false**：否则随后的 `resize` 会报「终端会话不存在」，
   * 把真正的失败原因（例如命令拼错、PTY 起不来）**覆盖掉**，用户看到的是一句
   * 与原因毫无关系的报错。
   */
  running: boolean;
  sessionId?: string;
  registerSink: (id: string, sink: TerminalSink) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // 尺寸回调挂在 ResizeObserver 里，拿不到最新的 prop，因此用一个 ref 镜像它。
  const runningRef = useRef(running);
  runningRef.current = running;

  /**
   * 自管输入是否显示。
   *
   * 初值由「是不是恢复历史会话」决定，之后由**第一段输出**把它打开——
   * 那是唯一的入口（旧平台的 `revealComposer()` 也只能被这条路径触发）。
   */
  const [composerVisible, setComposerVisible] = useState(() =>
    initialComposerVisible(sessionId)
  );
  const composerVisibleRef = useRef(composerVisible);
  composerVisibleRef.current = composerVisible;
  /** 每次挂载都从初值重新判断：换了一条终端（key 变了）就是另一路。 */
  useEffect(() => {
    setComposerVisible(initialComposerVisible(sessionId));
  }, [sessionId, terminalId]);

  /** 终端内查找栏是否打开。Ctrl+F 开、Escape 关。 */
  const [searchOpen, setSearchOpen] = useState(false);

  /**
   * 取当前缓冲区的文本行。
   *
   * xterm 的行对象有两种：**已折行**（`isWrapped`）的续行在逻辑上属于上一行，
   * 直接拼会在每处折行都不该有的地方断开搜索词。因此续行的文本要接到上一行之后。
   * 这一条不做的话，「一个词刚好被窗口宽度折断」就永远搜不到——而终端里
   * 长 JSON、长 URL 被折断是常态。
   */
  const readLines = useCallback((): string[] => {
    const term = termRef.current;
    if (!term) return [];
    const buffer = term.buffer.active;
    const total = buffer.length;
    const lines: string[] = [];
    for (let index = 0; index < total; index += 1) {
      const line = buffer.getLine(index);
      if (!line) {
        lines.push("");
        continue;
      }
      const text = line.translateToString(true);
      if (line.isWrapped && lines.length > 0) {
        lines[lines.length - 1] = `${lines[lines.length - 1]}${text}`;
      } else {
        lines.push(text);
      }
    }
    return lines;
  }, []);

  /**
   * 把一处匹配滚到可见并选中。
   *
   * 三件事都要做：
   * 1. **滚动**——不滚过去用户看不到，等于没找到；
   * 2. **选中**——让他知道命中的是哪一段；
   * 3. **进前台**——只有可见的终端才谈得上「看到」。
   */
  const revealMatch = useCallback((match: { row: number; column: number; length: number }) => {
    const term = termRef.current;
    if (!term) return;
    term.scrollToLine(match.row);
    term.select(match.column, match.row, match.length);
    term.focus();
  }, []);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontFamily: 'Consolas, "Cascadia Mono", "Courier New", monospace',
      fontSize: 13,
      theme: {
        background: "#0f172a",
        foreground: "#c9d1d9",
        cursor: "#7ee787"
      }
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);
    termRef.current = term;
    fitRef.current = fit;

    // 输出：把这一路的字节接进 xterm。
    registerSink(terminalId, (bytes) => {
      term.write(bytes);
      /**
       * **第一段输出到达时把自管输入露出来。**
       *
       * 恢复历史会话时它初值是隐藏的（那时终端正在重放历史，输入框会挡住输出），
       * 而这是唯一能把它们显示出来的路径——旧平台也是这么做的
       * （`embedded-terminal.tsx` 的 `onTerminalData` 里那句 `revealComposer()`）。
       *
       * 少了这一条，恢复的会话里用户**永远看不到**自管输入。
       */
      if (shouldRevealComposer(composerVisibleRef.current, bytes.length > 0)) {
        setComposerVisible(true);
        // 输入框占了一块高度，终端要重新量一次，否则最后几行会被它挡住。
        window.requestAnimationFrame(() => {
          try {
            fit.fit();
            if (runningRef.current) {
              void api.resizeTerminal(terminalId, term.cols, term.rows).catch(() => undefined);
            }
          } catch {
            /* 容器不可测量，忽略。 */
          }
        });
      }
    });

    // 输入：**这是终端能动的关键一环**（见文件头注释）。
    const dataSub = term.onData((data) => {
      void api.writeTerminal(terminalId, data).catch(() => undefined);
    });

    const syncSize = () => {
      try {
        fit.fit();
      } catch {
        // 容器还没布局完（宽高为 0）时会抛；下一轮观察器回调会再来一次。
        return;
      }
      // 尺寸同步的失败**不能**往上抛：PTY 没起来时它必然失败，而那个错误会盖住
      // 真正的启动失败原因（界面只剩一句「终端会话不存在」）。
      // PTY 没起来时连发都不发——省一次注定失败的往返。
      if (!runningRef.current) return;
      void api.resizeTerminal(terminalId, term.cols, term.rows).catch(() => undefined);
    };

    // 首次同步要等一帧：`term.open` 之后容器才被浏览器布局。
    const initial = window.requestAnimationFrame(syncSize);
    const observer = new ResizeObserver(syncSize);
    observer.observe(host);

    return () => {
      window.cancelAnimationFrame(initial);
      observer.disconnect();
      dataSub.dispose();
      unregisterSink(terminalId);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [terminalId, registerSink, unregisterSink, onError]);

  // 切回前台时重新量一次：隐藏期间容器的尺寸可能已经变了，而 xterm 不会自己知道。
  useEffect(() => {
    if (!active) return;
    const fit = fitRef.current;
    const term = termRef.current;
    if (!fit || !term) return;
    const raf = window.requestAnimationFrame(() => {
      try {
        fit.fit();
        void api.resizeTerminal(terminalId, term.cols, term.rows).catch(() => undefined);
      } catch {
        /* 容器仍不可测量，忽略。 */
      }
    });
    return () => window.cancelAnimationFrame(raf);
  }, [active, terminalId]);

  /**
   * 切到前台时**把焦点交给终端**。
   *
   * # 这是探针发现的真实缺陷
   *
   * 此前没有任何地方给 xterm 焦点。后果很隐蔽：终端起来了、CLI 也在跑，
   * 但**用户敲键盘没有反应**——按键落在背后的界面上，而不是 PTY 里。
   *
   * 真实场景里立刻会撞上：Codex 启动时会问一句
   * 「`Update available · 0.156.1 → 0.157.1` … `enter continue · esc skip`」，
   * 用户按回车/esc 却什么都没发生，只能以为程序卡死了。
   *
   * 只在 `active` 时抢焦点：非前台的终端仍然挂载（不能卸载，会丢输出），
   * 但绝不能把焦点从用户正在用的那一路抢走。
   */
  useEffect(() => {
    if (!active) return;
    const term = termRef.current;
    if (!term) return;
    // 自管输入显示时，它才是用户想打字的地方——抢焦点会把光标从输入框里拽走。
    if (composerVisible) return;
    try {
      term.focus();
    } catch {
      /* 还没挂到 DOM 上，下一轮 active 变化时再试。 */
    }
  }, [active, terminalId, composerVisible]);

  /**
   * 自管输入**只在前台显示**。
   *
   * 非前台的终端仍然挂载（不能卸载，会丢输出），但它的输入框不该占位置：
   * 用户在看另一路终端时，屏幕下方多出一个属于别处的输入框只会让人写错地方。
   */
  /**
   * Ctrl+F 打开查找栏。
   *
   * 只在**前台**时响应：非前台的终端仍然挂载（不能卸载，会丢输出），
   * 但绝不该抢走用户正在用的那一路的快捷键。
   */
  useEffect(() => {
    if (!active) return;
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        setSearchOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [active]);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    // 关掉查找栏把焦点还给终端：用户下一件事通常是继续打字。
    try {
      termRef.current?.focus();
    } catch {
      /* 终端没挂上，忽略。 */
    }
  }, []);

  return (
    <div
      className={`terminal-slot${active ? "" : " inactive"}`}
      // 非前台的终端**不卸载**：卸载会丢掉 PTY 的输出（事件没人接就丢弃了），
      // 用户切回来会发现中间断了一截。
      aria-hidden={!active}
    >
      <div className="terminal-host" ref={hostRef} />
      {active && searchOpen && (
        <TerminalSearchBar
          open={searchOpen}
          readLines={readLines}
          onReveal={revealMatch}
          onClose={closeSearch}
        />
      )}
      {active && composerVisible && (
        <ComposerInput
          terminalId={terminalId}
          onSent={() => {
            // 发完把焦点还给终端：用户下一件事通常是看 CLI 的回应。
            window.requestAnimationFrame(() => {
              try {
                termRef.current?.focus();
              } catch {
                /* 终端还没挂上，忽略。 */
              }
            });
          }}
          onError={onError}
        />
      )}
    </div>
  );
}
