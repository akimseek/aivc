// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ComposerInput } from "./composer-input";

/**
 * 自管输入组件。
 *
 * 钉住两件最容易写错的事：
 *
 * 1. **空白内容不发送**——它会给 CLI 塞一个多余的回车；
 * 2. **Enter 发送、Shift+Enter 换行**——反过来会让「写完按回车」这个最自然的动作失效。
 *
 * 另外两条盯的是**重做之后不能再丢的东西**：附件入口虽然合并成一个下拉，但
 * 「添加图片」与「添加文件」仍是两个动作（过滤器不同）；模型选择只在有来源时出现。
 *
 * 写入 PTY 的字节形状本身在 `composer-text.test.ts` 里逐条钉住，这里只验证
 * 「按钮/按键真的调到了写终端，并且带上了那一串字节」。
 */

function fakeRuntime() {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const listeners = new Map<string, ((event: { payload: unknown }) => void)[]>();
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Record<string, unknown> } | undefined;
        calls.push({ command: payload?.command ?? "", args: payload?.args ?? {} });
        if (payload?.command === "dialog:choose-file") return null;
        return null;
      }
    },
    event: {
      listen: async (channel: string, handler: (event: { payload: unknown }) => void) => {
        const current = listeners.get(channel) ?? [];
        current.push(handler);
        listeners.set(channel, current);
        return () => {
          listeners.set(
            channel,
            (listeners.get(channel) ?? []).filter((item) => item !== handler)
          );
        };
      }
    }
  };
  /** 模拟 Tauri 侧推事件（拖拽坐标是物理像素）。 */
  const emit = (channel: string, payload?: unknown) => {
    for (const handler of listeners.get(channel) ?? []) handler({ payload });
  };
  /** 某通道已注册的监听数——订阅是异步落地的，测试靠它等订阅就位。 */
  const listenerCount = (channel: string) => (listeners.get(channel) ?? []).length;
  return { calls, emit, listenerCount };
}

/** 输入卡的命中框（CSS 像素）。jsdom 没有布局，需要显式给一个。 */
const CARD_RECT = { left: 100, top: 100, right: 340, bottom: 220 };

function stubCardRect() {
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
    x: CARD_RECT.left,
    y: CARD_RECT.top,
    left: CARD_RECT.left,
    top: CARD_RECT.top,
    right: CARD_RECT.right,
    bottom: CARD_RECT.bottom,
    width: CARD_RECT.right - CARD_RECT.left,
    height: CARD_RECT.bottom - CARD_RECT.top,
    toJSON: () => ({})
  } as unknown as DOMRect);
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

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
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
    const send = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("发送")
    )!;
    expect(send.disabled).toBe(true);
  });

  it("写下内容后可以发送，且写进去的是括号粘贴 + 回车", async () => {
    const runtime = fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
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
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
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
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
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
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
    const area = view.container.querySelector("textarea")!;

    await act(async () => {
      type(area, "写给 t1 的");
    });
    view.rerender(<ComposerInput terminalId="t2" providerId="codex" onError={vi.fn()} />);

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
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={onError} />);
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
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
    const handle = view.container.querySelector('[role="separator"]');
    expect(handle).toBeTruthy();
    expect(handle!.getAttribute("aria-label")).toBe("拖动调整输入框高度");
  });

  it("附件是一个下拉，展开后两项（添加图片 / 添加文件）", () => {
    // 两个动作被合并成一个入口，但**动作本身没合并**：过滤器不同，
    // 合成一项会让图片选不中（那条 `.md` 过滤器的老毛病）。
    fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);

    const trigger = [...view.container.querySelectorAll("button")].find((item) =>
      item.textContent?.includes("附件")
    )!;
    expect(trigger).toBeTruthy();

    expect(view.container.querySelectorAll(".composer-menu-item")).toHaveLength(0);
    act(() => {
      trigger.click();
    });

    const items = [...view.container.querySelectorAll(".composer-menu-item")].map(
      (item) => item.textContent
    );
    expect(items).toEqual(["添加图片", "添加文件"]);
  });

  it("没有模型来源（无供应商、非 Qoder）时不渲染模型选择", () => {
    fakeRuntime();
    const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
    expect(view.container.querySelector(".model-select")).toBeNull();
  });

  describe("拖拽落点提示", () => {
    /**
     * 提示的触发源是 **Tauri 的窗口级拖拽事件 + 自己做的命中判定**。
     * 挂 DOM `dragover` 的老写法在 Windows 上从不触发（wry 关掉了 webview 的
     * 外部拖放默认行为），这几条用例把它换掉之后的行为钉死。
     */
    async function mountWithDragEvents() {
      const runtime = fakeRuntime();
      stubCardRect();
      const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
      await waitFor(() => expect(runtime.listenerCount("tauri://drag-enter")).toBe(1));
      const actions = () => view.container.querySelector(".composer-actions")!;
      return { runtime, view, actions };
    }

    it("拖进输入卡上方给出提示；移到卡外、拖出窗口后收回", async () => {
      const { runtime, actions } = await mountWithDragEvents();
      expect(actions().className).not.toContain("dragging");

      await act(async () => {
        runtime.emit("tauri://drag-enter", {
          paths: ["C:\\shot.png"],
          position: { x: 200, y: 160 }
        });
      });
      expect(actions().className).toContain("dragging");
      expect(actions().textContent).toContain("松手即添加");

      // 悬停在窗口别处（卡外）：提示收回。
      await act(async () => {
        runtime.emit("tauri://drag-over", { position: { x: 600, y: 400 } });
      });
      expect(actions().className).not.toContain("dragging");

      // 再回到卡内，然后彻底拖出窗口。
      await act(async () => {
        runtime.emit("tauri://drag-over", { position: { x: 200, y: 160 } });
      });
      expect(actions().className).toContain("dragging");
      await act(async () => {
        runtime.emit("tauri://drag-leave");
      });
      expect(actions().className).not.toContain("dragging");
    });

    it("坐标按 devicePixelRatio 换算后再命中（缩放屏上不偏移）", async () => {
      // Tauri 给的是物理像素；不除 dpr 的话，125%/200% 缩放的屏上命中框整体偏移。
      vi.stubGlobal("devicePixelRatio", 2);
      const { runtime, actions } = await mountWithDragEvents();

      // 物理 (400, 320) ÷ 2 = CSS (200, 160)，在卡内；不换算则 400 > 340 被误判为卡外。
      await act(async () => {
        runtime.emit("tauri://drag-enter", { paths: [], position: { x: 400, y: 320 } });
      });
      expect(actions().className).toContain("dragging");

      // 物理 (150, 160) ÷ 2 = CSS (75, 80)，在左/上边界外；不换算则 150、160 都落在卡内被误判。
      await act(async () => {
        runtime.emit("tauri://drag-over", { position: { x: 150, y: 160 } });
      });
      expect(actions().className).not.toContain("dragging");
    });

    it("松手（drop）后提示立即收回，不等 leave", async () => {
      const { runtime, actions } = await mountWithDragEvents();

      await act(async () => {
        runtime.emit("tauri://drag-enter", { paths: [], position: { x: 200, y: 160 } });
      });
      expect(actions().className).toContain("dragging");

      await act(async () => {
        runtime.emit("tauri://drag-drop", {
          paths: ["C:\\shot.png"],
          position: { x: 200, y: 160 }
        });
      });
      expect(actions().className).not.toContain("dragging");
    });

    it("载荷缺 position 时不崩，提示保持收回", async () => {
      // 载荷形状异常（上游换代/自定义）不该让渲染器抛——守卫按「不在卡上」处理。
      const { runtime, actions } = await mountWithDragEvents();
      await act(async () => {
        runtime.emit("tauri://drag-over", { paths: [] });
      });
      expect(actions().className).not.toContain("dragging");
    });

    it("卸载后监听全部退订，不留窗口级监听", async () => {
      // 「退订必须被调用」是 client.ts 的明示契约（漏退订 = 关掉的终端/输入区还收事件）。
      const { runtime, view } = await mountWithDragEvents();
      expect(runtime.listenerCount("tauri://drag-drop")).toBe(1);

      view.unmount();

      for (const channel of [
        "tauri://drag-enter",
        "tauri://drag-over",
        "tauri://drag-leave",
        "tauri://drag-drop"
      ]) {
        expect(runtime.listenerCount(channel)).toBe(0);
      }
    });

    it("没有 Tauri 运行时（纯浏览器打开）时挂载不报错", async () => {
      delete window.__TAURI__;
      stubCardRect();
      const view = render(<ComposerInput terminalId="t1" providerId="codex" onError={vi.fn()} />);
      await act(async () => {});
      expect(view.container.querySelector("textarea")).toBeTruthy();
    });
  });
});
