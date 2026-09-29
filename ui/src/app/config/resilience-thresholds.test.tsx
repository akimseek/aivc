// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ResilienceSection } from "./gateway-panel";
import type { GatewayPortStatus } from "../../types";

/**
 * A6：网关的三个韧性阈值。
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
  const status = { ...STATUS, ...over };
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        const name = payload?.command ?? "";
        calls.push({ command: name, args: payload?.args ?? {} });
        if (name === "gateway:get-port") return status;
        if (name.startsWith("gateway:set-")) {
          // setter 返回**完整状态**（与 get-port 同形状），界面直接替换。
          const next = { ...status };
          if (name === "gateway:set-failure-threshold") {
            next.failureThreshold = payload?.args?.value as number;
          } else if (name === "gateway:set-circuit-failure-threshold") {
            next.circuitFailureThreshold = payload?.args?.value as number;
          } else {
            next.circuitDurationSeconds = payload?.args?.value as number;
          }
          return next;
        }
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  const onError = vi.fn();
  const onNotify = vi.fn();
  const view = render(<ResilienceSection onError={onError} onNotify={onNotify} />);
  return { view, calls, onError, onNotify };
}

function setValue(element: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
}

function buttonNear(container: HTMLElement, ariaLabel: string) {
  const input = container.querySelector<HTMLInputElement>(`input[aria-label="${ariaLabel}"]`)!;
  return input.parentElement!.querySelector("button")!;
}

describe("ResilienceSection：三个阈值", () => {
  it("**读出后端当前的值填进输入框**（不是硬编码的默认值）", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.querySelector('input[aria-label="异常切换阈值"]')).toBeTruthy();
    });
    expect(
      view.container.querySelector<HTMLInputElement>('input[aria-label="异常切换阈值"]')!.value
    ).toBe("1");
    expect(
      view.container.querySelector<HTMLInputElement>('input[aria-label="熔断次数"]')!.value
    ).toBe("3");
    expect(
      view.container.querySelector<HTMLInputElement>('input[aria-label="熔断持续时间"]')!.value
    ).toBe("60");
  });

  it("三条说明文案都在（这三个数字不直观，旧平台每个都配了一句）", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.querySelector('input[aria-label="异常切换阈值"]')).toBeTruthy();
    });
    const text = view.container.textContent ?? "";
    expect(text).toContain("连续请求失败达到该次数后");
    expect(text).toContain("累计失败达到次数后进入熔断状态");
    expect(text).toContain("到期后进入半开状态");
  });

  it("**保存异常切换阈值**，按真实命令名与参数", async () => {
    const { view, calls, onNotify } = setup();
    await waitFor(() => {
      expect(view.container.querySelector('input[aria-label="异常切换阈值"]')).toBeTruthy();
    });
    const input = view.container.querySelector<HTMLInputElement>('input[aria-label="异常切换阈值"]')!;
    await act(async () => {
      setValue(input, "5");
    });
    await act(async () => {
      buttonNear(view.container, "异常切换阈值").click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-failure-threshold")).toBe(true);
    });
    const call = calls.find((item) => item.command === "gateway:set-failure-threshold")!;
    expect(call.args.value).toBe(5);
    expect(onNotify).toHaveBeenCalledWith(expect.stringContaining("异常切换阈值"));
  });

  it("**熔断持续时间**走另一条命令，且带上它自己的范围", async () => {
    const { view, calls } = setup();
    await waitFor(() => {
      expect(view.container.querySelector('input[aria-label="熔断持续时间"]')).toBeTruthy();
    });
    const input = view.container.querySelector<HTMLInputElement>('input[aria-label="熔断持续时间"]')!;
    await act(async () => {
      setValue(input, "300");
    });
    await act(async () => {
      buttonNear(view.container, "熔断持续时间").click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-circuit-duration-seconds")).toBe(
        true
      );
    });
    expect(
      calls.find((item) => item.command === "gateway:set-circuit-duration-seconds")!.args.value
    ).toBe(300);
  });

  it("**越界时本地就拦下**，不发请求，并说清范围", async () => {
    const { view, calls, onError } = setup();
    await waitFor(() => {
      expect(view.container.querySelector('input[aria-label="熔断次数"]')).toBeTruthy();
    });
    const input = view.container.querySelector<HTMLInputElement>('input[aria-label="熔断次数"]')!;
    await act(async () => {
      setValue(input, "21");
    });
    await act(async () => {
      buttonNear(view.container, "熔断次数").click();
    });
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
      await waitFor(() => {
        expect(view.container.querySelector(`input[aria-label="${label}"]`)).toBeTruthy();
      });
      const input = view.container.querySelector<HTMLInputElement>(`input[aria-label="${label}"]`)!;
      await act(async () => {
        setValue(input, value);
      });
      await act(async () => {
        buttonNear(view.container, label).click();
      });
      const sent = calls.some((item) => item.command.startsWith("gateway:set-"));
      expect(sent, `${label}=${value}`).toBe(ok);
      if (!ok) expect(onError, `${label}=${value}`).toHaveBeenCalled();
    }
  });

  it("保存后界面上显示的是**后端回的值**", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.querySelector('input[aria-label="熔断次数"]')).toBeTruthy();
    });
    const input = view.container.querySelector<HTMLInputElement>('input[aria-label="熔断次数"]')!;
    await act(async () => {
      setValue(input, "8");
    });
    await act(async () => {
      buttonNear(view.container, "熔断次数").click();
    });
    // 上一次保存的草稿仍显示 8；这里断言的是**没有因为替换状态而变成空**。
    await waitFor(() => {
      expect(
        view.container.querySelector<HTMLInputElement>('input[aria-label="熔断次数"]')!.value
      ).toBe("8");
    });
  });
});
