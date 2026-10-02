// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { GatewayExternalCard, buildExternalEndpoints } from "./gateway-external-card";
import type { AiProviderSummary, ExternalApiStatus } from "../../../types";

/**
 * 外部接入卡：端点清单 = **网关根地址 + 平台路径**。
 *
 * # 形制（2026-10-02 统一外部接入）
 *
 * 外部接入路径是 `/<平台>`（如 `http://127.0.0.1:8787/codex`），直达该平台的
 * 候选池；协议后缀（`/v1/responses` 等）由接入方按自己的协议追加、网关原样透传。
 * 设计与裁定见 `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`。
 *
 * 旧两种形制都不能用：demo 的 `/external/<平台>`（旧 Electron 版）与更早界面上的
 * `/gateway/<平台>/<routeId>` 占位符（Rust 版外部路由从未注册，那个地址必然 404）。
 */

const STATUS: ExternalApiStatus = { enabled: true, hasToken: true };

const HOST = "http://127.0.0.1:8787";

/**
 * 后端 `ai:list-providers` 的形制（顺序就是它给的顺序）。
 *
 * 端点清单与平台名都必须从这份数据派生：写死一份清单时，后端多一个平台
 * 前端不会编译报错，只会静默少一条端点。
 */
const FOUR_PROVIDERS: AiProviderSummary[] = [
  { id: "codex", label: "Codex", capabilities: {} },
  { id: "gemini", label: "Gemini", capabilities: {} },
  { id: "claude", label: "Claude Code", capabilities: {} },
  { id: "qoder", label: "Qoder", capabilities: {} }
];
const THREE_PROVIDERS = FOUR_PROVIDERS.filter((item) => item.id !== "gemini");

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

function findButton(container: HTMLElement, text: string) {
  return [...container.querySelectorAll("button")].find(
    (item) => (item.textContent ?? "").trim() === text
  );
}

describe("buildExternalEndpoints：`{base}/{平台}`，清单从 `providers` 派生", () => {
  it("**端点就是 `{网关根地址}/{平台}`**：没有 `/gateway`、没有 `/external`、没有占位符", () => {
    const endpoints = buildExternalEndpoints(HOST, FOUR_PROVIDERS);
    expect(endpoints.map((item) => item.url)).toEqual([
      `${HOST}/codex`,
      `${HOST}/gemini`,
      `${HOST}/claude`,
      `${HOST}/qoder`
    ]);
    // 显示名也取自 `providers`（与规则表同源），不从本地兜底表来。
    expect(endpoints.map((item) => item.label)).toEqual([
      "Codex",
      "Gemini",
      "Claude Code",
      "Qoder"
    ]);
  });

  it("**给三个平台就只出三条**：清单是 `providers` 的映射，不是本地那份写死的四个", () => {
    expect(buildExternalEndpoints(HOST, THREE_PROVIDERS).map((item) => item.url)).toEqual([
      `${HOST}/codex`,
      `${HOST}/claude`,
      `${HOST}/qoder`
    ]);
  });

  it("拿不到 `providers` 时退回本地兜底清单（穷举表；claude 与后端逐字一致，不再漂移）", () => {
    const endpoints = buildExternalEndpoints(HOST);
    expect(endpoints.map((item) => item.url)).toHaveLength(4);
    expect(endpoints.map((item) => item.label)).toEqual([
      "Codex",
      "Claude Code",
      "Gemini",
      "Qoder"
    ]);
  });

  it("host 的尾斜杠被去掉（`build_route_url` 就是 trim 之后拼接）", () => {
    expect(buildExternalEndpoints(`${HOST}///`)[0]!.url).toBe(`${HOST}/codex`);
  });

  it("没在监听（空地址）时返回空清单——编一个 `:0` 的地址只会误导", () => {
    expect(buildExternalEndpoints("")).toEqual([]);
    expect(buildExternalEndpoints("   ")).toEqual([]);
  });
});

describe("GatewayExternalCard", () => {
  it("端点清单渲染到界面上，并带「仅监听 127.0.0.1」的提示", () => {
    fakeRuntime({});
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    const text = view.container.textContent ?? "";
    expect(text).toContain("http://127.0.0.1:8787/codex");
    expect(text).toContain("仅监听 127.0.0.1");
    expect(text).toContain("API 基地址");
    // 凭据头列已移除（用户 2026-10-02 裁定）；地址里不能有占位符。
    expect(view.container.querySelectorAll(".gateway-endpoint-auth")).toHaveLength(0);
    expect(text).not.toContain("<routeId>");
  });

  it("端点清单与显示名都随 `providers` 走（三平台就渲染三条，名字照后端）", () => {
    fakeRuntime({});
    const view = render(
      <GatewayExternalCard
        baseUrl={HOST}
        providers={THREE_PROVIDERS}
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    const names = [
      ...view.container.querySelectorAll<HTMLElement>(".gateway-endpoint-platform")
    ].map((item) => item.textContent);
    expect(names).toEqual(["Codex", "Claude Code", "Qoder"]);
    expect(view.container.textContent).toContain(`${HOST}/qoder`);
  });

  it("没有可用地址时说清下一步，而不是给一份空清单", () => {
    fakeRuntime({});
    const view = render(
      <GatewayExternalCard
        baseUrl=""
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    expect(view.container.textContent).toContain("还没有可用地址");
  });

  it("**复制端点**走剪贴板命令，参数就是那一行的地址", async () => {
    const calls = fakeRuntime({});
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    const row = [...view.container.querySelectorAll<HTMLElement>(".gateway-endpoint")].find(
      (item) => item.textContent?.includes("Claude")
    )!;
    await act(async () => {
      [...row.querySelectorAll("button")].find((item) => item.textContent?.trim() === "复制")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "clipboard:copy-text")).toBe(true);
    });
    expect(
      calls.find((item) => item.command === "clipboard:copy-text")!.args.text
    ).toBe("http://127.0.0.1:8787/claude");
  });

  it("每一行的复制按钮带**平台名**的可访问名（四个「复制」读屏分不出谁是谁）", () => {
    fakeRuntime({});
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    const labels = [
      ...view.container.querySelectorAll<HTMLButtonElement>(".gateway-endpoint button")
    ].map((item) => item.getAttribute("aria-label"));
    expect(labels).toEqual([
      "复制 Codex 端点",
      "复制 Claude Code 端点",
      "复制 Gemini 端点",
      "复制 Qoder 端点"
    ]);
  });

  it("**切换在途时开关是禁用的**：第二下读到的还是切换前的状态，会把刚改完的又翻回去", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    fakeRuntime({
      "gateway:set-external-api-enabled": async (args) => {
        await gate;
        return { enabled: args.enabled, hasToken: true };
      }
    });
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    const pill = findButton(view.container, "外部接入已开启")!;
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

  it("**开关**走 `gateway:set-external-api-enabled`，并说清结果", async () => {
    const calls = fakeRuntime({
      "gateway:set-external-api-enabled": (args) => ({ enabled: args.enabled, hasToken: true })
    });
    const onNotify = vi.fn();
    const onStatusChange = vi.fn();
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={onStatusChange}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    await act(async () => {
      findButton(view.container, "外部接入已开启")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:set-external-api-enabled")).toBe(true);
    });
    expect(
      calls.find((item) => item.command === "gateway:set-external-api-enabled")!.args.enabled
    ).toBe(false);
    expect(onNotify).toHaveBeenCalledWith("外部 API 已关闭。");
    expect(onStatusChange).toHaveBeenCalledWith({ enabled: false, hasToken: true });
  });

  it("**轮换令牌**：明文只在这里出现一次，横幅带复制与收起", async () => {
    const calls = fakeRuntime({
      "gateway:rotate-external-api-token": () => ({ token: "sk-gw-abc123" })
    });
    const onNotify = vi.fn();
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
        onNotify={onNotify}
      />
    );
    await act(async () => {
      findButton(view.container, "轮换令牌")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "gateway:rotate-external-api-token")).toBe(true);
    });
    // 旧令牌**立即失效**，这句不说用户不会知道别的客户端已经断了。
    expect(onNotify).toHaveBeenCalledWith("已生成新令牌；旧令牌立即失效。");
    const text = view.container.textContent ?? "";
    expect(text).toContain("sk-gw-abc123");
    expect(text).toContain("再也看不到");
    // 复制新令牌。
    await act(async () => {
      findButton(view.container, "复制令牌")!.click();
    });
    await waitFor(() => {
      expect(calls.some((item) => item.command === "clipboard:copy-text")).toBe(true);
    });
    expect(calls.find((item) => item.command === "clipboard:copy-text")!.args.text).toBe(
      "sk-gw-abc123"
    );
    // 「我已保存」收起横幅：横幅存在的意义就是让用户当场复制走。
    await act(async () => {
      findButton(view.container, "我已保存")!.click();
    });
    expect(view.container.textContent).not.toContain("sk-gw-abc123");
  });

  it("**轮换令牌在途时按钮禁用**：每轮换一次旧令牌立即失效，连点会让明文与生效的对不上", async () => {
    let release!: () => void;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    const calls = fakeRuntime({
      "gateway:rotate-external-api-token": async () => {
        await gate;
        return { token: "sk-gw-once" };
      }
    });
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={STATUS}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    const button = findButton(view.container, "轮换令牌")!;
    await act(async () => {
      button.click();
    });
    await waitFor(() => expect(button.disabled).toBe(true));
    // 第二下点不动（`disabled` 的按钮不会派发 click）。
    button.click();
    expect(
      calls.filter((item) => item.command === "gateway:rotate-external-api-token")
    ).toHaveLength(1);

    await act(async () => {
      release();
      await gate;
    });
    await waitFor(() => expect(button.disabled).toBe(false));
    expect(view.container.textContent).toContain("sk-gw-once");
  });

  it("没有令牌时说清「现在连不上」（半配置状态）", () => {
    fakeRuntime({});
    const view = render(
      <GatewayExternalCard
        baseUrl="http://127.0.0.1:8787"
        status={{ enabled: true, hasToken: false }}
        onStatusChange={vi.fn()}
        onError={vi.fn()}
      />
    );
    expect(view.container.textContent).toContain("尚未生成访问令牌");
    expect(view.container.textContent).toContain("未设置");
  });
});
