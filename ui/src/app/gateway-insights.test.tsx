// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GatewayInsights } from "./gateway-insights";

/**
 * 网关面板的测试。
 *
 * # 为什么这个组件必须有自己的测试
 *
 * 它是界面上**唯一有跨组件接线**的面板：订阅后端事件、合并刷新、调一条命令。
 * 而这三件事此前**全都缺失或全都没被验证过**——
 * 「后端每转发一个请求就推一条事件，面板实时更新」这个承诺，
 * 在界面上从来没有成立过。留在一个 1380 行的大文件里就只能靠肉眼看界面。
 */

type Handler = (event: { payload: unknown }) => void;

/** 装一个假的运行时，并记录订阅了哪些通道。 */
function fakeRuntime(overrides: {
  health?: unknown[];
  usage?: unknown;
  logs?: unknown[];
} = {}) {
  const calls: string[] = [];
  const listeners = new Map<string, Handler>();

  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        const name = payload?.command ?? "";
        calls.push(name);
        if (name === "gateway:get-vendor-health") return overrides.health ?? [];
        if (name === "gateway:get-usage-summary") return overrides.usage ?? null;
        if (name === "gateway:query-logs") return overrides.logs ?? [];
        if (name === "gateway:reset-vendor-health") return 1;
        return {};
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

const HEALTH = [
  {
    vendorId: "vendor-a",
    providerId: "codex",
    status: "open",
    failureCount: 3,
    successCount: 0,
    failureRate: 1,
    lastFailureReason: "连不上"
  }
];

beforeEach(() => {
  vi.restoreAllMocks();
});

afterEach(() => {
  vi.useRealTimers();
});

async function renderInsights(runtime: ReturnType<typeof fakeRuntime>) {
  const onError = vi.fn();
  const view = render(<GatewayInsights onError={onError} />);
  await waitFor(() => {
    expect(runtime.calls).toContain("gateway:get-usage-summary");
  });
  return { view, onError };
}

describe("GatewayInsights", () => {
  it("订阅了「请求已记录」事件", async () => {
    // 后端每次都推；界面不订阅，就不用谈「实时」。
    const runtime = fakeRuntime();
    await renderInsights(runtime);
    expect(runtime.channels()).toContain("gateway:request-recorded");
  });

  it("收到事件之后会重新取数", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime({ usage: { requestCount: 1, failureCount: 0, switchedCount: 0 } });
    await renderInsights(runtime);

    const before = runtime.calls.filter((name) => name === "gateway:get-usage-summary").length;
    await runtime.emit("gateway:request-recorded");
    // 刷新是**合并**的，要等窗口过去。
    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    await waitFor(() => {
      const after = runtime.calls.filter((name) => name === "gateway:get-usage-summary").length;
      expect(after).toBeGreaterThan(before);
    });
  });

  it("一串事件只触发一次刷新", async () => {
    // 一次终端重试能推好几条，而刷新一次要拉三个接口。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    await renderInsights(runtime);

    const before = runtime.calls.filter((name) => name === "gateway:get-usage-summary").length;
    for (let index = 0; index < 5; index += 1) {
      await runtime.emit("gateway:request-recorded");
    }
    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    await waitFor(() => {
      const after = runtime.calls.filter((name) => name === "gateway:get-usage-summary").length;
      expect(after - before).toBe(1);
    });
  });

  it("健康表为空时说明原因，而不是显示一张空表", async () => {
    // 「还没有请求记录」和「所有供应商都健康」是两件事，界面不该让人分不清。
    const runtime = fakeRuntime({ health: [] });
    const { view } = await renderInsights(runtime);
    await waitFor(() => {
      expect(view.container.textContent).toContain("还没有请求记录");
    });
  });

  it("重置按钮走命令，而不是只清本地状态", async () => {
    // 这条命令要同时清数据库与**内存里正在生效的**熔断。
    // 只在界面上把表格清空是假的重置——熔断照样生效。
    const runtime = fakeRuntime({ health: HEALTH });
    const { view } = await renderInsights(runtime);

    const button = await waitFor(() => {
      const found = [...view.container.querySelectorAll("button")].find((item) =>
        item.textContent?.includes("重置")
      );
      if (!found) throw new Error("还没有重置按钮");
      return found;
    });

    await act(async () => {
      button.click();
    });

    await waitFor(() => {
      expect(runtime.calls).toContain("gateway:reset-vendor-health");
    });
    // 重置之后要重新取数，否则界面还显示着旧的熔断。
    const usageCalls = runtime.calls.filter((name) => name === "gateway:get-usage-summary").length;
    expect(usageCalls).toBeGreaterThan(1);
  });

  it("卸载之后解绑，不再订阅后端事件", async () => {
    // 不解绑的话，组件没了还在往它上面写状态——而且每卸载重开一次就多一个订阅，
    // 那些「幽灵订阅」会各自触发一次取数。
    const runtime = fakeRuntime();
    const { view } = await renderInsights(runtime);
    expect(runtime.channels()).toContain("gateway:request-recorded");

    view.unmount();

    await waitFor(() => {
      expect(runtime.channels()).not.toContain("gateway:request-recorded");
    });
  });
});
