import { useCallback, useEffect, useRef, useState } from "react";

import * as api from "../ipc/commands";
import type { AiTarget, TerminalAttention, TerminalDataPayload, TerminalStatusPayload } from "../types";
import {
  initialComposerVisible,
  shouldRevealComposer,
  type TerminalInputMode
} from "./composer-text";
import { TerminalOutputRouter } from "./terminal-output";
import type { TerminalSink } from "./terminal-output";

export type { TerminalSink } from "./terminal-output";

/**
 * 终端的会话管理。
 *
 * # 数据是怎么到 xterm 的
 *
 * 后端把所有终端的输出都推到**同一条事件通道**上，载荷里带 `terminalId`。因此这里
 * 只订阅一次，再按 `terminalId` 分发给各自的「接收端」（`TerminalPane` 挂载时注册）。
 *
 * 每个终端各自订阅一次也能work，但那样事件监听会随终端数增长，且关闭时漏掉一个就会
 * 留下一个永不触发的监听。
 *
 * # 终端只有一种：Codex 终端
 *
 * 这里原来还有一类「系统终端」（跑用户平时的 shell）。它已按用户要求**整体移除**，
 * 连同后端命令、底部停靠面板与类型下拉一起。因此 `kind` 字段也没了——
 * 只剩下的那一种不需要一个恒为同一个值的判别字段。
 *
 * # 缓冲与分发在 [`TerminalOutputRouter`] 里
 *
 * 那是纯逻辑，抽出去之后可以脱离 React 测——见 `terminal-output.test.ts`。
 */

/** 路由模式。`locked` = 不自动故障转移。 */
export type VendorRouteMode = "dynamic" | "locked";

/** 一次供应商切换的展示信息。 */
export type VendorSwitch = {
  vendorId: string;
  /** `manual` 用户切的；`failure` 上游失败自动切的；`mode` 只是模式变了。 */
  reason: "manual" | "failure" | "mode" | "candidate-pool";
};

/**
 * 每路终端的**输入侧**界面状态。
 *
 * # 为什么要上收到这里
 *
 * 状态栏上有「输入模式」胶囊（spec §6），它讲的是**当前前台那一路**终端；
 * 而状态此前只活在 `TerminalPane` 里。留在那里的话，状态栏要么拿不到它，
 * 要么得把整棵终端组件树的状态往上扛。
 *
 * 于是 `TerminalPane` 改成受控：它只报告「收到了输出」，可见性与模式由这里决定。
 */
export type TerminalUiState = { inputMode: TerminalInputMode; composerVisible: boolean };

/** 一路终端。 */
export type TerminalSession = {
  /** `terminalId`，同时用作标签的 key。 */
  id: string;
  title: string;
  /** PTY 是否还活着。收到退出事件后置 false。 */
  running: boolean;
  /**
   * 这路终端属于哪个平台。
   *
   * 界面要用它去取该平台的供应商列表（状态栏的选择器）。
   */
  providerId: string;
  /**
   * 当前由哪个供应商服务。
   *
   * 由**后端**在启动时选定并返回，之后靠 `gateway:vendor-switched` 更新。
   * 缺席表示这条终端直连官方 API（没配供应商，或该平台不支持网关）。
   */
  vendorId?: string;
  vendorMode?: VendorRouteMode;
  /** 最近一次切换。有值时界面上会标出来「已自动切换」之类。 */
  lastSwitch?: VendorSwitch;
  /**
   * 这一路是**干净退出**（退出码 0）的。
   *
   * 干净退出要把标签收掉（见退出处理的文档）；异常退出保留现场看报错，
   * 因此这里必须把两者区分开，不能只看 `running`。
   */
  exitedCleanly?: boolean;
  /**
   * 退出码。
   *
   * 与 `exitedCleanly` 不是一回事：后者只回答「要不要收标签」，
   * 而用户要看的是**这个数字**——`1`（自己报错）与 `137`（被 kill）指向完全不同的原因。
   */
  exitCode?: number;
};

export function useTerminals() {
  const [terminals, setTerminals] = useState<TerminalSession[]>([]);
  /**
   * 还没发出去的「启动提示词」，按终端编号排队。
   *
   * 用 `ref` 而不是 state：它不参与渲染，而且**必须**在数据回调里读到最新值——
   * 放进 state 就会读到闭包里的旧值（那个回调只订阅一次）。
   */
  const pendingPrompts = useRef<Map<string, string>>(new Map());
  /** 各终端待处理的提醒（等待确认等）。用户在界面上看过就该清掉。 */
  const [attention, setAttention] = useState<Record<string, TerminalAttention>>({});
  /** 各终端的输入侧界面状态（模式与输入框可见性）。见 [`TerminalUiState`]。 */
  const [uiStates, setUiStates] = useState<Record<string, TerminalUiState>>({});
  /**
   * 输出分发与缓冲。
   *
   * 用 ref 持有**同一个实例**：它内部有跨渲染的状态（缓冲的块），每次渲染新建一个
   * 就等于把没发出去的输出丢掉。
   */
  const routerRef = useRef<TerminalOutputRouter | null>(null);
  routerRef.current ??= new TerminalOutputRouter();
  const router = routerRef.current;
  const seqRef = useRef(0);

  /* ── 输出：单一订阅，按 terminalId 分发 ── */
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen<TerminalDataPayload>(api.CHANNEL_TERMINAL_DATA, (payload) => {
        // **「启动提示词」在这一刻才发出去。**
        //
        // 为什么不能更早：这个项目的终端是 ConPTY，**必须等 CLI 回完 DSR
        // （`\x1b[6n` → `\x1b[1;1R`）之后会话才开始推进**。在 `terminal:start`
        // 刚返回时就写，会赶在 CLI 准备好读输入之前——轻则提示词被吞掉，
        // 重则出现在 shell 提示符里变成乱码。
        //
        // 「收到第一段输出」是这里拿得到的最可靠的「它活了」信号。
        // 发出一次即从队里删掉，绝不重复发送。
        const pending = pendingPrompts.current.get(payload.terminalId);
        if (pending !== undefined && payload.data.length > 0) {
          pendingPrompts.current.delete(payload.terminalId);
          void api.writeTerminal(payload.terminalId, `${pending}\r`).catch(() => undefined);
        }
        // 接收端还没挂上时由 router 缓冲，`registerSink` 时补发——见它的文档。
        router.push(payload.terminalId, Uint8Array.from(payload.data));
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [router]);

  /* ── 状态：后端已经把检测做完了，前端只负责记住并显示 ──
   *
   * 检测放在后端是有意的：它需要跨数据块的缓冲（提示文本常被切成几块），
   * 在前端做就要把同一套逻辑再写一遍。
   */
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen<TerminalStatusPayload>(api.CHANNEL_TERMINAL_STATUS, (payload) => {
        setAttention((current) => {
          const next = { ...current };
          if (payload.attention) {
            // 盖一个界面侧的时间戳，只用于**排序**（最新的在最上面）。
            next[payload.terminalId] = { ...payload.attention, updatedAt: Date.now() };
            // **顺带发一条系统通知**：标签上的小点只在窗口可见时有用，
            // 而「程序在等你确认」恰恰常常发生在用户切到别的窗口之后。
            void api.notify(
              payload.attention.title,
              payload.attention.detail ?? "切回应用可继续。"
            );
          } else {
            // 状态变回「运行中/已完成」时提醒就不再成立，必须清掉——
            // 否则标签上的小点会一直亮着，用户会以为还有事没处理。
            delete next[payload.terminalId];
          }
          return next;
        });
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /* ── 退出：干净退出要**关掉**这一路的登记项，异常退出才保留 ──
   *
   * # 为什么不能一律保留
   *
   * 原先这里只把 `running` 置 false、什么都留着。后果是：用户敲 `/exit` 退出后，
   * 那一路终端**永远留在列表里**；而选中那条会话时又总会切到「已经挂着这条会话的
   * 那张标签」——于是再次进入会话时，用户看到的是**那一路已经死掉的终端**，
   * 既不能输入也不再产出，表现就是「被锁住」。
   *
   * # 与旧平台一致的语义
   *
   * 旧平台 `handleTerminalExit`：`if (exitCode !== 0) return;` ——**只有干净退出
   * （退出码 0）才收掉标签**；异常退出保留现场，让用户看得到最后几行报错
   * （`embedded-terminal.tsx` 里那句注释：非零退出保留现有终端错误输出）。
   *
   * 界面不能自己「关闭」终端——那要与后端确认。因此这里只标记「这一路已结束」，
   * 由 `app.tsx` 在标签层收掉它，并顺手 `closeTerminal` 把后端登记项也摘掉。 */
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen<{ terminalId: string; exitCode?: number | null }>(
        api.CHANNEL_TERMINAL_EXIT,
        (payload) => {
          const clean = (payload.exitCode ?? 0) === 0;
          setTerminals((current) =>
            current.map((item) =>
              item.id === payload.terminalId
                ? {
                    ...item,
                    running: false,
                    exitedCleanly: clean,
                    /**
                     * **退出码要记下来并显示出来。**
                     *
                     * 旧平台在终端里写了一行「Codex 已退出，退出码 N」，标签上也有状态。
                     * 只记「干净/不干净」是不够的：`1` 与 `137`（被 kill）指向完全不同的
                     * 原因，而用户手上只有这一个数字可查。
                     */
                    exitCode: payload.exitCode ?? 0
                  }
                : item
            )
          );
          setAttention((current) => {
            const next = { ...current };
            delete next[payload.terminalId];
            return next;
          });
        }
      )
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  /* ── 供应商切换：手动、模式变化、**以及自动故障转移** ──
   *
   * 最后一种是这个订阅存在的**主要原因**。自动转移发生在一次请求内部，用户的输入里
   * 没有任何迹象；没有这条事件，状态栏会一直写着已经不再服务的那一家，用户只有等到
   * 账单对不上才会发现。
   */
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen<{
        terminalId?: string;
        routeId?: string;
        vendorId: string;
        reason?: string;
        mode?: string;
      }>(api.CHANNEL_GATEWAY_VENDOR_SWITCHED, (payload) => {
        // `terminalId` 与 `routeId` 是同一个值；两个都给是让前端用哪个都行。
        const id = payload.terminalId || payload.routeId;
        if (!id) return;
        const mode: VendorRouteMode = payload.mode === "locked" ? "locked" : "dynamic";
        const reason = (payload.reason ?? "manual") as VendorSwitch["reason"];
        setTerminals((current) =>
          current.map((item) =>
            item.id === id
              ? {
                  ...item,
                  vendorId: payload.vendorId,
                  vendorMode: mode,
                  // 模式变化不是「换了供应商」，标成「已自动切换」会误导用户。
                  lastSwitch: reason === "mode" ? item.lastSwitch : { vendorId: payload.vendorId, reason }
                }
              : item
          )
        );
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const registerSink = useCallback(
    (id: string, sink: TerminalSink) => {
      // 补发这段等待期里攒下的输出——**顺序由 router 保证**，控制序列错乱会毁掉界面。
      router.register(id, sink);
    },
    [router]
  );

  const unregisterSink = useCallback(
    (id: string) => {
      router.release(id);
    },
    [router]
  );

  /** 生成一个新的终端编号。 */
  const nextId = useCallback(() => {
    seqRef.current += 1;
    return `term-${Date.now().toString(36)}-${seqRef.current}`;
  }, []);

  /**
   * 新开一路终端。
   *
   * `start` 负责实际启动；这里统一处理「先入列表、失败标记、绑定后端选定的供应商」
   * 的公共部分。
   *
   * # 界面上不挑供应商
   *
   * 与老平台一致：启动时不指定 `vendorId`，由后端按候选池选一家并**返回**它，
   * 界面把它绑到这路终端上。原因很实际——界面并不知道哪家的 Key 是有效的、
   * 哪家被禁用了；让它猜只会猜错，然后显示一个根本不存在的绑定。
   */
  const open = useCallback(
    async (options: {
      title: string;
      providerId: string;
      start: (terminalId: string) => Promise<unknown>;
      /** 「启动提示词」：会话可交互之后自动发一次，见 `pendingPrompts`。 */
      prompt?: string;
      /**
       * 这一路是**恢复**哪条会话（新会话为空）。
       *
       * 它决定自管输入框的**初始可见性**：恢复时先藏着（终端正在重放历史，
       * 输入框会挡住输出），等第一段输出再由 [`noteOutput`] 放出来。
       */
      sessionId?: string;
    }): Promise<string> => {
      const { title, providerId, start, prompt, sessionId } = options;
      const id = nextId();
      // 先入列表再启动：启动失败时用户至少能看到这一路的存在与失败原因，
      // 而不是「点了没反应」。
      setTerminals((current) => [...current, { id, title, running: true, providerId }]);
      if (prompt?.trim()) {
        pendingPrompts.current.set(id, prompt.trim());
      }
      try {
        const started = (await start(id)) as
          | { vendorId?: string; vendorMode?: VendorRouteMode }
          | undefined;
        // 绑定后端**实际选定**的供应商。缺席表示直连官方 API，这是合法的。
        if (started?.vendorId) {
          const vendorId = started.vendorId;
          const vendorMode = started.vendorMode;
          setTerminals((current) =>
            current.map((item) => (item.id === id ? { ...item, vendorId, vendorMode } : item))
          );
        }
        /**
         * 输入侧界面的初值。
         *
         * **启动成功之后才写**：启动失败的那一路没有终端可输入，写一份状态只会让
         * 状态栏显示一个不存在的模式。
         */
        setUiStates((current) => ({
          ...current,
          [id]: { inputMode: "composer", composerVisible: initialComposerVisible(sessionId) }
        }));
      } catch (error) {
        // 启动失败就别再把提示词留在队里：这路终端不会有输出了。
        pendingPrompts.current.delete(id);
        setTerminals((current) =>
          current.map((item) => (item.id === id ? { ...item, running: false } : item))
        );
        throw error;
      }
      return id;
    },
    [nextId]
  );

  /**
   * 把一路终端手动切到另一个供应商。
   *
   * **只在后端确认成功之后**才更新界面：切换会因为「供应商被禁用 / 配置不全 /
   * 平台不匹配」而失败，那时界面显示新供应商就是在撒谎。
   *
   * 本地不预先改状态还有一个好处：后端成功时会通过 `gateway:vendor-switched` 把新值
   * 推回来，那条路径同时也是自动转移的路径——只要它写得对，手动与自动就一致。
   */
  const switchVendor = useCallback(async (id: string, vendorId: string) => {
    await api.switchVendorRoute(id, vendorId);
  }, []);

  /** 设置一路终端的路由模式（`locked` = 不自动故障转移）。 */
  const setRouteMode = useCallback(async (id: string, mode: VendorRouteMode) => {
    await api.setVendorRouteMode(id, mode);
  }, []);

  /**
   * 切换一路终端的输入模式。
   *
   * **切到自管输入时顺带把输入框显出来**——否则用户看到的是「切过去了但看不见框」：
   * 输入框藏在恢复会话的初始状态里时，模式切了也没有任何可见变化。
   */
  const setInputMode = useCallback((id: string, mode: TerminalInputMode) => {
    setUiStates((current) => {
      const state = current[id] ?? { inputMode: "composer" as const, composerVisible: false };
      return {
        ...current,
        [id]: {
          ...state,
          inputMode: mode,
          composerVisible: mode === "composer" ? true : state.composerVisible
        }
      };
    });
  }, []);

  /**
   * 收到输出。
   *
   * 恢复历史会话时输入框是藏着的，而**第一段输出是唯一的显示时机**——
   * 旧平台也只在 `onTerminalData` 里调 `revealComposer()`。判据本身在
   * [`shouldRevealComposer`](./composer-text.ts) 里（有测试），这里只负责落状态。
   */
  const noteOutput = useCallback((id: string) => {
    setUiStates((current) => {
      const state = current[id];
      if (!state || state.composerVisible) return current;
      return shouldRevealComposer(state.composerVisible, true)
        ? { ...current, [id]: { ...state, composerVisible: true } }
        : current;
    });
  }, []);

  const openTerminal = useCallback(
    (
      target: AiTarget | null,
      cwd?: string,
      cliArgs?: string,
      prompt?: string,
      sessionId?: string
    ): Promise<string> =>
      open({
        title: target ? `终端 · ${target.label}` : "终端",
        providerId: target?.provider ?? "codex",
        prompt,
        sessionId,
        start: (terminalId) =>
          api.startTerminal({
            terminalId,
            providerId: target?.provider ?? "codex",
            // `targetId` 必须传：后端靠它判断目标是本机还是 WSL，并据此决定命令形态与
            // 网关上游地址。此前不传，WSL 目标会被静默当成本机目标。
            ...(target?.id ? { targetId: target.id } : {}),
            ...(cwd ? { cwd } : {}),
            // **恢复一次历史会话。** 后端把它变成 `--resume <id>`
            // （`aivc-terminal/src/command.rs` 的 `resume_args`）。
            //
            // 旧平台有一句要紧的注释：「Qoder 按项目目录定位 `--resume` 的历史文件，
            // **恢复时必须保留原始工作目录**」——所以恢复时 `cwd` 不能省。
            ...(sessionId ? { sessionId } : {}),
            // 额外的 CLI 参数。旧平台由 `parseCliArgs(params.cliArgs)` 追加到命令行上，
            // 后端这边 (`aivc-ipc/src/terminal.rs`) 已经收 `cliArgs`，这里只是把它递下去。
            ...(cliArgs?.trim() ? { cliArgs: cliArgs.trim() } : {}),
            ...(target?.codexHome ? { codexHome: target.codexHome } : {})
          })
      }),
    [open]
  );

  /** 关闭一路终端：先停 PTY，再移除。 */
  const closeTerminal = useCallback(
    async (id: string) => {
      // 连未发出的缓冲一起丢掉：这路终端已经没了，留着只是占内存。
      router.release(id);
      setTerminals((current) => current.filter((item) => item.id !== id));
      setAttention((current) => {
        const next = { ...current };
        delete next[id];
        return next;
      });
      // 输入侧状态跟着终端一起走：留着的话，同一个编号（不会复用，但列表会变）的
      // 旧模式会在状态栏上显示出来。
      setUiStates((current) => {
        const next = { ...current };
        delete next[id];
        return next;
      });
      try {
        await api.stopTerminal(id);
      } catch {
        // 进程可能已经自己退出了——那不是错误。
      }
    },
    [router]
  );

  /** 用户已经看过这一路的提醒，清掉标记。 */
  const clearAttention = useCallback((id: string) => {
    setAttention((current) => {
      if (!(id in current)) return current;
      const next = { ...current };
      delete next[id];
      return next;
    });
  }, []);

  return {
    terminals,
    attention,
    /** 输入侧的界面状态：状态栏读前台那一路（spec §6）。 */
    uiStates,
    openTerminal,
    closeTerminal,
    clearAttention,
    setInputMode,
    noteOutput,
    registerSink,
    unregisterSink,
    switchVendor,
    setRouteMode
  };
}
