// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GatewayInsights } from "./gateway-insights";

/** 健康表分页、实时刷新合并、重置与卸载接线。 */

type Handler = (event: { payload: unknown }) => void;

/** 装一个假的运行时，并记录订阅了哪些通道。 */
function fakeRuntime(overrides: {
  health?: unknown[];
  vendors?: unknown[];
} = {}) {
  const calls: string[] = [];
  /**
   * 带参数的调用。断言「区间真的传下去了」要靠它；`at` 记下调用时的时钟，
   * 断言自动刷新的**最小间隔**要靠它（配合假定时器，Date 一并前进）。
   */
  const callsWithArgs: { command: string; args: unknown; at: number }[] = [];
  const listeners = new Map<string, Handler>();

  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        const name = payload?.command ?? "";
        calls.push(name);
        callsWithArgs.push({ command: name, args: payload?.args ?? {}, at: Date.now() });
        if (name === "gateway:get-vendor-health") return overrides.health ?? [];
        if (name === "vendor:list") return overrides.vendors ?? [];
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
    callsWithArgs,
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
  cleanup();
  vi.useRealTimers();
});

async function renderInsights(runtime: ReturnType<typeof fakeRuntime>) {
  const onError = vi.fn();
  const view = render(<GatewayInsights onError={onError} />);
  await waitFor(() => {
    expect(runtime.calls).toContain("gateway:get-vendor-health");
  });
  return { view, onError };
}

describe("GatewayInsights", () => {
  it("健康按供应商编号关联名称，缺失的供应商保留编号供追溯", async () => {
    const runtime = fakeRuntime({
      health: [...HEALTH, { ...HEALTH[0], vendorId: "vendor-deleted" }],
      vendors: [{ id: "vendor-a", name: "中转服务", providerId: "codex" }],

    });
    const { view } = await renderInsights(runtime);
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(2));
    const rows = [...view.container.querySelectorAll("tbody tr")];
    expect(rows[0]!.querySelector("td")!.textContent).toBe("中转服务");
    expect(rows[1]!.querySelector("td")!.textContent).toBe("vendor-deleted");
    expect(runtime.calls).not.toContain("gateway:query-logs");
    expect(runtime.calls).not.toContain("gateway:get-usage-summary");
    expect(view.queryByText("最近请求")).toBeNull();
    expect(view.queryByText("用量")).toBeNull();
  });

  it("供应商健康默认每页 10 条，可翻页并切换 10 / 20 / 50 条", async () => {
    const runtime = fakeRuntime({
      health: Array.from({ length: 23 }, (_, index) => ({ ...HEALTH[0], vendorId: `v${index}` })),
      vendors: Array.from({ length: 23 }, (_, index) => ({ id: `v${index}`, name: `供应商 ${index + 1}` }))
    });
    const { view } = await renderInsights(runtime);
    const rows = () => view.container.querySelectorAll("tbody tr");
    await waitFor(() => expect(rows()).toHaveLength(10));
    const pager = within(view.getByRole("navigation", { name: "供应商健康分页" }));
    expect((pager.getByRole("button", { name: "上一页" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(pager.getByRole("button", { name: "下一页" }));
    expect(rows()[0]!.querySelector("td")!.textContent).toBe("供应商 11");
    fireEvent.click(pager.getByRole("button", { name: "最后一页" }));
    expect(rows()).toHaveLength(3);
    expect((pager.getByRole("button", { name: "下一页" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(pager.getByRole("button", { name: "上一页" }));
    expect(rows()[0]!.querySelector("td")!.textContent).toBe("供应商 11");
    fireEvent.change(pager.getByRole("combobox", { name: "每页数量" }), { target: { value: "20" } });
    expect(rows()).toHaveLength(20);
    expect(view.container.textContent).toContain("第 1 / 2 页");
    fireEvent.change(pager.getByRole("combobox", { name: "每页数量" }), { target: { value: "50" } });
    expect(rows()).toHaveLength(23);
    expect(view.container.textContent).toContain("第 1 / 1 页");
  });

  it("刷新后健康记录变少时页码不会越界", async () => {
    const overrides = {
      health: Array.from({ length: 11 }, (_, index) => ({ ...HEALTH[0], vendorId: `v${index}` }))
    };
    const runtime = fakeRuntime(overrides);
    const { view } = await renderInsights(runtime);
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(10));
    fireEvent.click(view.getByRole("button", { name: "下一页" }));
    expect(view.container.querySelectorAll("tbody tr")).toHaveLength(1);
    overrides.health = overrides.health.slice(0, 3);
    fireEvent.click(view.getByRole("button", { name: "重置全部健康" }));
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(3));
    expect(view.container.textContent).toContain("第 1 / 1 页");
    overrides.health = [];
    fireEvent.click(view.getByRole("button", { name: "重置全部健康" }));
    await waitFor(() => expect(view.queryByRole("navigation", { name: "供应商健康分页" })).toBeNull());
  });

  it("订阅了「请求已记录」事件", async () => {
    // 后端每次都推；界面不订阅，就不用谈「实时」。
    const runtime = fakeRuntime();
    await renderInsights(runtime);
    expect(runtime.channels()).toContain("gateway:request-recorded");
  });

  it("收到事件之后会重新取数", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    await renderInsights(runtime);

    const before = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
    await runtime.emit("gateway:request-recorded");
    // 刷新是**合并**的，且窗口就是自动刷新的最小间隔（1s），要等窗口过去。
    await act(async () => {
      vi.advanceTimersByTime(1500);
    });

    await waitFor(() => {
      const after = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
      expect(after).toBeGreaterThan(before);
    });
  });

  it("一串事件只触发一次刷新", async () => {
    // 一次终端重试能推好几条，而刷新一次要拉三个接口。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    await renderInsights(runtime);

    const before = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
    for (let index = 0; index < 5; index += 1) {
      await runtime.emit("gateway:request-recorded");
    }
    await act(async () => {
      vi.advanceTimersByTime(1500);
    });

    await waitFor(() => {
      const after = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
      expect(after - before).toBe(1);
    });
  });

  it("自动刷新不高于 1Hz：连续流量下相邻两次取数至少隔 1 秒", async () => {
    // 钉住**频率上限**：旧实现每次事件过 200ms 就刷一次（有流量时 5 次/秒 ×
    // 三个接口），本断言在旧实现下会红。上限之外还要保证不饿死：
    // 持续流量下面板仍要按秒级更新，只是不再对每条事件都响应。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    await renderInsights(runtime);

    const usageTimes = () =>
      runtime.callsWithArgs
        .filter((item) => item.command === "gateway:get-vendor-health")
        .map((item) => item.at);

    // 2.5 秒的连续流量：每 100ms 推一条事件（远密于窗口）。
    for (let index = 0; index < 25; index += 1) {
      await runtime.emit("gateway:request-recorded");
      await act(async () => {
        vi.advanceTimersByTime(100);
      });
    }
    // 末次事件排的刷新还欠一个窗口，等它落地。
    await act(async () => {
      vi.advanceTimersByTime(1000);
    });

    const times = usageTimes();
    // 不饿死：持续流量下刷了不止一次……
    expect(times.length).toBeGreaterThanOrEqual(3);
    // ……也不超频：相邻两次取数至少隔一个窗口。
    for (let index = 1; index < times.length; index += 1) {
      expect(times[index] - times[index - 1]).toBeGreaterThanOrEqual(1000);
    }
  });

  it("手动刷新不受 1s 窗口限制：点一下立刻取数", async () => {
    // 限流只针对**事件驱动**的自动刷新。人手动点「刷新」时界面必须立刻响应，
    // 等一个窗口才取数是不能接受的；这条测试在「把按钮也接进合并器」时会红。
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const runtime = fakeRuntime();
    const { view } = await renderInsights(runtime);

    const button = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === "刷新"
    );
    expect(button).toBeTruthy();

    const before = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
    await act(async () => {
      button!.click();
    });
    // 只给微任务机会，**不推进定时器**：手动刷新不该等窗口。
    for (let index = 0; index < 10; index += 1) {
      await act(async () => {});
    }

    const after = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
    expect(after).toBe(before + 1);
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
    const usageCalls = runtime.calls.filter((name) => name === "gateway:get-vendor-health").length;
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
