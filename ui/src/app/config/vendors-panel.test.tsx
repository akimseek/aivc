// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { VendorsPanel, parseAdvancedJson, parsePrice } from "./vendors-panel";
import type { AiProviderId } from "../../types";

/**
 * 供应商管理。
 *
 * # 这组测试的由来
 *
 * 用户报「**供应商没有模型厂商的选择**」。原因是明确的：表单保存时用的是
 * 外层的 `providerId`（侧栏当前平台），界面上**没有任何地方能改它**。
 *
 * 所以这里钉住的第一条就是：**保存时用的是草稿里的厂商，不是外层的当前平台**。
 * 这一条不测的话，界面看起来有选择器、保存时却被覆盖掉——那是最难发现的一类错。
 */

type Invoke = { command: string; args: Record<string, unknown> };

function fakeRuntime(existing: unknown[] = []) {
  const calls: Invoke[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        if (name === "vendor:list") return existing;
        if (name === "vendor:save") return { id: "new" };
        if (name === "vendor:set-enabled") return {};
        if (name === "vendor:delete") return true;
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return { calls };
}

const PROVIDERS = [
  { id: "codex" as const, label: "Codex", capabilities: {} },
  { id: "claude" as const, label: "Claude Code", capabilities: {} },
  { id: "gemini" as const, label: "Gemini", capabilities: {} },
  { id: "qoder" as const, label: "Qoder CN", capabilities: {} }
];

function setup(overrides: { existing?: unknown[]; providerId?: AiProviderId | "" } = {}) {
  const runtime = fakeRuntime(overrides.existing ?? []);
  const onError = vi.fn();
  const view = render(
    <VendorsPanel
      providerId={overrides.providerId === undefined ? "codex" : overrides.providerId}
      providers={PROVIDERS}
      onError={onError}
    />
  );
  const byText = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => item.textContent?.trim() === label
    );
    return found as HTMLButtonElement | undefined;
  };
  const typeInto = async (input: HTMLInputElement, value: string) => {
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
      setter.call(input, value);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const inputsByPlaceholder = (part: string) =>
    [...view.container.querySelectorAll<HTMLInputElement>("input")].find((item) =>
      (item.placeholder ?? "").includes(part)
    );
  return { view, runtime, onError, byText, typeInto, inputsByPlaceholder };
}

describe("VendorsPanel：模型厂商选择", () => {
  it("表单里有「模型厂商」，四个厂商都在", async () => {
    const { byText } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    const text = document.body.textContent ?? "";
    expect(text).toContain("模型厂商");
    for (const label of ["Codex", "Claude Code", "Gemini", "Qoder CN"]) {
      expect(text).toContain(label);
    }
  });

  it("「新增」时厂商**默认取当前平台**——多数情况就是它", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({ providerId: "gemini" });
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "测试家");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await act(async () => {
      byText("创建")!.click();
    });

    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.providerId).toBe("gemini");
    });
  });

  it("**改了厂商之后，保存用的是改后的那一个**（不是外层的当前平台）", async () => {
    // 这一条是这个缺陷的核心：界面有选择器、保存时被外层覆盖，是最难发现的错。
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({ providerId: "codex" });
    await act(async () => {
      byText("新增供应商")!.click();
    });

    // 点「Claude Code」把厂商改成 claude。
    await act(async () => {
      byText("Claude Code")!.click();
    });

    await typeInto(inputsByPlaceholder("官方")!, "中转站");
    await typeInto(inputsByPlaceholder("https://")!, "https://b/v1");
    await act(async () => {
      byText("创建")!.click();
    });

    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.providerId).toBe("claude");
      // 而外层的当前平台仍然是 codex —— 两者不同，才说明真的用了草稿里的值。
      expect(call?.args.providerId).not.toBe("codex");
    });
  });

  it("没有 API 地址时创建按钮禁用（地址是必填）", async () => {
    const { byText, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "只有名字");
    expect(byText("创建")!.disabled).toBe(true);
  });
});

describe("VendorsPanel：跨平台列表", () => {
  it("默认列出**全部平台**的供应商（不传 providerId）", async () => {
    const { runtime } = setup();
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:list");
      expect(call).toBeTruthy();
      // 全部平台时**不带** providerId——带了就只有当前平台那几家。
      expect(call!.args).not.toHaveProperty("providerId");
    });
  });

  it("有平台筛选，默认是「全部平台」", async () => {
    const { view } = setup();
    const select = view.container.querySelector("select")!;
    expect(select.value).toBe("all");
    expect([...select.options].map((item) => item.textContent)).toContain("全部平台");
  });

  it("选了某个平台就把筛选交给后端", async () => {
    const { view, runtime } = setup();
    const select = view.container.querySelector("select")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select, "claude");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await waitFor(() => {
      const call = runtime.calls.find(
        (item) => item.command === "vendor:list" && item.args.providerId === "claude"
      );
      expect(call).toBeTruthy();
    });
  });

  it("表格里有「厂商」列，显示的是**平台名**而不是编号", async () => {
    // 列表是跨平台的，不标出来同名供应商分不清；显示编号则没人认得出。
    const { view } = setup({
      existing: [
        {
          id: "v1",
          providerId: "claude",
          name: "中转站",
          apiBaseUrl: "https://b/v1",
          sort: 0,
          enabled: true
        }
      ]
    });
    const table = await waitFor(() => {
      const found = view.container.querySelector("table.data-table");
      if (!found) throw new Error("还没有表格");
      return found;
    });
    expect(table.textContent).toContain("厂商");
    expect(table.textContent).toContain("Claude Code");
  });

  it("筛选结果为空时说清是**这个平台**没有，而不是「一个都没有」", async () => {
    const { view } = setup({ existing: [] });
    const select = view.container.querySelector("select")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select, "gemini");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("Gemini 下还没有供应商");
    });
  });

  it("编辑时**不回填密钥**——接口本来就不返回它", async () => {
    const { view, byText } = setup({
      existing: [
        {
          id: "v1",
          providerId: "codex",
          name: "官方",
          apiBaseUrl: "https://a/v1",
          sort: 0,
          enabled: true
        }
      ]
    });
    await waitFor(() => {
      expect(byText("编辑")).toBeTruthy();
    });
    await act(async () => {
      byText("编辑")!.click();
    });
    const key = [...view.container.querySelectorAll<HTMLInputElement>("input")].find((item) =>
      (item.placeholder ?? "").includes("不修改")
    );
    expect(key).toBeTruthy();
    expect(key!.value).toBe("");
  });

  it("编辑时带出这条自己的厂商，而不是当前平台", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup({
      providerId: "codex",
      existing: [
        {
          id: "v1",
          providerId: "qoder",
          name: "Qoder 官方",
          apiBaseUrl: "https://q/v1",
          sort: 0,
          enabled: true
        }
      ]
    });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "Qoder 官方改");
    await act(async () => {
      byText("保存修改")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.providerId).toBe("qoder");
      expect(call?.args.id).toBe("v1");
    });
  });
});

/* ── A3 费率 / A5 高级 JSON ─────────────────────────────
 *
 * 解析层单独测：它是「填错会静默算出错数字」的那一层。
 * 后端 `VendorPricing` 两个字段都可选，`BalanceQueryConfig` / `ModelQueryConfig`
 * 是结构化对象——JSON 文本框是旧平台的做法，照做。
 */

describe("parsePrice：费率", () => {
  it("空串 = 未设置（**不是 0**）", () => {
    expect(parsePrice("", "输入费率")).toEqual({ ok: true, value: undefined });
    expect(parsePrice("   ", "输入费率")).toEqual({ ok: true, value: undefined });
  });

  it("「0」是免费，与未设置是两回事", () => {
    expect(parsePrice("0", "输入费率")).toEqual({ ok: true, value: 0 });
  });

  it("正常小数", () => {
    expect(parsePrice("0.55", "输入费率")).toEqual({ ok: true, value: 0.55 });
  });

  it("**0.55 与 2.2 必须被接受**——二进制不精确，精确比较会把它们判成三位小数", () => {
    // 后端原来就是精确比较，实测把 0.55 拒了。这条与后端那条测试成对。
    for (const price of ["0.55", "2.2", "0.29", "1.15", "3.75"]) {
      const result = parsePrice(price, "输入费率");
      expect(result.ok, price).toBe(true);
    }
  });

  it("真的三位小数要拒（容差不是放开）", () => {
    for (const price of ["1.234", "0.555", "9.999"]) {
      const result = parsePrice(price, "输入费率");
      expect(result.ok, price).toBe(false);
      expect(result.ok === false && result.message, price).toContain("2 位小数");
    }
  });

  it("**非数字要报错，不能悄悄变成 0**", () => {
    // 悄悄归零会让成本估算凭空变成 0，而且用户完全看不出来。
    const result = parsePrice("abc", "输入费率");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.message).toContain("输入费率");
  });

  it("负数要报错", () => {
    expect(parsePrice("-1", "输出费率").ok).toBe(false);
  });
});

describe("parseAdvancedJson：高级 JSON", () => {
  it("空串 = 不设置", () => {
    expect(parseAdvancedJson("", "余额查询配置")).toEqual({ ok: true, value: undefined });
  });

  it("对象可用", () => {
    expect(parseAdvancedJson('{"endpoint":"/x"}', "余额查询配置")).toEqual({
      ok: true,
      value: { endpoint: "/x" }
    });
  });

  it("**格式错时文案是「JSON 格式无效」**（旧平台的原话）", () => {
    const result = parseAdvancedJson("{oops", "余额查询配置");
    expect(result.ok).toBe(false);
    expect(result.ok === false && result.message).toContain("JSON 格式无效");
  });

  it("**数组与字面量要报「必须是 JSON 对象」**——后端要的是对象", () => {
    for (const text of ["[]", "[1,2]", "null", "42", '"x"']) {
      const result = parseAdvancedJson(text, "模型查询配置");
      expect(result.ok, text).toBe(false);
      expect(result.ok === false && result.message, text).toContain("必须是 JSON 对象");
    }
  });

  it("文案里带上是哪一项，两个框分得清", () => {
    const result = parseAdvancedJson("{", "模型查询配置");
    expect(result.ok === false && result.message.startsWith("模型查询配置")).toBe(true);
  });
});

describe("VendorsPanel：费率与高级 JSON 接线", () => {
  it("**两条费率会按 pricing 结构提交**（后端字段名是 inputPerMillionUsd）", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "带费率的");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await typeInto(inputsByPlaceholder("0.55")!, "0.55");
    await typeInto(inputsByPlaceholder("2.2")!, "2.2");
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args.pricing).toEqual({
        inputPerMillionUsd: 0.55,
        outputPerMillionUsd: 2.2
      });
    });
  });

  it("**两个费率都留空时整个 pricing 不传**（空对象是「设置了但什么都没有」）", async () => {
    const { byText, runtime, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "无费率");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    await act(async () => {
      byText("创建")!.click();
    });
    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:save");
      expect(call?.args).not.toHaveProperty("pricing");
    });
  });

  it("JSON 填错时**整体不提交**，并给出是哪一项", async () => {
    const { byText, view, runtime, onError, inputsByPlaceholder, typeInto } = setup();
    await act(async () => {
      byText("新增供应商")!.click();
    });
    await typeInto(inputsByPlaceholder("官方")!, "JSON 错");
    await typeInto(inputsByPlaceholder("https://")!, "https://a/v1");
    // **在本次渲染的容器里找**，不要在 `document` 里找：那个范围包含别的测试
    // 留下的节点，写进去的值不会进当前组件的 state——测试会以「没报错」的形式失败，
    // 看起来像产品缺陷，其实是测试读错了元素。
    const area = [...view.container.querySelectorAll("textarea")].find((item) =>
      (item.placeholder ?? "").includes("endpoint")
    )!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(
        HTMLTextAreaElement.prototype,
        "value"
      )!.set!;
      setter.call(area, "{oops");
      area.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => {
      byText("创建")!.click();
    });
    expect(onError).toHaveBeenCalledWith(expect.stringContaining("JSON 格式无效"));
    // **没有发出去**：提交一半再报错，用户不知道哪一项存进去了。
    expect(runtime.calls.some((item) => item.command === "vendor:save")).toBe(false);
  });

  it("编辑时回填费率与高级 JSON（不回填就会被「保存修改」清掉）", async () => {
    const { byText, view, inputsByPlaceholder } = setup({
      existing: [
        {
          id: "v1",
          providerId: "codex",
          name: "官方",
          apiBaseUrl: "https://a/v1",
          sort: 0,
          enabled: true,
          pricing: { inputPerMillionUsd: 1.5 },
          balanceQuery: { endpoint: "/balance" }
        }
      ]
    });
    await waitFor(() => expect(byText("编辑")).toBeTruthy());
    await act(async () => {
      byText("编辑")!.click();
    });
    expect((inputsByPlaceholder("0.55") as HTMLInputElement).value).toBe("1.5");
    const area = [...view.container.querySelectorAll("textarea")].find((item) =>
      (item.placeholder ?? "").includes("endpoint")
    ) as HTMLTextAreaElement;
    expect(area.value).toContain("/balance");
  });
});
