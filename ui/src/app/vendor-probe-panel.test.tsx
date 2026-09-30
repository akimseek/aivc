// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { VendorConfigReader, VendorModelProbe, balanceText } from "./vendor-probe-panel";

/**
 * 供应商探活的两块：**测连接（列模型）** 与 **读配置**。
 *
 * 三条后端命令（`vendor:list-models` / `vendor:read-configs`，以及余额那条
 * `vendor:refresh-balance`）此前在界面上**一个入口都没有**。这一组钉住的是
 * 「接线真的接上了」与**余额的三态**——把「上游没提供余额」显示成 `0`
 * 会让用户以为该充值了。
 *
 * 余额的行内状态与单家刷新入口在 `config/vendors-panel.test.tsx`：
 * 它已经不在这一层渲染（每个供应商行自己显示），探活这一层只剩上面两块。
 */

type Handler = (event: { payload: unknown }) => void;

function fakeRuntime(overrides: { models?: unknown[]; configs?: unknown[] } = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        if (name === "vendor:list-models") return overrides.models ?? [];
        if (name === "vendor:read-configs") return overrides.configs ?? [];
        return {};
      }
    },
    event: {
      listen: async (_channel: string, _handler: Handler) => () => undefined
    }
  };
  return { calls };
}

const VENDORS = [
  { id: "v1", providerId: "codex" as const, name: "官方", apiBaseUrl: "https://a/v1", sort: 0, enabled: true }
];

describe("balanceText", () => {
  it("有剩余额度时显示数字与单位", () => {
    expect(balanceText({ remaining: 12.5, unit: "USD" })).toBe("12.5 USD");
  });

  it("只有已用量时显示已用", () => {
    expect(balanceText({ used: 3, unit: "USD" })).toBe("已用 3 USD");
  });

  it("只有套餐名时显示套餐名", () => {
    expect(balanceText({ planName: "Pro" })).toBe("Pro");
  });

  it("**上游没提供余额时说「未提供」，不是 0**", () => {
    // 这一条最要紧：显示 `0` 会让用户以为钱用完了，而事实是那家中转站
    // 根本不返回余额。两者的下一步动作完全不同。
    expect(balanceText({})).toBe("未提供");
    expect(balanceText(undefined)).toBe("未提供");
  });
});

describe("VendorModelProbe", () => {
  it("没有供应商时说明原因，而不是给一排点不动的按钮", () => {
    fakeRuntime();
    const view = render(<VendorModelProbe vendors={[]} onError={vi.fn()} />);
    expect(view.container.textContent).toContain("先配一家供应商");
  });

  it("查询模型：把结果列出来", async () => {
    const runtime = fakeRuntime({ models: [{ id: "gpt-5" }, { id: "gpt-5-mini" }] });
    const view = render(<VendorModelProbe vendors={VENDORS} onError={vi.fn()} />);

    await act(async () => {
      const select = view.container.querySelector("select")!;
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select, "v1");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });

    const button = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("查询模型")
    )!;
    await act(async () => {
      button.click();
    });

    await waitFor(() => {
      expect(runtime.calls.some((item) => item.command === "vendor:list-models")).toBe(true);
      expect(view.container.textContent).toContain("gpt-5-mini");
    });
  });

  it("换供应商会清掉上一家的结果", async () => {
    // 留着上一家的模型列表会让人以为那是新选的这一家的。
    const runtime = fakeRuntime({ models: [{ id: "gpt-5" }] });
    const view = render(
      <VendorModelProbe
        vendors={[...VENDORS, { ...VENDORS[0]!, id: "v2", name: "备用" }]}
        onError={vi.fn()}
      />
    );

    const select = view.container.querySelector("select")!;
    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
    await act(async () => {
      setter.call(select, "v1");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const probe = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("查询模型")
    )!;
    await act(async () => {
      probe.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("gpt-5");
    });

    await act(async () => {
      setter.call(select, "v2");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(view.container.textContent).not.toContain("gpt-5");
    expect(runtime.calls.length).toBeGreaterThan(0);
  });
});

describe("VendorConfigReader", () => {
  it("读取配置：只提供白名单里的三个路径", async () => {
    // 后端强制路径白名单（必须是 `~/` 或 `$HOME/` 且落在 .codex/.gemini/.claude 下）。
    // 界面给一个随便填路径的输入框，用户填了只会收到「路径不允许」，还以为是坏了。
    const runtime = fakeRuntime({ configs: [{ path: "~/.codex/config.toml", content: "a = 1" }] });
    const view = render(<VendorConfigReader />);

    const button = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("Codex 配置")
    )!;
    await act(async () => {
      button.click();
    });

    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "vendor:read-configs");
      expect(call).toBeTruthy();
      expect(call!.args.paths).toEqual(["~/.codex/config.toml"]);
      expect(view.container.textContent).toContain("a = 1");
    });
  });

  it("白名单的理由写在界面上（三个固定路径之外没有入口）", () => {
    fakeRuntime();
    const view = render(<VendorConfigReader />);
    // 三个按钮就是全部入口：没有「自己填路径」的输入框。
    expect(view.container.querySelectorAll("input")).toHaveLength(0);
    expect(view.container.textContent).toContain("白名单");
  });
});
