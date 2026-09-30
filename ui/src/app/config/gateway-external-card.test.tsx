// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { GatewayExternalCard, buildExternalEndpoints } from "./gateway-external-card";
import type { ExternalApiStatus } from "../../types";

/**
 * 外部网关卡：端点清单必须是**后端真实路由**，不是 demo 里的字符串。
 *
 * # 为什么要专门钉这一条（裁定 B-R11）
 *
 * demo（第 1141-1162 行）写的是 `http://127.0.0.1:22755/external/codex/v1` 这类地址，
 * 那是**旧平台**的形制（`D:/code/ai/src/electron/gateway/vendor-gateway.ts:238-240`
 * 的 `/external/<平台>`）。Rust 侧的路由前缀来自
 * `crates/aivc-gateway/src/routes.rs:33/102-114`：
 *
 * - `ROUTE_PREFIX = "/gateway"`；
 * - `route_prefix()` 拼出 `/gateway/<平台>/<路由编号>`；
 * - `build_route_url()` 把 host 的尾斜杠去掉再接上前缀。
 *
 * 因此界面上必须是 `http://127.0.0.1:<port>/gateway/<平台>/<routeId>`。
 * 照抄 demo 会让用户把一个**永远 404** 的地址填进外部客户端。
 */

const STATUS: ExternalApiStatus = { enabled: true, hasToken: true };

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

describe("buildExternalEndpoints：与 routes.rs 的形制一致", () => {
  it("四个平台各一条，形如 `<base>/gateway/<平台>/<routeId>`", () => {
    const endpoints = buildExternalEndpoints("http://127.0.0.1:8787");
    expect(endpoints.map((item) => item.url)).toEqual([
      "http://127.0.0.1:8787/gateway/codex/<routeId>",
      "http://127.0.0.1:8787/gateway/claude/<routeId>",
      "http://127.0.0.1:8787/gateway/gemini/<routeId>",
      "http://127.0.0.1:8787/gateway/qoder/<routeId>"
    ]);
  });

  it("**没有一个端点长得像旧平台的 `/external/...`**（demo 那串字符串不能照抄）", () => {
    for (const item of buildExternalEndpoints("http://127.0.0.1:8787")) {
      expect(item.url).not.toContain("/external/");
    }
  });

  it("host 的尾斜杠被去掉（`build_route_url` 就是 trim 之后拼接）", () => {
    expect(buildExternalEndpoints("http://127.0.0.1:8787///")[0]!.url).toBe(
      "http://127.0.0.1:8787/gateway/codex/<routeId>"
    );
  });

  it("没在监听（空地址）时返回空清单——编一个 `:0` 的地址只会误导", () => {
    expect(buildExternalEndpoints("")).toEqual([]);
    expect(buildExternalEndpoints("   ")).toEqual([]);
  });

  it("平台清单可裁剪（顺序即显示顺序）", () => {
    expect(buildExternalEndpoints("http://127.0.0.1:8787", ["claude"]).map((item) => item.url)).toEqual(
      ["http://127.0.0.1:8787/gateway/claude/<routeId>"]
    );
  });

  it("每个平台带上它的凭据携带方式（照各家客户端的约定）", () => {
    const byProvider = new Map(
      buildExternalEndpoints("http://127.0.0.1:8787").map((item) => [item.provider, item.auth])
    );
    expect(byProvider.get("codex")).toContain("Authorization: Bearer");
    expect(byProvider.get("claude")).toContain("x-api-key");
    expect(byProvider.get("gemini")).toContain("x-goog-api-key");
    expect(byProvider.get("gemini")).toContain("?key=");
    expect(byProvider.get("qoder")).toContain("Authorization: Bearer");
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
    expect(text).toContain("http://127.0.0.1:8787/gateway/codex/<routeId>");
    expect(text).toContain("仅监听 127.0.0.1");
    // 认证协议说明要在：它是「怎么把令牌带上」的唯一线索。
    expect(text).toContain("Authorization: Bearer");
    expect(text).toContain("x-api-key");
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
    ).toBe("http://127.0.0.1:8787/gateway/claude/<routeId>");
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
      "复制 Claude 端点",
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
    const pill = findButton(view.container, "外部网关已开启")!;
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
      findButton(view.container, "外部网关已开启")!.click();
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
