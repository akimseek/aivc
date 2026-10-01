// @vitest-environment jsdom
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { GatewayPanel } from "./gateway-panel";
import type { GatewayPortStatus } from "../../../types";

/**
 * 网关页的**整页门禁**。
 *
 * # 为什么这一组非有不可
 *
 * 这一页要读两份状态（网关端口、外部 API），而它们**互不依赖**。此前两份用
 * `Promise.all` 捆在一起，于是 `get-external-api-status` 一失败整页就永远停在
 * 「读取网关状态…」——端口与熔断阈值那一团明明读得到、也用得上，用户却只能对着
 * 一句不会变的话等下去。现在：外部那份退化成「关着且没有令牌」继续渲染，但卡片上
 * 会写明这是占位值并给出重试；端口那份读不到是真的没有页面可渲染，同样给重试入口
 * （而不是一直等下去）。
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
        // 与后端一致：带分页参数时返回 `{ items, ... }` 外壳（「最近请求」走这条路）。
        if (name === "gateway:query-logs") {
          const paged = "page" in (payload?.args ?? {});
          return paged ? { items: [], total: 0, page: 1, pageSize: 50 } : [];
        }
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
  it("**外部状态读失败时端口卡照常渲染**，并在卡片上写明是占位值 + 给重试", async () => {
    const onError = vi.fn();
    let fail = true;
    fakeRuntime({
      "gateway:get-port": () => PORT,
      "gateway:get-external-api-status": () => {
        if (fail) throw new Error("设置读取失败");
        return { enabled: true, hasToken: true };
      }
    });
    const view = render(<GatewayPanel onError={onError} />);

    await waitFor(() => expect(view.container.textContent).toContain("网关端口与熔断韧性参数"));
    expect(view.container.textContent).not.toContain("读取网关状态…");
    // 外部那份退化成「关着且没有令牌」这一档，卡片本身照常在。
    expect(view.container.textContent).toContain("尚未生成访问令牌");
    // 退化值**必须在界面上可辨**：它看起来与真值没有任何区别，而 onError
    // 只是一条说完就没的提示。
    await waitFor(() =>
      expect(view.container.textContent).toContain("外部网关状态读取失败")
    );
    expect(onError).toHaveBeenCalledWith("设置读取失败");

    // 重试入口（这次让命令成功）：占位值换成真值，失败提示跟着消失。
    fail = false;
    await act(async () => {
      findButton(view.container, "重新读取")!.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("外部网关已开启"));
    expect(view.container.textContent).not.toContain("外部网关状态读取失败");
  });

  it("端口状态读失败时**给出重试入口**，而不是永远停在「读取网关状态…」", async () => {
    const onError = vi.fn();
    let fail = true;
    fakeRuntime({
      "gateway:get-port": () => {
        if (fail) throw new Error("设置读取失败");
        return PORT;
      },
      "gateway:get-external-api-status": () => ({ enabled: true, hasToken: true })
    });
    const view = render(<GatewayPanel onError={onError} />);

    await waitFor(() => expect(onError).toHaveBeenCalledWith("设置读取失败"));
    // 失败与「还在读」要长得不一样：此前两者都是同一句「读取网关状态…」。
    await waitFor(() => expect(view.container.textContent).toContain("读不到网关状态"));
    expect(view.container.textContent).toContain("设置读取失败");

    fail = false;
    await act(async () => {
      findButton(view.container, "重试")!.click();
    });
    await waitFor(() =>
      expect(view.container.textContent).toContain("网关端口与熔断韧性参数")
    );
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
