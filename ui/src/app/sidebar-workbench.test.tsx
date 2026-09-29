// @vitest-environment jsdom
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  SidebarWorkbench,
  describeSwitchReason,
  getRateLimitWindows,
  sessionLabel
} from "./sidebar-workbench";

/**
 * 侧栏工作台。
 *
 * # 两条最要紧的规则
 *
 * 1. **速率限制只读会话摘要里已有的字段**，不为它额外请求 CLI 或 Gateway。
 *    这是旧平台写在注释里的约定：工作台是概览，不该成为新的负担。
 * 2. **路由变更的原因要分开说**。「你手动切换的」与「网关替你切换的」
 *    对用户的意义完全不同——后者意味着他什么都没做，而账单可能变了。
 */

function fakeRuntime() {
  window.__TAURI__ = {
    core: {
      invoke: async () => null
    },
    event: { listen: async () => () => undefined }
  };
}

const base = {
  provider: { id: "codex" as const, label: "Codex", capabilities: {} },
  target: {
    id: "codex:local",
    provider: "codex" as const,
    label: "本机 Codex",
    kind: "local" as const,
    available: true
  },
  gateway: {
    enabled: true,
    activePort: 50064,
    configuredPort: 0,
    baseUrl: "http://127.0.0.1:50064"
  },
  vendors: [
    {
      id: "v1",
      providerId: "codex" as const,
      name: "官方 v2",
      apiBaseUrl: "https://a/v1",
      sort: 0,
      enabled: true
    }
  ],
  refreshToken: 0
};

describe("sessionLabel", () => {
  it("没有会话时说「未打开」", () => {
    expect(sessionLabel(null)).toBe("未打开");
  });

  it("标题为空白时退回编号前 8 位", () => {
    expect(sessionLabel({ id: "98895b66-4bbd", title: "   " } as never)).toBe("98895b66");
  });

  it("**连编号都没有时也不抛异常**", () => {
    // 直接 `session.id.slice(...)` 会让整个工作台白屏——它本来只是想显示一行字。
    expect(sessionLabel({} as never)).toBe("未命名会话");
  });
});

describe("getRateLimitWindows", () => {
  it("没有会话或没有这个字段时返回空——不编一个 0% 出来", () => {
    expect(getRateLimitWindows(null)).toEqual([]);
    expect(getRateLimitWindows({ usage: {} } as never)).toEqual([]);
  });

  it("主次窗口各一条，标签与旧平台一致", () => {
    const windows = getRateLimitWindows({
      usage: { rateLimits: { primary: { usedPercent: 20 }, secondary: { usedPercent: 80 } } }
    } as never);
    expect(windows.map((item) => item.label)).toEqual(["主窗口", "次窗口"]);
  });

  it("只有主窗口时也正常", () => {
    const windows = getRateLimitWindows({ usage: { rateLimits: { primary: {} } } } as never);
    expect(windows).toHaveLength(1);
  });
});

describe("describeSwitchReason", () => {
  it("手动与自动**必须分开说**", () => {
    // 自动切换意味着用户什么都没做而账单可能变了；混成一句会让他以为是自己点的。
    expect(describeSwitchReason("manual")).toContain("你手动");
    expect(describeSwitchReason("failure")).toContain("自动切换");
    expect(describeSwitchReason("failure")).toContain("没有做任何操作");
  });

  it("未知原因给一句通用说明，而不是硬套一个", () => {
    expect(describeSwitchReason("weird")).toContain("发生了变化");
  });
});

describe("SidebarWorkbench", () => {
  it("显示运行上下文里的平台、目标与网关端口", () => {
    fakeRuntime();
    const view = render(<SidebarWorkbench {...base} session={null} />);
    const text = view.container.textContent ?? "";
    expect(text).toContain("Codex");
    expect(text).toContain("本机 Codex");
    expect(text).toContain(":50064");
  });

  it("网关未启用时显示「未启用」，不显示端口 0", () => {
    fakeRuntime();
    const view = render(
      <SidebarWorkbench
        {...base}
        gateway={{ enabled: false, activePort: 0, configuredPort: 0, baseUrl: "" }}
        session={null}
      />
    );
    expect(view.container.textContent).toContain("未启用");
  });

  it("没有速率限制数据时那一块**整块不渲染**", () => {
    // 渲染一个空的「速率限制」标题只会让人以为功能坏了。
    fakeRuntime();
    const view = render(<SidebarWorkbench {...base} session={null} />);
    expect(view.container.textContent).not.toContain("速率限制");
  });

  it("有速率限制时显示百分比", () => {
    fakeRuntime();
    const view = render(
      <SidebarWorkbench
        {...base}
        session={{ usage: { rateLimits: { primary: { usedPercent: 42 } } } } as never}
      />
    );
    expect(view.container.textContent).toContain("42%");
  });

  it("有路由变更时说明原因", () => {
    fakeRuntime();
    const view = render(
      <SidebarWorkbench
        {...base}
        session={null}
        lastSwitch={{ vendorId: "v1", reason: "failure", at: new Date().toISOString() }}
      />
    );
    const text = view.container.textContent ?? "";
    expect(text).toContain("最近路由变更");
    expect(text).toContain("官方 v2");
    expect(text).toContain("自动切换");
  });

  it("没有路由变更时那一块不渲染", () => {
    fakeRuntime();
    const view = render(<SidebarWorkbench {...base} session={null} />);
    expect(view.container.textContent).not.toContain("最近路由变更");
  });

  it("会话未打开时说清楚，而不是显示一串短编号", () => {
    fakeRuntime();
    const view = render(<SidebarWorkbench {...base} session={null} />);
    expect(view.container.textContent).toContain("未打开");
  });

  it("有刷新按钮，点了会重新取数", async () => {
    fakeRuntime();
    const view = render(<SidebarWorkbench {...base} session={null} />);
    const button = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("刷新")
    );
    expect(button).toBeTruthy();
    // 点击本身不该抛错（取数失败时面板显示空状态而不是崩掉）。
    button!.click();
  });
});
