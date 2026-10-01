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
 *
 * 限流（自动刷新不高于 1Hz）与「只取首页 50 条」是后加的两条硬约束：
 * 前者防止有流量时 5 次/秒地拉三个接口，后者让每次刷新的 payload 有界——
 * 两条都不会在界面上肉眼可见，只能在这里钉住。
 */

type Handler = (event: { payload: unknown }) => void;

/** 装一个假的运行时，并记录订阅了哪些通道。 */
function fakeRuntime(overrides: {
  health?: unknown[];
  usage?: unknown;
  /**
   * 日志响应。数组按**后端真实契约**返回：不带分页参数是裸数组，带上则是
   * `{ items, ... }` 外壳。函数形式用来控制时序（每次调用的 Promise 由测试兑现），
   * 「旧响应晚到」那条测试靠它制造乱序。
   */
  logs?: unknown[] | (() => Promise<unknown>);
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
        if (name === "gateway:get-usage-summary") return overrides.usage ?? null;
        if (name === "gateway:query-logs") {
          if (typeof overrides.logs === "function") return overrides.logs();
          const items = overrides.logs ?? [];
          // 与后端一致：参数决定返回形状（两个参数总是一起给）。
          const paged = "page" in (payload?.args ?? {});
          return paged ? { items, total: items.length, page: 1, pageSize: 50 } : items;
        }
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
    // 刷新是**合并**的，且窗口就是自动刷新的最小间隔（1s），要等窗口过去。
    await act(async () => {
      vi.advanceTimersByTime(1500);
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
      vi.advanceTimersByTime(1500);
    });

    await waitFor(() => {
      const after = runtime.calls.filter((name) => name === "gateway:get-usage-summary").length;
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
        .filter((item) => item.command === "gateway:get-usage-summary")
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

    const before = runtime.calls.filter((name) => name === "gateway:query-logs").length;
    await act(async () => {
      button!.click();
    });
    // 只给微任务机会，**不推进定时器**：手动刷新不该等窗口。
    for (let index = 0; index < 10; index += 1) {
      await act(async () => {});
    }

    const after = runtime.calls.filter((name) => name === "gateway:query-logs").length;
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

  it("用量请求**带上了区间**——后端缺任一端会直接报错", async () => {
    // 这一条是这次接线里最要紧的：此前调 `getUsageSummary()` 一个参数都不传，
    // 后端 `period()` 立刻报「参数无效」，于是用量那几格**从来没显示过**。
    const runtime = fakeRuntime();
    await renderInsights(runtime);

    const call = runtime.callsWithArgs.find((item) => item.command === "gateway:get-usage-summary");
    expect(call).toBeTruthy();
    const args = call!.args as { periodStart?: string; periodEnd?: string };
    expect(typeof args.periodStart).toBe("string");
    expect(typeof args.periodEnd).toBe("string");
    expect(args.periodStart).not.toBe("");
  });

  it("最近请求只取首页，且 pageSize 固定为 50", async () => {
    // 「最近请求」是**概览**不是审计：固定首页 50 条（后端仓储的钳制上限），
    // 一次请求画完整张表，不需要翻页控件；审计要看全量时去日志页（那里本就有分页）。
    // 参数也决定返回形状：不带宽度的裸数组是全量，payload 没有上界。
    const runtime = fakeRuntime();
    await renderInsights(runtime);

    const call = runtime.callsWithArgs.find((item) => item.command === "gateway:query-logs");
    expect(call).toBeTruthy();
    const args = call!.args as Record<string, unknown>;
    expect(args.page).toBe(1);
    expect(args.pageSize).toBe(50);
    expect(typeof args.periodStart).toBe("string");
    expect(typeof args.periodEnd).toBe("string");
  });

  it("旧响应晚到不覆盖新结果：刷新按请求序号丢弃过期响应", async () => {
    // 连点或换区间时，先发的请求可能后回。没有序号判定，旧区间的响应会把
    // 新结果整表盖掉——界面显示的是用户已经离开的那段数据。
    const deferred: Array<(payload: unknown) => void> = [];
    const runtime = fakeRuntime({
      logs: () => new Promise((resolve) => deferred.push(resolve))
    });
    const onError = vi.fn();
    const view = render(<GatewayInsights onError={onError} />);

    // 初始那次取数挂着不兑现，先等它已经发出。
    await waitFor(() => {
      expect(runtime.calls).toContain("gateway:query-logs");
    });

    // 换一个区间：这是**新**的一次取数（第二次）。
    const all = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === "全部时间"
    );
    expect(all).toBeTruthy();
    await act(async () => {
      all!.click();
    });
    await waitFor(() => {
      expect(deferred).toHaveLength(2);
    });

    const page = (code: string) => ({
      items: [
        {
          requestId: code,
          createdAt: new Date().toISOString(),
          vendorId: "v1",
          providerId: "codex",
          outcome: "ok",
          errorCode: code
        }
      ],
      total: 1,
      page: 1,
      pageSize: 50
    });

    // 新请求先回：界面显示新结果。
    await act(async () => {
      deferred[1](page("code-new"));
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("code-new");
    });

    // 旧请求后回：必须被丢弃，不能把新结果盖掉。
    await act(async () => {
      deferred[0](page("code-old"));
    });
    expect(view.container.textContent).toContain("code-new");
    expect(view.container.textContent).not.toContain("code-old");
  });

  it("换时间区间会重新取数，且新区间与旧区间不同", async () => {
    // 「今天」与「全部时间」的起点差了几十年，这一点必须真的传到后端。
    const runtime = fakeRuntime();
    const { view } = await renderInsights(runtime);

    const all = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === "全部时间"
    );
    expect(all).toBeTruthy();

    await act(async () => {
      all!.click();
    });

    await waitFor(() => {
      const starts = runtime.callsWithArgs
        .filter((item) => item.command === "gateway:get-usage-summary")
        .map((item) => (item.args as { periodStart: string }).periodStart);
      // 至少两次，且起点里出现了纪元起点。
      expect(starts.length).toBeGreaterThan(1);
      expect(starts.some((value) => new Date(value).getTime() === 0)).toBe(true);
    });
  });

  it("失败诊断里那列错误代码必须存在", async () => {
    // 「路由不存在 / 鉴权失败」记的是 `ok`，与真正成功只差这一个字段。
    // 界面不显示它，查「CLI 为什么失败」的人只能看到一串成功。
    const runtime = fakeRuntime({
      logs: [
        {
          requestId: "r1",
          createdAt: new Date().toISOString(),
          vendorId: "v1",
          providerId: "codex",
          outcome: "ok",
          errorCode: "route-missing"
        }
      ]
    });
    const { view } = await renderInsights(runtime);
    await waitFor(() => {
      expect(view.container.textContent).toContain("route-missing");
    });
  });
});
