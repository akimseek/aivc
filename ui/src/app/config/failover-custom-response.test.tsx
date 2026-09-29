// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { FailoverRulesSection, parseCustomResponse } from "./gateway-panel";

/**
 * 故障规则：**自定义返回**（A1）与**试规则**（A2）。
 *
 * # 这两条的来历
 *
 * 审计（`38-legacy-parity-audit.md`）发现旧平台上规则能配两样东西，新平台没有：
 *
 * | 旧平台的文案 | 后端 | 新平台此前 |
 * | --- | --- | --- |
 * | `自定义返回状态码（可选）` / `自定义返回内容（JSON，可选）` | **有** `customResponseStatus` / `customResponseBody` | 类型和 `saveFailoverRule` 都收了，**表单没输入框** |
 * | `规则测试` / `输入状态码和供应商错误内容，查看是否命中` | 匹配逻辑在 core 里是纯函数 | 完全没有 |
 *
 * 第二条**不能**在前端重写匹配——两份逻辑一分叉就会出现
 * 「测试说会命中、实际没切换」。因此新加了一条命令 `gateway:test-failover-rule`，
 * 它调用的就是转发路径用的那一份 `match_rule`。
 */

function fakeRuntime(handlers: Record<string, (args: Record<string, unknown>) => unknown>) {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        const handler = handlers[name];
        return handler ? handler(payload?.args ?? {}) : null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return calls;
}

const RULE = {
  id: "r1",
  scope: "global" as const,
  pattern: "rate limit",
  statusCodes: [429],
  customResponseStatus: 503,
  customResponseBody: '{"error":"busy"}',
  enabled: true,
  createdAt: "2026-01-01T00:00:00.000Z",
  updatedAt: "2026-01-01T00:00:00.000Z"
};

function findButton(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(
    (item) => (item.textContent ?? "").trim() === text
  );
}

function setValue(element: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  setter.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("parseCustomResponse：四条校验（照旧平台）", () => {
  it("两项都留空 = 不设置", () => {
    expect(parseCustomResponse("", "")).toEqual({ ok: true });
  });

  it("**只填一项要报「必须同时配置」**", () => {
    for (const [status, body] of [
      ["503", ""],
      ["", '{"a":1}']
    ]) {
      const result = parseCustomResponse(status, body);
      expect(result.ok, `${status}|${body}`).toBe(false);
      expect(result.ok === false && result.message).toContain("必须同时配置");
    }
  });

  it("状态码必须在 100–599", () => {
    for (const bad of ["99", "600", "0", "abc", "50.5"]) {
      const result = parseCustomResponse(bad, '{"a":1}');
      expect(result.ok, bad).toBe(false);
      expect(result.ok === false && result.message, bad).toContain("100 到 599");
    }
    expect(parseCustomResponse("100", '{"a":1}')).toEqual({
      ok: true,
      status: 100,
      body: '{"a":1}'
    });
    expect(parseCustomResponse("599", '{"a":1}').ok).toBe(true);
  });

  it("内容必须是合法 JSON", () => {
    const result = parseCustomResponse("503", "{oops");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.message).toContain("必须是合法 JSON");
  });

  it("**超过 32 KB 要拒，而且按字节算**——中文一个字符占三字节", () => {
    // 11000 个中文字符 ≈ 33 KB，按**字符数**算会以为没超。
    const tooBig = `{"e":"${"测".repeat(11000)}"}`;
    expect(tooBig.length).toBeLessThan(32 * 1024);
    expect(new TextEncoder().encode(tooBig).length).toBeGreaterThan(32 * 1024);
    const result = parseCustomResponse("503", tooBig);
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.message).toContain("32 KB");
  });
});

describe("FailoverRulesSection：规则表单接线", () => {
  it("**能配自定义返回**，并按真实字段名提交", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [],
      "gateway:save-failover-rule": () => RULE
    });
    const view = render(<FailoverRulesSection onError={vi.fn()} />);
    await waitFor(() => expect(findButton(view.container, "新增规则")).toBeTruthy());
    await act(async () => {
      findButton(view.container, "新增规则")!.click();
    });
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await act(async () => {
      setValue(inputs.find((i) => i.placeholder.includes("insufficient_quota"))!, "rate limit");
      setValue(inputs.find((i) => i.placeholder.includes("429,500"))!, "429");
      setValue(inputs.find((i) => i.placeholder === "例如 503")!, "503");
      setValue(inputs.find((i) => i.placeholder.includes("上游不可用"))!, '{"error":"busy"}');
    });
    await act(async () => {
      findButton(view.container, "创建")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:save-failover-rule")).toBe(true);
    });
    const call = calls.find((item) => item.command === "gateway:save-failover-rule")!;
    expect(call.args.customResponseStatus).toBe(503);
    expect(call.args.customResponseBody).toBe('{"error":"busy"}');
  });

  it("**自定义返回只填一半时整体不提交**，并说明原因", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [],
      "gateway:save-failover-rule": () => RULE
    });
    const onError = vi.fn();
    const view = render(<FailoverRulesSection onError={onError} />);
    await waitFor(() => expect(findButton(view.container, "新增规则")).toBeTruthy());
    await act(async () => {
      findButton(view.container, "新增规则")!.click();
    });
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await act(async () => {
      setValue(inputs.find((i) => i.placeholder.includes("insufficient_quota"))!, "rate limit");
      setValue(inputs.find((i) => i.placeholder === "例如 503")!, "503");
      // 内容留空。
    });
    await act(async () => {
      findButton(view.container, "创建")!.click();
    });
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("必须同时配置"));
    expect(calls.some((item) => item.command === "gateway:save-failover-rule")).toBe(false);
  });

  it("**编辑时回填自定义返回**（不回填就会被「保存修改」清掉）", async () => {
    fakeRuntime({ "gateway:list-failover-rules": () => [RULE] });
    const view = render(<FailoverRulesSection onError={vi.fn()} />);
    await waitFor(() => expect(findButton(view.container, "编辑")).toBeTruthy());
    await act(async () => {
      findButton(view.container, "编辑")!.click();
    });
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    expect(inputs.find((i) => i.placeholder === "例如 503")!.value).toBe("503");
    expect(inputs.find((i) => i.placeholder.includes("上游不可用"))!.value).toBe(
      '{"error":"busy"}'
    );
  });
});

describe("FailoverRulesSection：试规则", () => {
  it("**试规则要问后端**，并显示是被哪一项命中的", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [RULE],
      "gateway:test-failover-rule": () => ({ matched: true, matchedBy: "pattern" })
    });
    const view = render(<FailoverRulesSection onError={vi.fn()} />);
    await waitFor(() => expect(findButton(view.container, "编辑")).toBeTruthy());
    await act(async () => {
      findButton(view.container, "编辑")!.click();
    });
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await act(async () => {
      setValue(inputs.find((i) => i.placeholder.includes("credit insufficient"))!, "rate limit hit");
    });
    await act(async () => {
      findButton(view.container, "试一下")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:test-failover-rule")).toBe(true);
    });
    const call = calls.find((item) => item.command === "gateway:test-failover-rule")!;
    // 规则本体按**与保存同形状**传下去，且带上模拟输入。
    expect((call.args.rule as Record<string, unknown>).pattern).toBe("rate limit");
    expect(call.args.message).toBe("rate limit hit");
    await waitFor(() => {
      expect(view.container.textContent).toContain("错误文本包含");
    });
  });

  it("没命中时说的是「不会命中」——与「还没试」不是一件事", async () => {
    fakeRuntime({
      "gateway:list-failover-rules": () => [RULE],
      "gateway:test-failover-rule": () => ({ matched: false, matchedBy: "none" })
    });
    const view = render(<FailoverRulesSection onError={vi.fn()} />);
    await waitFor(() => expect(findButton(view.container, "编辑")).toBeTruthy());
    await act(async () => {
      findButton(view.container, "编辑")!.click();
    });
    // 试之前不该有结论。
    expect(view.container.textContent).not.toContain("不会命中");
    await act(async () => {
      findButton(view.container, "试一下")!.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("不会命中"));
  });
});
