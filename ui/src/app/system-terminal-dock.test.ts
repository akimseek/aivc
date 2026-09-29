// @vitest-environment jsdom
import { describe, expect, it } from "vitest";

import { clampHeight } from "./system-terminal-dock";

/**
 * 系统终端停靠面板的高度。
 *
 * # 为什么单测的是 `clampHeight` 而不是整个面板
 *
 * 面板本身要挂 xterm 与 PTY，单测跑不起来（那是探针的活）。而**高度夹取**是纯计算，
 * 也是这里唯一会算错的地方：
 *
 * - 不夹下界，用户把面板拖到 0 高，标签条都点不到了，且**没有任何办法拖回来**——
 *   拖拽手柄本身就在那条被拖没的边上；
 * - 不夹上界，面板能把工作区整个吃掉；
 * - 窗口很矮时上界会小于下界，夹取顺序写反就会返回一个比下界还小的值。
 */
describe("clampHeight", () => {
  const withViewport = (height: number, run: () => void) => {
    const original = window.innerHeight;
    Object.defineProperty(window, "innerHeight", { value: height, configurable: true });
    try {
      run();
    } finally {
      Object.defineProperty(window, "innerHeight", { value: original, configurable: true });
    }
  };

  it("正常值原样返回", () => {
    withViewport(1000, () => {
      expect(clampHeight(260)).toBe(260);
    });
  });

  it("**下界**：拖到 0 也会留出可用高度", () => {
    // 这一条挡住的是「拖没了就再也拖不回来」。
    withViewport(1000, () => {
      expect(clampHeight(0)).toBe(120);
      expect(clampHeight(-500)).toBe(120);
    });
  });

  it("**上界**：最多占视口 72%，不会把工作区吃掉", () => {
    withViewport(1000, () => {
      expect(clampHeight(5000)).toBe(720);
    });
  });

  it("窗口很矮时**不会返回一个比下界还小的值**", () => {
    // 视口 100px 时 72% 是 72，比下界 120 还小。先取上界、再夹下界就会得到 72，
    // 面板比最小可用高度还矮——夹取顺序写反就会这样。
    withViewport(100, () => {
      expect(clampHeight(50)).toBe(120);
      expect(clampHeight(9999)).toBe(120);
    });
  });

  it("小数会被取整——像素值不该出现半格", () => {
    withViewport(1000, () => {
      expect(clampHeight(260.7)).toBe(261);
      expect(Number.isInteger(clampHeight(300.4))).toBe(true);
    });
  });

  it("NaN 不会让它返回 NaN（会让内联样式失效）", () => {
    withViewport(1000, () => {
      // 读 localStorage 失败或值被改坏时会拿到 NaN。
      // `Math.min/max` 会把这个 NaN 一路带出去，内联样式 `height: NaNpx` 无效，
      // 面板就没有高度——表现是「面板不见了」，而不是「高度不对」。
      //
      // 兜底值是默认高度；这里断言的是**契约**（有限、在范围内），
      // 而不是某个具体数字——具体数字改了不该让这条测试失败。
      const value = clampHeight(Number.NaN);
      expect(Number.isFinite(value)).toBe(true);
      expect(value).toBeGreaterThanOrEqual(120);
      expect(value).toBeLessThanOrEqual(720);
    });
  });
});
