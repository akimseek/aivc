// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { GatewayRulesCard } from "./gateway-rules-card";
import { parseCustomResponse } from "./gateway-rule-model";

/**
 * 故障规则：**自定义返回**（A1）、**试规则**（A2）与作用域级联（批次二）。
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
 *
 * # 批次二补的第三条
 *
 * 作用域有 provider / vendor 两档，但后端**要求**带上 providerId（供应商级还要
 * vendorId，见 `crates/aivc-core/src/gateway.rs:270-289`），而此前表单里根本没有
 * 这两项——按平台/按供应商的规则在界面上存不下去。批次二补上级联选择，
 * 并顺手补上「立即启用」勾选（此前编辑一条停用规则再保存会把它重新启用）。
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

const VENDORS = [
  {
    id: "v1",
    providerId: "codex" as const,
    name: "官方 v2",
    apiBaseUrl: "https://api.example.com",
    sort: 0,
    enabled: true
  }
];

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

function setSelect(element: HTMLSelectElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
  setter.call(element, value);
  element.dispatchEvent(new Event("change", { bubbles: true }));
}

/** 打开表单（新增或编辑），返回查输入框的小工具。 */
async function openForm(view: { container: HTMLElement }, by: string) {
  await waitFor(() => expect(findButton(view.container, by)).toBeTruthy());
  await act(async () => {
    findButton(view.container, by)!.click();
  });
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

describe("GatewayRulesCard：规则表单接线", () => {
  it("**能配自定义返回**，并按真实字段名提交", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [],
      "gateway:save-failover-rule": () => RULE
    });
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "新增规则");
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
    const view = render(<GatewayRulesCard onError={onError} />);
    await openForm(view, "新增规则");
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
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "编辑");
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    expect(inputs.find((i) => i.placeholder === "例如 503")!.value).toBe("503");
    expect(inputs.find((i) => i.placeholder.includes("上游不可用"))!.value).toBe(
      '{"error":"busy"}'
    );
  });

  it("**编辑停用规则时勾选框是未选中，保存时把停用状态传回去**", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [{ ...RULE, enabled: false }],
      "gateway:save-failover-rule": () => RULE
    });
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "编辑");
    const checkbox = view.container.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    expect(checkbox.checked).toBe(false);
    await act(async () => {
      findButton(view.container, "保存修改")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:save-failover-rule")).toBe(true);
    });
    expect(
      calls.find((item) => item.command === "gateway:save-failover-rule")!.args.enabled
    ).toBe(false);
  });

  it("**作用域级联**：按平台必须带上 providerId，按供应商还要 vendorId", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [],
      "vendor:list": () => VENDORS,
      "gateway:save-failover-rule": () => RULE
    });
    const onError = vi.fn();
    const view = render(<GatewayRulesCard onError={onError} />);
    await openForm(view, "新增规则");
    await act(async () => {
      setValue(
        [...view.container.querySelectorAll<HTMLInputElement>("input")].find((i) =>
          i.placeholder.includes("insufficient_quota")
        )!,
        "rate limit"
      );
      setSelect(view.container.querySelector<HTMLSelectElement>('select[aria-label="作用域"]')!, "provider");
    });
    // 选了「按平台」才会出现目标平台下拉。
    const provider = view.container.querySelector<HTMLSelectElement>('select[aria-label="目标平台"]')!;
    expect(provider).toBeTruthy();
    await act(async () => {
      setSelect(provider, "codex");
    });
    await act(async () => {
      findButton(view.container, "创建")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:save-failover-rule")).toBe(true);
    });
    const payload = calls.find((item) => item.command === "gateway:save-failover-rule")!.args;
    expect(payload.scope).toBe("provider");
    expect(payload.providerId).toBe("codex");
    // 按平台作用域**不能**带供应商：后端会判「Provider 级规则不能指定供应商」。
    expect(payload.vendorId).toBeUndefined();

    // 全局作用域相反：不能带 providerId / vendorId。
    await openForm(view, "新增规则");
    await act(async () => {
      setValue(
        [...view.container.querySelectorAll<HTMLInputElement>("input")].find((i) =>
          i.placeholder.includes("insufficient_quota")
        )!,
        "rate limit"
      );
    });
    await act(async () => {
      findButton(view.container, "创建")!.click();
    });
    await waitFor(() => {
      expect(
        calls.filter((item) => item.command === "gateway:save-failover-rule").length
      ).toBe(2);
    });
    const globals = calls.filter((item) => item.command === "gateway:save-failover-rule");
    expect(globals[1]!.args.providerId).toBeUndefined();
    expect(globals[1]!.args.vendorId).toBeUndefined();
  });

  it("按供应商没选供应商时**本地就拦下**（后端原话），不发请求", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [],
      "vendor:list": () => VENDORS,
      "gateway:save-failover-rule": () => RULE
    });
    const onError = vi.fn();
    const view = render(<GatewayRulesCard onError={onError} />);
    await openForm(view, "新增规则");
    await act(async () => {
      setValue(
        [...view.container.querySelectorAll<HTMLInputElement>("input")].find((i) =>
          i.placeholder.includes("insufficient_quota")
        )!,
        "rate limit"
      );
      setSelect(view.container.querySelector<HTMLSelectElement>('select[aria-label="作用域"]')!, "vendor");
    });
    await act(async () => {
      setSelect(
        view.container.querySelector<HTMLSelectElement>('select[aria-label="目标平台"]')!,
        "codex"
      );
    });
    await act(async () => {
      findButton(view.container, "创建")!.click();
    });
    expect(onError).toHaveBeenCalledWith("供应商级规则必须指定供应商。");
    expect(calls.some((item) => item.command === "gateway:save-failover-rule")).toBe(false);
  });
});

describe("GatewayRulesCard：试规则", () => {
  it("**试规则要问后端**，并显示是被哪一项命中的", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [RULE],
      "gateway:test-failover-rule": () => ({ matched: true, matchedBy: "pattern" })
    });
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "编辑");
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
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "编辑");
    // 试之前不该有结论。
    expect(view.container.textContent).not.toContain("不会命中");
    await act(async () => {
      findButton(view.container, "试一下")!.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("不会命中"));
  });

  it("状态码命中时说的是「状态码匹配」（三态里的另一个）", async () => {
    fakeRuntime({
      "gateway:list-failover-rules": () => [RULE],
      "gateway:test-failover-rule": () => ({ matched: true, matchedBy: "status" })
    });
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "编辑");
    await act(async () => {
      findButton(view.container, "试一下")!.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("状态码匹配"));
  });

  it("**试规则的状态码留空时整个字段都不发**（`Number(\"\")` 恰好是 0）", async () => {
    const calls = fakeRuntime({
      "gateway:list-failover-rules": () => [RULE],
      "gateway:test-failover-rule": () => ({ matched: false, matchedBy: "none" })
    });
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await openForm(view, "编辑");
    const testCalls = () =>
      calls.filter((item) => item.command === "gateway:test-failover-rule");
    /** 改一格模拟状态码再点「试一下」，等这一趟请求真的发出去了。 */
    const testWith = async (value: string) => {
      const input = [...view.container.querySelectorAll<HTMLInputElement>("input")].find(
        (i) => i.placeholder === "例如 429"
      )!;
      await act(async () => {
        setValue(input, value);
      });
      const before = testCalls().length;
      await act(async () => {
        findButton(view.container, "试一下")!.click();
      });
      await waitFor(() => expect(testCalls().length).toBe(before + 1));
      return testCalls().at(-1)!.args;
    };

    // 留空 = 这次不按状态码判定：`statusCode: 0` 会让后端拿 0 去比规则里的状态码列表
    // （0 不是合法的 HTTP 状态码，旧实现是省略该字段）。
    const empty = await testWith("");
    expect("statusCode" in empty).toBe(false);
    // 错误文本也留空时连 `message` 都不发。
    expect("message" in empty).toBe(false);

    // 非整数同理：填了一串字也不是状态码。
    expect("statusCode" in (await testWith("abc"))).toBe(false);

    // 填了合法状态码才带上它（否则上面两条可能是因为整个字段永远不发）。
    expect((await testWith("503")).statusCode).toBe(503);
  });

  it("作用域列写**显示名**：平台 `codex` → `Codex`，供应商写它自己的名字", async () => {
    fakeRuntime({
      "gateway:list-failover-rules": () => [
        { ...RULE, id: "r-provider", scope: "provider", providerId: "codex" },
        { ...RULE, id: "r-vendor", scope: "vendor", providerId: "codex", vendorId: "v1" },
        // 供应商已被删掉时退回编号：留一个空单元格会被读成「没有目标」。
        { ...RULE, id: "r-gone", scope: "vendor", providerId: "codex", vendorId: "v-404" }
      ],
      "vendor:list": () => VENDORS
    });
    const view = render(<GatewayRulesCard onError={vi.fn()} />);
    await waitFor(() => expect(view.container.textContent).toContain("官方 v2"));
    const scopes = [...view.container.querySelectorAll("tbody tr td:first-child")].map(
      (cell) => cell.textContent
    );
    expect(scopes).toEqual(["按平台 · Codex", "按供应商 · 官方 v2", "按供应商 · v-404"]);
  });
});
