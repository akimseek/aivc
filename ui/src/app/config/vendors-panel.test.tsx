// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { VendorsPanel } from "./vendors-panel";
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
