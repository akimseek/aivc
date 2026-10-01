// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Pager } from "./pager";

/**
 * 分页控件（P1-E 抽出的共用件）。
 *
 * 三处调用点的行为各自已有更细的套件钉住（`messages-modal` /
 * `gateway-log-panel` / `gateway-failure-panel` 的测试）；这里只钉 Pager
 * **自己的** API 契约：按钮序、边界禁用、回调实参、差异开关（`withLast` /
 * `extra` / `labelClassName` / `ariaLive` / `disabled`）。
 */

function setup(props: Partial<Parameters<typeof Pager>[0]> = {}) {
  const onPage = vi.fn();
  const view = render(
    <div>
      <Pager page={1} pageCount={3} onPage={onPage} {...props} />
    </div>
  );
  const buttons = [...view.container.querySelectorAll("button")];
  const byText = (label: string) =>
    buttons.find((item) => (item.textContent ?? "").trim() === label) as
      | HTMLButtonElement
      | undefined;
  return { view, onPage, buttons, byText };
}

describe("Pager", () => {
  it("按钮与页码序：第一页 / 上一页 / 第 x / y 页 / 下一页（默认没有「最后一页」）", () => {
    const { view, buttons, byText } = setup({ page: 2, pageCount: 3 });
    expect(buttons.map((item) => (item.textContent ?? "").trim())).toEqual([
      "第一页",
      "上一页",
      "下一页"
    ]);
    expect(byText("最后一页")).toBeUndefined();
    expect(view.container.textContent).toContain("第 2 / 3 页");
  });

  it("边界禁用：第 1 页禁前两个、最后一页禁后两个；disabled 与边界或起来", () => {
    const first = setup({ page: 1 });
    expect(first.byText("第一页")!.disabled).toBe(true);
    expect(first.byText("上一页")!.disabled).toBe(true);
    expect(first.byText("下一页")!.disabled).toBe(false);

    const last = setup({ page: 3 });
    expect(last.byText("下一页")!.disabled).toBe(true);
    expect(last.byText("上一页")!.disabled).toBe(false);

    // 额外禁用（弹框的「加载中」）：四个按钮一起禁，避免并发翻页。
    const loading = setup({ page: 2, disabled: true });
    for (const label of ["第一页", "上一页", "下一页"]) {
      expect(loading.byText(label)!.disabled).toBe(true);
    }
  });

  it("回调实参：第一页给 1、上一页给 page-1、下一页给 page+1（不钳制）", () => {
    const { onPage, byText } = setup({ page: 2 });
    act(() => {
      byText("第一页")!.click();
    });
    expect(onPage).toHaveBeenLastCalledWith(1);
    act(() => {
      byText("上一页")!.click();
    });
    expect(onPage).toHaveBeenLastCalledWith(1);
    act(() => {
      byText("下一页")!.click();
    });
    expect(onPage).toHaveBeenLastCalledWith(3);
  });

  it("withLast：末尾多一个「最后一页」，回传 pageCount", () => {
    const { onPage, buttons, byText } = setup({
      page: 2,
      pageCount: 3,
      withLast: true
    });
    expect(buttons.map((item) => (item.textContent ?? "").trim())).toEqual([
      "第一页",
      "上一页",
      "下一页",
      "最后一页"
    ]);
    act(() => {
      byText("最后一页")!.click();
    });
    expect(onPage).toHaveBeenLastCalledWith(3);
  });

  it("extra 渲染在按钮序列之前；labelClassName / ariaLive 落在页码文字上", () => {
    const { view } = setup({
      page: 2,
      extra: <span data-testid="extra">每页数量</span>,
      labelClassName: "messages-range",
      ariaLive: true
    });
    const extra = view.container.querySelector('[data-testid="extra"]')!;
    // `compareDocumentPosition`：extra 在第一个按钮**之前**。
    const firstButton = view.container.querySelector("button")!;
    expect(extra.compareDocumentPosition(firstButton) & Node.DOCUMENT_POSITION_FOLLOWING).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING
    );
    const range = view.container.querySelector(".messages-range")!;
    expect(range.textContent).toBe("第 2 / 3 页");
    expect(range.getAttribute("aria-live")).toBe("polite");
    // 默认档：不带 aria-live（卡片头那两处本来就没有）。
    const plain = setup();
    expect(plain.view.container.querySelector(".row-meta")!.getAttribute("aria-live")).toBeNull();
  });
});
