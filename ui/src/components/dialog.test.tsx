// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Dialog } from "./dialog";

/**
 * 弹层外壳的 Escape 语义。
 *
 * # 为什么必须有「同类之间让位」
 *
 * 每个 `Dialog` 都在 `document` 上监听键盘，而它们之间没有父子关系。不判定的话，
 * 一次 Escape 会**同时**收掉所有叠着的层——退出确认框（后端推来，随时可能压在
 * 用户正填着的表单上）与新会话弹框一起消失，用户丢的是刚敲了一半的工作目录。
 *
 * `overlay-stack.ts` 的类名单解决不了这件事：`.dialog-overlay` 是**每一个**
 * `Dialog` 的类名，拿它问「上面还有没有 Dialog」只会得到「我自己」。同类之间
 * 只有挂载顺序说得清，所以判定落在 `dialog.tsx` 里的实例栈上。
 *
 * 本工程的 vitest 没开 `globals`，testing-library 注册不上自动清理——不手动
 * `cleanup` 的话，上一个用例挂着的实例会在下一次按键时一起响应。
 */
afterEach(cleanup);

/**
 * 两层叠放：下层是「用户打开的新会话弹框」，上层模拟「后端推来的退出确认框」
 * ——它永远挂得晚，因此那一次 Escape 该归它。
 */
function Stacked() {
  const [top, setTop] = useState(true);
  const [bottom, setBottom] = useState(true);
  return (
    <>
      {bottom && (
        <Dialog title="下层弹框" onClose={() => setBottom(false)} footer={<></>}>
          <p>下层内容</p>
        </Dialog>
      )}
      {top && (
        <Dialog title="上层弹框" onClose={() => setTop(false)} footer={<></>}>
          <p>上层内容</p>
        </Dialog>
      )}
    </>
  );
}

describe("Dialog", () => {
  it("单层：Escape 关闭；点遮罩默认**不**关闭", () => {
    const onClose = vi.fn();
    const view = render(
      <Dialog title="单层弹框" onClose={onClose} footer={<></>}>
        <p>内容</p>
      </Dialog>
    );

    // 默认 `closeOnOverlay === false`：里面可能有敲了一半的东西，点偏一下不该丢。
    fireEvent.click(view.container.querySelector(".dialog-overlay")!);
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("`closeOnOverlay` 为真时点遮罩关闭（既有语义不动）", () => {
    const onClose = vi.fn();
    const view = render(
      <Dialog title="单层弹框" closeOnOverlay onClose={onClose} footer={<></>}>
        <p>内容</p>
      </Dialog>
    );

    fireEvent.click(view.container.querySelector(".dialog-overlay")!);
    expect(onClose).toHaveBeenCalledTimes(1);

    // 点面板内部不算「点了外面」——遮罩上的处理器不该被面板的点击借道。
    fireEvent.click(view.getByText("内容"));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("两层叠放：一次 Escape 只收上层，下层**还在**；再按一次才轮到下层", () => {
    const view = render(<Stacked />);
    expect(view.getByText("上层内容")).toBeTruthy();
    expect(view.getByText("下层内容")).toBeTruthy();

    fireEvent.keyDown(document, { key: "Escape" });
    // 上层收了，下层原封不动——用户在下层填的东西没丢。
    expect(view.queryByText("上层内容")).toBeNull();
    expect(view.getByText("下层内容")).toBeTruthy();

    // 栈也正确地弹出了一层：下一次 Escape 归现在还开着的那一个。
    fireEvent.keyDown(document, { key: "Escape" });
    expect(view.queryByText("下层内容")).toBeNull();
  });

  it("叠放的层各自卸载后互不影响：关掉上层不改变单层时的行为", () => {
    const view = render(<Stacked />);

    // 先关上层（点它自己的遮罩路径走的是 Escape，这里用控件更直接：直接按两次）。
    fireEvent.keyDown(document, { key: "Escape" });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(view.container.querySelector(".dialog-overlay")).toBeNull();

    // 全部卸载之后再按一次不该抛错（栈已空）。
    fireEvent.keyDown(document, { key: "Escape" });
    expect(view.container.childElementCount).toBe(0);
  });

  it("压在上面的浮层（`.popover`）优先：Dialog 那一次让位", () => {
    const onClose = vi.fn();
    const view = render(
      <Dialog title="弹框" onClose={onClose} footer={<></>}>
        <p>内容</p>
      </Dialog>
    );

    // 与右键菜单、设置中心同一条口径：跨类仍按类名单判。
    const popover = document.createElement("div");
    popover.className = "popover";
    document.body.appendChild(popover);
    try {
      fireEvent.keyDown(document, { key: "Escape" });
      expect(onClose).not.toHaveBeenCalled();
    } finally {
      popover.remove();
    }

    // 它让开之后，Escape 仍然是弹框的关闭键。
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(view.getByText("内容")).toBeTruthy();
  });

  it("面板是 role=dialog + aria-modal，且焦点进面板（键盘用户不从背后开始 Tab）", () => {
    const view = render(
      <Dialog title="弹框标题" onClose={() => undefined} footer={<></>}>
        <p>内容</p>
      </Dialog>
    );
    const panel = view.getByRole("dialog");
    expect(panel.getAttribute("aria-modal")).toBe("true");
    // 不传 `ariaLabel` 时用标题文本当无障碍名称。
    expect(panel.getAttribute("aria-label")).toBe("弹框标题");
    expect(document.activeElement).toBe(panel);
  });
});
