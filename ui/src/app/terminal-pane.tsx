import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import * as api from "../ipc/commands";
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
    registerSink(terminalId, (bytes) => term.write(bytes));

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

  return (
    <div
      className={`terminal-slot${active ? "" : " inactive"}`}
      // 非前台的终端**不卸载**：卸载会丢掉 PTY 的输出（事件没人接就丢弃了），
      // 用户切回来会发现中间断了一截。
      aria-hidden={!active}
    >
      <div className="terminal-host" ref={hostRef} />
    </div>
  );
}
