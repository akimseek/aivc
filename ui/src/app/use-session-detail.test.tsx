// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { useSessionDetail } from "./use-session-detail";

/**
 * 会话详情的取数。
 *
 * 钉住三处**看起来正常但会错**的地方：
 *
 * 1. **总数不能因为某一页返回 0 就被清零**——后端的 `0` 是「本次无从得知」，
 *    清掉之后页数算出来永远是 1；
 * 2. **连点翻页时后回的旧响应不能覆盖新结果**；
 * 3. **切换会话必须回到第 1 页**——否则从 300 条的会话切到 20 条的会停在空白页。
 */

type Page = { offset: number; messages: unknown[]; hasMore: boolean; messageCount: number };

function fakeRuntime(pages: Map<number, Page>) {
  const calls: { offset: number; limit: number }[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (_command: string, args?: unknown) => {
        const payload = args as { args?: { offset: number; limit: number } } | undefined;
        const offset = payload?.args?.offset ?? 0;
        const limit = payload?.args?.limit ?? 0;
        calls.push({ offset, limit });
        return pages.get(offset) ?? { offset, messages: [], hasMore: false, messageCount: 0 };
      }
    },
    event: { listen: async () => () => undefined }
  };
  return { calls };
}

/**
 * 手动落地的假运行时：每个 offset 的响应先挂起，由测试决定什么时候、按什么顺序返回。
 *
 * # 为什么需要它
 *
 * 「后回的旧响应覆盖新结果」要求第 2 页的响应**晚于**第 3 页。固定延迟（或按调用序
 * 给同一个延迟）的假运行时里，第 2 页必然先挂起、先返回——那一幕永远构造不出来，
 * 断言恒真。只有把「谁先返回」交给测试，这条竞态才真的被测到。
 */
function deferredRuntime(pages: Map<number, Page>) {
  const calls: { offset: number; limit: number }[] = [];
  const pending = new Map<number, { resolve: () => void; reject: (reason: unknown) => void }[]>();
  window.__TAURI__ = {
    core: {
      invoke: (_command: string, args?: unknown) => {
        const payload = args as { args?: { offset: number; limit: number } } | undefined;
        const offset = payload?.args?.offset ?? 0;
        const limit = payload?.args?.limit ?? 0;
        calls.push({ offset, limit });
        return new Promise((resolve, reject) => {
          const queue = pending.get(offset) ?? [];
          queue.push({
            resolve: () =>
              resolve(pages.get(offset) ?? { offset, messages: [], hasMore: false, messageCount: 0 }),
            reject
          });
          pending.set(offset, queue);
        });
      }
    },
    event: { listen: async () => () => undefined }
  };
  /** 让某个 offset 最近一次在途请求落地（或失败）；没有在途请求就是测试写错了。 */
  function settle(offset: number, outcome: "resolve" | "reject" = "resolve", reason?: unknown) {
    const next = pending.get(offset)?.shift();
    if (!next) throw new Error(`offset ${offset} 没有在途请求可落地`);
    if (outcome === "reject") next.reject(reason);
    else next.resolve();
  }
  return { calls, settle };
}

const target = { id: "codex:local", provider: "codex" as const, label: "本机", kind: "local" as const, available: true };
const session = {
  id: "s1",
  title: "t",
  filePath: "/a.jsonl",
  messageCount: 5,
  preview: []
};

/** 渲染 hook 并等到第一次取数完成。 */
async function renderDetail(pageSize = 50) {
  let state: ReturnType<typeof useSessionDetail> | null = null;
  function Probe() {
    state = useSessionDetail(target as never, session as never, pageSize);
    return null;
  }
  const view = render(<Probe />);
  await waitFor(() => {
    expect(view.container).toBeTruthy();
  });
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();
  });
  return { view, get: () => state! };
}

describe("useSessionDetail", () => {
  it("第一页的 offset 是 0，limit 是每页数量", async () => {
    const runtime = fakeRuntime(
      new Map([[0, { offset: 0, messages: [{ role: "user", text: "a" }], hasMore: true, messageCount: 120 }]])
    );
    const { get } = await renderDetail(50);
    await waitFor(() => {
      expect(runtime.calls[0]).toEqual({ offset: 0, limit: 50 });
      expect(get().messages).toHaveLength(1);
      expect(get().total).toBe(120);
    });
  });

  it("翻到第 3 页时 offset = (3-1) × 每页", async () => {
    const runtime = fakeRuntime(
      new Map([
        [0, { offset: 0, messages: [], hasMore: true, messageCount: 120 }],
        [100, { offset: 100, messages: [{ role: "user", text: "p3" }], hasMore: true, messageCount: 120 }]
      ])
    );
    const { get } = await renderDetail(50);
    await act(async () => {
      get().setPage(3);
    });
    await waitFor(() => {
      expect(runtime.calls.some((item) => item.offset === 100)).toBe(true);
    });
  });

  it("**某一页返回总数 0 时不清零**——后端的 0 是「本次无从得知」", async () => {
    // 清零之后页数算出来永远是 1，用户按「下一页」会跳到不存在的位置。
    const runtime = fakeRuntime(
      new Map([
        [0, { offset: 0, messages: [], hasMore: true, messageCount: 120 }],
        [50, { offset: 50, messages: [{ role: "user", text: "p2" }], hasMore: false, messageCount: 0 }]
      ])
    );
    const { get } = await renderDetail(50);
    await waitFor(() => expect(get().total).toBe(120));

    await act(async () => {
      get().setPage(2);
    });
    await waitFor(() => {
      expect(runtime.calls.some((item) => item.offset === 50)).toBe(true);
      expect(get().total).toBe(120);
      expect(get().pageCount).toBe(3);
    });
  });

  it("页数至少是 1——不会是「第 1 / 0 页」", async () => {
    fakeRuntime(new Map());
    const { get } = await renderDetail(50);
    await waitFor(() => {
      expect(get().pageCount).toBe(1);
    });
  });

  it("**连点翻页时后回的旧响应不覆盖新结果**", async () => {
    // 用户点了 2、再点 3，再让第 3 页**先**回、第 2 页后回——网络乱序时这一幕真实存在。
    // 旧实现里第 2 页会盖掉第 3 页，界面停在旧结果上。
    const runtime = deferredRuntime(
      new Map([
        [0, { offset: 0, messages: [{ role: "user", text: "第一页" }], hasMore: true, messageCount: 300 }],
        [50, { offset: 50, messages: [{ role: "user", text: "第二页" }], hasMore: true, messageCount: 300 }],
        [100, { offset: 100, messages: [{ role: "user", text: "第三页" }], hasMore: true, messageCount: 300 }]
      ])
    );
    const { get } = await renderDetail(50);
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 0)).toBe(true));
    await act(async () => runtime.settle(0));
    await waitFor(() => expect(get().messages[0]?.text).toBe("第一页"));

    await act(async () => {
      get().setPage(2);
    });
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 50)).toBe(true));
    await act(async () => {
      get().setPage(3);
    });
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 100)).toBe(true));

    // 后发的第 3 页先回来。
    await act(async () => runtime.settle(100));
    await waitFor(() => expect(get().messages[0]?.text).toBe("第三页"));

    // 旧的第 2 页这时才回来——不允许落地（act 已把微任务放完，落地了这里就会看见）。
    await act(async () => runtime.settle(50));
    expect(get().messages[0]?.text).toBe("第三页");
    expect(get().loading).toBe(false);
  });

  it("**旧响应被丢弃时把 loading 留给新请求**——不出现「旧响应把 loading 关掉」的闪烁", async () => {
    const runtime = deferredRuntime(
      new Map([
        [0, { offset: 0, messages: [{ role: "user", text: "第一页" }], hasMore: true, messageCount: 300 }],
        [50, { offset: 50, messages: [{ role: "user", text: "第二页" }], hasMore: true, messageCount: 300 }],
        [100, { offset: 100, messages: [{ role: "user", text: "第三页" }], hasMore: true, messageCount: 300 }]
      ])
    );
    const { get } = await renderDetail(50);
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 0)).toBe(true));
    await act(async () => runtime.settle(0));
    await waitFor(() => expect(get().loading).toBe(false));

    await act(async () => {
      get().setPage(2);
    });
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 50)).toBe(true));
    await act(async () => {
      get().setPage(3);
    });
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 100)).toBe(true));

    // 第 3 页还在路上，旧的第 2 页先回来了：它既不能覆盖内容，也不能替新请求把
    // loading 关掉（否则界面闪一下「没在加载」又转回去）。
    await act(async () => runtime.settle(50));
    expect(get().loading).toBe(true);
    expect(get().messages[0]?.text).toBe("第一页");

    await act(async () => runtime.settle(100));
    expect(get().messages[0]?.text).toBe("第三页");
    expect(get().loading).toBe(false);
  });

  it("**旧响应失败也丢弃**——错误与清空都归最新一次请求", async () => {
    const runtime = deferredRuntime(
      new Map([
        [0, { offset: 0, messages: [{ role: "user", text: "第一页" }], hasMore: true, messageCount: 300 }],
        [100, { offset: 100, messages: [{ role: "user", text: "第三页" }], hasMore: true, messageCount: 300 }]
      ])
    );
    const { get } = await renderDetail(50);
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 0)).toBe(true));
    await act(async () => runtime.settle(0));
    await waitFor(() => expect(get().loading).toBe(false));

    await act(async () => {
      get().setPage(2);
    });
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 50)).toBe(true));
    await act(async () => {
      get().setPage(3);
    });
    await waitFor(() => expect(runtime.calls.some((item) => item.offset === 100)).toBe(true));

    // 第 3 页已经落地。
    await act(async () => runtime.settle(100));
    await waitFor(() => expect(get().messages[0]?.text).toBe("第三页"));

    // 旧的第 2 页这时失败：不能报错、更不能把第 3 页的结果清空。
    await act(async () => runtime.settle(50, "reject", new Error("第 2 页读不了")));
    expect(get().error).toBe("");
    expect(get().messages[0]?.text).toBe("第三页");
  });

  it("读失败时报错，并把消息清空——不留上一页的内容冒充这一页", async () => {
    window.__TAURI__ = {
      core: {
        invoke: async () => {
          throw new Error("文件读不了");
        }
      },
      event: { listen: async () => () => undefined }
    };
    const { get } = await renderDetail(50);
    await waitFor(() => {
      expect(get().error).toContain("文件读不了");
      expect(get().messages).toHaveLength(0);
    });
  });

  it("没有选中会话时不发请求", async () => {
    const runtime = fakeRuntime(new Map());
    function Probe() {
      useSessionDetail(null, null, 50);
      return null;
    }
    const view = render(<Probe />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(runtime.calls).toHaveLength(0);
    expect(view.container).toBeTruthy();
  });

  it("reload 会重新跑当前页", async () => {
    const runtime = fakeRuntime(
      new Map([[0, { offset: 0, messages: [], hasMore: false, messageCount: 5 }]])
    );
    const { get } = await renderDetail(50);
    await waitFor(() => expect(runtime.calls).toHaveLength(1));
    await act(async () => {
      get().reload();
    });
    await waitFor(() => expect(runtime.calls.length).toBeGreaterThan(1));
  });
});
