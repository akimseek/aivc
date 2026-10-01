// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiProviderSummary, AiTarget, GatewayPortStatus } from "../types";
import { REQUEST_REFRESH_WINDOW_MS } from "./refresh-coalescer";
import { App } from "./app";

/**
 * App 级：工作台的刷新计数器（P0-F）。
 *
 * # 为什么必须挂在 App 上
 *
 * 缺陷在 `app.tsx` 的订阅闭包里：改用合并器之前是 `clearTimeout + setTimeout`
 * 的**纯 trailing 去抖**——每来一条 `gateway:request-recorded` 就把计时器重置一次。
 * 事件间隔小于窗口时（重试风暴 / 流式会话里很常见），计时器**永远等不到**自己
 * 到期：持续流量下工作台的数字停在旧值，而且没有任何报错。
 *
 * `refresh-coalescer` 的单测只能证明合并器本身对，证明不了 App 有没有用它；
 * 这条接线也没有别的渲染入口，所以这里渲染整个 `App`、像后端那样推事件，
 * 断言**工作台真的重新取数了**。
 *
 * # 怎么观察「刷没刷」
 *
 * 工作台拿的是 `gateway:get-usage-summary` 与 `gateway:get-failure-diagnostics`
 * （见 `workbench-sidebar.tsx`）：刷新计数器一变就以新身份重新取数。因此数这两个
 * 命令的调用次数即可，不必去读内部 state。
 *
 * # 判别力（RED→GREEN）
 *
 * 「持续流量」那条横跨 2.5 秒、每 100ms 推一条（远密于 1s 窗口），断言
 * **流量还在进行时**就已经刷过：旧去抖实现下这里是 0（RED）；换成合并器后
 * 每个窗口各刷一次（GREEN）。
 *
 * # 终端与主题的替身
 *
 * 与 `app-resume.test.tsx` 同一套理由：`terminal-pane`（xterm）要真实画布，
 * jsdom 没有；`matchMedia` 同样不存在，补一个。
 */

vi.mock("../features/workspace/terminal-pane", () => ({ TerminalPane: () => null }));

const PROVIDERS: AiProviderSummary[] = [
  {
    id: "codex",
    label: "Codex",
    capabilities: {
      trash: true,
      batchActions: true,
      duplicate: true,
      branch: true,
      skills: true,
      export: true,
      sessionSettings: true,
      customCwd: true,
      vendorManagement: true,
      usage: true
    }
  }
];

const GATEWAY: GatewayPortStatus = {
  enabled: true,
  configuredPort: 0,
  activePort: 50064,
  baseUrl: "http://127.0.0.1:50064"
};

const TARGET: AiTarget = {
  id: "codex:local",
  provider: "codex",
  label: "本机 Codex",
  kind: "local",
  available: true,
  detail: "可读写"
};

type Handler = (event: { payload: unknown }) => void;

/** 假运行时：记录所有命令调用，并按通道留住事件处理器供 `emit` 使用。 */
function fakeRuntime() {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const listeners = new Map<string, Handler>();

  const answer = (command: string): unknown => {
    switch (command) {
      case "app:get-version":
        return "0.0.0-test";
      case "ai:list-providers":
        return PROVIDERS;
      case "gateway:get-port":
        return GATEWAY;
      case "codex:list-session-quick-access":
        return { favorites: [], recent: [] };
      case "codex:list-cached-targets":
      case "codex:list-targets":
        return [TARGET];
      case "codex:list-cached-sessions":
      case "codex:list-sessions":
        return [];
      // 工作台的两条取数命令：刷新计数器一变就会被重新调用，次数就是「刷了几次」。
      case "gateway:get-usage-summary":
        return null;
      case "gateway:get-failure-diagnostics":
        return { items: [], total: 0, page: 1, pageSize: 5 };
      default:
        return null;
    }
  };

  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, raw?: unknown) => {
        const payload = raw as { command?: string; args?: Record<string, unknown> } | undefined;
        const command = payload?.command ?? "";
        calls.push({ command, args: payload?.args ?? {} });
        return answer(command);
      }
    },
    event: {
      listen: async (channel: string, handler: Handler) => {
        listeners.set(channel, handler);
        return () => listeners.delete(channel);
      }
    }
  };

  return {
    calls,
    /** 推一条事件，模拟后端转发了一个请求。 */
    emit: async (channel: string) => {
      const handler = listeners.get(channel);
      if (!handler) throw new Error(`没有人订阅 ${channel}`);
      await act(async () => {
        handler({ payload: {} });
      });
    },
    channels: () => [...listeners.keys()]
  };
}

const usageCount = (runtime: ReturnType<typeof fakeRuntime>) =>
  runtime.calls.filter((call) => call.command === "gateway:get-usage-summary").length;

/** 切到工作台侧栏，并等它完成第一次取数（之后的每一次都是「刷新」）。 */
async function openWorkbench(runtime: ReturnType<typeof fakeRuntime>) {
  fireEvent.click(screen.getByLabelText("工作台与运行概览"));
  await waitFor(() => {
    expect(usageCount(runtime)).toBeGreaterThanOrEqual(1);
  });
}

beforeEach(() => {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
    addListener: () => undefined,
    removeListener: () => undefined,
    dispatchEvent: () => false
  })) as unknown as typeof window.matchMedia;
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("工作台的网关事件刷新（App 级）", () => {
  it("持续流量不会饿死刷新：事件间隔远小于窗口时仍按窗口刷新", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    render(<App />);
    await openWorkbench(runtime);
    const before = usageCount(runtime);

    // 2.5 秒的连续流量：每 100ms 一条（一个终端在重试循环里的样子）。
    for (let index = 0; index < 25; index += 1) {
      await runtime.emit("gateway:request-recorded");
      await act(async () => {
        vi.advanceTimersByTime(100);
      });
      if (index === 14) {
        // 流量**还在进行**时就必须已经刷过——旧去抖实现下这里是 0（RED）。
        expect(usageCount(runtime) - before).toBeGreaterThanOrEqual(1);
      }
    }
    // 末条事件排的那次刷新还欠一个窗口，等它落地。
    await act(async () => {
      vi.advanceTimersByTime(REQUEST_REFRESH_WINDOW_MS);
    });

    const delta = usageCount(runtime) - before;
    // 不饿死：持续流量下按窗口各刷一次……
    expect(delta).toBeGreaterThanOrEqual(2);
    // ……也不超频：25 条事件最多换来 3 次（4 秒跨度 / 1 秒窗口）。
    expect(delta).toBeLessThanOrEqual(3);
    expect(runtime.channels()).toContain("gateway:request-recorded");
  });

  it("窗口内的一串事件只换来一次刷新", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    render(<App />);
    await openWorkbench(runtime);
    const before = usageCount(runtime);

    // 窗口内的重复事件直接丢弃，只执行排在那里的那一次。
    for (let index = 0; index < 5; index += 1) {
      await runtime.emit("gateway:request-recorded");
    }
    await act(async () => {
      vi.advanceTimersByTime(REQUEST_REFRESH_WINDOW_MS);
    });

    expect(usageCount(runtime) - before).toBe(1);
  });

  it("卸载之后不再刷新：排好的那一次也一并取消", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    const view = render(<App />);
    await openWorkbench(runtime);
    const before = usageCount(runtime);

    await runtime.emit("gateway:request-recorded");
    view.unmount();
    await act(async () => {
      vi.advanceTimersByTime(REQUEST_REFRESH_WINDOW_MS * 2);
    });

    // 卸载后还去取数，就是往已经不存在的组件上写状态。
    expect(usageCount(runtime)).toBe(before);
    expect(runtime.channels()).not.toContain("gateway:request-recorded");
  });
});
