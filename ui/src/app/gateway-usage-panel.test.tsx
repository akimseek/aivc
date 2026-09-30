// @vitest-environment jsdom
import { act, render, waitFor, type RenderResult } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { GatewayUsagePanel } from "./gateway-usage-panel";

/**
 * 用量页的三张汇总卡（请求数 / 累计 Tokens / 预估费用）。
 *
 * # 为什么这组断言非有不可
 *
 * 卡片上的数是**这一页的结论**，而汇总字段在真实后端上缺失是常态（不同上游记录的
 * 信息多寡不一样）。缺字段时给 `0` 或 `NaN` 都会被当真：`0` 读作「确实没有」、
 * `NaN` 读作「坏了」。因此这里钉住：**缺数据一律 `-`**。
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
    const { view } = setup({
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
  });

  it("**字段缺失时给 `-`**，不是 `NaN`、也不是假的 `0`", async () => {
    const { view } = setup({
      // 真实后端的「未知」就是字段不出现（不同上游记录的信息多寡不一样）。
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
