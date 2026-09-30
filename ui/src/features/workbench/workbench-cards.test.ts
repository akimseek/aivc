// @vitest-environment jsdom
import { createElement, type ComponentProps } from "react";
import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import {
  describeSwitchReason,
  getRateLimitWindows,
  progressTone,
  rateLimitText,
  sessionLabel,
  successRateText,
  vendorName
} from "./workbench-cards";
import { WorkbenchSidebar } from "./workbench-sidebar";

/**
 * 工作台：纯逻辑（卡片怎么算）+ 侧栏装配（卡片怎么摆）。
 *
 * # 为什么这个文件是 `.ts` 却还在渲染组件
 *
 * 卡片的判断（速率窗口、进度条语气、无数据文案、会话标签、路由原因）**必须**能
 * 不渲染就断言——它们是「算错一处、显示就错一处」的东西。渲染级的断言也留在
 * 同一个文件里，因为它们讲的是同一件事（这些数字最终怎么落到屏幕上）。
 * 文件后缀是 `.ts`，所以渲染用 `createElement` 而不是 JSX。
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

/** 侧栏装配的渲染入口：关闭 / 用量 / 异常三个出口在这里给空实现，测试只关心显示。 */
function renderSidebar(overrides: Partial<ComponentProps<typeof WorkbenchSidebar>> = {}) {
  fakeRuntime();
  return render(
    createElement(WorkbenchSidebar, {
      ...base,
      session: null,
      onClose: () => undefined,
      onOpenUsage: () => undefined,
      onOpenFailures: () => undefined,
      ...overrides
    })
  );
}

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

  it("**两个窗口都缺席时是空数组，不是一个 `undefined` 的洞**", () => {
    // 留着那个洞，卡片会照着它渲染出一行空壳（标签 undefined、百分比 undefined）。
    const windows = getRateLimitWindows({ usage: { rateLimits: {} } } as never);
    expect(windows).toEqual([]);
    expect(windows.every(Boolean)).toBe(true);
  });
});

describe("progressTone", () => {
  it("用满了八成转 warn——再往上就该有人管了", () => {
    expect(progressTone(80)).toBe("warn");
    expect(progressTone(95)).toBe("warn");
  });

  it("没到八成是常规色", () => {
    expect(progressTone(79)).toBe("accent");
    expect(progressTone(0)).toBe("accent");
    // 没有数字谈不上「快满了」：条根本不画，这里更不能给 warn。
    expect(progressTone(undefined)).toBe("accent");
  });
});

describe("rateLimitText", () => {
  it("没有数字时说「无数据」，不说 `undefined%`", () => {
    expect(rateLimitText({})).toBe("无数据");
  });

  it("有数字时给出百分比；带窗口时把它一起说清", () => {
    expect(rateLimitText({ usedPercent: 42 })).toBe("42%");
    expect(rateLimitText({ usedPercent: 42, windowMinutes: 60 })).toContain("42%");
    expect(rateLimitText({ usedPercent: 42, windowMinutes: 60 })).toContain("60");
  });
});

describe("vendorName", () => {
  it("认得出就翻成名字，认不出退回编号", () => {
    const vendors = base.vendors;
    expect(vendorName(vendors, "v1")).toBe("官方 v2");
    expect(vendorName(vendors, "v9")).toBe("v9");
  });
});

describe("successRateText", () => {
  it("按请求数算成功率", () => {
    expect(successRateText(200, 199)).toBe("99.5%");
  });

  it("**一条请求都没有时给 `-`**，0/0 的 `0.0%` 会被读成「全都失败了」", () => {
    expect(successRateText(0, 0)).toBe("-");
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

describe("WorkbenchSidebar", () => {
  it("显示运行上下文里的平台、目标与网关端口", () => {
    const view = renderSidebar();
    const text = view.container.textContent ?? "";
    expect(text).toContain("Codex");
    expect(text).toContain("本机 Codex");
    expect(text).toContain(":50064");
  });

  it("网关未启用时显示「未启用」，不显示端口 0", () => {
    const view = renderSidebar({
      gateway: { enabled: false, activePort: 0, configuredPort: 0, baseUrl: "" }
    });
    expect(view.container.textContent).toContain("未启用");
  });

  it("没有速率限制数据时那一块**整块不渲染**", () => {
    // 渲染一个空的「速率限制」标题只会让人以为功能坏了。
    const view = renderSidebar();
    expect(view.container.textContent).not.toContain("速率限制");
  });

  it("有速率限制时显示百分比", () => {
    const view = renderSidebar({
      session: { usage: { rateLimits: { primary: { usedPercent: 42 } } } } as never
    });
    expect(view.container.textContent).toContain("42%");
  });

  it("有路由变更时说明原因", () => {
    const view = renderSidebar({
      lastSwitch: { vendorId: "v1", reason: "failure", at: new Date().toISOString() }
    });
    const text = view.container.textContent ?? "";
    expect(text).toContain("最近路由变更");
    expect(text).toContain("官方 v2");
    expect(text).toContain("自动切换");
  });

  it("没有路由变更时那一块不渲染", () => {
    const view = renderSidebar();
    expect(view.container.textContent).not.toContain("最近路由变更");
  });

  it("会话未打开时说清楚，而不是显示一串短编号", () => {
    const view = renderSidebar();
    expect(view.container.textContent).toContain("未打开");
  });

  it("有刷新按钮，点了会重新取数", async () => {
    const view = renderSidebar();
    const button = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("刷新")
    );
    expect(button).toBeTruthy();
    // 点击本身不该抛错（取数失败时面板显示空状态而不是崩掉）。
    button!.click();
  });

  it("头部有「运行概览与工作台」与「实时监控」，关闭按钮回会话", () => {
    const onClose = { called: 0 };
    const view = renderSidebar({
      onClose: () => {
        onClose.called += 1;
      }
    });
    const text = view.container.textContent ?? "";
    expect(text).toContain("运行概览与工作台");
    expect(text).toContain("实时监控");
    const close = view.container.querySelector<HTMLButtonElement>("[aria-label='关闭工作台']")!;
    close.click();
    expect(onClose.called).toBe(1);
  });

  it("「用量明细」与「查看详情」把出口接出去", () => {
    const calls = { usage: 0, failures: 0 };
    const view = renderSidebar({
      onOpenUsage: () => {
        calls.usage += 1;
      },
      onOpenFailures: () => {
        calls.failures += 1;
      }
    });
    const buttons = [...view.container.querySelectorAll("button")];
    buttons.find((item) => item.textContent?.includes("用量明细"))!.click();
    buttons.find((item) => item.textContent?.includes("查看详情"))!.click();
    expect(calls.usage).toBe(1);
    expect(calls.failures).toBe(1);
  });

  it("侧栏自带滚动区与头部（R24）", () => {
    // 真正让卡片区滚起来的是 `.workbench-body { overflow-y: auto }`（app.css），
    // 而 jsdom 不算样式——这里守住结构：头部与内容区都在侧栏自己的 `<aside>` 里。
    // 结构在，样式规则就挂得上；结构没了，规则写得再对也没用。
    const view = renderSidebar();
    const aside = view.container.querySelector("aside.sidebar.workbench")!;
    expect(aside).toBeTruthy();
    expect(aside.querySelector(".workbench-head")).toBeTruthy();
    expect(aside.querySelector(".workbench-body")).toBeTruthy();
  });
});
