// @vitest-environment jsdom
import { act, render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

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

function buttonNamed(container: HTMLElement, name: string): HTMLButtonElement {
  return [...container.querySelectorAll("button")].find((node) =>
    (node.textContent ?? "").includes(name)
  ) as HTMLButtonElement;
}

/**
 * 假后端：按请求的页码切出一页，`total` 固定——这正是真实命令的行为
 * （`{ entries, total, page, pageSize }`，`total` 是匹配总数而不是本页条数）。
 */
function setup(
  options: { confirm?: boolean; removed?: number; total?: number } = {}
) {
  const total = options.total ?? 0;
  const invocations = fakeRuntime((command, args) => {
    if (command === "gateway:query-logs") {
      const current = typeof args?.page === "number" ? args.page : 1;
      const size = typeof args?.pageSize === "number" ? args.pageSize : PAGE_SIZE;
      const start = (current - 1) * size;
      const count = Math.max(0, Math.min(size, total - start));
      return {
        entries: Array.from({ length: count }, (_, index) => entry(`r${start + index + 1}`)),
        total,
        page: current,
        pageSize: size
      };
    }
    if (command === "gateway:clear-logs") return options.removed ?? 12;
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

describe("GatewayLogPanel：清空全部日志", () => {
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
  it("首屏就按后端分页取数（带上 page/pageSize）", async () => {
    const { invocations } = setup({ total: 3 });
    await waitFor(() => expect(invocations).toHaveLength(1));
    expect(invocations[0]!.command).toBe("gateway:query-logs");
    expect(invocations[0]!.args).toMatchObject({ page: 1, pageSize: PAGE_SIZE });
  });

  it("翻页 ⇒ 带新页码重查，页数由后端的 total 决定；边界页不能再翻", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations).toHaveLength(1));
    // 21 条 = 两页。
    expect(view.container.textContent).toContain(`第 1 / 2 页`);
    expect(buttonNamed(view.container, "上一页").disabled).toBe(true);
    expect(buttonNamed(view.container, "第一页").disabled).toBe(true);
    expect(buttonNamed(view.container, "下一页").disabled).toBe(false);

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });

    await waitFor(() => expect(invocations).toHaveLength(2));
    expect(invocations[1]!.args).toMatchObject({ page: 2, pageSize: PAGE_SIZE });
    // 最后一页：不能再往后翻，但可以回到第一页。
    await waitFor(() => expect(buttonNamed(view.container, "下一页").disabled).toBe(true));
    expect(buttonNamed(view.container, "上一页").disabled).toBe(false);

    await act(async () => {
      buttonNamed(view.container, "第一页").click();
    });
    await waitFor(() => expect(invocations).toHaveLength(3));
    expect(invocations[2]!.args).toMatchObject({ page: 1 });
  });

  it("一页装得下时不出现分页控件", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE });
    await waitFor(() => expect(invocations).toHaveLength(1));
    expect(
      [...view.container.querySelectorAll("button")].some((node) =>
        (node.textContent ?? "").includes("下一页")
      )
    ).toBe(false);
  });

  it("「全选当前页」只勾当前页，且翻页会清空勾选", async () => {
    const { view, invocations, rowBoxes } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations).toHaveLength(1));
    expect(rowBoxes()).toHaveLength(PAGE_SIZE);

    await act(async () => {
      buttonNamed(view.container, "全选当前页").click();
    });
    expect(rowBoxes().every((box) => box.checked)).toBe(true);
    // 计数是这一页的 20 条（不是 `total` 的 21 条）。
    expect(buttonNamed(view.container, "删除选中").textContent).toContain("20");

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });
    await waitFor(() => expect(invocations).toHaveLength(2));
    // 翻页清空勾选：屏幕上看不见的条目不该还留在「删除选中」里。
    expect(buttonNamed(view.container, "删除选中").textContent).toContain("0");
    expect(rowBoxes().every((box) => !box.checked)).toBe(true);
  });

  it("删光当前页（且不是第一页）⇒ 退回上一页重查", async () => {
    const { view, invocations } = setup({ total: PAGE_SIZE + 1 });
    await waitFor(() => expect(invocations).toHaveLength(1));

    await act(async () => {
      buttonNamed(view.container, "下一页").click();
    });
    await waitFor(() => expect(invocations).toHaveLength(2));

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
