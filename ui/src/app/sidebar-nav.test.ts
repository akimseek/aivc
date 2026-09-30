import { describe, expect, it } from "vitest";

import type { AiProviderSummary } from "../types";
import { resolveSidebarListMode } from "../features/sessions/sidebar-search";
import { supportsBatchActions, supportsTrash } from "./sidebar-nav";
import { shouldLoadSessionList } from "./use-console";

/**
 * 侧栏的「地方 → 内容」映射与平台能力门控。
 *
 * # 为什么值得单测
 *
 * 一级导航上收到活动栏之后，侧栏里描述「显示什么」的只剩 `view` 与关键词两条输入。
 * 它们的组合直接决定列表区渲染哪一批数据，而**收藏夹那一支最特别**：
 * 它看起来像另一个筛选，实际是另一批数据（`quickAccess`）——判错了不会报错，
 * 只会在界面上显示一批不该出现的会话。
 */

const PROVIDERS: AiProviderSummary[] = [
  { id: "codex", label: "Codex", capabilities: { trash: true, batchActions: true } },
  { id: "qoder", label: "Qoder CN", capabilities: { trash: false, batchActions: false } }
];

describe("view 到列表内容的映射", () => {
  const base = { query: "", globalSearching: false, globalQuery: "" };

  it("全部（active）→ 当前目标的会话列表", () => {
    expect(resolveSidebarListMode({ ...base, view: "active" })).toEqual({ kind: "list" });
  });

  it("回收站（trash）→ 会话列表（回收站那一份）", () => {
    expect(resolveSidebarListMode({ ...base, view: "trash" })).toEqual({ kind: "list" });
  });

  it("收藏夹（favorites）→ quickAccess 两组，**不发会话列表请求**", () => {
    expect(resolveSidebarListMode({ ...base, view: "favorites" })).toEqual({ kind: "favorites" });
    // 数据来自 `quickAccess`（跨目标的收藏 + 最近打开），列表请求既多余、
    // 结果也不是用户要的那一批。判据与 effect 共用同一个纯函数。
    expect(shouldLoadSessionList("favorites")).toBe(false);
  });

  it("关键词非空 → 过滤结果；全局关键词非空 → 跨目标结果", () => {
    expect(resolveSidebarListMode({ ...base, view: "active", query: "x" })).toEqual({
      kind: "filtered"
    });
    expect(resolveSidebarListMode({ ...base, view: "active", globalQuery: "x" })).toEqual({
      kind: "global"
    });
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
