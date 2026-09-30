// @vitest-environment jsdom
import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ListRow } from "./list";

/**
 * 通用行的键盘可达性。
 *
 * # 为什么这一条单独测
 *
 * `ListRow` 是快捷访问（收藏 / 最近）与全局搜索结果共用的行——它们都只挂了一个
 * `onClick`。行本体是 `<div>`，没有 `role` / `tabIndex` / 键处理时，这三处列表
 * 在键盘上**完全够不着**：会话卡片改了也只修好了主列表。
 *
 * 反过来，**没有 `onClick` 的行是纯展示**（例如配置面里那些只读行），
 * 它们不该进 Tab 序、也不该被读成按钮。
 */
describe("ListRow", () => {
  it("可点的行：role=button + tabIndex=0，Enter / Space 触发 onClick", () => {
    const onClick = vi.fn();
    const view = render(<ListRow title="一条结果" onClick={onClick} />);
    const row = view.container.querySelector<HTMLElement>(".row")!;
    expect(row.getAttribute("role")).toBe("button");
    expect(row.tabIndex).toBe(0);

    fireEvent.keyDown(row, { key: "Enter" });
    expect(onClick).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(row, { key: " " });
    expect(onClick).toHaveBeenCalledTimes(2);

    // 方向键不被行劫持；其他键什么都不做。
    fireEvent.keyDown(row, { key: "ArrowDown" });
    fireEvent.keyDown(row, { key: "a" });
    expect(onClick).toHaveBeenCalledTimes(2);
  });

  it("纯展示的行不进 Tab 序、不被读成按钮", () => {
    const view = render(<ListRow title="只读的一行" />);
    const row = view.container.querySelector<HTMLElement>(".row")!;
    expect(row.getAttribute("role")).toBeNull();
    expect(row.tabIndex).toBe(-1);
  });
});
