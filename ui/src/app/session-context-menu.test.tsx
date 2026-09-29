// @vitest-environment jsdom
import { act, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { SessionContextMenu } from "./session-context-menu";
import type { AiSession } from "../types";

/**
 * 会话右键菜单。
 *
 * # 这组测试是为什么来的
 *
 * 用户报的清单 4.1：**回收站里右键一条会话，没有「恢复」**。
 * 恢复此前只能先勾选那一行、再到批量条上点「恢复」——右键一条会话却恢复不了它，
 * 而右键正是「就着这一条做事」的位置。旧平台的回收站菜单里就有「恢复」。
 *
 * 因此这里钉住的第一条是：**回收站里必须出现「恢复」，非回收站里必须不出现**。
 */

function session(overrides: Partial<AiSession> = {}): AiSession {
  return {
    id: "s1",
    title: "一条会话",
    filePath: "C:/x/s1.jsonl",
    providerId: "codex",
    ...overrides
  } as AiSession;
}

function setup(trash: boolean) {
  const handlers = {
    onClose: vi.fn(),
    onOpenFolder: vi.fn(),
    onRename: vi.fn(),
    onDuplicate: vi.fn(),
    onToggleFavorite: vi.fn(),
    onCopyId: vi.fn(),
    onCopyPath: vi.fn(),
    onRemove: vi.fn(),
    onRestore: vi.fn()
  };
  const view = render(
    <SessionContextMenu x={10} y={10} session={session()} trash={trash} {...handlers} />
  );
  const labels = [...view.container.querySelectorAll("button")].map((item) =>
    (item.textContent ?? "").trim()
  );
  const click = (label: string) => {
    const button = [...view.container.querySelectorAll("button")].find(
      (item) => (item.textContent ?? "").trim() === label
    );
    if (!button) throw new Error(`没有这一项：${label}`);
    act(() => {
      button.click();
    });
  };
  return { view, handlers, labels, click };
}

describe("SessionContextMenu：回收站的恢复", () => {
  it("回收站里有「恢复」", () => {
    const { labels } = setup(true);
    expect(labels).toContain("恢复");
  });

  it("不在回收站时**没有**「恢复」——那里没有东西可恢复", () => {
    const { labels } = setup(false);
    expect(labels).not.toContain("恢复");
  });

  it("「恢复」排在「彻底删除」**之前**", () => {
    // 顺序不是格式问题：恢复是这一步更常用的选择，而销毁不可撤销。
    // 反过来放会让人顺手点到销毁。
    const { labels } = setup(true);
    expect(labels.indexOf("恢复")).toBeGreaterThanOrEqual(0);
    expect(labels.indexOf("恢复")).toBeLessThan(labels.indexOf("彻底删除"));
  });

  it("点「恢复」调用的是恢复，不是删除", () => {
    const { handlers, click } = setup(true);
    click("恢复");
    expect(handlers.onRestore).toHaveBeenCalledTimes(1);
    expect(handlers.onRemove).not.toHaveBeenCalled();
  });

  it("两点菜单项都做完就关掉菜单", () => {
    const { handlers, click } = setup(true);
    click("恢复");
    expect(handlers.onClose).toHaveBeenCalledTimes(1);
  });

  it("回收站里的删除项写「彻底删除」，别处写「移入回收站」", () => {
    expect(setup(true).labels).toContain("彻底删除");
    expect(setup(false).labels).toContain("移入回收站");
  });
});
