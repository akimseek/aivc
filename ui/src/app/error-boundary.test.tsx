// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { lazy, Suspense } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as api from "../ipc/commands";
import { ErrorBoundary } from "./error-boundary";

/**
 * 崩溃兜底（`ErrorBoundary`）。
 *
 * # 钉的是四件事
 *
 * ① 子树抛错**不能**把整棵树带走——边界接住并给出可见出口（标题 + 说明 + 重试）；
 * ② 「重试」必须**真的重新渲染子树**，而不是把同一份坏输出再画一遍；
 * ③ 「重试」按错误来源分流：chunk 取不到类错误（`lazy()` 的 rejection 被 React
 *    缓存在组件上，重置 state 必**同一帧**再抛回去）改为整页 `location.reload()`——
 *    死按钮是本工程反复钉过的缺陷类，见「chunk 取不到 / CSS 预载失败」两条用例；
 * ④ 埋点（`performance:log`）失败**不能**让错误处理路径再炸一次。
 *
 * # 为什么把 `logPerformance` 换成假实现
 *
 * ④ 要能**同步抛错**才钉得住 `componentDidCatch` 里那层 try：真实实现只返回
 * rejected promise，碰不到同步路径。
 *
 * # 为什么 stub 整个 `location` 而不是 `location.reload`
 *
 * jsdom 的 `Location` 是 LegacyUnforgeable：`reload` 在实例上是
 * `{ configurable: false, writable: false }`，`vi.spyOn` 与赋值都钉不进去。
 * 而 vitest 的 jsdom 环境里 `window === globalThis` 且 `location` 是可覆盖属性，
 * `vi.stubGlobal("location", …)` 正好能用——afterEach 里 `unstubAllGlobals` 放回真身。
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
  vi.unstubAllGlobals();
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

  it("普通渲染错误：点「重试」重置边界、子树被重新渲染（不碰整页 reload）", () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { reload });
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
    // 对照的另一半：普通错误走 state 重置，不需要、也不许整页重载。
    expect(reload).not.toHaveBeenCalled();
  });

  it("懒加载 chunk 取不到：点「重试」整页 reload（lazy 的 rejection 已被缓存，重置必同帧再抛）", async () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { reload });
    /*
      复刻真实失败形态：Vite 的动态 import 在 WebView2/Chromium 上取不到 chunk 时，
      `lazy()` 收到的 rejection 就是这条消息（`terminal-view.tsx` 的 TerminalPane 同款）。
    */
    const BrokenChunk = lazy(() =>
      Promise.reject(
        new Error("Failed to fetch dynamically imported module: http://tauri.localhost/assets/terminal-pane-abc123.js")
      )
    );
    const view = render(
      <ErrorBoundary label="终端加载失败">
        <Suspense fallback={<div>正在加载终端…</div>}>
          <BrokenChunk />
        </Suspense>
      </ErrorBoundary>
    );
    // `lazy()` 的 rejection 在微任务里落地，React 重试渲染才把错误抛进边界。
    await act(async () => {});

    const text = view.container.textContent ?? "";
    expect(text).toContain("终端加载失败");
    // 文案如实告知「重试 = 重新加载界面」，不许再写「再试一次」。
    expect(text).toContain("会重新加载界面");
    expect(getRetryButton(view.container)).toBeTruthy();
    expect(api.logPerformance).toHaveBeenCalledTimes(1); // 首次捕获只上报一次

    act(() => {
      getRetryButton(view.container).click();
    });

    expect(reload).toHaveBeenCalledTimes(1);
    // 兜底原样在：reload 被 stub 拦住、页面没真刷新，但也没有被 state 重置——
    // 旧实现每次点击只是重挂子树、被 lazy 同步重抛打回原形（屏幕纹丝不动）。
    expect(view.container.textContent).toContain("终端加载失败");
    // 点重试不再多写一条 `ui.error-boundary`：同一次崩溃不许被反复上报。
    expect(api.logPerformance).toHaveBeenCalledTimes(1);
  });

  it("懒加载 chunk 的 CSS 预载失败：同样走整页 reload（Vite helper 先于 import() 抛错）", async () => {
    const reload = vi.fn();
    vi.stubGlobal("location", { reload });
    /*
      复刻另一支真实失败形态：构建产物里 chunk 的预载依赖数组含 CSS
      （`__vite__mapDeps([0,1])` = terminal-pane 的 js + css），Vite 的 preload helper
      在 CSS 预载 rejected 时**先于**真正的 `import()` 抛出这条消息。判据漏掉它，
      点「重试」就重回「重置 state ⇒ 复读 lazy 缓存的 rejection ⇒ 死按钮」的老路。
    */
    const BrokenCss = lazy(() =>
      Promise.reject(new Error("Unable to preload CSS for /assets/terminal-pane-xxx.css"))
    );
    const view = render(
      <ErrorBoundary label="终端加载失败">
        <Suspense fallback={<div>正在加载终端…</div>}>
          <BrokenCss />
        </Suspense>
      </ErrorBoundary>
    );
    // `lazy()` 的 rejection 在微任务里落地，React 重试渲染才把错误抛进边界。
    await act(async () => {});

    const text = view.container.textContent ?? "";
    expect(text).toContain("终端加载失败");
    // hint 与「重试」的真实行为同支：CSS 预载失败也是「重试 = 重新加载界面」。
    expect(text).toContain("会重新加载界面");

    act(() => {
      getRetryButton(view.container).click();
    });

    expect(reload).toHaveBeenCalledTimes(1);
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
