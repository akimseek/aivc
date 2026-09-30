// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { EnvironmentPanel } from "./environment-panel";
import type { AiTarget, CliEnvironmentStatus } from "../../types";

/**
 * CLI 环境面板。
 *
 * # 这一组是为什么来的
 *
 * 后端 `CliEnvironmentStatus` 发二十来个字段，而前端类型此前只声明了十个，
 * **正好把 `minimumNodeMajor` / `recommendedNodeMajor` / `nvmVersion` / `hasNvm`
 * / `cliVersion` 裁掉了**。于是「官方 Node 最低版本」「nvm 状态」这些后端早就在发的
 * 信息，在界面上根本不存在——而且两边都能编译，没有任何提示。
 *
 * 同时 `cli:install` 早就支持 `nodeMajor` 参数（默认是后端的推荐值），
 * 但界面从来没传过，因此「24 LTS 推荐 / 22 LTS 兼容」这个选择也不存在。
 *
 * 因此这里钉住三件事：**五个状态行都在**、**Node 版本可选且会传给后端**、
 * **没有 Node 但有 nvm 时仍然给得出安装入口**。
 */

function status(over: Partial<CliEnvironmentStatus> = {}): CliEnvironmentStatus {
  return {
    providerId: "codex",
    targetId: "codex:local",
    scope: "local",
    platform: "windows",
    minimumNodeMajor: 22,
    recommendedNodeMajor: 24,
    nodeVersion: "v22.11.0",
    npmVersion: "10.9.0",
    nvmVersion: "1.1.12",
    cliVersion: "0.157.1",
    nodePath: "C:\\node\\node.exe",
    npmPath: "C:\\node\\npm.cmd",
    hasNode: true,
    hasNpm: true,
    hasNvm: true,
    hasCli: false,
    nodeManagedByNvm: false,
    nodeMeetsMinimum: true,
    canInstallCli: true,
    canInstallNodeWithNvm: true,
    messages: ["未检测到 codex。"],
    ...over
  };
}

/** 造一个目标。WSL 卡只在 `kind === "wsl"` 时出现，因此两种都要测。 */
function target(over: Partial<AiTarget> = {}): AiTarget {
  return {
    id: "codex:local",
    provider: "codex",
    label: "本机",
    kind: "local",
    available: true,
    ...over
  };
}

function setup(
  over: Partial<CliEnvironmentStatus> = {},
  props: {
    selectedTarget?: AiTarget | null;
    onOpenSessionSettings?: () => void;
    /** 应用版本。由 `SettingsCenter` 传进来（面板自己不再取），默认值只为省事。 */
    version?: string;
    /** 让检测**永远不返回**，用于钉住「检测中…」那个早返回分支。 */
    pending?: boolean;
    /**
     * 当前平台是否支持会话设置。默认 true（Codex 的取值）——
     * 「不支持」那一条由专门的门控用例显式传 false。
     */
    supportsSessionSettings?: boolean;
  } = {}
) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const current = status(over);
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> };
        calls.push({ command: payload?.command ?? "", args: payload?.args ?? {} });
        if (payload?.command === "cli:check-environment") {
          if (props.pending) return new Promise<never>(() => undefined);
          return current;
        }
        if (payload?.command === "cli:install") return {};
        // 这里**没有** `app:get-version` 的应答：面板若自己去取版本，就会拿到 `null`
        // 并在界面上显示 `-`——下面「版本只有一条来源」那一条会因此红。
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  const view = render(
    <EnvironmentPanel
      providerId="codex"
      targetId="codex:local"
      selectedTarget={props.selectedTarget}
      supportsSessionSettings={props.supportsSessionSettings ?? true}
      onOpenSessionSettings={props.onOpenSessionSettings}
      version={props.version ?? "0.1.0"}
      onError={vi.fn()}
    />
  );
  const button = (label: string) =>
    [...view.container.querySelectorAll("button")].find(
      (item) => (item.textContent ?? "").trim().startsWith(label)
    ) as HTMLButtonElement | undefined;
  const select = () => view.container.querySelector("select") as HTMLSelectElement | null;
  return { view, calls, button, select };
}

describe("EnvironmentPanel：后端发了但此前被裁掉的字段", () => {
  it("五个状态行都在（官方 Node / Node / npm / nvm / CLI）", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.textContent).toContain("官方 Node");
    });
    const text = view.container.textContent ?? "";
    for (const label of ["官方 Node", "Node", "npm", "nvm", "CLI"]) {
      expect(text).toContain(label);
    }
  });

  it("**nvm 那一行显示真实版本**（此前界面上没有这一行）", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.textContent).toContain("1.1.12");
    });
  });

  it("官方 Node 显示的是最低版本而不是推荐版本", async () => {
    const { view } = setup({ minimumNodeMajor: 22, recommendedNodeMajor: 24 });
    await waitFor(() => {
      expect(view.container.textContent).toContain("22+");
    });
  });

  it("CLI 版本显示出来", async () => {
    const { view } = setup({ cliVersion: "1.2.3" });
    await waitFor(() => {
      expect(view.container.textContent).toContain("1.2.3");
    });
  });

  it("目标环境与平台都写出来", async () => {
    const { view } = setup();
    await waitFor(() => {
      expect(view.container.textContent).toContain("windows");
    });
    expect(view.container.textContent).toContain("codex:local");
  });

  it("Node 由 nvm 管理时说清切换会影响别的项目", async () => {
    const { view } = setup({ nodeManagedByNvm: true });
    await waitFor(() => {
      expect(view.container.textContent).toContain("由 nvm 管理");
    });
  });
});

describe("EnvironmentPanel：Node 大版本选择", () => {
  it("两个选项就是推荐与兼容两个 LTS", async () => {
    const { select } = setup({ minimumNodeMajor: 22, recommendedNodeMajor: 24 });
    await waitFor(() => expect(select()).toBeTruthy());
    const options = [...select()!.options].map((item) => item.textContent);
    expect(options).toEqual(["24 LTS 推荐", "22 LTS 兼容"]);
  });

  it("默认选中的是后端给的**推荐**版本，不是界面写死的", async () => {
    const { select } = setup({ minimumNodeMajor: 20, recommendedNodeMajor: 22 });
    await waitFor(() => expect(select()).toBeTruthy());
    expect(select()!.value).toBe("22");
    expect([...select()!.options].map((item) => item.textContent)).toEqual([
      "22 LTS 推荐",
      "20 LTS 兼容"
    ]);
  });

  it("推荐版本与最低版本相同时不重复列一项", async () => {
    const { select } = setup({ minimumNodeMajor: 24, recommendedNodeMajor: 24 });
    await waitFor(() => expect(select()).toBeTruthy());
    expect([...select()!.options]).toHaveLength(1);
  });

  it("**选中的版本会传给后端**（此前从不传，于是没有这个选择）", async () => {
    const { calls, button, select } = setup({ minimumNodeMajor: 22, recommendedNodeMajor: 24 });
    await waitFor(() => expect(select()).toBeTruthy());
    // 换成 22。
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, "value")!.set!;
      setter.call(select()!, "22");
      select()!.dispatchEvent(new Event("change", { bubbles: true }));
    });
    await act(async () => {
      button("安装")!.click();
    });
    await waitFor(() => {
      const call = calls.find((item) => item.command === "cli:install");
      expect(call?.args.nodeMajor).toBe(22);
    });
  });
});

describe("EnvironmentPanel：安装入口与文案", () => {
  it("能直接装 CLI 时文案是「安装 CLI」", async () => {
    const { button } = setup({ canInstallCli: true });
    await waitFor(() => expect(button("安装")).toBeTruthy());
    expect(button("安装")!.textContent).toContain("安装 CLI");
  });

  it("**没有 Node 但有 nvm 时仍然给得出安装入口**", async () => {
    // 此前只判 `canInstallCli`，于是这种环境**根本拿不到按钮**——
    // 而那正是最需要这个按钮的情况。旧平台的条件是二者取或。
    const { button } = setup({
      hasNode: false,
      hasCli: false,
      nodeMeetsMinimum: false,
      canInstallCli: false,
      canInstallNodeWithNvm: true
    });
    await waitFor(() => expect(button("安装 Node")).toBeTruthy());
    expect(button("安装 Node")!.textContent).toContain("并安装 CLI");
  });

  it("两者都不可行时不给按钮（点了必然失败）", async () => {
    const { button, view } = setup({
      hasCli: false,
      canInstallCli: false,
      canInstallNodeWithNvm: false
    });
    await waitFor(() => {
      expect(view.container.textContent).toContain("未检测到 codex");
    });
    expect(button("安装")).toBeUndefined();
  });
});

/**
 * 「设置会话」的新入口（被移除的菜单栏那一项，spec §11）。
 *
 * 只有 WSL 目标有 `distro` 可写，因此卡片只对 WSL 目标出现；本机目标上出现它
 * 只会让人以为漏配了什么。
 */
describe("EnvironmentPanel：WSL 会话目录入口", () => {
  it("WSL 目标时显示「设置会话目录…」并回传意图", async () => {
    const onOpenSessionSettings = vi.fn();
    const { button, view } = setup(
      {},
      {
        selectedTarget: target({
          id: "codex:wsl:Ubuntu",
          label: "Ubuntu",
          kind: "wsl",
          distro: "Ubuntu"
        }),
        onOpenSessionSettings
      }
    );
    await waitFor(() => expect(button("设置会话目录")).toBeTruthy());
    expect(view.container.textContent).toContain("Ubuntu");
    await act(async () => {
      button("设置会话目录")!.click();
    });
    expect(onOpenSessionSettings).toHaveBeenCalledTimes(1);
  });

  it("本机目标不显示", async () => {
    const { button } = setup({}, { selectedTarget: target(), onOpenSessionSettings: vi.fn() });
    await waitFor(() => expect(button("重新检测")).toBeTruthy());
    expect(button("设置会话目录")).toBeUndefined();
  });

  it("**平台不支持会话设置时不显示**（claude/qoder 的 WSL 目标也不显示）", async () => {
    // 后端对 claude/qoder `sessionSettings=false`：放行的话保存写的是 Codex 的全局
    // 设置（`codex:set-wsl-codex-home`），点了必然失败——所以即便目标是 WSL、
    // 入口也接上了，这张卡仍然不该出现。去掉面板里的能力条件这条必红。
    const { button, view } = setup(
      {},
      {
        supportsSessionSettings: false,
        selectedTarget: target({
          id: "claude:wsl:Ubuntu",
          provider: "claude",
          label: "Ubuntu",
          kind: "wsl",
          distro: "Ubuntu"
        }),
        onOpenSessionSettings: vi.fn()
      }
    );
    // 面板确实渲染了（不是被早返回挡住），只是这张卡不该出现。
    await waitFor(() => expect(button("重新检测")).toBeTruthy());
    expect(button("设置会话目录")).toBeUndefined();
    expect(view.container.textContent).not.toContain("WSL 会话目录");
  });

  it("没有接动作时不显示（缺入口就不渲染，而不是死按钮）", async () => {
    const { button } = setup({}, { selectedTarget: target({ kind: "wsl", distro: "Ubuntu" }) });
    await waitFor(() => expect(button("重新检测")).toBeTruthy());
    expect(button("设置会话目录")).toBeUndefined();
  });
});

describe("EnvironmentPanel：早返回分支也有面板头", () => {
  it("**「检测中…」时不是一句裸提示**（检测结果没回来前，用户仍知道自己在哪一页）", () => {
    const { view } = setup({}, { pending: true });
    expect(view.container.querySelector(".panel-header")).toBeTruthy();
    expect(view.container.textContent).toContain("检测中…");
  });
});

/**
 * 应用版本**全应用只取一次**（`app.tsx`），经 `SettingsCenter` 的 `version` prop
 * 传到这一页与关于页。
 *
 * 此前这一页自己又发了一次 `app:get-version`：版本是进程级常量，多取一次只是多一次
 * IPC，而且「读不到」的兜底分散在两处。这两条断言就是那份重复取值的守卫——
 * 面板若再自己取，第一句会因为假后端不应答而显示 `-`，第二句会直接抓到那次调用。
 */
describe("EnvironmentPanel：应用版本只有一条来源", () => {
  it("版本显示的是 prop 给的值，且面板不发 `app:get-version`", async () => {
    const { view, calls } = setup({}, { version: "9.9.9" });
    await waitFor(() => {
      expect(view.container.textContent).toContain("9.9.9");
    });
    expect(calls.some((call) => call.command === "app:get-version")).toBe(false);
  });
});
