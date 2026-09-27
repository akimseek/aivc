import { useCallback, useEffect, useRef, useState } from "react";

import * as api from "../ipc/commands";
import type { AiTarget, TerminalAttention, TerminalDataPayload, TerminalStatusPayload } from "../types";
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
 * # 系统终端与 Codex 终端
 *
 * 两者只有「起什么命令」不同，标签、输出、尺寸、关闭流程完全一样，因此共用一套状态。
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

/** 一路终端。 */
export type TerminalSession = {
  /** `terminalId`，同时用作标签的 key。 */
  id: string;
  title: string;
  /** PTY 是否还活着。收到退出事件后置 false。 */
  running: boolean;
  /** Codex 终端还是系统终端（只影响展示）。 */
  kind: "codex" | "system";
  /**
   * 这路终端属于哪个平台。
   *
   * 界面要用它去取该平台的供应商列表（状态栏的选择器）。系统终端没有平台概念，
   * 因此是空串。
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
};

export function useTerminals() {
  const [terminals, setTerminals] = useState<TerminalSession[]>([]);
  /** 各终端待处理的提醒（等待确认等）。用户在界面上看过就该清掉。 */
  const [attention, setAttention] = useState<Record<string, TerminalAttention>>({});
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
            next[payload.terminalId] = payload.attention;
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

  /* ── 退出：标记为已退出，不自动关闭标签 ──
   *
   * 自动关闭会让用户来不及看到最后几行输出——而最后几行往往正是失败原因。 */
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen<{ terminalId: string }>(api.CHANNEL_TERMINAL_EXIT, (payload) => {
        setTerminals((current) =>
          current.map((item) =>
            item.id === payload.terminalId ? { ...item, running: false } : item
          )
        );
        setAttention((current) => {
          const next = { ...current };
          delete next[payload.terminalId];
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
   * 的公共部分——两条路径（Codex / 系统）的差别只有那一行调用。
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
      kind: TerminalSession["kind"];
      providerId: string;
      start: (terminalId: string) => Promise<unknown>;
    }): Promise<string> => {
      const { title, kind, providerId, start } = options;
      const id = nextId();
      // 先入列表再启动：启动失败时用户至少能看到这一路的存在与失败原因，
      // 而不是「点了没反应」。
      setTerminals((current) => [...current, { id, title, running: true, kind, providerId }]);
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
      } catch (error) {
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

  const openTerminal = useCallback(
    (target: AiTarget | null, cwd?: string): Promise<string> =>
      open({
        title: target ? `终端 · ${target.label}` : "终端",
        kind: "codex",
        providerId: target?.provider ?? "codex",
        start: (terminalId) =>
          api.startTerminal({
            terminalId,
            providerId: target?.provider ?? "codex",
            // `targetId` 必须传：后端靠它判断目标是本机还是 WSL，并据此决定命令形态与
            // 网关上游地址。此前不传，WSL 目标会被静默当成本机目标。
            ...(target?.id ? { targetId: target.id } : {}),
            ...(cwd ? { cwd } : {}),
            ...(target?.codexHome ? { codexHome: target.codexHome } : {})
          })
      }),
    [open]
  );

  /**
   * 新开一路系统终端。
   *
   * `kind` 省略时由后端按平台决定；若 `target` 是 WSL 目标，后端会开**那个发行版的**
   * shell——在本机开一个 PowerShell 是答非所问。
   */
  const openSystemTerminal = useCallback(
    (target: AiTarget | null, cwd?: string, kind?: string): Promise<string> =>
      open({
        title: "系统终端",
        kind: "system",
        // 系统终端不注入任何环境、也不绑路由，因此没有供应商可言。
        providerId: "",
        start: (terminalId) =>
          api.startSystemTerminal({
            terminalId,
            ...(kind ? { kind } : {}),
            ...(target ? { targetId: target.id } : {}),
            ...(cwd ? { cwd } : {})
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
    openTerminal,
    openSystemTerminal,
    closeTerminal,
    clearAttention,
    registerSink,
    unregisterSink,
    switchVendor,
    setRouteMode
  };
}
