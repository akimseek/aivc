// @vitest-environment jsdom
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PresetsPanel } from "./presets-panel";
import type { WorkspacePreset } from "../../../types";

/**
 * 工作区预设面板。
 *
 * 钉住三条路径与它们的失败上报：载入（列表 / 空态）、保存（trim 后提交、
 * 清表单、重载）、删除（重载）。**失败必须报出去**是这一页最要紧的契约——
 * 静默失败时用户以为存上了，下次开终端才发现目录没带上。
 */

function preset(id: string, name: string, cwd: string): WorkspacePreset {
  return { id, name, cwd, updatedAt: "2026-10-01T00:00:00.000Z" };
}

function fakeRuntime(answers: Record<string, (args: Record<string, unknown>) => unknown>) {
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, payload?: unknown) => {
        const { command = "", args = {} } = (payload ?? {}) as {
          command?: string;
          args?: Record<string, unknown>;
        };
        calls.push({ command, args });
        return answers[command]?.(args) ?? null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return calls;
}

/** 按顺序取输入框写值——受控输入必须走原生 setter，React 才收得到变更。 */
async function typeInto(input: HTMLInputElement, value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const findButton = (container: HTMLElement, text: string) =>
  [...container.querySelectorAll<HTMLButtonElement>("button")].find(
    (item) => (item.textContent ?? "").trim() === text
  );

afterEach(cleanup);

describe("PresetsPanel", () => {
  it("载入后列出预设；列表为空时给空态", async () => {
    fakeRuntime({ "workspace:list-presets": () => [preset("p1", "项目甲", "/w/甲")] });
    const first = render(<PresetsPanel onError={vi.fn()} />);
    await waitFor(() => expect(first.container.textContent).toContain("项目甲"));
    expect(first.container.querySelectorAll(".list-row")).toHaveLength(1);

    cleanup();
    fakeRuntime({ "workspace:list-presets": () => [] });
    const second = render(<PresetsPanel onError={vi.fn()} />);
    await waitFor(() => expect(second.container.textContent).toContain("还没有工作区预设。"));
  });

  it("目录为空时「新增预设」不可点——空目录的预设没有意义", async () => {
    fakeRuntime({ "workspace:list-presets": () => [] });
    const view = render(<PresetsPanel onError={vi.fn()} />);
    await waitFor(() => expect(view.container.textContent).toContain("还没有工作区预设。"));
    expect(findButton(view.container, "新增预设")!.disabled).toBe(true);

    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await typeInto(inputs[1]!, "   "); // 只有空白也不算填了
    expect(findButton(view.container, "新增预设")!.disabled).toBe(true);
  });

  it("保存：名称与目录去空白后提交，表单清空，列表重载", async () => {
    const calls = fakeRuntime({
      "workspace:list-presets": () => [preset("p1", "项目甲", "/w/甲")],
      "workspace:save-preset": () => null
    });
    const view = render(<PresetsPanel onError={vi.fn()} />);
    await waitFor(() => expect(view.container.textContent).toContain("项目甲"));

    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await typeInto(inputs[0]!, "  新预设  ");
    await typeInto(inputs[1]!, "  /w/新  ");
    await act(async () => {
      findButton(view.container, "新增预设")!.click();
    });

    const saved = calls.filter((item) => item.command === "workspace:save-preset");
    expect(saved).toHaveLength(1);
    expect(saved[0]!.args).toEqual({ name: "新预设", cwd: "/w/新" });
    // 存完清表单 + 重载列表（首次载入 + 保存后各一次）。
    await waitFor(() => {
      expect(inputs[0]!.value).toBe("");
      expect(inputs[1]!.value).toBe("");
    });
    expect(calls.filter((item) => item.command === "workspace:list-presets")).toHaveLength(2);
  });

  it("删除：按编号提交并重载列表", async () => {
    const calls = fakeRuntime({
      "workspace:list-presets": () => [preset("p1", "项目甲", "/w/甲")],
      "workspace:delete-preset": () => true
    });
    const view = render(<PresetsPanel onError={vi.fn()} />);
    await waitFor(() => expect(view.container.textContent).toContain("项目甲"));

    await act(async () => {
      findButton(view.container, "删除")!.click();
    });

    const removed = calls.filter((item) => item.command === "workspace:delete-preset");
    expect(removed).toHaveLength(1);
    expect(removed[0]!.args).toEqual({ id: "p1" });
    await waitFor(() =>
      expect(calls.filter((item) => item.command === "workspace:list-presets")).toHaveLength(2)
    );
  });

  it("保存失败要报出去，且**不清空**已填的表单", async () => {
    const calls = fakeRuntime({
      "workspace:list-presets": () => [],
      "workspace:save-preset": () => {
        throw new Error("同名预设已存在");
      }
    });
    const onError = vi.fn();
    const view = render(<PresetsPanel onError={onError} />);
    const inputs = [...view.container.querySelectorAll<HTMLInputElement>("input")];
    await typeInto(inputs[0]!, "新预设");
    await typeInto(inputs[1]!, "/w/新");
    await act(async () => {
      findButton(view.container, "新增预设")!.click();
    });

    await waitFor(() => expect(onError).toHaveBeenCalledWith("同名预设已存在"));
    // 失败时表单还在——用户改一下就能重试，而不是重打一遍。
    expect(inputs[0]!.value).toBe("新预设");
    expect(inputs[1]!.value).toBe("/w/新");
    expect(calls.filter((item) => item.command === "workspace:list-presets")).toHaveLength(1);
  });

  it("删除失败与载入失败同样要报出去", async () => {
    fakeRuntime({
      "workspace:list-presets": () => [preset("p1", "项目甲", "/w/甲")],
      "workspace:delete-preset": () => {
        throw new Error("预设不存在");
      }
    });
    const onError = vi.fn();
    const view = render(<PresetsPanel onError={onError} />);
    await waitFor(() => expect(view.container.textContent).toContain("项目甲"));
    await act(async () => {
      findButton(view.container, "删除")!.click();
    });
    await waitFor(() => expect(onError).toHaveBeenCalledWith("预设不存在"));

    cleanup();
    fakeRuntime({
      "workspace:list-presets": () => {
        throw new Error("读取失败");
      }
    });
    const onLoadError = vi.fn();
    render(<PresetsPanel onError={onLoadError} />);
    await waitFor(() => expect(onLoadError).toHaveBeenCalledWith("读取失败"));
  });
});
