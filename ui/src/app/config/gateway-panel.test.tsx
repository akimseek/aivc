// @vitest-environment jsdom
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { GatewayPanel } from "./gateway-panel";
import type { GatewayPortStatus } from "../../types";

/**
 * 网关页的**整页门禁**。
 *
 * # 为什么这一组非有不可
 *
 * 这一页要读两份状态（网关端口、外部 API），而它们**互不依赖**。此前两份用
 * `Promise.all` 捆在一起，于是 `get-external-api-status` 一失败整页就永远停在
 * 「读取网关状态…」——端口与熔断阈值那一团明明读得到、也用得上，用户却只能对着
 * 一句不会变的话等下去。现在：外部那份退化成「关着且没有令牌」继续渲染（失败照报），
 * 端口那份读不到才是真的没有页面可渲染。
 *
 * 顺带钉住总开关：切换在途时它必须是禁用的，否则第二下读到的仍是切换前的状态，
 * 会把刚改完的又翻回去。
 */

const PORT: GatewayPortStatus = {
  enabled: true,
  configuredPort: 8787,
  activePort: 8787,
  baseUrl: "http://127.0.0.1:8787",
  failureThreshold: 1,
  circuitFailureThreshold: 3,
  circuitDurationSeconds: 60
};

/** 装一个假的 Tauri 运行时；`answers` 里抛出的异常就是「这条命令读失败」。 */
function fakeRuntime(answers: Record<string, (args: Record<string, unknown>) => unknown>) {
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        const name = payload?.command ?? "";
        const handler = answers[name];
        if (handler) return handler(payload?.args ?? {});
        // 下面这些都是页面里那两张只读卡的取数：本组测的是门禁，给它们空数据，
        // 免得无关的读失败混进这一组断言。
        if (name === "gateway:get-vendor-health") return [];
        if (name === "gateway:query-logs") return [];
        if (name === "gateway:list-failover-rules") return [];
        if (name === "vendor:list") return [];
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
}

function findButton(container: HTMLElement, text: string) {
  return [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) => (item.textContent ?? "").trim() === text
  );
}

afterEach(cleanup);

describe("GatewayPanel：整页门禁", () => {
  it("**外部状态读失败时端口卡照常渲染**，而不是整页停在「读取网关状态…」", async () => {
    const onError = vi.fn();
    fakeRuntime({
      "gateway:get-port": () => PORT,
      "gateway:get-external-api-status": () => {
        throw new Error("设置读取失败");
      }
    });
    const view = render(<GatewayPanel onError={onError} />);

    await waitFor(() => expect(view.container.textContent).toContain("网关端口与熔断韧性参数"));
    expect(view.container.textContent).not.toContain("读取网关状态…");
    // 外部那份退化成「关着且没有令牌」这一档，卡片本身照常在。
    expect(view.container.textContent).toContain("外部网关已关闭");
    expect(view.container.textContent).toContain("尚未生成访问令牌");
    // 退化值**必须报出来**：它看起来与真值没有任何区别。
    expect(onError).toHaveBeenCalledWith("设置读取失败");
  });

  it("端口状态读失败时**确实没有页面可渲染**：保持等待态，并把原因报出去", async () => {
    const onError = vi.fn();
    fakeRuntime({
      "gateway:get-port": () => {
        throw new Error("设置读取失败");
      },
      "gateway:get-external-api-status": () => ({ enabled: true, hasToken: true })
    });
    const view = render(<GatewayPanel onError={onError} />);

    await waitFor(() => expect(onError).toHaveBeenCalledWith("设置读取失败"));
    expect(view.container.textContent).toContain("读取网关状态…");
  });

  it("**总开关在途时是禁用的**：第二下会把刚改完的状态又翻回去", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    fakeRuntime({
      "gateway:get-port": () => PORT,
      "gateway:get-external-api-status": () => ({ enabled: false, hasToken: false }),
      "gateway:set-enabled": async () => {
        await gate;
        return { ...PORT, enabled: false };
      }
    });
    const view = render(<GatewayPanel onError={vi.fn()} />);

    const pill = await waitFor(() => {
      const found = findButton(view.container, "本地网关已启用");
      expect(found).toBeTruthy();
      return found!;
    });
    await act(async () => {
      pill.click();
    });
    await waitFor(() => expect(pill.disabled).toBe(true));

    await act(async () => {
      release();
      await gate;
    });
    await waitFor(() => expect(pill.disabled).toBe(false));
  });
});
