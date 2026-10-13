// @vitest-environment jsdom
import { act, cleanup, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ExitConfirm } from "./exit-confirm";

/**
 * 退出确认框。
 *
 * # 这组测试是为什么来的
 *
 * 它是唯一一个**后端发起**的弹层：用户在窗口关闭按钮上拦下了一次关闭，界面必须
 * 回答（`app:exit-choice`）——后端会一直等着，既不超时也不自己退出。
 * 而它此前是自绘骨架，因此**没有 Escape**：键盘用户除了点那三个鼠标按钮没有出口。
 * 现在骨架换成共用的 `Dialog`，Escape 等价于「取消」，这里把它钉住。
 *
 * # 假运行时要能把事件推回组件
 *
 * `app:request-exit` 是后端推来的事件，测试里没有后端，因此把 `listen` 挂上的
 * 处理器留个把手（`requestExit`），由测试自己触发「用户点了窗口关闭按钮」。
 */

function fakeRuntime() {
  const calls: Array<{ command: string; args: unknown }> = [];
  let notifyExit: (() => void) | undefined;
  /** 置真后回答一定失败（后端拒绝），用来钉「失败不收起」。 */
  let fail = false;
  /** 置真后 `invoke` 永不落地：用来制造「请求在路上」的忙碌态。 */
  let hang = false;

  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: unknown };
        calls.push({ command: payload.command ?? "", args: payload.args });
        if (hang) return new Promise<never>(() => undefined);
        if (fail) throw new Error("后端拒绝了这次回答");
        return null;
      }
    },
    event: {
      listen: async (channel: string, handler: (event: { payload: unknown }) => void) => {
        if (channel === "app:request-exit") notifyExit = () => handler({ payload: null });
        return () => undefined;
      }
    }
  };

  return {
    calls,
    fail: () => {
      fail = true;
    },
    hang: () => {
      hang = true;
    },
    requestExit: () => {
      if (!notifyExit) throw new Error("`app:request-exit` 的监听还没挂上");
      notifyExit();
    },
    hasListener: () => notifyExit !== undefined
  };
}

/** 渲染并触发一次退出询问，返回已经弹着确认框的视图。 */
async function setup() {
  const runtime = fakeRuntime();
  const onError = vi.fn();
  const view = render(<ExitConfirm onError={onError} />);
  await waitFor(() => expect(runtime.hasListener()).toBe(true));
  await act(async () => {
    runtime.requestExit();
  });
  const button = (label: string) => {
    const found = [...view.container.querySelectorAll("button")].find(
      (item) => (item.textContent ?? "").trim() === label
    );
    if (!found) throw new Error(`没有按钮 ${label}`);
    return found as HTMLButtonElement;
  };
  return { view, onError, button, ...runtime };
}

/*
 * 每个用例之后卸载上一个实例。
 *
 * 它渲染的 `Dialog` 在 `document` 上监听键盘，而回答会调 `app:exit-choice`——留着上一次
 * 的实例，一次 Escape 会让**所有**还挂着的实例一起回答：去掉这一句之后，
 * 「请求没落地前不重复发」那条会数到 3 次而不是 1 次，断言就不再是「这一次」的。
 */
afterEach(cleanup);

describe("ExitConfirm", () => {
  it("三个选项与说明文案都在", async () => {
    const { view, button } = await setup();
    const text = view.container.textContent ?? "";
    expect(text).toContain("退出 AI 可视化控制台");
    expect(text).toContain("最小化后应用会继续在后台运行");
    expect(view.container.querySelector(".dialog-close")).toBeTruthy();
    expect(button("最小化到托盘")).toBeTruthy();
    expect(button("直接退出").className).toContain("danger");
  });

  it("**Escape 等于取消**——键盘用户此前没有出口", async () => {
    const { calls, view } = await setup();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() =>
      expect(calls).toContainEqual({ command: "app:exit-choice", args: { choice: "cancel" } })
    );
    // 回答完就收起：后端那边已经在最小化/退出，不该还挂着一个「要不要退出」的框。
    await waitFor(() => expect(view.container.querySelector(".dialog-overlay")).toBeNull());
  });

  it("三个按钮各自回传自己的选择", async () => {
    // 分开渲染：成功回答之后这个框就收起来了。
    const choices: Array<[string, string]> = [
      ["最小化到托盘", "minimize"],
      ["直接退出", "quit"]
    ];
    for (const [label, choice] of choices) {
      const { calls, button } = await setup();
      await act(async () => {
        button(label).click();
      });
      await waitFor(() =>
        expect(calls).toContainEqual({ command: "app:exit-choice", args: { choice } })
      );
    }
  });

  it("回答失败时**留在屏幕上**并报错——收起会让这次询问像从未发生过", async () => {
    const { fail, onError, view } = await setup();
    fail();
    await act(async () => {
      view.container.querySelector<HTMLButtonElement>(".dialog-close")!.click();
    });
    await waitFor(() => expect(onError).toHaveBeenCalledWith("后端拒绝了这次回答"));
    expect(view.container.querySelector(".dialog-panel")).toBeTruthy();
  });

  it("点遮罩**不**当作回答：每个选项都会改变应用状态，误点不算数", async () => {
    const { view, calls } = await setup();
    await act(async () => {
      view.container
        .querySelector(".dialog-overlay")!
        .dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(calls).toHaveLength(0);
    expect(view.container.querySelector(".dialog-panel")).toBeTruthy();
  });

  it("确认中的那一次请求没落地前，再按 Escape 不重复发请求", async () => {
    const { calls, hang } = await setup();
    hang();
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    await waitFor(() => expect(calls).toHaveLength(1));
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(calls).toHaveLength(1);
  });

  it("没有退出询问时什么都不渲染", async () => {
    const runtime = fakeRuntime();
    const view = render(<ExitConfirm onError={vi.fn()} />);
    await waitFor(() => expect(runtime.hasListener()).toBe(true));
    expect(view.container.childElementCount).toBe(0);
  });
});
