// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GatewayLogPanel } from "./gateway-log-panel";

/**
 * Gateway 日志面板。
 *
 * # 两组断言，两个来由
 *
 * 1. **「清空全部日志」**：它是菜单栏「帮助 → 清理 Gateway 日志」的**新家**
 *    （spec §11 的入口迁移表）。菜单栏被删掉之后，少了它这个动作就彻底没了；
 *    而它又是**不可撤销**的全清，因此钉住「必须先问一次」「取消时一个字节都不许删」。
 * 2. **分页（batch3 T3）**：分页下推到 Rust 之后，翻页必须**带上新页码重查**、
 *    页数由后端的 `total` 决定；「全选当前页」是字面意思（只勾这一页），
 *    并且翻页会清空勾选——不然「删除选中」会删掉屏幕上看不见的那些。
 */

/** 每页条数（面板里的常量，断言里要算页数）。 */
const PAGE_SIZE = 20;

type Call = { command: string; args: Record<string, unknown> | null };

/** 装一个假的 Tauri 运行时；`answer` 按命令名与参数给返回值。 */
function fakeRuntime(
  answer: (command: string, args: Record<string, unknown> | null) => unknown
) {
  const invocations: Call[] = [];
  window.__TAURI__ = {
    core: {
      invoke: async (command: string, args?: unknown) => {
        const payload = args as { command?: string; args?: Call["args"] } | undefined;
        const name = payload?.command ?? command;
        const call: Call = { command: name, args: payload?.args ?? null };
        invocations.push(call);
        return answer(name, call.args);
      }
    },
    event: { listen: async () => () => undefined }
  };
  return invocations;
}

/** 一条日志；只给渲染用得上的字段。 */
function entry(id: string) {
  return {
    requestId: id,
    createdAt: "2026-09-30T10:00:00.000Z",
    vendorId: "v1",
    providerId: "codex",
    outcome: "ok"
  };
}

describe("请求日志实时刷新", () => {
  it("收到请求记录事件后显示每条请求，并在卸载时解绑", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const entries: ReturnType<typeof entry>[] = [];
    fakeRuntime((command, args) => {
      if (command === "vendor:list") return [];
      if (command === "gateway:query-logs") return { items: [...entries], total: entries.length, page: args?.page, pageSize: args?.pageSize };
      return {};
    });
    let recorded: (() => void) | undefined;
    window.__TAURI__!.event!.listen = async (channel, handler) => {
      if (channel === "gateway:request-recorded") recorded = () => handler({ payload: {} });
      return () => { recorded = undefined; };
    };
    const view = render(<GatewayLogPanel onError={vi.fn()} />);
    await waitFor(() => expect(recorded).toBeTruthy());
    entries.push(entry("request-1"), entry("request-2"));
    await act(async () => { recorded!(); vi.advanceTimersByTime(1100); });
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(2));
    expect(view.container.textContent).toContain("request-1");
    expect(view.container.textContent).toContain("request-2");
    view.unmount();
    expect(recorded).toBeUndefined();
    vi.useRealTimers();
  });
});

function buttonNamed(container: HTMLElement, name: string): HTMLButtonElement {
  return [...container.querySelectorAll("button")].find((node) =>
    (node.textContent ?? "").includes(name)
  ) as HTMLButtonElement;
}

/**
 * 假后端：按请求的页码切出一页，`total` 固定——这正是真实命令的行为
 * （`{ items, total, page, pageSize }`，`total` 是匹配总数而不是本页条数）。
 */
function setup(
  options: { confirm?: boolean; removed?: number; total?: number; vendors?: unknown[] } = {}
) {
  const total = options.total ?? 0;
  const invocations = fakeRuntime((command, args) => {
    if (command === "gateway:query-logs") {
      const current = typeof args?.page === "number" ? args.page : 1;
      const size = typeof args?.pageSize === "number" ? args.pageSize : PAGE_SIZE;
      const start = (current - 1) * size;
      const count = Math.max(0, Math.min(size, total - start));
      return {
        items: Array.from({ length: count }, (_, index) => entry(`r${start + index + 1}`)),
        total,
        page: current,
        pageSize: size
      };
    }
    if (command === "gateway:clear-logs") return options.removed ?? 12;
    if (command === "gateway:delete-log-entries") return options.removed ?? 12;
    if (command === "vendor:list") return options.vendors ?? [];
    return null;
  });
  const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(options.confirm ?? true);
  const onError = vi.fn();
  const onNotify = vi.fn();
  const view = render(<GatewayLogPanel onError={onError} onNotify={onNotify} />);
  return {
    view,
    invocations,
    confirmSpy,
    onError,
    onNotify,
    clearButton: () => buttonNamed(view.container, "清空全部日志"),
    rowBoxes: () =>
      [...view.container.querySelectorAll<HTMLInputElement>('tbody input[type="checkbox"]')]
  };
}

beforeEach(() => {
  vi.restoreAllMocks();
});

afterEach(cleanup);

describe("GatewayLogPanel：清空全部日志", () => {
  it("删除选中与清空全部使用同一全局提示通道", async () => {
    const { view, rowBoxes, onNotify } = setup({ total: 1, removed: 1 });
    await waitFor(() => expect(rowBoxes()).toHaveLength(1));
    await act(async () => { rowBoxes()[0]!.click(); });
    await act(async () => { buttonNamed(view.container, "删除选中").click(); });
    expect(onNotify).toHaveBeenCalledWith("已删除 1 条日志。");
    expect(view.container.querySelector(".export-hint")).toBeNull();
  });
  it("工具行里有「清空全部日志」，且是危险样式", async () => {
    const { clearButton } = setup();
    expect(clearButton()).toBeTruthy();
    await waitFor(() => expect(clearButton()).toBeTruthy());
    // 不可撤销的动作必须在视觉上就能认出来（与其他「删除选中」一致）。
    expect(clearButton().className).toContain("btn-danger");
  });

  it("点击时先确认，确认文案说清是不可撤销的全清", async () => {
    const { confirmSpy, clearButton } = setup();
    await act(async () => {
      clearButton().click();
    });
    expect(confirmSpy).toHaveBeenCalledTimes(1);
    expect(confirmSpy.mock.calls[0]![0]).toContain("清空所有网关审计日志");
  });

  it("取消确认时**不删任何东西**", async () => {
    const { invocations, onNotify, clearButton } = setup({ confirm: false });
    await act(async () => {
      clearButton().click();
    });
    expect(invocations.some((item) => item.command === "gateway:clear-logs")).toBe(false);
    expect(onNotify).not.toHaveBeenCalled();
  });

  it("确认后清空并报告删了多少条", async () => {
    const { invocations, onNotify, clearButton } = setup({ confirm: true, removed: 12 });
    await act(async () => {
      clearButton().click();
    });

    await waitFor(() => expect(onNotify).toHaveBeenCalled());
    expect(invocations.some((item) => item.command === "gateway:clear-logs")).toBe(true);
    // 条数要说出来：静默完成与没执行在用户看来是一样的。
    expect(onNotify).toHaveBeenCalledWith("已清理 12 条 Gateway 日志。");
  });

  it("清空后重新查询（列表要跟着变空，而不是留着已经删掉的条目）", async () => {
    const { invocations, clearButton } = setup({ confirm: true });
    await waitFor(() => {
      expect(invocations.some((item) => item.command === "gateway:query-logs")).toBe(true);
    });
    const before = invocations.filter((item) => item.command === "gateway:query-logs").length;

    await act(async () => {
      clearButton().click();
    });

    await waitFor(() => {
      const after = invocations.filter((item) => item.command === "gateway:query-logs").length;
      expect(after).toBeGreaterThan(before);
    });
  });
});

describe("GatewayLogPanel：分页", () => {
  it("一条汇总日志展示重试次数、供应商尝试顺序与错误摘要", async () => {
    fakeRuntime((command) => {
      if (command === "vendor:list") return [];
      if (command === "gateway:query-logs") return {
        items: [{ ...entry("trace-1"), outcome: "error", retryCount: 2, switched: true,
          upstreamStatus: 401, errorCode: "UPSTREAM_RESPONSE_ERROR", errorMessage: "invalid key",
          attempts: [
            { number: 1, vendorId: "v1", vendorName: "供应商甲", upstreamStatus: 429, durationMs: 50, errorMessage: "rate limited" },
            { number: 2, vendorId: "v1", vendorName: "供应商甲", upstreamStatus: 429, durationMs: 50, errorMessage: "rate limited" },
            { number: 3, vendorId: "v2", vendorName: "供应商乙", upstreamStatus: 401, durationMs: 30, errorMessage: "invalid key" }
          ] }], total: 1, page: 1, pageSize: 20
      };
      return null;
    });
    const view = render(<GatewayLogPanel onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(1));
    expect(view.container.textContent).toContain("2 次 · 已切换");
    expect(view.container.querySelector("summary")?.textContent).toContain("3 次尝试");
    const attempts = [...view.container.querySelectorAll("details li")].map((li) => li.textContent);
    expect(attempts).toEqual([
      "供应商甲 · 429 · 50 ms · rate limited",
      "供应商甲 · 429 · 50 ms · rate limited",
      "供应商乙 · 401 · 30 ms · invalid key"
    ]);
  });
  it("供应商列显示名称，并在翻页后继续关联；名称查不到时保留编号", async () => {
    const { view } = setup({ total: 21, vendors: [{ id: "v1", name: "日志中转商" }] });
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(20));
    expect(view.container.querySelector("tbody tr")!.querySelectorAll("td")[3]!.textContent).toBe("日志中转商");
    await act(async () => { view.getByRole("button", { name: "下一页" }).click(); });
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(1));
    expect(view.container.querySelector("tbody tr")!.querySelectorAll("td")[3]!.textContent).toBe("日志中转商");
    view.unmount();
    const missing = setup({ total: 1 });
    await waitFor(() => expect(missing.view.container.querySelector("tbody tr")).toBeTruthy());
    expect(missing.view.container.querySelector("tbody tr")!.querySelectorAll("td")[3]!.textContent).toBe("v1");
  });

  it("首屏就按后端分页取数（带上 page/pageSize）", async () => {
    const { invocations } = setup({ total: 3 });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(1));
    expect(invocations.filter((item) => item.command === "gateway:query-logs")[0]!.command).toBe("gateway:query-logs");
    expect(invocations.filter((item) => item.command === "gateway:query-logs")[0]!.args).toMatchObject({ page: 1, pageSize: PAGE_SIZE });
  });

  it("翻页 ⇒ 带新页码重查，页数由后端的 total 决定；边界页不能再翻", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(1));
    // 21 条 = 两页。
    expect(view.container.textContent).toContain(`第 1 / 2 页`);
    expect(buttonNamed(view.container, "上一页").disabled).toBe(true);
    expect(buttonNamed(view.container, "第一页").disabled).toBe(true);
    expect(buttonNamed(view.container, "下一页").disabled).toBe(false);

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });

    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(2));
    expect(invocations.filter((item) => item.command === "gateway:query-logs")[1]!.args).toMatchObject({ page: 2, pageSize: PAGE_SIZE });
    // 最后一页：不能再往后翻，但可以回到第一页。
    await waitFor(() => expect(buttonNamed(view.container, "下一页").disabled).toBe(true));
    expect(buttonNamed(view.container, "上一页").disabled).toBe(false);

    await act(async () => {
      buttonNamed(view.container, "第一页").click();
    });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(3));
    expect(invocations.filter((item) => item.command === "gateway:query-logs")[2]!.args).toMatchObject({ page: 1 });
  });

  it("一页装得下时仍有完整分页栏和数量选择，翻页按钮禁用", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(1));
    expect(view.getByRole("navigation", { name: "请求日志分页" })).toBeTruthy();
    expect(buttonNamed(view.container, "下一页").disabled).toBe(true);
    expect(view.getByRole("combobox", { name: "每页数量" })).toBeTruthy();
  });

  it("切换每页数量下推到后端，同时回首页并清空勾选", async () => {
    const { view, invocations, rowBoxes } = setup({ total: 63 });
    const queries = () => invocations.filter((call) => call.command === "gateway:query-logs");
    await waitFor(() => expect(rowBoxes()).toHaveLength(20));
    await act(async () => { buttonNamed(view.container, "最后一页").click(); });
    await waitFor(() => expect(rowBoxes()).toHaveLength(3));
    await act(async () => { buttonNamed(view.container, "全选当前页").click(); });
    expect(buttonNamed(view.container, "删除选中").textContent).toBe("删除选中（3）");
    fireEvent.change(view.getByRole("combobox", { name: "每页数量" }), { target: { value: "50" } });
    await waitFor(() => expect(rowBoxes()).toHaveLength(50));
    expect(queries().at(-1)!.args).toMatchObject({ page: 1, pageSize: 50 });
    expect(buttonNamed(view.container, "删除选中").textContent).toBe("删除选中（0）");
    expect(rowBoxes().some((box) => box.checked)).toBe(false);
    fireEvent.change(view.getByRole("combobox", { name: "每页数量" }), { target: { value: "10" } });
    await waitFor(() => expect(rowBoxes()).toHaveLength(10));
    expect(queries().at(-1)!.args).toMatchObject({ page: 1, pageSize: 10 });
  });

  it("重查发现当前页已不存在时退到有效末页并重新查询", async () => {
    let total = 41;
    const calls = fakeRuntime((command, args) => {
      if (command === "vendor:list") return [];
      if (command !== "gateway:query-logs") return null;
      const current = Number(args?.page);
      const size = Number(args?.pageSize);
      return { items: current <= Math.ceil(total / size) ? [entry(`page-${current}`)] : [], total, page: current, pageSize: size };
    });
    const view = render(<GatewayLogPanel onError={vi.fn()} />);
    await waitFor(() => expect(view.container.querySelectorAll("tbody tr")).toHaveLength(1));
    await act(async () => { buttonNamed(view.container, "最后一页").click(); });
    await waitFor(() => expect(view.container.textContent).toContain("第 3 / 3 页"));
    total = 21;
    await act(async () => { buttonNamed(view.container, "重新查询").click(); });
    await waitFor(() => expect(view.container.textContent).toContain("第 2 / 2 页"));
    expect(calls.filter((call) => call.command === "gateway:query-logs").at(-1)!.args).toMatchObject({ page: 2 });
    total = 0;
    await act(async () => { buttonNamed(view.container, "重新查询").click(); });
    await waitFor(() => expect(view.container.textContent).toContain("第 1 / 1 页"));
    expect(view.container.querySelectorAll("tbody tr")).toHaveLength(0);
  });

  it("「全选当前页」只勾当前页，且翻页会清空勾选", async () => {
    const { view, invocations, rowBoxes } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(1));
    expect(rowBoxes()).toHaveLength(PAGE_SIZE);

    await act(async () => {
      buttonNamed(view.container, "全选当前页").click();
    });
    expect(rowBoxes().every((box) => box.checked)).toBe(true);
    // 计数是这一页的 20 条（不是 `total` 的 21 条）；用等值断言，
    // `toContain("20")` 会匹配到别的文本（页数、总数里都有 20）。
    expect(buttonNamed(view.container, "删除选中").textContent).toBe("删除选中（20）");

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(2));
    // 翻页清空勾选：屏幕上看不见的条目不该还留在「删除选中」里。
    expect(buttonNamed(view.container, "删除选中").textContent).toBe("删除选中（0）");
    expect(rowBoxes().every((box) => !box.checked)).toBe(true);
  });

  it("手动「重新查询」清空勾选，但留在当前页", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(1));

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(2));

    await act(async () => {
      buttonNamed(view.container, "全选当前页").click();
    });
    // 第 2 页只有 1 条。
    expect(buttonNamed(view.container, "删除选中").textContent).toBe("删除选中（1）");

    await act(async () => {
      buttonNamed(view.container, "重新查询").click();
    });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(3));

    // 刷新后行会位移：留着的旧勾选会让「删除选中（N）」数到屏幕上看不见的 id。
    expect(buttonNamed(view.container, "删除选中").textContent).toBe("删除选中（0）");
    // 它只是重查、不是换筛选：页码不动（还在第 2 页）。
    expect(invocations.filter((item) => item.command === "gateway:query-logs")[2]!.args).toMatchObject({ page: 2 });
  });

  it("删光当前页（且不是第一页）⇒ 退回上一页重查", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(1));

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });
    await waitFor(() => expect(invocations.filter((item) => item.command === "gateway:query-logs")).toHaveLength(2));

    // 第 2 页只有 1 条：勾上它、删掉，整页就空了。
    await act(async () => {
      buttonNamed(view.container, "全选当前页").click();
    });
    await act(async () => {
      buttonNamed(view.container, "删除选中").click();
    });

    const queries = () => invocations.filter((item) => item.command === "gateway:query-logs");
    await waitFor(() => expect(queries().length).toBeGreaterThan(2));
    // 留在空的第 2 页只会让人以为「删多了」：退回上一页才是自然结果。
    expect(queries()[queries().length - 1]!.args).toMatchObject({ page: 1 });
  });
});
