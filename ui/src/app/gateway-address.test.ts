import { describe, expect, it } from "vitest";

import type { GatewayPortStatus } from "../types";
import { gatewayAddressHint, gatewayPortLabel, gatewayTone } from "./gateway-address";

/**
 * 网关那一格该显示什么。
 *
 * # 这一组是为什么来的
 *
 * 状态栏与工作台此前各写了一遍 `activePort || configuredPort`，而**后端根本不发
 * 这两个字段名**（它发的是 `port`）。后果是那一格永远显示「未启用」——
 * 包括网关真的开着的时候。`status-bar.tsx` 的注释甚至专门写了
 * 「把『没取到』显示成『未启用』，用户会去设置里找一个其实已经开着的网关」，
 * 而字段错位正好制造了它要避免的那个后果。
 *
 * 因此这里逐种处境钉死，尤其是**「开了但没在监听」**这一种：
 * 它此前完全没有被表达过。
 */

const status = (over: Partial<GatewayPortStatus> = {}): GatewayPortStatus => ({
  enabled: true,
  configuredPort: 0,
  activePort: 50064,
  baseUrl: "http://127.0.0.1:50064",
  ...over
});

describe("gatewayPortLabel", () => {
  it("还没取到 → 读取中（**不是**未启用）", () => {
    expect(gatewayPortLabel(null)).toBe("读取中…");
  });

  it("设置里没开 → 未启用", () => {
    expect(gatewayPortLabel(status({ enabled: false, activePort: 0, baseUrl: "" }))).toBe(
      "未启用"
    );
  });

  it("**开了但没在监听 → 已启用，未监听**", () => {
    // 这一种此前完全没有表达：端口被占、启动失败、还在启动中都会走到这里。
    // 显示成「未启用」会让人去设置里翻一个本来就是开的开关。
    expect(gatewayPortLabel(status({ activePort: 0, baseUrl: "" }))).toBe("已启用，未监听");
  });

  it("正在监听 → 显示真实端口", () => {
    expect(gatewayPortLabel(status())).toBe(":50064");
  });

  it("**端口为 0（自动分配）时不显示配置端口**", () => {
    // 这正是原来那句 `activePort || configuredPort` 的毛病：
    // 自动分配时配置端口是 0，显示 `:0` 是个永远不会有人监听的地址。
    const label = gatewayPortLabel(status({ configuredPort: 0, activePort: 0, baseUrl: "" }));
    expect(label).not.toContain(":0");
    expect(label).toBe("已启用，未监听");
  });

  it("固定端口但没有监听时也不显示那个端口", () => {
    // 「配置成 8080」与「正在 8080 上服务」是两件事，不能混着显示。
    expect(gatewayPortLabel(status({ configuredPort: 8080, activePort: 0, baseUrl: "" }))).toBe(
      "已启用，未监听"
    );
  });
});

describe("gatewayTone", () => {
  it("在监听才算 ok", () => {
    expect(gatewayTone(status())).toBe("ok");
  });

  it("开了但没监听是 warn——它需要人看一眼", () => {
    expect(gatewayTone(status({ activePort: 0, baseUrl: "" }))).toBe("warn");
  });

  it("没开或没取到都是 off", () => {
    expect(gatewayTone(status({ enabled: false }))).toBe("off");
    expect(gatewayTone(null)).toBe("off");
  });
});

describe("gatewayAddressHint", () => {
  it("**自动分配时必须提醒地址会变**", () => {
    // 用户拿一个「这次有效」的地址去配外部客户端，下次重启连不上，
    // 那种故障看起来像客户端坏了。这句提示就是为了防止它。
    const hint = gatewayAddressHint(status({ configuredPort: 0 }));
    expect(hint).toContain("自动分配");
    expect(hint).toContain("每次启动");
  });

  it("固定端口时说明它是稳定的", () => {
    const hint = gatewayAddressHint(status({ configuredPort: 8080 }));
    expect(hint).toContain("8080");
    expect(hint).toContain("不变");
  });

  it("没取到时没有可说的", () => {
    expect(gatewayAddressHint(null)).toBe("");
  });
});
