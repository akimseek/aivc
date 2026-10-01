// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  CUSTOM_BOUNDS_MESSAGES,
  customBounds,
  failureSummary,
  formatCost,
  formatDuration,
  formatLogTime,
  formatTokenCount,
  outcomeLabel,
  outcomeTone,
  rangeBounds
} from "./gateway-range";

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

describe("显示格式", () => {
  it("费用保留 4 位小数——一次请求常常只花几厘钱", () => {
    expect(formatCost(0.1234)).toBe("$0.1234");
    expect(formatCost(undefined)).toBe("-");
  });

  it("耗时没数据时给 `-`，不是 `0 ms`", () => {
    // `0 ms` 会被读成「瞬间完成」，那是另一件事。
    expect(formatDuration(undefined)).toBe("-");
    expect(formatDuration(12.6)).toBe("13 ms");
  });

  it("token 过千收成 k，过百万收成 M", () => {
    expect(formatTokenCount(999)).toBe("999");
    expect(formatTokenCount(12_345)).toBe("12.3k");
    expect(formatTokenCount(2_500_000)).toBe("2.50M");
    expect(formatTokenCount(undefined)).toBe("-");
  });

  it("日志时间显示到分钟", () => {
    const shown = formatLogTime("2025-10-08T14:32:59.000Z");
    // 时区不同会得到不同的时刻，但格式必须是 `MM-DD HH:mm`。
    expect(shown).toMatch(/^\d{2}-\d{2} \d{2}:\d{2}$/);
  });

  it("空时间给 `-`", () => {
    expect(formatLogTime(undefined)).toBe("-");
  });
});

describe("failureSummary", () => {
  it("错误代码优先——它最可操作", () => {
    expect(
      failureSummary({ errorCode: "insufficient_quota", upstreamStatus: 429, errorMessage: "很长的 JSON" })
    ).toBe("insufficient_quota");
  });

  it("没有错误代码时退到上游状态码", () => {
    expect(failureSummary({ upstreamStatus: 429, errorMessage: "很长的 JSON" })).toBe("HTTP 429");
  });

  it("什么都没有时说「未记录」，不编一个原因", () => {
    expect(failureSummary({})).toBe("未记录");
  });
});

describe("outcomeLabel", () => {
  it("已知结果给中文", () => {
    expect(outcomeLabel("ok")).toBe("成功");
    expect(outcomeLabel("timeout")).toBe("超时");
    expect(outcomeLabel("client-aborted")).toBe("已取消");
  });

  it("未知结果原样返回，而不是硬套一个「失败」", () => {
    // 后端将来加了新结果时，界面显示原文比显示错误的中文好。
    expect(outcomeLabel("weird")).toBe("weird");
  });
});

describe("outcomeTone", () => {
  it("只有 ok 是成功色，其余全部收进中性色", () => {
    // 三张网关表（请求日志、最近请求、异常诊断）的结果列此前各写各的映射，
    // 异常诊断甚至把 timeout 显示成裸文本。这个函数是那三处的**唯一**判据。
    expect(outcomeTone("ok")).toBe("accent");
    expect(outcomeTone("error")).toBe("neutral");
    expect(outcomeTone("timeout")).toBe("neutral");
    expect(outcomeTone("client-aborted")).toBe("neutral");
    expect(outcomeTone("weird")).toBe("neutral");
  });
});
