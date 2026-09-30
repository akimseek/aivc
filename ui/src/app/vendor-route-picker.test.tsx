// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { VendorRoutePicker } from "./vendor-route-picker";
import type { TerminalSession } from "./use-terminals";

/**
 * 状态栏供应商选择器（浮层）的测试。
 *
 * # 为什么它值得有测试
 *
 * 它承担的是「我现在花的是谁的钱」——走网关之后供应商可能被**自动**换掉，
 * 界面不显示或显示错，用户只能从账单上发现。它的分支又不少：直连、有绑定、
 * 当前供应商已停用、「已自动切换」徽标只对 `failure` 出现、健康快照取不到时
 * **静默省略**。
 *
 * # 这一轮从 `<select>` 改成了浮层
 *
 * 断言因此从「下拉框的值」改成三条交互：**点触发按钮打开浮层**、
 * **点列表项请求切换**、**锁定复选框回传模式**。
 */

type Handler = (event: { payload: unknown }) => void;

/** 装一个假的 Tauri 运行时；`answers` 按命令名给返回值，值为 `"fail"` 表示这条命令报错。 */
function fakeRuntime(answers: Record<string, unknown>) {
  let invocations: { command: string; args: unknown }[] = [];

  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        const payload = args as { command?: string } | undefined;
        const name = payload?.command ?? command;
        invocations.push({ command: name, args });
        const answer = answers[name];
        if (answer === "fail") throw new Error(`${name} 失败`);
        return answer ?? {};
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
  options: {
    vendors?: unknown[];
    /** 健康快照的返回；`"fail"` 表示这条命令报错。 */
    health?: unknown;
    onSwitch?: () => Promise<void>;
    onSetMode?: () => Promise<void>;
  } = {}
) {
  fakeRuntime({
    "vendor:list": options.vendors ?? VENDORS,
    "gateway:get-vendor-health": options.health ?? []
  });
  const onSwitch = options.onSwitch ?? vi.fn(async () => undefined);
  const onSetMode = options.onSetMode ?? vi.fn(async () => undefined);
  const view = render(
    <VendorRoutePicker terminal={current} onSwitch={onSwitch} onSetMode={onSetMode} />
  );
  // 供应商清单是异步取的。等它落进浮层里再继续——否则后面的断言会跑在空列表上。
  if (current?.vendorId) {
    await waitFor(() => expect(view.container.querySelector(".vendor-trigger")).not.toBeNull());
  }
  return { view, onSwitch, onSetMode };
}

/** 点触发按钮，等浮层出现。 */
async function openPopover(view: ReturnType<typeof render>) {
  const trigger = view.container.querySelector<HTMLButtonElement>(".vendor-trigger")!;
  await act(async () => {
    trigger.click();
  });
  await waitFor(() => expect(view.container.querySelector(".vendor-popover")).not.toBeNull());
}

/** 按名字点某一项供应商。 */
async function clickVendor(view: ReturnType<typeof render>, name: string) {
  const option = [...view.container.querySelectorAll<HTMLButtonElement>(".vendor-option")].find(
    (item) => (item.textContent ?? "").includes(name)
  );
  expect(option, `浮层里没有「${name}」`).toBeTruthy();
  await act(async () => {
    option!.click();
  });
}

describe("VendorRoutePicker", () => {
  it("没有终端时不渲染任何东西", () => {
    const view = render(
      <VendorRoutePicker terminal={undefined} onSwitch={vi.fn()} onSetMode={vi.fn()} />
    );
    expect(view.container.textContent).toBe("");
  });

  it("没有 providerId 时不显示供应商", () => {
    // 终端都不知道自己属于哪个平台时，取不到供应商列表，选择器无从谈起。
    const view = render(
      <VendorRoutePicker
        terminal={terminal({ providerId: "" })}
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
    // 直连时没有可切换的东西，因此没有浮层触发按钮（也就没有死按钮）。
    expect(view.container.querySelector(".vendor-trigger")).toBeNull();
  });

  it("点触发按钮打开浮层，当前项标「活跃」、其余标「候选」", async () => {
    const { view } = await renderPicker(terminal({ vendorId: "vendor-a" }));

    // 打开之前浮层不在 DOM 里。
    expect(view.container.querySelector(".vendor-popover")).toBeNull();
    await openPopover(view);

    const options = [...view.container.querySelectorAll(".vendor-option")];
    expect(options).toHaveLength(2);
    const active = options.find((item) => item.className.includes("on"));
    expect(active?.textContent).toContain("甲");
    expect(active?.textContent).toContain("活跃");
    expect(
      options.find((item) => (item.textContent ?? "").includes("乙"))?.textContent
    ).toContain("候选");
  });

  it("点列表项请求切换，成功后收起浮层", async () => {
    const { view, onSwitch } = await renderPicker(terminal({ vendorId: "vendor-a" }));
    await openPopover(view);

    await clickVendor(view, "乙");

    expect(onSwitch).toHaveBeenCalledWith("term-1", "vendor-b");
    // 切完就收起：值由 `gateway:vendor-switched` 推回，浮层留在屏幕上只会挡住状态栏。
    await waitFor(() => expect(view.container.querySelector(".vendor-popover")).toBeNull());
  });

  it("锁定复选框：勾上回传 locked，取消回传 dynamic", async () => {
    const { view, onSetMode } = await renderPicker(terminal({ vendorId: "vendor-a" }));
    await openPopover(view);

    const box = view.container.querySelector<HTMLInputElement>(".vendor-lock input")!;
    expect(box.checked).toBe(false);

    await act(async () => {
      box.click();
    });
    expect(onSetMode).toHaveBeenCalledWith("term-1", "locked");

    const locked = await renderPicker(
      terminal({ vendorId: "vendor-a", vendorMode: "locked" })
    );
    await openPopover(locked.view);
    const box2 = locked.view.container.querySelector<HTMLInputElement>(".vendor-lock input")!;
    expect(box2.checked).toBe(true);
    await act(async () => {
      box2.click();
    });
    expect(locked.onSetMode).toHaveBeenCalledWith("term-1", "dynamic");
  });

  it("锁定时触发按钮带「锁定」徽章", async () => {
    const { view } = await renderPicker(terminal({ vendorId: "vendor-a", vendorMode: "locked" }));
    expect(view.container.querySelector(".vendor-trigger")?.textContent).toContain("锁定");
  });

  it("当前供应商已停用时仍然列出来并标明", async () => {
    // 供应商可能刚被禁用或删掉。浮层**不能悄悄跳过它**——那会让用户以为绑定还在。
    const { view } = await renderPicker(terminal({ vendorId: "vendor-gone" }));
    await openPopover(view);

    const gone = [...view.container.querySelectorAll(".vendor-option")].find((item) =>
      (item.textContent ?? "").includes("vendor-gone")
    );
    expect(gone).toBeTruthy();
    expect(gone?.textContent).toContain("已停用");
  });

  it("取到健康快照时标注「熔断中」，「网关在线」随之显示", async () => {
    // 健康是可选增强：取到时用它标出异常的供应商，用户不必等到请求失败才知道。
    const { view } = await renderPicker(terminal({ vendorId: "vendor-a" }), {
      health: [
        {
          vendorId: "vendor-b",
          providerId: "codex",
          status: "open",
          failureCount: 5,
          successCount: 1,
          failureRate: 0.8
        }
      ]
    });
    await openPopover(view);

    await waitFor(() => {
      expect(view.container.textContent).toContain("熔断中");
    });
    expect(view.container.textContent).toContain("网关在线");
  });

  it("健康快照取不到时**静默省略**，不影响切换", async () => {
    // 后端那张健康表在某些版本里恒为空、命令也可能失败。为它弹一条错误会把
    // 「切换供应商」这件正事盖掉。
    const { view, onSwitch } = await renderPicker(terminal({ vendorId: "vendor-a" }), {
      health: "fail"
    });
    await openPopover(view);

    // 没有健康信息，也没有报错文案。
    expect(view.container.textContent).not.toContain("熔断中");
    expect(view.container.textContent).not.toContain("网关在线");
    expect(view.container.querySelector(".err")).toBeNull();

    await clickVendor(view, "乙");
    expect(onSwitch).toHaveBeenCalledWith("term-1", "vendor-b");
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

  it("切换失败时不谎报成功：错误显示出来、浮层不关", async () => {
    // 切换会因为「供应商被禁用 / 配置不全 / 平台不匹配」而失败。
    // 组件只在后端确认之后才改显示，这里断言失败会被显示出来。
    const failing = vi.fn(async () => {
      throw new Error("供应商协议与终端类型不匹配。");
    });
    const { view } = await renderPicker(terminal({ vendorId: "vendor-a" }), {
      onSwitch: failing
    });
    await openPopover(view);

    await clickVendor(view, "乙");

    await waitFor(() => {
      expect(view.container.textContent).toContain("不匹配");
    });
    // 失败时留在原地：用户还要看错误、再试另一家。
    expect(view.container.querySelector(".vendor-popover")).not.toBeNull();
    // 当前项仍然是原来那一家——界面不预改。
    const active = view.container.querySelector(".vendor-option.on");
    expect(active?.textContent).toContain("甲");
  });
});
