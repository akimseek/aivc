// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { GatewayRulesCard } from "./gateway-rules-card";
import type { AiProviderSummary, FailoverRule } from "../../types";

/**
 * 规则卡里两件「说出去的话必须与做出去的事一致」的事：
 *
 * 1. **试规则的状态码**：留空 = 不按状态码判定。`Number("")` 与 `Number("0")`
 *    都是 0，前者该省略该字段、后者（用户明确输入的 0）同样不能发——0 不是
 *    合法的 HTTP 状态码，后端会拿它去比规则里的状态码列表。
 * 2. **作用域列的平台名**：必须用后端 `providers` 的 label（侧栏、标签栏都是它），
 *    否则同一个平台在两张表上叫两个名字。
 */

function fakeRuntime(rules: FailoverRule[]) {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        if (name === "gateway:list-failover-rules") return rules;
        if (name === "vendor:list") return [];
        if (name === "gateway:test-failover-rule") return { matched: false, matchedBy: "none" };
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return calls;
}

function findButton(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) => (item.textContent ?? "").trim() === text
  );
}

/** 按 placeholder 找输入框并写入——受控输入必须走原生 setter，否则 state 收不到。 */
async function typeInto(container: HTMLElement, placeholder: string, value: string) {
  const input = [...container.querySelectorAll<HTMLInputElement>("input")].find((item) =>
    (item.placeholder ?? "").includes(placeholder)
  )!;
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const testCalls = (calls: Array<{ command: string; args: Record<string, unknown> }>) =>
  calls.filter((item) => item.command === "gateway:test-failover-rule");

afterEach(cleanup);

describe("GatewayRulesCard：试规则的状态码", () => {
  it("**填「0」按未填处理**，不把 `statusCode: 0` 发给后端", async () => {
    const calls = fakeRuntime([]);
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await act(async () => {
      findButton(view.container, "新增规则")!.click();
    });
    await typeInto(view.container, "例如 insufficient_quota", "insufficient_quota");
    await typeInto(view.container, "例如 429", "0");
    await act(async () => {
      findButton(view.container, "试一下")!.click();
    });

    await waitFor(() => expect(testCalls(calls)).toHaveLength(1));
    // 0 与空串同档：后端收到它只会拿 0 去比规则里的状态码，永远比不上。
    expect(testCalls(calls)[0]!.args).not.toHaveProperty("statusCode");
  });

  it("正常状态码照发（说明上面那条不是「一律不发」）", async () => {
    const calls = fakeRuntime([]);
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await act(async () => {
      findButton(view.container, "新增规则")!.click();
    });
    await typeInto(view.container, "例如 insufficient_quota", "insufficient_quota");
    await typeInto(view.container, "例如 429", "503");
    await act(async () => {
      findButton(view.container, "试一下")!.click();
    });

    await waitFor(() => expect(testCalls(calls)).toHaveLength(1));
    expect(testCalls(calls)[0]!.args.statusCode).toBe(503);
  });
});

describe("GatewayRulesCard：作用域列的平台名", () => {
  const RULE: FailoverRule = {
    id: "r1",
    scope: "provider",
    providerId: "claude",
    pattern: "insufficient_quota",
    enabled: true,
    createdAt: "2026-09-30T00:00:00Z",
    updatedAt: "2026-09-30T00:00:00Z"
  };
  const PROVIDERS: AiProviderSummary[] = [
    { id: "claude", label: "Claude Code", capabilities: {} }
  ];

  it("用后端 `providers` 的 label（本地那份表只兜底）", async () => {
    fakeRuntime([RULE]);
    const view = render(<GatewayRulesCard providers={PROVIDERS} onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelector("tbody tr")).toBeTruthy());
    // 整格相等：只断言「包含 Claude Code」的话，本地表那份「Claude」也能被包含进去。
    expect(view.container.querySelector("tbody tr td")!.textContent).toBe("按平台 · Claude Code");
  });

  it("拿不到平台列表时退回本地表（`Claude`），而不是留空", async () => {
    fakeRuntime([RULE]);
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelector("tbody tr")).toBeTruthy());
    expect(view.container.querySelector("tbody tr td")!.textContent).toBe("按平台 · Claude");
  });

  it("表单里「目标平台」的选项同样用后端 label", async () => {
    fakeRuntime([]);
    const view = render(<GatewayRulesCard providers={PROVIDERS} onError={vi.fn()} />);
    await act(async () => {
      findButton(view.container, "新增规则")!.click();
    });
    const scope = view.container.querySelector<HTMLSelectElement>('select[aria-label="作用域"]')!;
    await act(async () => {
      fireEvent.change(scope, { target: { value: "provider" } });
    });
    const options = [
      ...view.container.querySelectorAll<HTMLOptionElement>('select[aria-label="目标平台"] option')
    ].map((item) => item.textContent);
    expect(options).toContain("Claude Code");
    expect(options).not.toContain("Claude");
  });
});
