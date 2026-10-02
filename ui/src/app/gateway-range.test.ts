// @vitest-environment node
import { describe, expect, it } from "vitest";

import { CUSTOM_BOUNDS_MESSAGES, customBounds, rangeBounds } from "./gateway-range";

/**
 * 时间区间。
 *
 * 这些函数的错**看起来完全正常**：区间算错一天，界面上的数字只是小了一点，
 * 没有任何异常。所以每一条边界都要钉住。
 */

/** 构造一个固定的「现在」，避免测试依赖运行时刻。 */
const at = (text: string) => new Date(text);

describe("rangeBounds", () => {
  it("今天：起点是**本地零点**，不是 UTC 零点", () => {
    // 东八区用户在北京时间早上 8 点之前，UTC 零点算出来的「今天」其实是昨天。
    const now = at("2025-03-05T02:00:00.000Z");
    const { start, end } = rangeBounds("today", now);
    const localZero = new Date(now.getFullYear(), now.getMonth(), now.getDate());
    expect(new Date(start).getTime()).toBe(localZero.getTime());
    expect(end).toBe(now.toISOString());
  });

  it("近 7 天与近 30 天按毫秒差算，且终点是当下", () => {
    const now = at("2025-03-05T12:00:00.000Z");
    const seven = rangeBounds("7d", now);
    expect(new Date(seven.end).getTime() - new Date(seven.start).getTime()).toBe(
      7 * 24 * 60 * 60 * 1000
    );
    const thirty = rangeBounds("30d", now);
    expect(new Date(thirty.end).getTime() - new Date(thirty.start).getTime()).toBe(
      30 * 24 * 60 * 60 * 1000
    );
  });

  it("全部时间给的是**具体**区间，不是空值", () => {
    // 后端 `period()` 缺任一端就直接报错，所以「全部」也必须算出两个时间戳。
    const now = at("2025-03-05T12:00:00.000Z");
    const { start, end } = rangeBounds("all", now);
    expect(new Date(start).getTime()).toBe(0);
    expect(end).toBe(now.toISOString());
  });
});

describe("customBounds", () => {
  it("结束日期包含**一整天**", () => {
    // 用户选「到 10 月 8 日」的意思是包含 8 号那天。当成 00:00 会把最后一天整个漏掉。
    const bounds = customBounds("2025-10-01", "2025-10-08");
    expect(bounds.ok).toBe(true);
    if (!bounds.ok) return;
    const end = new Date(bounds.end);
    expect(end.getDate()).toBe(8);
    expect(end.getHours()).toBe(23);
    expect(end.getMinutes()).toBe(59);
  });

  it("缺任一端 → `incomplete`（调用方**不发请求**，并提示「请选择完整的起止日期」）", () => {
    // 拿一个自己编的区间去查，会让用户以为「自定义」没生效。
    for (const [start, end] of [
      ["", "2025-10-08"],
      ["2025-10-01", ""],
      ["", ""]
    ]) {
      const result = customBounds(start, end);
      expect(result.ok, `${start}|${end}`).toBe(false);
      expect(result.ok === false && result.reason).toBe("incomplete");
    }
  });

  it("**起止颠倒 → `reversed`，不再静默交换**", () => {
    // 原来这里是交换两端并照常查询。改成报错的理由：
    // 静默重排用户输入会让他从此不知道自己填的到底被怎么理解了；
    // 而旧平台本来就是报错（`起始日期不能晚于结束日期。`）。
    const result = customBounds("2025-10-08", "2025-10-01");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.reason).toBe("reversed");
  });

  it("同一天是合法的（不是颠倒）", () => {
    const result = customBounds("2025-10-08", "2025-10-08");
    expect(result.ok).toBe(true);
  });

  it("无法解析的日期按「没填全」处理（提示用户补日期，而不是一个内部错）", () => {
    const result = customBounds("不是日期", "2025-10-08");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.reason).toBe("incomplete");
  });

  it("两条提示语与旧平台逐字一致", () => {
    expect(CUSTOM_BOUNDS_MESSAGES.incomplete).toBe("请选择完整的起止日期。");
    expect(CUSTOM_BOUNDS_MESSAGES.reversed).toBe("起始日期不能晚于结束日期。");
  });
});
