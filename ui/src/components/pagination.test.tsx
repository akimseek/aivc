// @vitest-environment jsdom
import { cleanup, fireEvent, render, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Pagination } from "./pagination";
import { useListPagination } from "./use-list-pagination";

afterEach(cleanup);

describe("Pagination", () => {
  it("展示总数和范围，翻页回调保留真实页码，末页禁用向后翻页", () => {
    const onPageChange = vi.fn();
    const view = render(<Pagination total={23} page={2} pageSize={10} onPageChange={onPageChange} onPageSizeChange={vi.fn()} />);
    expect(view.container.textContent).toContain("共 23 条 · 11–20");
    fireEvent.click(view.getByRole("button", { name: "最后一页" }));
    expect(onPageChange).toHaveBeenLastCalledWith(3);
    view.rerender(<Pagination total={23} page={3} pageSize={10} onPageChange={onPageChange} onPageSizeChange={vi.fn()} />);
    expect(view.container.textContent).toContain("21–23");
    expect((view.getByRole("button", { name: "下一页" }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(view.getByRole("button", { name: "第一页" }));
    expect(onPageChange).toHaveBeenLastCalledWith(1);
  });

  it("换每页数量同时回到首页；加载中禁用翻页和数量选择", () => {
    const onPageChange = vi.fn();
    const onPageSizeChange = vi.fn();
    const view = render(<Pagination total={53} page={3} pageSize={10} onPageChange={onPageChange} onPageSizeChange={onPageSizeChange} />);
    fireEvent.change(view.getByRole("combobox", { name: "每页数量" }), { target: { value: "50" } });
    expect(onPageChange).toHaveBeenLastCalledWith(1);
    expect(onPageSizeChange).toHaveBeenLastCalledWith(50);
    view.rerender(<Pagination total={53} page={3} pageSize={10} onPageChange={onPageChange} onPageSizeChange={onPageSizeChange} disabled />);
    expect((view.getByRole("combobox") as HTMLSelectElement).disabled).toBe(true);
    for (const button of view.getAllByRole("button")) expect((button as HTMLButtonElement).disabled).toBe(true);
  });

  it("空列表仍有分页栏，范围为 0–0，翻页按钮全部禁用", () => {
    const view = render(<Pagination total={0} page={1} pageSize={10} onPageChange={vi.fn()} onPageSizeChange={vi.fn()} />);
    expect(view.getByRole("navigation").textContent).toContain("共 0 条 · 0–0");
    expect(view.container.textContent).toContain("第 1 / 1 页");
    for (const button of view.getAllByRole("button")) expect((button as HTMLButtonElement).disabled).toBe(true);
  });
});

function List({ rows, filter = "all" }: { rows: number[]; filter?: string }) {
  const pagination = useListPagination(rows, filter);
  return <><output>{pagination.items.join(",")}</output><Pagination {...pagination} label="列表分页" /></>;
}

describe("useListPagination", () => {
  it("切片后记录既不遗漏也不重复，数据减少后收回到末页，筛选变化回到首页", () => {
    const rows = Array.from({ length: 23 }, (_, index) => index + 1);
    const view = render(<List rows={rows} />);
    const pager = within(view.getByRole("navigation"));
    expect(view.container.querySelector("output")!.textContent).toBe(rows.slice(0, 10).join(","));
    fireEvent.click(pager.getByRole("button", { name: "下一页" }));
    expect(view.container.querySelector("output")!.textContent).toBe(rows.slice(10, 20).join(","));
    fireEvent.click(pager.getByRole("button", { name: "最后一页" }));
    expect(view.container.querySelector("output")!.textContent).toBe("21,22,23");
    view.rerender(<List rows={rows.slice(0, 11)} />);
    expect(view.container.querySelector("output")!.textContent).toBe("11");
    expect(view.container.textContent).toContain("第 2 / 2 页");
    view.rerender(<List rows={rows} filter="codex" />);
    expect(view.container.querySelector("output")!.textContent).toBe(rows.slice(0, 10).join(","));
    fireEvent.change(pager.getByRole("combobox"), { target: { value: "20" } });
    expect(view.container.querySelector("output")!.textContent).toBe(rows.slice(0, 20).join(","));
    view.rerender(<List rows={[]} filter="codex" />);
    expect(view.container.querySelector("output")!.textContent).toBe("");
    expect(view.container.textContent).toContain("第 1 / 1 页");
  });
});
