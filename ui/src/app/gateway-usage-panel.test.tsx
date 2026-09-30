// @vitest-environment jsdom
import { act, fireEvent, render, waitFor, type RenderResult } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { GatewayUsagePanel } from "./gateway-usage-panel";
import { rangeBounds } from "./gateway-range";

/**
 * 用量页的三张汇总卡（请求数 / 累计 Tokens / 预估费用）。
 *
 * # 为什么这组断言非有不可
 *
 * 卡片上的数是**这一页的结论**，而汇总里**token 与费用**那几项在真实后端上缺失是
 * 常态（不同上游记录的信息多寡不一样；`requestCount` / `successCount` 则必有字段，
 * 但界面按可缺处理）。缺字段时给 `0` 或 `NaN` 都会被当真：`0` 读作「确实没有」、
 * `NaN` 读作「坏了」。因此这里钉住：**缺数据一律 `-`**，成功率的两项缺任何一个
 * 也给 `-`（补成 `0` 会算出 `0.0%`，那读起来是「全都失败了」）。
 *
 * 零请求那份还要单独钉一次：`0/0` 算出来的成功率会被读成「全都失败了」，
 * 它必须是 `-`。
 */

type Call = { command: string; args: { periodStart?: string; periodEnd?: string } | null };

/** 装一个假的 Tauri 运行时；`answer` 按命令名给返回值（抛错即模拟读失败）。 */
function fakeRuntime(answer: (command: string, args: Call["args"]) => unknown) {
  const invocations: Call[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Call["args"] } | undefined;
        const name = payload?.command ?? command;
        const call: Call = { command: name, args: payload?.args ?? null };
        invocations.push(call);
        return answer(name, call.args);
      }
    },
    event: { listen: async () => () => undefined }
  };
  return invocations;
}

/** 一份用量报表；只给本组测试关心的字段，其余留空。 */
function report(summary: Record<string, unknown>) {
  return {
    summary: {
      failureCount: 0,
      switchedCount: 0,
      periodStart: "2026-09-23T00:00:00.000Z",
      periodEnd: "2026-09-30T00:00:00.000Z",
      ...summary
    },
    vendors: [],
    models: []
  };
}

/** 三张卡各自的标签、读数与备注。 */
function cards(view: RenderResult) {
  return [...view.container.querySelectorAll(".stat-card")].map((card) => ({
    label: card.querySelector(".stat-card-label")?.textContent ?? "",
    value: card.querySelector(".stat-card-value")?.textContent ?? "",
    note: card.querySelector(".stat-card-note")?.textContent ?? ""
  }));
}

function setup(answers: Record<string, unknown>) {
  const invocations = fakeRuntime((command) => answers[command] ?? null);
  const view = render(<GatewayUsagePanel onError={() => undefined} />);
  return { view, invocations };
}

describe("GatewayUsagePanel：三张汇总卡", () => {
  it("请求数（含成功率）、累计 Tokens、预估费用各一张", async () => {
    const { view, invocations } = setup({
      "gateway:get-usage-report": report({
        requestCount: 1280,
        successCount: 1216,
        failureCount: 64,
        totalTokens: 2_850_000,
        costUsd: 3.42
      })
    });

    await waitFor(() => expect(cards(view)).toHaveLength(3));
    expect(cards(view)).toEqual([
      { label: "请求数", value: "1280", note: "成功率 95.0%" },
      { label: "累计 Tokens", value: "2.85M", note: "" },
      { label: "预估费用", value: "$3.4200", note: "" }
    ]);

    // 请求参数就是区间边界：后端 `period()` 缺 `periodStart` / `periodEnd` 会直接报错。
    // 默认区间是「近 7 天」（由 `rangeBounds` 算出），因此两端正好差 7 天。
    expect(invocations[0]).toEqual({
      command: "gateway:get-usage-report",
      args: { periodStart: expect.any(String), periodEnd: expect.any(String) }
    });
    expect(
      Date.parse(invocations[0]!.args!.periodEnd!) - Date.parse(invocations[0]!.args!.periodStart!)
    ).toBe(7 * 24 * 60 * 60 * 1000);
  });

  it("**字段缺失时给 `-`**，不是 `NaN`、也不是假的 `0`", async () => {
    const { view } = setup({
      // 「未知」在真实后端上表现为字段不出现——**token 与费用**那几个分项尤其如此
      // （不同上游记录的信息多寡不一样）。
      "gateway:get-usage-report": report({
        requestCount: undefined,
        successCount: undefined,
        totalTokens: undefined,
        costUsd: undefined
      })
    });

    await waitFor(() => expect(cards(view)).toHaveLength(3));
    expect(cards(view).map((item) => item.value)).toEqual(["-", "-", "-"]);
    expect(view.container.textContent).not.toContain("NaN");
    // 「不知道」被画成 0 的话，用户会以为这个区间真的没有用量。
    expect(cards(view).some((item) => item.value === "0")).toBe(false);
    expect(cards(view)[0]!.note).toContain("-");
  });

  it("有请求数但**缺成功数**时成功率是 `-`：补成 0 会算出 `0.0%`（读起来是全都失败了）", async () => {
    const { view } = setup({
      "gateway:get-usage-report": report({
        requestCount: 1280,
        successCount: undefined,
        totalTokens: 0,
        costUsd: 0
      })
    });

    await waitFor(() => expect(cards(view)).toHaveLength(3));
    // 请求数这一格照常显示（它自己有值），只有比值退到 `-`。
    expect(cards(view)[0]).toEqual({ label: "请求数", value: "1280", note: "成功率 -" });
    expect(view.container.textContent).not.toContain("0.0%");
    expect(view.container.textContent).not.toContain("NaN");
  });

  it("零请求时成功率是 `-`（`0/0` 算出来的 `0.0%` 会被读成全都失败了）", async () => {
    const { view } = setup({
      "gateway:get-usage-report": report({
        requestCount: 0,
        successCount: 0,
        totalTokens: 0,
        costUsd: 0
      })
    });

    await waitFor(() => expect(cards(view)).toHaveLength(3));
    expect(cards(view)[0]).toEqual({ label: "请求数", value: "0", note: "成功率 -" });
    expect(view.container.textContent).not.toContain("0.0%");
    expect(view.container.textContent).not.toContain("NaN");
  });

  it("读失败时不摆出三张卡，但「重新查询」仍在——这时候才最需要它", async () => {
    // 第一次失败、之后成功：既钉住失败态，也钉住重试真的会重发。
    let broken = true;
    const invocations = fakeRuntime(() => {
      if (broken) throw new Error("数据库被占用");
      return report({ requestCount: 3, successCount: 3 });
    });
    const view = render(<GatewayUsagePanel onError={() => undefined} />);

    await waitFor(() =>
      expect(view.container.textContent).toContain("用量报表读取失败")
    );
    // 把失败画成三张「-」的卡会被当成「这个区间没有用量」。
    expect(cards(view)).toHaveLength(0);

    const again = [...view.container.querySelectorAll("button")].find((item) =>
      (item.textContent ?? "").includes("重新查询")
    )!;
    expect(again).toBeTruthy();

    broken = false;
    const before = invocations.length;
    await act(async () => {
      again.click();
    });
    await waitFor(() => expect(invocations.length).toBeGreaterThan(before));
    await waitFor(() => expect(cards(view)).toHaveLength(3));
  });

  it("区间内没有记录时只有空态，没有卡", async () => {
    const { view } = setup({ "gateway:get-usage-report": null });

    await waitFor(() =>
      expect(view.container.textContent).toContain("这个区间内没有用量记录。")
    );
    expect(cards(view)).toHaveLength(0);
  });
});

/**
 * 重查语义（batch3 T1）。
 *
 * # 为什么钉在「请求次数」上
 *
 * 批次二终审的发现是：换了区间**不重新取数**，于是批次二新加的汇总卡会把
 * 「旧区间的数字」和「新选中的区间」并排摆着——看起来像后端返回了错的数据。
 * 反过来「每次重渲染都重查」也不能接受：面板常驻，界面上的任何风吹草动都会打后端。
 * 两条都要钉，缺一条修法就会滑向另一边。
 */
describe("GatewayUsagePanel：重查语义", () => {
  it("换区间 ⇒ 用新区间再查一次（旧区间的数字不许留在屏幕上）", async () => {
    const { view, invocations } = setup({
      "gateway:get-usage-report": report({ requestCount: 1, successCount: 1 })
    });
    await waitFor(() => expect(invocations).toHaveLength(1));

    const picker = view.container.querySelector<HTMLSelectElement>(
      'select[aria-label="时间范围"]'
    )!;
    await act(async () => {
      fireEvent.change(picker, { target: { value: "today" } });
    });

    await waitFor(() => expect(invocations).toHaveLength(2));
    // 新请求真的是「今天」：起点是本地零点（`rangeBounds` 的契约）。
    expect(invocations[1]!.args!.periodStart).toBe(rangeBounds("today").start);
  });

  it("重渲染（区间没变）⇒ 一次都不重查", async () => {
    const { view, invocations } = setup({
      "gateway:get-usage-report": report({ requestCount: 1, successCount: 1 })
    });
    await waitFor(() => expect(invocations).toHaveLength(1));

    // 换一个新建的回调逼面板重渲染——区间并没有变，不该有第二次请求。
    // （如果 `load` 的身份不稳定，这条会退化成死循环，而不是悄悄多查一次。）
    view.rerender(<GatewayUsagePanel onError={() => undefined} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(invocations).toHaveLength(1);
  });
});
