// @vitest-environment node
import { describe, expect, it } from "vitest";

import {
  failureSummary,
  formatCost,
  formatDuration,
  formatLogTime,
  formatTokenCount,
  outcomeLabel,
  outcomeTone
} from "./gateway-format";

/**
 * 网关的显示层折叠。
 *
 * 这些函数的错**看起来像没数据**：`-` 与 `0 ms` 摆在一起时，用户分不出
 * 「没读到」和「真的很小」。所以每条占位与收尾边界都要钉住。
 */
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
