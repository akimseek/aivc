// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AiTarget } from "../../types";
import { SettingsCenter } from "./settings-center";

/**
 * 设置中心外壳。
 *
 * 最要紧的一条：**十个面板各自都真的有分支能被打开**。只断言「导航里有十项」
 * 拦不住「点了没反应」那类缺陷——少写一个面板分支时，导航看起来仍然完好。
 *
 * # 用假 Tauri 核心而不是 mock 面板
 *
 * 面板全部用真的（它们的业务逻辑是既有资产），只把 `__TAURI__.core.invoke` 换成
 * 一张按命令名给值的表：外壳测试关心的是「面板有没有渲染出来」，不关心数据。
 */
function fakeCore(): void {
  const answers: Record<string, unknown> = {
    "vendor:list": [],
    "skill:list": [],
    "skill:list-trash": [],
    "workspace:list-presets": [],
    "gateway:query-logs": [],
    "gateway:list-failover-rules": [],
    "gateway:get-vendor-health": [],
    "gateway:get-port": null,
    "gateway:get-external-api-status": null,
    "gateway:get-usage-summary": null,
    "gateway:get-usage-report": null,
    "gateway:get-failure-diagnostics": null,
    // 环境面板在状态读回来之前只有一句「检测中…」——要给一份完整的检测结果，
    // 它的 WSL 会话目录卡才会渲染出来。
    "cli:check-environment": {
      providerId: "codex",
      targetId: "codex:wsl:Ubuntu",
      scope: "wsl",
      platform: "linux",
      minimumNodeMajor: 22,
      recommendedNodeMajor: 24,
      nodeMeetsMinimum: true,
      hasNpm: true,
      hasNvm: false,
      hasCli: true,
      nodeManagedByNvm: false,
      canInstallCli: false,
      canInstallNodeWithNvm: false,
      messages: []
    },
    "shell:path-exists": false,
    "app:get-version": "0.1.0"
  };
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string };
        return answers[payload?.command ?? ""] ?? null;
      }
    },
    event: { listen: async () => () => undefined }
  };
}

const wslTarget: AiTarget = {
  id: "codex:wsl:Ubuntu",
  provider: "codex",
  label: "Ubuntu",
  kind: "wsl",
  distro: "Ubuntu",
  available: true
};

function setup(overrides: { providerId?: ""; targetId?: string } = {}) {
  fakeCore();
  const onClose = vi.fn();
  const onQuit = vi.fn();
  const onOpenLogDirectory = vi.fn();
  const onOpenSessionSettings = vi.fn();
  const view = render(
    <SettingsCenter
      providerId={overrides.providerId ?? "codex"}
      providers={[]}
      targetId={overrides.targetId ?? "codex:wsl:Ubuntu"}
      targets={[wslTarget]}
      selectedTarget={wslTarget}
      version="0.1.0"
      onOpenSessionSettings={onOpenSessionSettings}
      onQuit={onQuit}
      onOpenLogDirectory={onOpenLogDirectory}
      onClose={onClose}
      onError={vi.fn()}
    />
  );
  const items = () =>
    [...view.container.querySelectorAll<HTMLButtonElement>(".settings-nav-item")];
  const button = (label: string) =>
    [...view.container.querySelectorAll("button")].find((item) =>
      (item.textContent ?? "").trim().startsWith(label)
    ) as HTMLButtonElement | undefined;
  return { view, items, button, onClose, onQuit, onOpenLogDirectory, onOpenSessionSettings };
}

describe("SettingsCenter 导航", () => {
  /**
   * 每一项点开之后必须看到**只有那一页才有**的内容。
   *
   * 只断言「面板区非空」拦不住两类缺陷：漏写一个分支（内容为空）与两页串线
   * （点 A 显示 B，内容仍然非空）。因此每页各钉一个标记。
   */
  const markers: Record<string, string> = {
    供应商管理: "还没有配置任何供应商。",
    "本地网关 & 路由": "读取网关状态…",
    用量报表: "这个区间内没有用量记录。",
    异常诊断: "没有符合条件的异常记录。",
    请求日志: "这个条件下没有日志。",
    "平台技能 (Skills)": "这个目标下还没有技能。",
    运行时环境: "环境正常，没有需要注意的问题。",
    工作区预设: "还没有工作区预设。",
    系统诊断: "命令状态",
    关于控制台: "React 19"
  };

  it("十个面板各有一个可点的入口，且每一个打开的都是它自己那一页", async () => {
    const { view, items } = setup();
    expect(items()).toHaveLength(10);

    for (const item of items()) {
      const label = (item.textContent ?? "").trim();
      expect(markers[label], `没有为「${label}」准备标记`).toBeTruthy();
      await act(async () => {
        item.click();
      });
      // 激活态恰好一个，且就是点的那一项。
      expect(view.container.querySelectorAll(".settings-nav-item.on")).toHaveLength(1);
      expect(view.container.querySelector(".settings-nav-item.on")).toBe(item);
      // 读屏那一份：当前页在 `aria-current` 上，而不是只靠 `.on`。
      expect(item.getAttribute("aria-current")).toBe("page");
      const content = view.container.querySelector(".settings-content");
      await waitFor(() => expect(content?.textContent ?? "").toContain(markers[label]));
    }
  });

  it("默认打开供应商管理，并用 aria-current 标出当前页", () => {
    const { view, items } = setup();
    const active = view.container.querySelector(".settings-nav-item.on");
    expect(active?.textContent).toBe("供应商管理");
    // `.on` 只是视觉高亮，读屏读不到——当前页要写在 `aria-current` 上。
    expect(active?.getAttribute("aria-current")).toBe("page");
    expect(items().filter((item) => item.hasAttribute("aria-current"))).toHaveLength(1);
  });

  it("**十个面板每一个都套了 `.panel-header`**（标题 + 说明），不靠各页自己拼", async () => {
    // 面板头统一是批次二的要求：只有十个都真的用上，用户在页与页之间才不会
    // 一会儿看到卡片头、一会儿看到裸标题。
    const { view, items } = setup();
    for (const item of items()) {
      const label = (item.textContent ?? "").trim();
      await act(async () => {
        item.click();
      });
      await waitFor(() => {
        const header = view.container.querySelector(".settings-content .panel-header");
        expect(header, `「${label}」没有面板头`).toBeTruthy();
        // 标题非空即可——说明可以短，但不能一个字都没有。
        expect((header!.textContent ?? "").trim().length, `「${label}」的标题是空的`).toBeGreaterThan(0);
      });
    }
  });

  it("**早返回分支也带面板头**：没有目标的技能页 / 没有平台的环境页", async () => {
    // 这两个分支都会真的出现（没选目标、没选平台时导航仍然能点进来），而此前它们
    // 只剩一句裸提示——与 `gateway-panel` 的等待态、`diagnostics-panel` 的空目标
    // 分支处理不一致，用户会以为自己离开了设置中心。
    const { view, button } = setup({ providerId: "", targetId: "" });

    await act(async () => {
      button("平台技能 (Skills)")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("请先选择目标。");
    });
    expect(view.container.querySelector(".settings-content .panel-header")).toBeTruthy();

    await act(async () => {
      button("运行时环境")!.click();
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("请先选择平台。");
    });
    expect(view.container.querySelector(".settings-content .panel-header")).toBeTruthy();
  });
});

describe("SettingsCenter：关于与关闭", () => {
  it("关于页显示版本，并在两个菜单动作上回传", async () => {
    const { view, button, onQuit, onOpenLogDirectory } = setup();
    await act(async () => {
      button("关于控制台")!.click();
    });
    await waitFor(() => expect(view.container.textContent).toContain("0.1.0"));

    await act(async () => {
      button("打开日志目录")!.click();
    });
    expect(onOpenLogDirectory).toHaveBeenCalledTimes(1);

    await act(async () => {
      button("退出控制台")!.click();
    });
    expect(onQuit).toHaveBeenCalledTimes(1);
  });

  it("标题栏的关闭走 onClose", async () => {
    const { view, onClose } = setup();
    const closeButton = view.container.querySelector(".settings-head .icon-btn") as HTMLButtonElement;
    await act(async () => {
      closeButton.click();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("Escape 关闭——键盘用户此前没有出口", async () => {
    // 这个模态是自绘的（比 `Dialog` 多一层两栏），因此没继承 `Dialog` 的 Escape。
    // 右上角那个 ✕ 是鼠标用户的出口，键盘用户只有这一条路。
    const { onClose } = setup();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("别层开着时不让 Escape 穿透到这里", async () => {
    // 面板里能再开对话框；那层自己处理 Escape。不判断的话一次按键会同时关掉
    // 内层对话框与整个设置中心。
    const { onClose } = setup();
    const nested = document.createElement("div");
    nested.className = "dialog-overlay";
    document.body.appendChild(nested);
    try {
      await act(async () => {
        document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
      });
      expect(onClose).not.toHaveBeenCalled();
    } finally {
      nested.remove();
    }
  });
});

describe("SettingsCenter：环境面板的 WSL 会话目录入口", () => {
  it("WSL 目标上打开环境页能拿到「设置会话目录…」，点了回传意图", async () => {
    const { button, onOpenSessionSettings } = setup();
    await act(async () => {
      button("运行时环境")!.click();
    });
    await waitFor(() => expect(button("设置会话目录")).toBeTruthy());
    await act(async () => {
      button("设置会话目录")!.click();
    });
    expect(onOpenSessionSettings).toHaveBeenCalledTimes(1);
  });
});
