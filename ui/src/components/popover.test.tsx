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
});
