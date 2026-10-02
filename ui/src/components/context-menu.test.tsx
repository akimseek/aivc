// @vitest-environment jsdom
import { cleanup, fireEvent, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ContextMenu, ContextMenuItem } from "./context-menu";

/**
 * 右键菜单外壳（会话 / 标签 / 终端文本三个菜单共用）。
 *
 * 契约四件套：**点别处关闭、点菜单自身不关**；**Escape 关闭，但上面压着
 * 模态/浮层时让位**；**先做事、再关菜单**；危险项能认出来、禁用项不可点。
 * 这些此前由三个调用方各自保证——外壳抽出来之后，钉在这里等于三处都钉住。
 */
afterEach(cleanup);

function setup() {
  const onClose = vi.fn();
  const view = render(
    <ContextMenu x={10} y={20} ariaLabel="操作菜单" onClose={onClose}>
      <ContextMenuItem onClick={() => {}}>打开</ContextMenuItem>
      <ContextMenuItem danger onClick={() => {}}>
        彻底删除
      </ContextMenuItem>
      <ContextMenuItem disabled onClick={() => {}}>
        不可用
      </ContextMenuItem>
    </ContextMenu>
  );
  const backdrop = view.container.querySelector(".menu-backdrop") as HTMLElement;
  const menu = view.container.querySelector(".context-menu") as HTMLElement;
  const items = [...view.container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')];
  return { onClose, backdrop, menu, items };
}

describe("ContextMenu", () => {
  it("是 role=menu 且带无障碍名字；条目是 menuitem", () => {
    const { menu, items } = setup();
    expect(menu.getAttribute("role")).toBe("menu");
    expect(menu.getAttribute("aria-label")).toBe("操作菜单");
    expect(items).toHaveLength(3);
  });

  it("点别处关闭；点菜单自身不关", () => {
    const first = setup();
    fireEvent.click(first.backdrop);
    expect(first.onClose).toHaveBeenCalledTimes(1);
    cleanup();

    const second = setup();
    fireEvent.click(second.menu); // 菜单本体的空白处
    expect(second.onClose).not.toHaveBeenCalled();
  });

  it("Escape 关闭；上面压着模态/浮层时**让位**", () => {
    const { onClose } = setup();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
    onClose.mockClear();

    // 压一层 `.dialog-overlay`（退出确认框随时可能从后端推来）——这一次不能让，
    // 否则一次 Escape 会同时收掉菜单与上面那层。
    const layer = document.createElement("div");
    layer.className = "dialog-overlay";
    document.body.appendChild(layer);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();
    layer.remove();
  });

  it("**先做事、再关菜单**——顺序反了会读到已作废的菜单状态", () => {
    const order: string[] = [];
    render(
      <ContextMenu x={0} y={0} ariaLabel="操作菜单" onClose={() => order.push("close")}>
        <ContextMenuItem onClick={() => order.push("action")}>打开</ContextMenuItem>
      </ContextMenu>
    );
    fireEvent.click(document.querySelector('[role="menuitem"]') as HTMLElement);
    expect(order).toEqual(["action", "close"]);
  });

  it("危险项带 danger 类；禁用项不可点", () => {
    const { items, onClose } = setup();
    expect(items[1]!.className).toBe("danger");
    expect(items[2]!.disabled).toBe(true);
    expect(items[0]!.disabled).toBe(false);

    fireEvent.click(items[0]!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
