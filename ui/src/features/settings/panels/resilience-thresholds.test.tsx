// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";

import { GatewayPortCard } from "./gateway-port-card";
import type { GatewayPortStatus } from "../../../types";

/**
 * 网关端口与三条韧性阈值（原 A6 一组，批次二随「三卡重排」搬到端口卡上）。
 *
 * # 这一组补的是什么
 *
 * 后端的三个 setter（`set_failure_threshold` / `set_circuit_failure_threshold` /
 * `set_circuit_duration_seconds`）**一直是完整实现的**——边界校验与错误文案都在，
 * 只是从没登记进 `COMMANDS`。因此命令面里查不到、界面上也没有这三个输入框，
 * 用户无法调整「连续失败几次切换」「失败几次熔断」「熔断多久」。
 *
 * # 三条校验照旧平台
 *
 * | 项 | 范围 | 旧平台的原话 |
 * | --- | --- | --- |
 * | 异常切换阈值 | 1–10 | `异常切换阈值必须是 1 到 10 之间的整数。` |
 * | 熔断次数 | 1–20 | `熔断次数必须是 1 到 20 之间的整数。` |
 * | 熔断持续时间 | 10–86400 秒 | `熔断持续时间必须是 10 到 86400 秒之间的整数。` |
 *
 * # 批次二的变化：逐项保存 → 集中保存
 *
 * 现在只有卡片头那一个「保存网关与熔断配置」按钮（demo 第 1031 行）。因此这一组
 * 测的还是同一批行为（命令名、参数值、越界拦截与错误文案），只是**点的是那一个按钮**；
 * 另外补了两条集中保存独有的行为：只发改动过的项、没改动就不发请求。
 */

const STATUS: GatewayPortStatus = {
  enabled: true,
  configuredPort: 8787,
  activePort: 8787,
  baseUrl: "http://127.0.0.1:8787",
  failureThreshold: 1,
  circuitFailureThreshold: 3,
  circuitDurationSeconds: 60
};

function setup(over: Partial<GatewayPortStatus> = {}) {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const initial: GatewayPortStatus = { ...STATUS, ...over };
  // 假后端：setter 返回**完整状态**（与真后端同形状），界面直接替换。
  let current = initial;
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        if (!name.startsWith("gateway:set-")) return null;
        const next = { ...current };
        if (name === "gateway:set-port") {
          next.configuredPort = payload?.args?.port as number;
        } else if (name === "gateway:set-failure-threshold") {
          next.failureThreshold = payload?.args?.value as number;
        } else if (name === "gateway:set-circuit-failure-threshold") {
          next.circuitFailureThreshold = payload?.args?.value as number;
        } else {
          next.circuitDurationSeconds = payload?.args?.value as number;
        }
        current = next;
        return next;
      }
    },
    event: { listen: async () => () => undefined }
  };
  const onError = vi.fn();
  const onNotify = vi.fn();
  /** 外层持有状态：卡片由父级传 `status`，与 `GatewayPanel` 里的接法一致。 */
  function Harness() {
    const [status, setStatus] = useState<GatewayPortStatus>(initial);
    return (
      <GatewayPortCard
        status={status}
        onStatusChange={setStatus}
        onError={onError}
        onNotify={onNotify}
      />
    );
  }
  const view = render(<Harness />);
  return { view, calls, onError, onNotify };
}

function input(container: HTMLElement, ariaLabel: string) {
  return container.querySelector<HTMLInputElement>(`input[aria-label="${ariaLabel}"]`)!;
}

function setValue(element: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
}

/** 集中保存按钮（卡片头那一个）。 */
function saveButton(container: HTMLElement) {
  return [...container.querySelectorAll("button")].find((item) =>
    (item.textContent ?? "").includes("保存网关与熔断配置")
  );
}

async function save(view: { container: HTMLElement }) {
  await act(async () => {
    saveButton(view.container)!.click();
  });
}

describe("GatewayPortCard：端口与三个阈值", () => {
  it("**状态里的值填进输入框**（不是硬编码的默认值）", () => {
    const { view } = setup();
    expect(input(view.container, "配置端口").value).toBe("8787");
    expect(input(view.container, "异常切换阈值").value).toBe("1");
    expect(input(view.container, "熔断次数").value).toBe("3");
    expect(input(view.container, "熔断持续时间").value).toBe("60");
  });

  it("三条说明文案都在（这三个数字不直观，旧平台每个都配了一句）", () => {
    const { view } = setup();
    const text = view.container.textContent ?? "";
    expect(text).toContain("连续请求失败达到该次数后");
    expect(text).toContain("累计失败达到次数后进入熔断状态");
    expect(text).toContain("到期后进入半开状态");
  });

  it("**活跃端口读数与端口回退说明**都在（demo 第 1042-1044 行）", () => {
    const { view } = setup();
    const text = view.container.textContent ?? "";
    expect(text).toContain("活跃端口 8787 · 监听中");
    // 「端口被占用会自动回退」：不说这句，用户会以为填了固定端口就一定绑得上。
    expect(text).toContain("自动回退");
    expect(text).toContain("重启应用");
  });

  it("没在监听时读数是「未监听」", () => {
    const { view } = setup({ activePort: 0, baseUrl: "" });
    expect(view.container.textContent).toContain("未监听");
  });

  it("**保存异常切换阈值**，按真实命令名与参数", async () => {
    const { view, calls, onNotify } = setup();
    await act(async () => {
      setValue(input(view.container, "异常切换阈值"), "5");
    });
    await save(view);
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-failure-threshold")).toBe(true);
    });
    const call = calls.find((item) => item.command === "gateway:set-failure-threshold")!;
    expect(call.args.value).toBe(5);
    expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("异常切换阈值"));
  });

  it("**熔断持续时间**走另一条命令，且带上它自己的范围", async () => {
    const { view, calls } = setup();
    await act(async () => {
      setValue(input(view.container, "熔断持续时间"), "300");
    });
    await save(view);
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-circuit-duration-seconds")).toBe(
        true
      );
    });
    expect(
      calls.find((item) => item.command === "gateway:set-circuit-duration-seconds")!.args.value
    ).toBe(300);
  });

  it("**只发改动过的项**：一次改两项就发两个命令，没改的一项不发", async () => {
    const { view, calls } = setup();
    await act(async () => {
      setValue(input(view.container, "异常切换阈值"), "5");
      setValue(input(view.container, "熔断次数"), "8");
    });
    await save(view);
    await waitFor(() => {
      expect(calls.filter((item) => item.command.startsWith("gateway:set-")).length).toBe(2);
    });
    expect(calls.map((item) => item.command)).toEqual([
      "gateway:set-failure-threshold",
      "gateway:set-circuit-failure-threshold"
    ]);
  });

  it("**一项都没改时连请求都不发**，并说清没有改动", async () => {
    const { view, calls, onNotify } = setup();
    await save(view);
    expect(calls.filter((item) => item.command.startsWith("gateway:set-")).length).toBe(0);
    expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("没有改动"));
  });

  it("**越界时本地就拦下**，不发请求，并说清范围", async () => {
    const { view, calls, onError } = setup();
    await act(async () => {
      setValue(input(view.container, "熔断次数"), "21");
    });
    await save(view);
    expect(onError).toHaveBeenCalledWith("熔断次数必须是 1 到 20 之间的整数。");
    expect(calls.some((item) => item.command.startsWith("gateway:set-"))).toBe(false);
  });

  it("**三条范围各不相同**（不能共用一条「1 到 N」的规则）", async () => {
    // 边界值逐个试：10 对异常切换阈值合法、对熔断次数也合法，
    // 但 86400 只对持续时间合法 —— 混用会把这类值判错。
    const cases: Array<[string, string, boolean]> = [
      ["异常切换阈值", "10", true],
      ["异常切换阈值", "11", false],
      ["熔断次数", "20", true],
      ["熔断次数", "21", false],
      ["熔断持续时间", "10", true],
      ["熔断持续时间", "9", false],
      ["熔断持续时间", "86400", true],
      ["熔断持续时间", "86401", false]
    ];
    for (const [label, value, ok] of cases) {
      const { view, calls, onError } = setup();
      await act(async () => {
        setValue(input(view.container, label), value);
      });
      await save(view);
      const sent = calls.some((item) => item.command.startsWith("gateway:set-"));
      expect(sent, `${label}=${value}`).toBe(ok);
      if (!ok) expect(onError, `${label}=${value}`).toHaveBeenCalled();
    }
  });

  it("保存后界面上显示的是**后端回的值**", async () => {
    const { view } = setup();
    await act(async () => {
      setValue(input(view.container, "熔断次数"), "8");
    });
    await save(view);
    // 断言的是**没有因为替换状态而变成空**，且与后端回的值一致。
    await waitFor(() => {
      expect(input(view.container, "熔断次数").value).toBe("8");
    });
  });

  it("**端口保存**走 `gateway:set-port`，并提示重启后生效", async () => {
    const { view, calls, onNotify } = setup();
    await act(async () => {
      setValue(input(view.container, "配置端口"), "9000");
    });
    await save(view);
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-port")).toBe(true);
    });
    expect(calls.find((item) => item.command === "gateway:set-port")!.args.port).toBe(9000);
    // 端口改了但**不会立刻生效**（要重启才重绑），这句话不能说丢。
    expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("重启应用后生效"));
  });

  it("端口填 0 = 自动分配，提示里说清这一点", async () => {
    const { view, calls, onNotify } = setup();
    await act(async () => {
      setValue(input(view.container, "配置端口"), "0");
    });
    await save(view);
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-port")).toBe(true);
    });
    expect(calls.find((item) => item.command === "gateway:set-port")!.args.port).toBe(0);
    expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("自动分配"));
  });

  it("端口越界也在本地拦下（后端原话是「0 到 65535」）", async () => {
    const { view, calls, onError } = setup();
    await act(async () => {
      setValue(input(view.container, "配置端口"), "65536");
    });
    await save(view);
    expect(onError).toHaveBeenCalledWith(
      "网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。"
    );
    expect(calls.some((item) => item.command.startsWith("gateway:set-"))).toBe(false);
  });
});
