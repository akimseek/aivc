import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { createRefreshCoalescer } from "./refresh-coalescer";

/**
 * 合并刷新的测试。
 *
 * 这段逻辑的价值全在**边界**上：窗口内要不要重复执行、窗口过去之后还灵不灵、
 * 卸载之后会不会还刷一次。三条都不是看一眼界面能确认的。
 */
describe("createRefreshCoalescer", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("窗口内的多次请求只会真正刷新一次", () => {
    // 一次终端重试能推好几条事件，而每次刷新要拉三个接口。
    const run = vi.fn();
    const coalescer = createRefreshCoalescer(100, run);

    coalescer.schedule();
    coalescer.schedule();
    coalescer.schedule();
    vi.advanceTimersByTime(100);

    expect(run).toHaveBeenCalledTimes(1);
  });

  it("窗口过去之后还能再刷新", () => {
    // 合并不能变成「只刷一次」。
    const run = vi.fn();
    const coalescer = createRefreshCoalescer(100, run);

    coalescer.schedule();
    vi.advanceTimersByTime(100);
    coalescer.schedule();
    vi.advanceTimersByTime(100);

    expect(run).toHaveBeenCalledTimes(2);
  });

  it("连续的请求既不会无限推迟，也不会每条都刷", () => {
    // 这一条是**最容易写错**的地方，两个方向都能错：
    //
    // - 如果把重复请求「重新计时」，持续不断的流量会让刷新永远排不上——面板反而不更新；
    // - 如果不合并，每条事件都要拉三个接口。
    //
    // 20 条事件横跨 200ms、窗口 100ms，因此**正好刷 2 次**：每个窗口一次。
    const run = vi.fn();
    const coalescer = createRefreshCoalescer(100, run);

    for (let index = 0; index < 20; index += 1) {
      coalescer.schedule();
      vi.advanceTimersByTime(10);
    }
    vi.advanceTimersByTime(100);

    expect(run).toHaveBeenCalledTimes(2);
    // 没有残留的定时器：该刷的都刷完了。
    expect(vi.getTimerCount()).toBe(0);
  });

  it("取消之后不再刷新", () => {
    // 组件卸载后还去拉接口，就是往已经不存在的组件上写状态。
    const run = vi.fn();
    const coalescer = createRefreshCoalescer(100, run);

    coalescer.schedule();
    coalescer.cancel();
    vi.advanceTimersByTime(500);

    expect(run).not.toHaveBeenCalled();
  });

  it("取消之后再调度仍然有效", () => {
    // 取消只是取消「那一次」，不是把这个合并器废掉。
    const run = vi.fn();
    const coalescer = createRefreshCoalescer(100, run);

    coalescer.schedule();
    coalescer.cancel();
    coalescer.schedule();
    vi.advanceTimersByTime(100);

    expect(run).toHaveBeenCalledTimes(1);
  });
});
