// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ComposerInput } from "./composer-input";

/**
 * 自管输入组件。
 *
 * 钉住两件最容易写错的事：
 *
 * 1. **空白内容不发送**——它会给 CLI 塞一个多余的回车；
 * 2. **Enter 发送、Shift+Enter 换行**——反过来会让「写完按回车」这个最自然的动作失效。
 *
 * 写入 PTY 的字节形状本身在 `composer-text.test.ts` 里逐条钉住，这里只验证
 * 「按钮/按键真的调到了写终端，并且带上了那一串字节」。
 */

function fakeRuntime() {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        calls.push({ command: payload?.command ?? "", args: payload?.args ?? {} });
        if (payload?.command === "dialog:choose-file") return null;
        return null;
      }
    },
    event: { listen: async () => () => undefined }
  };
  return { calls };
}

/** 往受控 textarea 里写值。React 需要走原生 setter 才会收到变更。 */
function type(area: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
  setter.call(area, value);
  area.dispatchEvent(new Event("input", { bubbles: true }));
}

describe("ComposerInput", () => {
  it("空白内容时发送按钮是禁用的", () => {
    // 空提交会往 CLI 里塞一个多余的回车。
    fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" onError={vi.fn()} />);
    const send = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("发送")
    )!;
    expect(send.disabled).toBe(true);
  });

  it("写下内容后可以发送，且写进去的是括号粘贴 + 回车", async () => {
    const runtime = fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" onError={vi.fn()} />);
    const area = view.container.querySelector("textarea")!;

    await act(async () => {
      type(area, "帮我看看这个 bug");
    });

    const send = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("发送")
    )!;
    expect(send.disabled).toBe(false);

    await act(async () => {
      send.click();
    });

    await waitFor(() => {
      const call = runtime.calls.find((item) => item.command === "terminal:write");
      expect(call).toBeTruthy();
      expect(call!.args.data).toBe("\x1b[200~帮我看看这个 bug\x1b[201~\r");
    });
  });

  it("Enter 发送、Shift+Enter **不**发送", async () => {
    const runtime = fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" onError={vi.fn()} />);
    const area = view.container.querySelector("textarea")!;

    await act(async () => {
      type(area, "一句话");
    });

    // Shift+Enter：只换行，不该写终端。
    await act(async () => {
      area.dispatchEvent(
        new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true })
      );
    });
    expect(runtime.calls.some((item) => item.command === "terminal:write")).toBe(false);

    await act(async () => {
      area.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });
    await waitFor(() => {
      expect(runtime.calls.some((item) => item.command === "terminal:write")).toBe(true);
    });
  });

  it("发送成功后清空输入框", async () => {
    fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" onError={vi.fn()} />);
    const area = view.container.querySelector("textarea")!;

    await act(async () => {
      type(area, "一句话");
    });
    await act(async () => {
      area.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    await waitFor(() => {
      expect(area.value).toBe("");
    });
  });

  it("换终端会清掉草稿——草稿属于那一路终端", async () => {
    fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" onError={vi.fn()} />);
    const area = view.container.querySelector("textarea")!;

    await act(async () => {
      type(area, "写给 t1 的");
    });
    view.rerender(<ComposerInput terminalId="t2" onError={vi.fn()} />);

    await waitFor(() => {
      expect(view.container.querySelector("textarea")!.value).toBe("");
    });
  });

  it("写入失败时报错，且**不清空**草稿", async () => {
    // 清空了就意味着用户写的一段话没了——那是比报错严重得多的事。
    const onError = vi.fn();
    window.__TAURI__ = {
      core: {
        invoke: async (_command: string, args?: unknown) => {
          const payload = args as { command?: string } | undefined;
          if (payload?.command === "terminal:write") throw new Error("终端已关闭");
          return null;
        }
      },
      event: { listen: async () => () => undefined }
    };
    const view = render(<ComposerInput terminalId="t1" onError={onError} />);
    const area = view.container.querySelector("textarea")!;

    await act(async () => {
      type(area, "别丢了我");
    });
    await act(async () => {
      area.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    });

    await waitFor(() => {
      expect(onError).toHaveBeenCalled();
      expect(area.value).toBe("别丢了我");
    });
  });

  it("有拖动手柄，且它是可访问的分隔条", () => {
    fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" onError={vi.fn()} />);
    const handle = view.container.querySelector('[role="separator"]');
    expect(handle).toBeTruthy();
    expect(handle!.getAttribute("aria-label")).toBe("拖动调整输入框高度");
  });
});
