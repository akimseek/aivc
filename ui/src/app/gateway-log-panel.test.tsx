// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { GatewayLogPanel } from "./gateway-log-panel";

/**
 * Gateway 日志面板的「清空全部日志」。
 *
 * # 这组测试是为什么来的
 *
 * 这个按钮是菜单栏「帮助 → 清理 Gateway 日志」的**新家**（spec §11 的入口迁移表）。
 * 菜单栏被删掉之后，少了它这个动作就彻底没了；而它又是**不可撤销**的全清，
 * 因此钉住两件事：**必须先问一次**、**取消时一个字节都不许删**。
 */

/** 装一个假的 Tauri 运行时；`answers` 按命令名给返回值。 */
function fakeRuntime(answers: Record<string, unknown>) {
  const invocations: { command: string; args: unknown }[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        const payload = args as { command?: string } | undefined;
        const name = payload?.command ?? command;
        invocations.push({ command: name, args });
        return answers[name] ?? null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return invocations;
}

function setup(options: { confirm: boolean; removed?: number } = { confirm: true }) {
  const invocations = fakeRuntime({
    "gateway:query-logs": [],
    "gateway:clear-logs": options.removed ?? 12
  });
  const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(options.confirm);
  const onError = vi.fn();
  const onNotify = vi.fn();
  const view = render(<GatewayLogPanel onError={onError} onNotify={onNotify} />);
  const button = () =>
    [...view.container.querySelectorAll("button")].find((item) =>
      (item.textContent ?? "").includes("清空全部日志")
    ) as HTMLButtonElement | undefined;
  return { view, invocations, confirmSpy, onError, onNotify, button };
}

beforeEach(() => {
  vi.restoreAllMocks();
});

describe("GatewayLogPanel：清空全部日志", () => {
  it("工具行里有「清空全部日志」，且是危险样式", async () => {
    const { button } = setup();
    expect(button()).toBeTruthy();
    await waitFor(() => expect(button()).toBeTruthy());
    // 不可撤销的动作必须在视觉上就能认出来（与其他「删除选中」一致）。
    expect(button()?.className).toContain("btn-danger");
  });

  it("点击时先确认，确认文案说清是不可撤销的全清", async () => {
    const { confirmSpy, button } = setup();
    await act(async () => {
      button()!.click();
    });
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(confirmSpy.mock.calls[0]![0]).toContain("清空所有网关审计日志");
  });

  it("取消确认时**不删任何东西**", async () => {
    const { invocations, onNotify, button } = setup({ confirm: false });
    await act(async () => {
      button()!.click();
    });
    expect(invocations.some((item) => item.command === "gateway:clear-logs")).toBe(false);
    expect(onNotify).not.toHaveBeenCalled();
  });

  it("确认后清空并报告删了多少条", async () => {
    const { invocations, onNotify, button } = setup({ confirm: true, removed: 12 });
    await act(async () => {
      button()!.click();
    });

    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(invocations.some((item) => item.command === "gateway:clear-logs")).toBe(true);
    // 条数要说出来：静默完成与没执行在用户看来是一样的。
    expect(onNotify).toHaveBeenCalledWith("已清理 12 条 Gateway 日志。");
  });

  it("清空后重新查询（列表要跟着变空，而不是留着已经删掉的条目）", async () => {
    const { invocations, button } = setup({ confirm: true });
    await waitFor(() => {
      expect(invocations.some((item) => item.command === "gateway:query-logs")).toBe(true);
    });
    const before = invocations.filter((item) => item.command === "gateway:query-logs").length;

    await act(async () => {
      button()!.click();
    });

    await waitFor(() => {
      const after = invocations.filter((item) => item.command === "gateway:query-logs").length;
      expect(after).toBeGreaterThan(before);
    });
  });
});
