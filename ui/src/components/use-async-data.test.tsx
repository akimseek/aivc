// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { useCallback } from "react";
import { describe, expect, it } from "vitest";

import { useAsyncData } from "./use-async-data";

/**
 * `useAsyncData` 的重查语义：**跟着 `load` 的身份走**。
 *
 * # 这组测试为什么非有不可
 *
 * 批次二终审记下过一条：观测页改了区间 / 筛选 / 页码**不会重新取数**——effect 的
 * 依赖里没有 `load`，`reload` 又恒稳定，于是只在挂载时查一次。修法是把 `load` 加进
 * 依赖，而这条修法有**两个方向**的翻车方式，这里各钉一条：
 *
 * - 拿掉 `load` ⇒ 输入换了不重查（旧区间的数字摆在新区间旁边，看起来像后端错了）；
 * - 让 `load` 每次渲染都换身份 ⇒ 重渲染也要重查，重查 setState、setState 又重渲染，
 *   转成死循环。
 *
 * 因此「输入变了要重查」与「输入没变不许重查」必须成对断言。
 */

/** 每次取数用到的输入。放在组件外，免得它自己成为 `load` 的依赖。 */
const calls: string[] = [];

/** 模范调用方：`load` 用 `useCallback` 包住，依赖**恰好**是这次查询的输入。 */
function Probe({ filter }: { filter: string }) {
  const load = useCallback(async () => {
    calls.push(filter);
    return `结果：${filter}`;
  }, [filter]);
  const { data } = useAsyncData(load);
  return <span>{data ?? ""}</span>;
}

describe("useAsyncData：跟随 load 身份重查", () => {
  it("输入变化 ⇒ 重新取数；输入不变（重渲染）⇒ 不重复取数", async () => {
    calls.length = 0;
    const view = render(<Probe filter="近 7 天" />);
    await waitFor(() => expect(calls).toEqual(["近 7 天"]));

    // 父级重渲染但输入没变：多查一次就是「每次无关的界面刷新都打后端」。
    view.rerender(<Probe filter="近 7 天" />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(calls).toEqual(["近 7 天"]);

    // 输入换了：这一次必须真的重查——否则界面摆着旧区间的数字。
    view.rerender(<Probe filter="近 30 天" />);
    await waitFor(() => expect(calls).toEqual(["近 7 天", "近 30 天"]));
    // 落地的是新输入的结果（旧结果不许盖回来）。
    await waitFor(() => expect(view.container.textContent).toBe("结果：近 30 天"));
  });

  it("重查时在途的旧请求不许盖掉新结果", async () => {
    calls.length = 0;
    /** 什么时候让哪一次查询落地，由测试说了算——竞态窗口就开在这里。 */
    const settle = new Map<string, (value: string) => void>();

    function SlowProbe({ filter }: { filter: string }) {
      const load = useCallback(
        () =>
          new Promise<string>((resolve) => {
            calls.push(filter);
            settle.set(filter, resolve);
          }),
        [filter]
      );
      const { data } = useAsyncData(load);
      return <span>{data ?? ""}</span>;
    }

    const view = render(<SlowProbe filter="近 7 天" />);
    await waitFor(() => expect(calls).toEqual(["近 7 天"]));
    view.rerender(<SlowProbe filter="近 30 天" />);
    await waitFor(() => expect(calls).toEqual(["近 7 天", "近 30 天"]));

    // 先发的后回：那次结果必须被丢掉，否则界面会跳回旧区间的数字。
    await act(async () => {
      settle.get("近 7 天")?.("结果：近 7 天");
    });
    expect(view.container.textContent).toBe("");

    await act(async () => {
      settle.get("近 30 天")?.("结果：近 30 天");
    });
    expect(view.container.textContent).toBe("结果：近 30 天");
  });
});
