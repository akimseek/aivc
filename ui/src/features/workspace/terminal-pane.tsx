import { useCallback, useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";

import { errorText } from "../../components";
import * as api from "../../ipc/commands";
import { ComposerInput } from "./composer/composer-input";
import { TerminalSearchBar } from "./terminal-search-bar";
import { TERMINAL_FONT_FAMILY, terminalThemeFor } from "../../app/terminal-theme";
import { TerminalPasteDialog, TerminalTextContextMenu } from "./terminal-text-menu";
import type { TerminalSink } from "../../app/terminal-output";

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
 *
 * # 输入侧的状态是**受控**的
 *
 * 输入模式与输入框可见性此前是本组件的本地状态。现在它们在
 * [`useTerminals`](./use-terminals.ts) 里（状态栏也要读它，spec §6），
 * 这里只做两件事：报告「收到了输出」、按 props 渲染。
 * 模式的切换入口也已经在状态栏上（输入区不再有那个按钮）。
 *
 * # 主题也是受控的，而且**不重建终端**
 *
 * xterm 用 canvas 画字符，读不到 CSS 变量——底色只能由
 * [`terminalThemeFor`](./terminal-theme.ts) 在 JS 侧给。主题变化时只改
 * `terminal.options.theme`：重建终端会把缓冲区（回滚历史）清空，那正是用户
 * 切主题时正在看的东西。
 */
export function TerminalPane({
  terminalId,
  active,
  running,
  /**
   * 自管输入框此刻该不该显示。初值与「收到输出后放出来」的规则都在
   * [`useTerminals`](./use-terminals.ts)（纯判据在
   * [`composer-text.ts`](../features/workspace/composer/composer-text.ts)）。
   */
  composerVisible,
  /**
   * 「把焦点交给终端」的请求（状态栏胶囊切到「终端原生输入」时发）。
   *
   * **只有变化才算一次请求**：与 `findSignal` 同一种消费式信号，ref 的初值就是挂载时
   * 拿到的那个数。不请求就不抢焦点——「焦点该在谁那里」由下面那条 `active` 的 effect 管。
   */
  focusRequest = 0,
  /** 收到一段输出。**在数据回调里调**——这是「恢复的会话把输入框放出来」的唯一时机。 */
  onOutput,
  registerSink,
  unregisterSink,
  onError,
  /** 当前解析出的主题。改它只换 xterm 的配色，不重建终端。 */
  resolvedTheme,
  /**
   * 递增即「打开查找栏」（标签栏那个查找按钮）。
   *
   * **只有变化才算一次请求**：不变（包括挂载时拿到的那个数）都不触发。
   */
  findSignal = 0,
  /** 这一路终端的平台 / 供应商 / 目标：输入区里的模型选择要用它决定清单从哪来。 */
  providerId,
  vendorId,
  targetId
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
  composerVisible: boolean;
  onOutput: () => void;
  registerSink: (id: string, sink: TerminalSink) => void;
  unregisterSink: (id: string) => void;
  onError: (message: string) => void;
  resolvedTheme: "light" | "dark";
  findSignal?: number;
  focusRequest?: number;
  providerId: string;
  vendorId?: string;
  targetId?: string;
}) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  // 尺寸回调挂在 ResizeObserver 里，拿不到最新的 prop，因此用一个 ref 镜像它。
  const runningRef = useRef(running);
  runningRef.current = running;
  /**
   * 输出回调同样要镜像。
   *
   * 它是上层每次渲染新建的箭头函数（`() => noteOutput(id)`）；若把它写进挂载
   * effect 的依赖里，**每次渲染都会重建 xterm**——缓冲区里的历史就没了。
   */
  const onOutputRef = useRef(onOutput);
  onOutputRef.current = onOutput;

  /** 文本右键菜单。`canCopy` 在右键那一刻取定，见 `onContextMenu`。 */
  const [textMenu, setTextMenu] = useState<{
    x: number;
    y: number;
    canCopy: boolean;
  } | null>(null);
  /** 待确认的粘贴内容。非空即显示确认框，见 `TerminalPasteDialog`。 */
  const [pasteText, setPasteText] = useState<string | null>(null);

  /**
   * 复制选中内容。
   *
   * 拿不到选中就什么都不做——不要退化成「复制整屏」，
   * 那会把几万行塞进剪贴板，用户下一次粘贴才知道闯了祸。
   */
  const copySelection = useCallback(() => {
    const selection = termRef.current?.getSelection();
    if (!selection) return;
    void api.copyText(selection).catch(() => undefined);
  }, []);

  /** 读剪贴板并弹出确认框；空内容不弹（弹一个空白框只会让人以为坏了）。 */
  const requestPaste = useCallback(() => {
    void api
      .readText()
      .then((text) => {
        if (text) setPasteText(text);
      })
      .catch((error: unknown) =>
        onError(errorText(error))
      );
  }, [onError]);

  /**
   * 确认粘贴。
   *
   * 走 xterm 自己的 `paste`：它**会处理括号粘贴模式**（应用开启了
   * `\x1b[?2004h` 就自动加标记），手写这段标记迟早会和模式不同步。
   *
   * 先把结尾换行去掉：从网页里拷来的内容几乎总带一个，
   * 而终端里的换行等于**替用户按下回车**。
   */
  const confirmPaste = useCallback(() => {
    const raw = pasteText;
    setPasteText(null);
    if (!raw) return;
    const text = raw.replace(/\r\n/g, "\n").replace(/\r/g, "\n").replace(/\n+$/, "");
    const term = termRef.current;
    if (!text || !term) return;
    term.focus();
    term.paste(text);
  }, [pasteText]);

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
    const host = mountRef.current;
    if (!host) return;

    const term = new Terminal({
      convertEol: false,
      cursorBlink: true,
      fontFamily: TERMINAL_FONT_FAMILY,
      fontSize: 13,
      // 主题由 props 决定（xterm 读不到 CSS 变量，值在 `terminal-theme.ts`）。
      theme: terminalThemeFor(resolvedTheme)
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
       * **把「收到输出」这件事报上去。**
       *
       * 恢复历史会话时输入框初值是隐藏的（那时终端正在重放历史，输入框会挡住输出），
       * 而这是唯一能把它们显示出来的时机——旧平台也是这么做的
       * （`embedded-terminal.tsx` 的 `onTerminalData` 里那句 `revealComposer()`）。
       *
       * 判据（要不要放出来）在 `useTerminals` 里，这里只报告事实。
       */
      if (bytes.length > 0) onOutputRef.current();
    });

    // 输入：**这是终端能动的关键一环**（见文件头注释）。
    const dataSub = term.onData((data) => {
      void api.writeTerminal(terminalId, data).catch(() => undefined);
    });

    /**
     * 右键：拦掉浏览器菜单，开自己的「复制 / 粘贴」。
     *
     * 终端是个 `canvas`，浏览器默认菜单对它没有意义（「在新标签页打开图片」之类），
     * 因此必须拦掉。而一旦拦掉，**复制一段终端输出就只剩「选中 + Ctrl+C」这一条路**，
     * 鼠标用户没有出口。
     *
     * `canCopy` 在这一刻取：菜单弹出后再取就已经晚了（用户可能已经点了别处，
     * 选中也就没了）。
     */
    const onContextMenu = (event: MouseEvent) => {
      event.preventDefault();
      setTextMenu({ x: event.clientX, y: event.clientY, canCopy: term.hasSelection() });
    };
    host.addEventListener("contextmenu", onContextMenu);

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
      host.removeEventListener("contextmenu", onContextMenu);
      dataSub.dispose();
      unregisterSink(terminalId);
      term.dispose();
      termRef.current = null;
      fitRef.current = null;
    };
  }, [terminalId, registerSink, unregisterSink, onError]);
  // 注意：`resolvedTheme` **不在**上面那份依赖里。放进去会在每次切主题时重建终端，
  // 而重建会把回滚缓冲清空——用户切主题时正在看的历史就没了。主题变化走下面那条。

  /**
   * 明暗切换：只换 xterm 的配色。
   *
   * `terminal.options.theme` 是 xterm 支持的原地更新（不重建、不动缓冲）。
   * 终端外面那圈留白（`.terminal-stack` / `.terminal-host`）由 CSS 变量跟着
   * `<html data-theme>` 一起换，两条路必须同时发生——否则就是「终端底色变了、
   * 外圈没变」的半截错位。
   */
  useEffect(() => {
    const term = termRef.current;
    if (!term) return;
    term.options.theme = terminalThemeFor(resolvedTheme);
  }, [resolvedTheme]);

  /**
   * 标签栏那个查找按钮：计数**变化**就打开查找栏。
   *
   * 与下面的 Ctrl+F 是**同一处状态**的两个入口（都在本组件里），不需要事件总线。
   * 只在**前台**时响应：非前台的终端仍然挂着（不能卸载，会丢输出），
   * 但打开一个看不见的查找栏没有意义。信号被消费掉（不落账）是有意的——
   * 否则切到前台时那一次「顺手」会把它弹出来。
   *
   * # 「变化」而不是「非零」
   *
   * 挂载本身不是一次点击。若只挡住 `0`，用户在点过一次查找（计数 ≥ 1）之后
   * 新建的每一路终端都会在挂到前台的那一刻自弹查找栏，而
   * `TerminalSearchBar` 打开即 `focus()` + `select()`——用户在新终端里敲的
   * 第一串字会进搜索框，终端看起来「没反应」。因此把**已消费的计数**记在 ref 里，
   * 只有它变了才是一次新请求；ref 的初值就是挂载时那个数（那次不算）。
   */
  const consumedFindSignal = useRef(findSignal);
  useEffect(() => {
    if (findSignal === consumedFindSignal.current) return;
    consumedFindSignal.current = findSignal;
    if (active) setSearchOpen(true);
    // 只在计数变化时触发；`active` 不放依赖里——切前台不该顺手弹出查找栏。
  }, [findSignal]);

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
   * 输入框出现或消失时重新量一次。
   *
   * 它占了一块高度，终端要跟着缩——不重量的话最后几行会被输入框挡住。
   * 此前这一步写在输出回调里（那时状态是本地的），状态上收之后改成跟着 prop 走。
   */
  useEffect(() => {
    if (!composerVisible) return;
    const fit = fitRef.current;
    const term = termRef.current;
    if (!fit || !term) return;
    const raf = window.requestAnimationFrame(() => {
      try {
        fit.fit();
        if (runningRef.current) {
          void api.resizeTerminal(terminalId, term.cols, term.rows).catch(() => undefined);
        }
      } catch {
        /* 容器不可测量，忽略。 */
      }
    });
    return () => window.cancelAnimationFrame(raf);
  }, [composerVisible, terminalId]);

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

  /**
   * 「把焦点交给终端」的请求：计数**变化**才交一次。
   *
   * 它是**显式请求**，因此不受 `composerVisible` 影响——用户刚点了「终端原生输入」，
   * 那一刻焦点必须落到终端上，哪怕自管输入框还在屏幕上（切回自管输入是下一个动作）。
   * 不这样写的话，请求会被上面那条 effect 的 `if (composerVisible) return` 吃掉，
   * 用户敲的字仍然进不了 PTY。
   *
   * ref 的初值就是挂载时那个数（那次不算）：与 `findSignal` 同一条理由——挂载本身
   * 不是一次点击，否则新建的终端一挂上去就把焦点抢走。
   */
  const consumedFocusRequest = useRef(focusRequest);
  useEffect(() => {
    if (focusRequest === consumedFocusRequest.current) return;
    consumedFocusRequest.current = focusRequest;
    if (!active) return;
    try {
      termRef.current?.focus();
    } catch {
      /* 终端还没挂到 DOM 上，忽略。 */
    }
    // 只在请求变化时触发；`active` 不放依赖里——切前台是上面那条 effect 的事。
  }, [focusRequest]);

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
      {/*
        挂载点**不带 padding**，外框的留白在 `.terminal-host` 上。

        # 为什么必须分两层（真机实测的缺陷：最后一行被裁）

        `FitAddon` 量的是 `term.open(el)` 那个元素的**父元素**，且只减
        xterm 自身的 padding——而全站 `box-sizing: border-box` 下
        `getComputedStyle(host).height` 返回的是**含 padding 的框高**。
        挂载点若直接带 `padding: 8px 10px 10px`，可用高度就被多算 18px
        （横向多算 20px），行/列各多出一档：最后一行落在宿主框外，被
        `overflow: hidden` 裁掉——看起来就像「底部被自管输入框挡住」。
      */}
      <div className="terminal-host">
        <div className="terminal-mount" ref={mountRef} />
      </div>
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
          providerId={providerId}
          vendorId={vendorId}
          targetId={targetId}
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
      {active && textMenu && (
        <TerminalTextContextMenu
          x={textMenu.x}
          y={textMenu.y}
          canCopy={textMenu.canCopy}
          // 有终端就能粘：粘贴是往这一路 PTY 写，不需要它是不是在跑。
          canPaste
          onCopy={copySelection}
          onPaste={requestPaste}
          onDismiss={() => setTextMenu(null)}
        />
      )}
      {active && pasteText !== null && (
        <TerminalPasteDialog
          text={pasteText}
          onCancel={() => setPasteText(null)}
          onConfirm={confirmPaste}
        />
      )}
    </div>
  );
}
