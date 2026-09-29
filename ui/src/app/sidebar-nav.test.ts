import { describe, expect, it } from "vitest";

import type { AiProviderSummary } from "../types";
import {
  paneForViewNav,
  showsSessionSearchRow,
  showsViewNav,
  supportsBatchActions,
  supportsTrash,
  viewNavKey
} from "./sidebar-nav";

/**
 * 侧栏两级导航的映射。
 *
 * # 为什么值得单测
 *
 * 一级与二级是**两个控件**，表达的是同一件事的两个层次。两处各写一遍映射，
 * 就会出现「二级说在收藏夹、一级还高亮着会话」这种自相矛盾的状态——
 * 而这种错在界面上很难一眼看出（两个控件都在，只是高亮不对）。
 */

const PROVIDERS: AiProviderSummary[] = [
  { id: "codex", label: "Codex", capabilities: { trash: true, batchActions: true } },
  { id: "qoder", label: "Qoder CN", capabilities: { trash: false, batchActions: false } }
];

describe("二级导航高亮", () => {
  it("会话 + 活跃 → 当前会话", () => {
    expect(viewNavKey("sessions", "active")).toBe("active");
  });

  it("会话 + 回收站 → 回收站", () => {
    expect(viewNavKey("sessions", "trash")).toBe("trash");
  });

  it("收藏夹 → 收藏夹，**不受 view 影响**", () => {
    // 收藏夹与回收站是两个不同的 pane；view 那个值此时不该影响高亮。
    expect(viewNavKey("favorites", "active")).toBe("favorites");
    expect(viewNavKey("favorites", "trash")).toBe("favorites");
  });
});

describe("点二级导航后的状态", () => {
  it("当前会话 → sessions/active", () => {
    expect(paneForViewNav("active")).toEqual({ pane: "sessions", view: "active" });
  });

  it("回收站 → sessions/trash", () => {
    expect(paneForViewNav("trash")).toEqual({ pane: "sessions", view: "trash" });
  });

  it("收藏夹 → favorites", () => {
    expect(paneForViewNav("favorites")).toEqual({ pane: "favorites", view: "active" });
  });

  it("**往返一致**：算出来的键再映射回去，等于原来的状态", () => {
    const states: [Parameters<typeof viewNavKey>[0], Parameters<typeof viewNavKey>[1]][] = [
      ["sessions", "active"],
      ["sessions", "trash"],
      ["favorites", "active"]
    ];
    for (const [pane, view] of states) {
      const key = viewNavKey(pane, view);
      expect(paneForViewNav(key)).toEqual({ pane, view });
    }
  });
});

describe("哪一行该出现", () => {
  it("二级导航在会话与收藏夹下出现，工作台与全局搜索下不出现", () => {
    // 工作台是一处独立的地方（不列会话，没有「哪种会话」可筛）。
    expect(showsViewNav("sessions")).toBe(true);
    expect(showsViewNav("favorites")).toBe(true);
    expect(showsViewNav("workbench")).toBe(false);
    expect(showsViewNav("search")).toBe(false);
  });

  it("搜索行只在会话列表下出现——收藏夹里放搜索框会以为搜的是收藏夹内部", () => {
    expect(showsSessionSearchRow("sessions")).toBe(true);
    expect(showsSessionSearchRow("favorites")).toBe(false);
    expect(showsSessionSearchRow("workbench")).toBe(false);
  });
});

describe("能力矩阵判定", () => {
  it("回收站按平台能力判定", () => {
    expect(supportsTrash(PROVIDERS, "codex")).toBe(true);
    expect(supportsTrash(PROVIDERS, "qoder")).toBe(false);
  });

  it("批量操作按平台能力判定", () => {
    expect(supportsBatchActions(PROVIDERS, "codex")).toBe(true);
    expect(supportsBatchActions(PROVIDERS, "qoder")).toBe(false);
  });

  it("**缺字段时按不支持处理**——不能因为少一个键就放行一个会报错的功能", () => {
    const sparse: AiProviderSummary[] = [{ id: "codex", label: "Codex", capabilities: {} }];
    expect(supportsTrash(sparse, "codex")).toBe(false);
    expect(supportsBatchActions(sparse, "codex")).toBe(false);
  });

  it("平台不在列表里时也不支持", () => {
    expect(supportsTrash(PROVIDERS, "gemini")).toBe(false);
  });
});
