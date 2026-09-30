// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TitleBar } from "./title-bar";
// 源码原文（Vite 的 `?raw`）。**不能**用 `readFileSync` + `import.meta.url`：本文件是
// jsdom 环境，`import.meta.url` 是 http 形式的，`fileURLToPath` 会直接抛。
import titleBarSource from "./title-bar.tsx?raw";

/**
 * 标题栏的**窗口控制契约**。
 *
 * # 为什么这些断言值得单独钉住
 *
 * 无边框之后，用户能不能挪窗口、关窗口全落在这一条标题栏上。按钮接没接回调由 props
 * 的必填性兜住，但**事件怎么绑**类型系统看不见：
 *
 * - 拖拽与双击最大化只能由**同一次 mousedown 按 `detail` 分流**。后端 `start_dragging()`
 *   一按下就进系统的模态移动循环，`dblclick` 能不能送达无法保证，因此不能把最大化挂在
 *   `dblclick` 上；反过来，一次双击的第二次 mousedown（`detail === 2`）若也去发起拖拽，
 *   双击就会退化成「拖走窗口」；
 * - 两次触发源（`detail === 2` 与 `dblclick`）并存时，一次双击会把最大化切换两次
 *   ——等于什么都没做，所以 `dblclick` 必须没有绑定；
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
  it("左键单击的 mousedown（detail 1）发起拖拽", () => {
    const { view, onStartDrag, onToggleMaximize } = setup();
    fireEvent.mouseDown(dragArea(view.container), { button: 0, detail: 1 });
    expect(onStartDrag).toHaveBeenCalledTimes(1);
    expect(onToggleMaximize).not.toHaveBeenCalled();
  });

  it("右键按下既不拖拽也不最大化", () => {
    const { view, onStartDrag, onToggleMaximize } = setup();
    const area = dragArea(view.container);
    // 左键的两种 detail 都试一遍：排掉右键要按 button 判断，而不是碰巧没命中 detail。
    fireEvent.mouseDown(area, { button: 2, detail: 1 });
    fireEvent.mouseDown(area, { button: 2, detail: 2 });
    expect(onStartDrag).not.toHaveBeenCalled();
    expect(onToggleMaximize).not.toHaveBeenCalled();
  });

  it("双击的第二次 mousedown（detail 2）只切换最大化，不发起拖拽", () => {
    const { view, onStartDrag, onToggleMaximize } = setup();
    const area = dragArea(view.container);

    // 真实双击的事件序列：第一次按下（detail 1）→ 第二次按下（detail 2）→ dblclick。
    // 两次按下走的是同一个 handler，分流全靠 detail。
    fireEvent.mouseDown(area, { button: 0, detail: 1 });
    expect(onStartDrag).toHaveBeenCalledTimes(1);

    // 清掉第一次的计数，好断言第二次按下**只**做最大化——这正是原先被拖拽吃掉的那一下。
    onStartDrag.mockClear();
    fireEvent.mouseDown(area, { button: 0, detail: 2 });
    expect(onStartDrag).not.toHaveBeenCalled();
    expect(onToggleMaximize).toHaveBeenCalledTimes(1);
  });

  it("dblclick 本身没有绑定动作（否则与 detail 2 叠加，一次双击切换两次）", () => {
    const { view, onToggleMaximize } = setup();
    // 只派发 dblclick、不派发第二次 mousedown：模拟「系统吞掉了后续 mousedown」的情形。
    // 切换只能来自 mousedown，这里必须一次都不触发。
    fireEvent.doubleClick(dragArea(view.container));
    expect(onToggleMaximize).not.toHaveBeenCalled();
  });

  it("拖拽区不渲染内联样式（弱守卫，强守卫见下一条）", () => {
    // 两套拖拽机制（CSS 属性 + 命令）同时存在会互相打架：系统先按 drag 区域吃掉
    // mousedown，命令就再也发不出去。这里钉住「只有命令这一套」。
    //
    // 注意这条**强度有限**：jsdom 不会把 `WebkitAppRegion` 写进 `style` 属性，
    // 因此它在改动前也是通过的。真正拦得住回归的是下一条「读源码」的断言。
    const { view } = setup();
    expect(dragArea(view.container).getAttribute("style")).toBeNull();
  });

  it("源码里不再出现 WebkitAppRegion（连字符形式只出现在注释里，不会误伤）", () => {
    // 上一条断言在 jsdom 下拦不住「把内联样式加回来」：React 不会把它序列化到
    // style 属性上。这里直接读组件源码——`WebkitAppRegion` 是 React 的驼峰键名，
    // 只会出现在**内联样式对象**里；注释里写的是 `-webkit-app-region`（连字符），
    // 两者拼写不同，所以这条守卫不会因为解释性注释而误报。
    expect(titleBarSource).not.toContain("WebkitAppRegion");
  });
});
