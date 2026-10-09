// @vitest-environment jsdom
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { AppOverlays, type OverlaysActions, type OverlaysState } from "./overlay-layer";
import type { AiTarget } from "../types";

/**
 * 弹层的**排布顺序**。
 *
 * # 为什么这件事必须被钉住
 *
 * 这些弹层同属 `--ui-z-overlay=60` 这一档，谁盖住谁**完全由 DOM 顺序决定**。
 * 退出确认框此前排在设置中心与消息弹框之前，于是「点了窗口关闭按钮却什么都看不到」
 * ——而后端会一直等一个回答（它既不超时也不自己退出），用户只能再点一次关闭按钮。
 * 排到最后就等于「它永远在最上面」，这正是它该有的位置。
 *
 * 「设置会话」是**同形态的第二处**：它的入口就在设置中心的环境页里，而它此前也排在
 * 设置中心之前——从那一页点开之后什么都看不见。两处各钉一条断言。
 *
 * 每个用例一条断言就够：把两层同时挂上，看最后一个浮层是谁。
 */

/** 全部动作都是必填的（见 `OverlaysActions` 的说明）——这里一次性给一组桩。 */
function stubActions(): OverlaysActions {
  const stub = () => vi.fn();
  return {
    closeToast: stub(),
    closeExported: stub(),
    openExported: stub(),
    changeSessionSettingsPath: stub(),
    closeSessionSettings: stub(),
    restoreSessionSettings: stub(),
    saveSessionSettings: stub(),
    openSessionSettings: stub(),
    quit: stub(),
    openLogDirectory: stub(),
    changeNewSession: stub(),
    chooseNewSessionDirectory: stub(),
    closeNewSession: stub(),
    confirmNewSession: stub(),
    closeMenu: stub(),
    openSessionFolder: stub(),
    exportSession: stub(),
    toggleFavorite: stub(),
    copyText: stub(),
    removeSession: stub(),
    restoreSession: stub(),
    startRename: stub(),
    startDuplicate: stub(),
    closeTitleDialog: stub(),
    confirmRename: stub(),
    confirmDuplicate: stub(),
    closeTabMenu: stub(),
    closeTabThis: stub(),
    closeTabOthers: stub(),
    closeTabAll: stub(),
    closeDetail: stub(),
    resumeSession: stub(),
    branchFromMessage: stub(),
    exportDetail: stub(),
    closeSettings: stub(),
    reportError: stub(),
    notify: stub()
  };
}

function baseState(overrides: Partial<OverlaysState> = {}): OverlaysState {
  return {
    toastMessage: "",
    exportedTo: "",
    sessionSettings: null,
    newSession: null,
    menu: null,
    tabMenu: null,
    titleDialog: null,
    detailSession: null,
    settingsOpen: false,
    // 「关于」页不再取任何数据：这一组测试要的只是**这层浮层在场**。
    settingsTab: "about",
    ...overrides
  };
}

/** 退出的监听器留在测试手里：没有后端，得由测试来推那一次「用户点了关闭按钮」。 */
function fakeRuntime() {
  let notifyExit: (() => void) | undefined;
  window.__TAURI__ = {
    core: { invoke: async () => null },
    event: {
      listen: async (channel: string, handler: (event: { payload: unknown }) => void) => {
        if (channel === "app:request-exit") notifyExit = () => handler({ payload: null });
        return () => undefined;
      }
    }
  };
  return {
    requestExit: () => {
      if (!notifyExit) throw new Error("`app:request-exit` 的监听还没挂上");
      notifyExit();
    },
    hasListener: () => notifyExit !== undefined
  };
}

function renderOverlays(
  state: OverlaysState,
  over: {
    canCustomCwd?: boolean;
    canSessionSettings?: boolean;
    selectedTarget?: AiTarget | null;
    actions?: OverlaysActions;
  } = {}
) {
  return render(
    <AppOverlays
      state={state}
      actions={over.actions ?? stubActions()}
      providers={[]}
      providerId=""
      appVersion="0.1.0"
      targetId=""
      targets={[]}
      selectedTarget={over.selectedTarget ?? null}
      trash={false}
      canCustomCwd={over.canCustomCwd ?? false}
      canSessionSettings={over.canSessionSettings ?? false}
      canDuplicate={false}
      canBranch={false}
      resolveSession={() => undefined}
      tabLabel=""
      closableTabCount={0}
    />
  );
}

afterEach(cleanup);

describe("AppOverlays 的排布顺序", () => {
  it("设置中心的失败通过右上角提示反馈，不写入底部状态栏", async () => {
    fakeRuntime();
    window.__TAURI__!.core!.invoke = async (_command, args) => {
      const payload = args as { command?: string };
      if (payload.command === "vendor:list") throw new Error("查询余额失败 (404)");
      return null;
    };
    const actions = stubActions();
    const state = baseState({ settingsOpen: true, settingsTab: "vendors" });
    renderOverlays(state, { actions });
    await waitFor(() => expect(actions.notify).toHaveBeenCalledWith("查询余额失败 (404)"));
    expect(actions.reportError).not.toHaveBeenCalled();
    cleanup();
    const toastView = renderOverlays({ ...state, toastMessage: "查询余额失败 (404)" }, { actions });
    expect(toastView.container.querySelector(".hint-float")?.textContent).toContain("查询余额失败 (404)");
  });
  it("退出确认框排在**最后**：设置中心开着时它也盖在上面", async () => {
    const runtime = fakeRuntime();
    const view = renderOverlays(baseState({ settingsOpen: true }));
    await waitFor(() => expect(runtime.hasListener()).toBe(true));

    await act(async () => {
      runtime.requestExit();
    });

    const layers = Array.from(
      view.container.querySelectorAll<HTMLElement>(".settings-overlay, .dialog-overlay")
    );
    // 两层都在场，而且退出确认框是最后一个——同档里 DOM 顺序就是层序。
    expect(layers).toHaveLength(2);
    expect(layers[0]!.className).toContain("settings-overlay");
    expect(layers[layers.length - 1]!.className).toContain("dialog-overlay");
    expect(view.getByText("退出 AI 可视化控制台")).toBeTruthy();
  });

  it("「设置会话」排在设置中心**之后**：从环境页点开时不会被盖住", () => {
    const view = renderOverlays(
      baseState({
        settingsOpen: true,
        sessionSettings: { distro: "Ubuntu", path: "~/.codex" }
      })
    );

    const layers = Array.from(
      view.container.querySelectorAll<HTMLElement>(".settings-overlay, .dialog-overlay")
    );
    // 两层都在场，而且「设置会话」是最后一个——同档里 DOM 顺序就是层序。
    expect(layers).toHaveLength(2);
    expect(layers[0]!.className).toContain("settings-overlay");
    expect(layers[layers.length - 1]!.querySelector(".session-settings-dialog")).toBeTruthy();
  });
});

/**
 * 能力门控的**转发**。
 *
 * `supportsCustomCwd` 与 `supportsSessionSettings` 曾经是 JSX 布尔简写（恒真）——
 * 于是 claude/qoder 的 WSL 目标也能打开「设置会话」，保存写的是 Codex 的全局设置。
 * 下面每对「假 / 真」断言就是那两个简写的守卫：
 * **写回简写（恒真）时「假」那几条必红；反过来恒假传死时「真」那几条必红。**
 *
 * 完整链路（providers 数据 → 查询函数 → 面板/对话框）另在
 * `settings-center.test.tsx` 与 `environment-panel.test.tsx` 里钉住。
 */
describe("AppOverlays：能力布尔原样转发到对话框", () => {
  const wslTarget: AiTarget = {
    id: "codex:wsl:Ubuntu",
    provider: "codex",
    label: "Ubuntu",
    kind: "wsl",
    distro: "Ubuntu",
    available: true
  };
  const draft = { cwd: "D:\\work", title: "", prompt: "", cliArgs: "" };

  it("canCustomCwd=false：新会话对话框不出现「设置工作目录」", () => {
    const view = renderOverlays(baseState({ newSession: draft }), { canCustomCwd: false });
    expect(view.queryByText("设置工作目录")).toBeNull();
  });

  it("canCustomCwd=true：出现", () => {
    const view = renderOverlays(baseState({ newSession: draft }), { canCustomCwd: true });
    expect(view.getByText("设置工作目录")).toBeTruthy();
  });

  it("canSessionSettings=false：WSL 目标下「设置会话」的输入与两个按钮全部禁用", () => {
    // 只有能力为假时整体禁用——isWsl 为真，禁用的原因只可能是能力。
    const view = renderOverlays(
      baseState({ sessionSettings: { distro: "Ubuntu", path: "~/.codex" } }),
      { canSessionSettings: false, selectedTarget: wslTarget }
    );
    const input = view.container.querySelector<HTMLInputElement>(".session-settings-dialog input")!;
    expect(input.disabled).toBe(true);
    const byText = (label: string) =>
      [...view.container.querySelectorAll("button")].find(
        (item) => item.textContent?.trim() === label
      ) as HTMLButtonElement;
    expect(byText("保存").disabled).toBe(true);
    expect(byText("恢复自动探测").disabled).toBe(true);
  });

  it("canSessionSettings=true：同样条件下可用", () => {
    const view = renderOverlays(
      baseState({ sessionSettings: { distro: "Ubuntu", path: "~/.codex" } }),
      { canSessionSettings: true, selectedTarget: wslTarget }
    );
    const input = view.container.querySelector<HTMLInputElement>(".session-settings-dialog input")!;
    expect(input.disabled).toBe(false);
  });
});
