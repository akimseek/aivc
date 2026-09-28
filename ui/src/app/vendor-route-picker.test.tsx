// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { VendorRoutePicker } from "./vendor-route-picker";
import type { TerminalSession } from "./use-terminals";

/**
 * 状态栏供应商选择器的测试。
 *
 * # 为什么它值得有测试
 *
 * 这是界面这一侧**唯一没有测试的新增件**，而它承担的是「我现在花的是谁的钱」——
 * 走网关之后供应商可能被**自动**换掉，界面不显示或显示错，用户只能从账单上发现。
 * 它的分支又不少：直连、有绑定、当前供应商已停用、以及「自动切换」徽标只对
 * `failure` 出现（`candidate-pool` 是路由调整，不该当成异常提示）。
 */

type Handler = (event: { payload: unknown }) => void;

/** 装一个假的 Tauri 运行时；`vendors` 是 `vendor:list` 的返回。 */
function fakeRuntime(vendors: unknown[]) {
  let invocations: { command: string; args: unknown }[] = [];

  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        const payload = args as { command?: string } | undefined;
        invocations.push({ command: payload?.command ?? command, args });
        if (payload?.command === "vendor:list") return vendors;
        return {};
      }
    },
    // 这个组件不订阅事件，但运行时对象要完整。
    event: { listen: async (_channel: string, _handler: Handler) => () => undefined }
  };

  return { invocations: () => invocations };
}

/** 一路终端。默认是「有绑定」的那一种，各用例按需覆盖。 */
function terminal(overrides: Partial<TerminalSession> = {}): TerminalSession {
  return {
    id: "term-1",
    title: "终端",
    running: true,
    kind: "codex",
    providerId: "codex",
    ...overrides
  };
}

const VENDORS = [
  { id: "vendor-a", providerId: "codex", name: "甲", apiBaseUrl: "https://a", enabled: true },
  { id: "vendor-b", providerId: "codex", name: "乙", apiBaseUrl: "https://b", enabled: true }
];

beforeEach(() => {
  vi.restoreAllMocks();
});

async function renderPicker(
  current: TerminalSession | undefined,
  options: { vendors?: unknown[]; onSwitch?: () => Promise<void>; onSetMode?: () => Promise<void> } = {}
) {
  const runtime = fakeRuntime(options.vendors ?? VENDORS);
  const onSwitch = options.onSwitch ?? vi.fn(async () => undefined);
  const onSetMode = options.onSetMode ?? vi.fn(async () => undefined);
  const view = render(
    <VendorRoutePicker
      terminal={current}
      onSwitch={onSwitch}
      onSetMode={onSetMode}
    />
  );
  // 供应商清单是异步取的；等它落到下拉框里。
  await waitFor(() => {
    if (current?.vendorId) {
      expect(view.container.querySelectorAll("option").length).toBeGreaterThan(0);
    }
  });
  return { view, onSwitch, onSetMode, runtime };
}

describe("VendorRoutePicker", () => {
  it("没有终端时不渲染任何东西", () => {
    const view = render(
      <VendorRoutePicker terminal={undefined} onSwitch={vi.fn()} onSetMode={vi.fn()} />
    );
    expect(view.container.textContent).toBe("");
  });

  it("系统终端不显示供应商", () => {
    // 系统终端不注入环境、也不绑路由，没有供应商可言。
    const view = render(
      <VendorRoutePicker
        terminal={terminal({ kind: "system", providerId: "" })}
        onSwitch={vi.fn()}
        onSetMode={vi.fn()}
      />
    );
    expect(view.container.querySelector(".status-vendor")).toBeNull();
  });

  it("没有绑定时**显眼地**说明「没走网关」，而不是一个中性的「直连」", async () => {
    // 用户真的因为这个吃过亏：界面上写着「直连」，他就以为一切正常，
    // 而请求其实去了 `~/.codex/config.toml` 里那个地址、用的是他自己的密钥，
    // 一直到余额耗尽看到 403 才发现。
    //
    // 因此这里断言的不只是「有字」，而是**那层警示**：`warn` 类、
    // 明说「未走网关」、并且点出「用自己的密钥」。
    const { view } = await renderPicker(terminal());
    const node = view.container.querySelector(".status-vendor");
    const text = node?.textContent ?? "";

    expect(text).toContain("未走网关");
    expect(text).toContain("直连");
    expect(text).toContain("用自己的密钥");
    expect(node?.className).toContain("warn");
    expect(view.container.querySelector(".vendor-pick")).toBeNull();
  });

  it("有绑定时下拉框选中它，改选会请求切换", async () => {
    const { view, onSwitch } = await renderPicker(terminal({ vendorId: "vendor-a" }));

    const select = view.container.querySelector<HTMLSelectElement>(".vendor-pick");
    expect(select?.value).toBe("vendor-a");

    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select, "vendor-b");
      select!.dispatchEvent(new Event("change", { bubbles: true }));
    });

    expect(onSwitch).toHaveBeenCalledWith("term-1", "vendor-b");
  });

  it("当前供应商已停用时仍然列出来并标明", async () => {
    // 供应商可能刚被禁用或删掉。选择器**不能悄悄跳到另一家**——那会让用户以为绑定还在。
    const { view } = await renderPicker(
      terminal({ vendorId: "vendor-gone" }),
      { vendors: VENDORS }
    );
    const select = view.container.querySelector<HTMLSelectElement>(".vendor-pick");
    expect(select?.value).toBe("vendor-gone");
    const labels = [...view.container.querySelectorAll("option")].map((item) => item.textContent);
    expect(labels.some((label) => label?.includes("已停用"))).toBe(true);
  });

  it("只有 failure 才显示「已自动切换」", async () => {
    // `candidate-pool` 是路由调整（原供应商被禁用），不是请求异常，不该当成故障提示。
    const failed = await renderPicker(
      terminal({ vendorId: "vendor-b", lastSwitch: { vendorId: "vendor-b", reason: "failure" } })
    );
    expect(failed.view.container.querySelector(".vendor-auto")).not.toBeNull();

    const pooled = await renderPicker(
      terminal({
        vendorId: "vendor-b",
        lastSwitch: { vendorId: "vendor-b", reason: "candidate-pool" }
      })
    );
    expect(pooled.view.container.querySelector(".vendor-auto")).toBeNull();
  });

  it("切换失败时不谎报成功", async () => {
    // 切换会因为「供应商被禁用 / 配置不全 / 平台不匹配」而失败。
    // 组件只在后端确认之后才改显示，这里断言失败会被显示出来。
    const failing = vi.fn(async () => {
      throw new Error("供应商协议与终端类型不匹配。");
    });
    const { view } = await renderPicker(terminal({ vendorId: "vendor-a" }), {
      onSwitch: failing
    });

    const select = view.container.querySelector<HTMLSelectElement>(".vendor-pick")!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select, "vendor-b");
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });

    await waitFor(() => {
      expect(view.container.textContent).toContain("不匹配");
    });
    // 选中的仍然是原来那一家。
    expect(select.value).toBe("vendor-a");
  });
});
