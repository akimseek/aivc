// @vitest-environment jsdom
import { cleanup, createEvent, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

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
 *
 * # 手动清理
 *
 * 本工程的 vitest 没开 `globals`，testing-library 注册不上自动清理。这一组用
 * `container` 查询还不受影响，但「插槽里有什么」那条要按角色找按钮——
 * 上一个用例留下的 DOM 会让它撞见两份。
 */
afterEach(cleanup);

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

  it("行内控件自己吃 Enter / Space：行**让位**，既不触发行点击也不取消它的激活", () => {
    // 潜伏项：今天的调用方（快捷访问 / 全局搜索）没往 `leading` / `trailing` 插槽里
    // 放可聚焦元素，所以没有真实症状；但少了守卫，将来一放就会复现会话卡片那类劫持
    // ——点勾选顺带触发行点击，而且 `preventDefault()` 还会让勾选框勾不动。
    const onClick = vi.fn();
    const onToggle = vi.fn();
    const view = render(
      <ListRow
        title="一条结果"
        leading={<input type="checkbox" aria-label="选择这一条" onChange={onToggle} />}
        onClick={onClick}
      />
    );
    const box = view.container.querySelector<HTMLInputElement>("input")!;

    for (const key of [" ", "Enter"]) {
      const event = createEvent.keyDown(box, { key });
      fireEvent(box, event);
      // 行不抢键：既不该自己触发点击，也不该取消勾选框自己的激活。
      expect(event.defaultPrevented).toBe(false);
      expect(onClick).not.toHaveBeenCalled();
    }

    // jsdom **不实现**原生控件的键盘激活（按 Space 不会真的变成 click），这里补上
    // 浏览器随后会合成的那一步，证明勾选这一路仍然有效。行自己的 `onClick` 会不会
    // 被这次点击冒泡上来，取决于调用方给插槽控件加了什么——见下一条。
    fireEvent.click(box);
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it("插槽里的按钮自己吃点击：一次点击只做一件事", () => {
    // 鼠标那一半的让位。行内插槽里放的是「勾选 / 收藏 / 打开菜单」这类**自己的**动作，
    // 再顺带触发行点击（在这里就是「打开这条会话」）就是一次点击做两件事——
    // 而其中一件用户根本没要求。
    const onClick = vi.fn();
    const onAction = vi.fn();
    const view = render(
      <ListRow
        title="一条结果"
        trailing={
          <button type="button" onClick={onAction}>
            收藏
          </button>
        }
        onClick={onClick}
      />
    );

    fireEvent.click(view.getByRole("button", { name: "收藏" }));
    expect(onAction).toHaveBeenCalledTimes(1);
    expect(onClick).not.toHaveBeenCalled();

    // 行本体照旧可点：插槽之外的区域（标题、空白处）不受这条守卫影响。
    fireEvent.click(view.container.querySelector(".row-title")!);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("没传插槽时不多渲染那一层容器", () => {
    // `.row-slot` 只为事件而存在；没有插槽就不该在 DOM 里多出空元素。
    const view = render(<ListRow title="只读的一行" onClick={vi.fn()} />);
    expect(view.container.querySelectorAll(".row-slot")).toHaveLength(0);
  });
});
