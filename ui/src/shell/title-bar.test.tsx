// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TitleBar } from "./title-bar";

/**
 * 标题栏的**窗口控制契约**。
 *
 * # 为什么这些断言值得单独钉住
 *
 * 无边框之后，用户能不能挪窗口、关窗口全落在这一条标题栏上。按钮接没接回调由 props
 * 的必填性兜住，但**事件怎么绑**类型系统看不见：
 *
 * - 双击最大化的那次双击里包含**两次 mousedown**（第二次 `detail === 2`）。若两次都去
 *   发起拖拽，双击就会退化成「拖走窗口」，最大化永远触发不了；
 * - 拖拽只能由左键发起（右键该走上下文菜单，不是挪窗口）。
 *
 * 因此这里逐条断言，而不是只断言「元素渲染出来了」。
 */
function setup() {
  const handlers = {
    onCycleTheme: vi.fn(),
    onSelectAttention: vi.fn(),
    onDismissAttention: vi.fn(),
    onMinimize: vi.fn(),
    onToggleMaximize: vi.fn(),
    onStartDrag: vi.fn(),
    onClose: vi.fn()
  };
  const view = render(
    <TitleBar version="0.1.0" preference="system" resolved="light" attention={[]} {...handlers} />
  );
  return { view, ...handlers };
}

function button(container: HTMLElement, label: string): HTMLElement {
  const found = container.querySelector<HTMLElement>(`[aria-label="${label}"]`);
  if (!found) throw new Error(`找不到按钮：${label}`);
  return found;
}

function dragArea(container: HTMLElement): HTMLElement {
  const found = container.querySelector<HTMLElement>(".title-drag");
  if (!found) throw new Error("找不到拖拽区");
  return found;
}

describe("TitleBar 的窗口控制按钮", () => {
  it("三个按钮都渲染出来，点击分别触发最小化 / 最大化 / 关闭", () => {
    const { view, onMinimize, onToggleMaximize, onClose } = setup();

    const minimize = button(view.container, "最小化窗口");
    const maximize = button(view.container, "最大化窗口");
    const close = button(view.container, "关闭窗口");
    // 落在标题栏右侧那一组里（而不是飘在品牌区）。
    const group = view.container.querySelector(".title-window")!;
    expect(group.contains(minimize)).toBe(true);
    expect(group.contains(maximize)).toBe(true);
    expect(group.contains(close)).toBe(true);

    fireEvent.click(minimize);
    fireEvent.click(maximize);
    fireEvent.click(close);
    expect(onMinimize).toHaveBeenCalledTimes(1);
    expect(onToggleMaximize).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe("TitleBar 的拖拽区", () => {
  it("左键单击的 mousedown 发起拖拽", () => {
    const { view, onStartDrag } = setup();
    fireEvent.mouseDown(dragArea(view.container), { button: 0, detail: 1 });
    expect(onStartDrag).toHaveBeenCalledTimes(1);
  });

  it("右键按下不发起拖拽", () => {
    const { view, onStartDrag } = setup();
    fireEvent.mouseDown(dragArea(view.container), { button: 2, detail: 1 });
    expect(onStartDrag).not.toHaveBeenCalled();
  });

  it("双击的第二次 mousedown（detail 2）不发起拖拽，双击本身切换最大化", () => {
    const { view, onStartDrag, onToggleMaximize } = setup();
    const area = dragArea(view.container);

    // 真实双击的事件序列：第一次按下（detail 1）→ 第二次按下（detail 2）→ dblclick。
    fireEvent.mouseDown(area, { button: 0, detail: 1 });
    fireEvent.mouseDown(area, { button: 0, detail: 2 });
    fireEvent.doubleClick(area);

    expect(onStartDrag).toHaveBeenCalledTimes(1);
    expect(onToggleMaximize).toHaveBeenCalledTimes(1);
  });

  it("拖拽区不再带 -webkit-app-region 内联样式", () => {
    // 两套拖拽机制（CSS 属性 + 命令）同时存在会互相打架：系统先按 drag 区域吃掉
    // mousedown，命令就再也发不出去。这里钉住「只有命令这一套」。
    const { view } = setup();
    expect(dragArea(view.container).getAttribute("style")).toBeNull();
  });
});
