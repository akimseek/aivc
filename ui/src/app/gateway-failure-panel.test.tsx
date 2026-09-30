// @vitest-environment jsdom
import { act, fireEvent, render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { GatewayFailurePanel } from "./gateway-failure-panel";

/**
 * 异常诊断页的重查语义（batch3 T1）。
 *
 * # 为什么单钉这一页
 *
 * 这一页的查询输入最多（区间 + 平台 + 结果 + 供应商 + 页码），而修之前 `load` 的
 * 依赖里挂着一个**每次渲染都新建**的区间对象：那种写法配上「跟 `load` 身份重查」
 * 就是死循环。所以这里三条一起钉：改筛选 ⇒ 重查、翻页 ⇒ 重查、重渲染 ⇒ 不重查。
 */

/** 每页条数（面板里的常量，断言里要算页数）。 */
const PAGE_SIZE = 20;

type Call = { command: string; args: Record<string, unknown> | null };

/** 装一个假的 Tauri 运行时；`answer` 按命令名给返回值。 */
function fakeRuntime(answer: (command: string) => unknown) {
  const invocations: Call[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Call["args"] } | undefined;
        const name = payload?.command ?? command;
        const call: Call = { command: name, args: payload?.args ?? null };
        invocations.push(call);
        return answer(name);
      }
    },
    event: { listen: async () => () => undefined }
  };
  return invocations;
}

/** 一条诊断明细；只给渲染用得上的字段。 */
function item(index: number) {
  return {
    vendorId: `vendor-${index}`,
    providerId: "codex",
    outcome: "error",
    retryCount: 0,
    durationMs: 120,
    createdAt: "2026-09-30T10:00:00.000Z"
  };
}

/** 一页诊断结果。 */
function page(total: number, current = 1) {
  return { items: [item(1)], total, page: current, pageSize: PAGE_SIZE };
}

/** 按字段标签找下拉框——面板里有两个 `select`，按顺序取会在改结构时静默取错。 */
function selectByLabel(container: HTMLElement, label: string): HTMLSelectElement {
  const field = [...container.querySelectorAll("label.field")].find(
    (node) => node.querySelector(".field-label")?.textContent === label
  );
  return field!.querySelector("select")!;
}

function buttonNamed(container: HTMLElement, name: string): HTMLButtonElement {
  return [...container.querySelectorAll("button")].find((node) =>
    (node.textContent ?? "").includes(name)
  )!;
}

function setup(total = 0) {
  const invocations = fakeRuntime(() => page(total));
  const view = render(<GatewayFailurePanel providerId="" onError={() => undefined} />);
  return { view, invocations };
}

describe("GatewayFailurePanel：重查语义", () => {
  it("换「结果」筛选 ⇒ 重新请求，且请求带上新筛选", async () => {
    const { view, invocations } = setup();
    await waitFor(() => expect(invocations).toHaveLength(1));

    await act(async () => {
      fireEvent.change(selectByLabel(view.container, "按结果筛选"), {
        target: { value: "timeout" }
      });
    });

    await waitFor(() => expect(invocations).toHaveLength(2));
    expect(invocations[1]!.args).toMatchObject({ outcome: "timeout", page: 1 });
  });

  it("翻页 ⇒ 重新请求那一页（不是只在第一页上打转）", async () => {
    // 两页才出现分页按钮（`pageCount > 1`）。
    const { view, invocations } = setup(PAGE_SIZE + 1);
    await waitFor(() => expect(invocations).toHaveLength(1));

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });

    await waitFor(() => expect(invocations).toHaveLength(2));
    expect(invocations[1]!.args).toMatchObject({ page: 2 });
  });

  it("重渲染（筛选、页码都没变）⇒ 一次都不重查", async () => {
    const { view, invocations } = setup();
    await waitFor(() => expect(invocations).toHaveLength(1));

    // 新建的回调逼面板重渲染：`load` 的身份若不稳，这里会退化成死循环。
    view.rerender(<GatewayFailurePanel providerId="" onError={() => undefined} />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(invocations).toHaveLength(1);
  });
});
