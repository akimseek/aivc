// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { GatewayRangePicker } from "./gateway-range-picker";

/**
 * 「时间范围」选择器。
 *
 * # 为什么单独测这个组件
 *
 * 三个观测面板（用量 / 异常 / 日志）都用它。旧平台是**三处各写一遍**，
 * 于是三处的提示语与行为略有不同——用户会觉得「同一件事在不同页面上不一样」。
 * 合成一个组件之后，这里测一遍就等于三处都测了。
 *
 * 这里钉的两条是旧平台**明确写给用户看的话**（`请选择完整的起止日期。` /
 * `起始日期不能晚于结束日期。`）。只把按钮置灰而不说原因的话，
 * 用户不知道差什么、也不知道自己填反了，会以为页面坏了。
 */

function findSelect(container: HTMLElement) {
  return container.querySelector<HTMLSelectElement>('select[aria-label="时间范围"]')!;
}

function setInput(element: HTMLInputElement | HTMLSelectElement, value: string) {
  const proto =
    element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, "value")!.set!.call(element, value);
  element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", {
    bubbles: true
  }));
}

describe("GatewayRangePicker", () => {
  it("预设区间**不显示**日期框", () => {
    const view = render(
      <GatewayRangePicker range="7d" start="" end="" onChange={vi.fn()} />
    );
    expect(view.container.querySelector('input[aria-label="开始日期"]')).toBeNull();
    expect(view.container.querySelector('input[aria-label="结束日期"]')).toBeNull();
  });

  it("选「自定义」后才出现开始与结束日期", () => {
    const view = render(
      <GatewayRangePicker range="custom" start="" end="" onChange={vi.fn()} />
    );
    expect(view.container.querySelector('input[aria-label="开始日期"]')).toBeTruthy();
    expect(view.container.querySelector('input[aria-label="结束日期"]')).toBeTruthy();
  });

  it("**只填一端时提示「请选择完整的起止日期。」**", () => {
    const view = render(
      <GatewayRangePicker range="custom" start="2025-10-01" end="" onChange={vi.fn()} />
    );
    expect(view.container.textContent).toContain("请选择完整的起止日期。");
  });

  it("**起止颠倒时提示「起始日期不能晚于结束日期。」**", () => {
    const view = render(
      <GatewayRangePicker range="custom" start="2025-10-08" end="2025-10-01" onChange={vi.fn()} />
    );
    expect(view.container.textContent).toContain("起始日期不能晚于结束日期。");
  });

  it("填全且顺序正确时没有提示", () => {
    const view = render(
      <GatewayRangePicker range="custom" start="2025-10-01" end="2025-10-08" onChange={vi.fn()} />
    );
    expect(view.container.textContent).not.toContain("请选择完整的起止日期。");
    expect(view.container.textContent).not.toContain("起始日期不能晚于结束日期。");
  });

  it("预设区间不显示日期相关提示（自定义才需要）", () => {
    const view = render(<GatewayRangePicker range="all" start="" end="" onChange={vi.fn()} />);
    expect(view.container.textContent).not.toContain("请选择完整的起止日期。");
  });

  it("日期与区间的选择都会回报给调用方", async () => {
    const onChange = vi.fn();
    const view = render(
      <GatewayRangePicker range="custom" start="" end="" onChange={onChange} />
    );
    await act(async () => {
      setInput(view.container.querySelector<HTMLInputElement>('input[aria-label="开始日期"]')!, "2025-10-01");
    });
    expect(onChange).toHaveBeenCalledWith({ range: "custom", start: "2025-10-01", end: "" });
    await act(async () => {
      setInput(findSelect(view.container), "today");
    });
    expect(onChange).toHaveBeenCalledWith({ range: "today", start: "", end: "" });
  });
});
