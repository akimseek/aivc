// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { GatewayExternalCard } from "./gateway-external-card";
import { GatewayRulesCard } from "./gateway-rules-card";
import type { AiProviderSummary, FailoverRule } from "../../../types";

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
  /** 后端 `ai:list-providers` 的形制（顺序即它给的顺序）。 */
  const PROVIDERS: AiProviderSummary[] = [
    { id: "codex", label: "Codex", capabilities: {} },
    { id: "gemini", label: "Gemini", capabilities: {} },
    { id: "claude", label: "Claude Code", capabilities: {} },
    { id: "qoder", label: "Qoder", capabilities: {} }
  ];
  const THREE_PROVIDERS = PROVIDERS.filter((item) => item.id !== "gemini");

  it("用后端 `providers` 的 label（本地那份表只兜底）", async () => {
    fakeRuntime([RULE]);
    const view = render(<GatewayRulesCard providers={PROVIDERS} onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelector("tbody tr")).toBeTruthy());
    // 整格相等：只断言「包含 Claude Code」的话，本地表那份「Claude」也能被包含进去。
    expect(view.container.querySelector("tbody tr td")!.textContent).toBe("按平台 · Claude Code");
  });

  it("拿不到平台列表时退回本地兜底表（claude 已与后端同名，不再漂移），而不是留空", async () => {
    fakeRuntime([RULE]);
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelector("tbody tr")).toBeTruthy());
    expect(view.container.querySelector("tbody tr td")!.textContent).toBe("按平台 · Claude Code");
  });

  it("表单的平台下拉 = `providers` 的清单与名字（给三个平台就只有三个选项）", async () => {
    fakeRuntime([]);
    const view = render(<GatewayRulesCard providers={THREE_PROVIDERS} onError={vi.fn()} />);
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
    // 逐个相等：清单与名字（含顺序）都照 `providers` —— 本地那份写死的「四个平台」里
    // 有 Gemini、且 claude 曾叫「Claude」，两者都会在这里露馅。
    expect(options).toEqual(["请选择平台", "Codex", "Claude Code", "Qoder"]);
  });

  it("同屏两个平台名同源：同一份 `providers` 下规则表与外卡逐字同名", async () => {
    fakeRuntime([RULE]);
    const view = render(
      <>
        <GatewayExternalCard
          baseUrl="http://127.0.0.1:8787"
          providers={PROVIDERS}
          status={{ enabled: true, hasToken: true }}
          onStatusChange={vi.fn()}
          onError={vi.fn()}
        />
        <GatewayRulesCard providers={PROVIDERS} onError={vi.fn()} />
      </>
    );
    await waitFor(() => expect(view.container.querySelector("tbody tr")).toBeTruthy());
    // 外卡：端点行的平台名逐格等于后端 label（旧实现这里是本地表的「Claude」）。
    const endpointNames = [
      ...view.container.querySelectorAll<HTMLElement>(".gateway-endpoint-platform")
    ].map((item) => item.textContent);
    expect(endpointNames).toEqual(["Codex", "Gemini", "Claude Code", "Qoder"]);
    // 规则表：同一屏上同一平台的写法与上面逐字一致。
    expect(view.container.querySelector("tbody tr td")!.textContent).toBe("按平台 · Claude Code");
  });
});
