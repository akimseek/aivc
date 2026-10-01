// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../ipc/commands";
import { ErrorBoundary } from "./error-boundary";

/**
 * 崩溃兜底（`ErrorBoundary`）。
 *
 * # 钉的是三件事
 *
 * ① 子树抛错**不能**把整棵树带走——边界接住并给出可见出口（标题 + 说明 + 重试）；
 * ② 「重试」必须**真的重新渲染子树**，而不是把同一份坏输出再画一遍；
 * ③ 埋点（`performance:log`）失败**不能**让错误处理路径再炸一次。
 *
 * # 为什么把 `logPerformance` 换成假实现
 *
 * ③ 要能**同步抛错**才钉得住 `componentDidCatch` 里那层 try：真实实现只返回
 * rejected promise，碰不到同步路径。
 */

vi.mock("../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../ipc/commands")>();
  return { ...actual, logPerformance: vi.fn() };
});

/** 可控抛错子组件：外部决定下一次渲染抛不抛，并记录渲染次数。 */
function makeBomb() {
  const control = { shouldThrow: true, renders: 0 };
  function Bomb() {
    control.renders += 1;
    if (control.shouldThrow) throw new Error("子树渲染失败");
    return <span>子树已恢复</span>;
  }
  return { control, Bomb };
}

function getRetryButton(container: HTMLElement): HTMLButtonElement {
  const found = [...container.querySelectorAll("button")].find(
    (item) => (item.textContent ?? "").trim() === "重试"
  );
  if (!found) throw new Error("兜底里没有「重试」按钮");
  return found as HTMLButtonElement;
}

/*
 * 这些用例**故意**让子树抛错，React 与 jsdom 会把预期内的错误打到 console.error。
 * 静音只是让输出可读——断言失败照样会以正常错误抛出。
 */
let consoleError: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  vi.mocked(api.logPerformance).mockReset();
  consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
});

afterEach(() => {
  consoleError.mockRestore();
  cleanup();
});

describe("ErrorBoundary", () => {
  it("子树抛错时接住，显示兜底（标题 + 一句话说明 + 重试）", () => {
    const { Bomb } = makeBomb();
    const view = render(
      <ErrorBoundary label="界面出错">
        <Bomb />
      </ErrorBoundary>
    );

    const text = view.container.textContent ?? "";
    expect(text).toContain("界面出错");
    expect(text).toContain("点「重试」再试一次");
    expect(getRetryButton(view.container).textContent).toBe("重试");
    // 抛错的子树整个让位，不残留半截坏输出。
    expect(text).not.toContain("子树已恢复");
  });

  it("点「重试」重置边界，子树被重新渲染", () => {
    const { control, Bomb } = makeBomb();
    const view = render(
      <ErrorBoundary label="界面出错">
        <Bomb />
      </ErrorBoundary>
    );
    expect(getRetryButton(view.container)).toBeTruthy();

    control.shouldThrow = false;
    const rendersBeforeRetry = control.renders;
    act(() => {
      getRetryButton(view.container).click();
    });

    expect(control.renders).toBeGreaterThan(rendersBeforeRetry);
    expect(view.container.textContent).toContain("子树已恢复");
    expect(view.container.querySelector("button")).toBeNull();
  });

  it("重试后仍抛错：兜底再次出现，错误不会漏到边界之外", () => {
    const { Bomb } = makeBomb(); // 一直抛错
    const view = render(
      <ErrorBoundary label="终端加载失败">
        <Bomb />
      </ErrorBoundary>
    );

    act(() => {
      getRetryButton(view.container).click();
    });

    expect(view.container.textContent).toContain("终端加载失败");
    expect(getRetryButton(view.container)).toBeTruthy();
  });

  it("崩溃落 performance:log 埋点：区域名与错误消息都要带上", () => {
    const { Bomb } = makeBomb();
    render(
      <ErrorBoundary label="终端加载失败">
        <Bomb />
      </ErrorBoundary>
    );

    expect(api.logPerformance).toHaveBeenCalledWith(
      "ui.error-boundary",
      0,
      "终端加载失败：子树渲染失败"
    );
  });

  it("埋点同步抛错也静默：兜底照常显示，不许因日志再炸一次", () => {
    vi.mocked(api.logPerformance).mockImplementation(() => {
      throw new Error("埋点炸了");
    });
    const { Bomb } = makeBomb();
    const view = render(
      <ErrorBoundary label="界面出错">
        <Bomb />
      </ErrorBoundary>
    );

    expect(api.logPerformance).toHaveBeenCalled();
    expect(view.container.textContent).toContain("界面出错");
    expect(getRetryButton(view.container)).toBeTruthy();
  });
});
