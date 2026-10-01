// @vitest-environment jsdom
import { fireEvent, render, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { Icon } from "./icon";
import { IconButton } from "./icon-button";

/**
 * IconButton（P1-E 抽出的共用件）。
 *
 * 全部查询都限定在本次 render 的容器内：本套件没有全局 cleanup
 * （vitest 没开 `globals`，RTL 的自动清理不生效），同名按钮会在
 * document 上跨用例累积，`view.getByRole` 会直接撞出「multiple elements」。
 */
describe("IconButton", () => {
  it("label 是无障碍名字，title 缺省与它相同", () => {
    const view = render(
      <IconButton label="刷新列表" onClick={() => {}}>
        <Icon name="refresh" size={14} />
      </IconButton>
    );
    const button = within(view.container).getByRole("button", { name: "刷新列表" });
    expect(button.getAttribute("title")).toBe("刷新列表");
    expect(button.className).toBe("icon-btn");
    expect(button.getAttribute("type")).toBe("button");
  });

  it("title 显式给出时只补充悬停说明，不改无障碍名字", () => {
    const view = render(
      <IconButton label="回收站" title="查看回收站" onClick={() => {}}>
        <Icon name="trash" size={15} />
      </IconButton>
    );
    const button = within(view.container).getByRole("button", { name: "回收站" });
    expect(button.getAttribute("title")).toBe("查看回收站");
  });

  it("active 补 on 类；aria-pressed 由调用方透传", () => {
    const view = render(
      <IconButton label="回收站" active aria-pressed onClick={() => {}}>
        <Icon name="trash" size={15} />
      </IconButton>
    );
    const button = within(view.container).getByRole("button", { name: "回收站" });
    expect(button.className).toBe("icon-btn on");
    expect(button.getAttribute("aria-pressed")).toBe("true");
  });

  it("禁用时点击不回调", () => {
    const onClick = vi.fn();
    const view = render(
      <IconButton label="回收站" disabled onClick={onClick}>
        <Icon name="trash" size={15} />
      </IconButton>
    );
    fireEvent.click(within(view.container).getByRole("button", { name: "回收站" }));
    expect(onClick).not.toHaveBeenCalled();
  });
});
