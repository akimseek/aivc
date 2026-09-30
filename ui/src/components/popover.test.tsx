// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { createRef } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { Popover } from "./popover";

/**
 * 浮层的关闭行为。
 *
 * 两条必须有的：**点外部关闭**（不关的话浮层会挡住下面的内容且找不到出口）与
 * **Escape 关闭**（键盘用户的唯一出口）。
 */

// 本工程的 vitest 没开 `globals`，testing-library 就注册不上自己的自动清理，
// 上一个用例渲染的 DOM 会留在 body 里——后面用 `getByText` 会撞见两份同样的文字。
afterEach(cleanup);

describe("Popover", () => {
  it("打开时渲染内容，关闭时不渲染", () => {
    const anchor = createRef<HTMLButtonElement>();
    const closed = render(
      <button ref={anchor}>
        <Popover open={false} onClose={() => undefined} anchorRef={anchor}>
          内容
        </Popover>
      </button>
    );
    expect(closed.queryByText("内容")).toBeNull();

    const opened = render(
      <button ref={anchor}>
        <Popover open onClose={() => undefined} anchorRef={anchor}>
          内容
        </Popover>
      </button>
    );
    expect(opened.getByText("内容")).toBeTruthy();
  });

  it("点浮层外部与按 Escape 都会调 onClose", () => {
    const anchor = createRef<HTMLButtonElement>();
    const onClose = vi.fn();
    const view = render(
      <div>
        <button ref={anchor}>锚</button>
        <Popover open onClose={onClose} anchorRef={anchor}>
          内容
        </Popover>
      </div>
    );
    fireEvent.mouseDown(document.body);
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(2);

    // 点内部不该关。
    fireEvent.mouseDown(view.getByText("内容"));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it("上面还压着模态时**让位**——那一次 Escape 归模态", () => {
    // 退出确认框（`.dialog-overlay`）随时可能从后端推来并与浮层同时在场。
    // 从前它是唯一一个「开着就吃 Escape」的浮层：一次按键会同时收掉浮层与那个框，
    // 而框收掉等于替用户回答了「取消」——后端本来在等一个明确的回答。
    const anchor = createRef<HTMLButtonElement>();
    const onClose = vi.fn();
    render(
      <div>
        <button ref={anchor}>锚</button>
        <Popover open onClose={onClose} anchorRef={anchor}>
          内容
        </Popover>
      </div>
    );

    const modal = document.createElement("div");
    modal.className = "dialog-overlay";
    document.body.appendChild(modal);
    try {
      fireEvent.keyDown(document, { key: "Escape" });
      expect(onClose).not.toHaveBeenCalled();
    } finally {
      modal.remove();
    }

    // 那一层收掉之后，Escape 仍然是浮层的关闭键。
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("**嵌在**模态里面的浮层不被自己的外层容器挡住（消息弹框的导出菜单）", () => {
    // `.messages-overlay` 在这里是容器而不是压在上面的一层。不区分的话两者会互相
    // 让位：浮层说「上面有模态」，模态说「上面有浮层」，Escape 谁都不响应。
    const anchor = createRef<HTMLButtonElement>();
    const onClose = vi.fn();
    render(
      <div className="messages-overlay">
        <button ref={anchor}>导出</button>
        <Popover open onClose={onClose} anchorRef={anchor}>
          导出 Markdown
        </Popover>
      </div>
    );

    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("`ariaLabel` 接到面板上（读屏只念得出「对话框」是不够的）", () => {
    const anchor = createRef<HTMLButtonElement>();
    const view = render(
      <div>
        <button ref={anchor}>锚</button>
        <Popover open onClose={() => undefined} anchorRef={anchor} ariaLabel="附件菜单">
          内容
        </Popover>
      </div>
    );

    expect(view.getByRole("dialog").getAttribute("aria-label")).toBe("附件菜单");
  });

  it("锚滚动或被窗口缩放带走后，浮层跟着挪回锚边（不脱锚）", () => {
    // 浮层用的是**视口坐标**。不跟随的话，消息列表一滚，附件菜单就悬在半空——
    // 还在屏幕上，却已经指不到触发它的那个按钮。
    const anchor = createRef<HTMLButtonElement>();
    const view = render(
      <div>
        <button ref={anchor}>锚</button>
        <Popover open onClose={() => undefined} anchorRef={anchor}>
          内容
        </Popover>
      </div>
    );
    const panel = view.getByRole("dialog");
    /** jsdom 里所有 rect 都是 0，只能自己给几何。 */
    const rect = (over: { left?: number; top?: number; bottom?: number; width?: number; height?: number }) =>
      ({
        left: 0,
        top: 0,
        bottom: 0,
        right: 0,
        width: 0,
        height: 0,
        x: 0,
        y: 0,
        toJSON: () => ({}),
        ...over
      }) as DOMRect;
    const anchorRect = vi
      .spyOn(anchor.current!, "getBoundingClientRect")
      .mockReturnValue(rect({ left: 100, top: 100, bottom: 130, width: 80, height: 30 }));
    vi.spyOn(panel, "getBoundingClientRect").mockReturnValue(rect({ width: 200, height: 100 }));

    // 滚动一回：浮层落在锚的下方（`bottom + 6`）、左对齐。
    fireEvent.scroll(window);
    expect(panel.style.top).toBe("136px");
    expect(panel.style.left).toBe("100px");

    // 锚被滚下去之后，浮层要跟到新的位置。
    anchorRect.mockReturnValue(rect({ left: 100, top: 300, bottom: 330, width: 80, height: 30 }));
    fireEvent.scroll(window);
    expect(panel.style.top).toBe("336px");

    // 窗口缩放同理（走的是同一个重算函数）。
    anchorRect.mockReturnValue(rect({ left: 100, top: 100, bottom: 130, width: 80, height: 30 }));
    fireEvent.resize(window);
    expect(panel.style.top).toBe("136px");
  });
});
