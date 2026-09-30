import { describe, expect, it } from "vitest";

import type { AiSession } from "../../types";
import { matchesSessionSummary, resolveSidebarListMode } from "./sidebar-search";

/**
 * 列表区的四种内容与它们的优先级。
 *
 * # 为什么值得单测
 *
 * 判定顺序就是优先级，而「谁先谁后」是**有理由的**（见 `resolveSidebarListMode`
 * 的注释）。写成渲染里的四个 `if` 时，顺序错了界面也照常渲染，只是内容不对——
 * 那种错很难在图上看出来。
 */

const base = { view: "active" as const, query: "", globalSearching: false, globalQuery: "" };

const session = (patch: Partial<AiSession>): AiSession => ({
  id: "s1",
  title: "会话",
  filePath: "/sessions/s1.jsonl",
  messageCount: 1,
  preview: [],
  ...patch
});

describe("列表内容判定", () => {
  it("什么都没输入、不是收藏夹 → 普通列表", () => {
    expect(resolveSidebarListMode(base)).toEqual({ kind: "list" });
    expect(resolveSidebarListMode({ ...base, view: "trash" })).toEqual({ kind: "list" });
  });

  it("本地关键词非空 → 过滤结果（后端 `search-sessions` 那条路）", () => {
    expect(resolveSidebarListMode({ ...base, query: "  关键词  " })).toEqual({ kind: "filtered" });
  });

  it("收藏夹 → quickAccess 两组，与本地关键词无关", () => {
    expect(resolveSidebarListMode({ ...base, view: "favorites" })).toEqual({ kind: "favorites" });
    // 在收藏里打字是在收藏内部筛，**不是**跳到别处搜：来源仍然是 quickAccess。
    expect(resolveSidebarListMode({ ...base, view: "favorites", query: "x" })).toEqual({
      kind: "favorites"
    });
  });

  it("有全局关键词（或在搜）→ 全局结果，压过收藏夹与过滤", () => {
    expect(resolveSidebarListMode({ ...base, globalQuery: "跨目标" })).toEqual({ kind: "global" });
    // 结果还没回来时也要停在搜索结果区：否则列表区会先闪回当前目标再跳回去。
    expect(resolveSidebarListMode({ ...base, globalSearching: true })).toEqual({ kind: "global" });
    expect(
      resolveSidebarListMode({
        ...base,
        view: "favorites",
        query: "本地词",
        globalQuery: "全局词"
      })
    ).toEqual({ kind: "global" });
  });

  it("优先级：收藏夹 > 过滤结果（同一次输入不改变列表来源）", () => {
    expect(resolveSidebarListMode({ ...base, view: "favorites", query: "x" })).toEqual({
      kind: "favorites"
    });
    expect(resolveSidebarListMode({ ...base, view: "active", query: "x" })).toEqual({
      kind: "filtered"
    });
  });
});

describe("摘要命中（「搜索消息正文」关掉时的筛子）", () => {
  const item = session({
    id: "abc123",
    title: "重构会话列表",
    cwd: "/home/me/project",
    model: "gpt-5",
    preview: [{ role: "user", text: "正文里的一句话" }]
  });

  it("命中标题、目录、模型与预览正文的任一字段都算摘要命中", () => {
    expect(matchesSessionSummary(item, "重构")).toBe(true);
    expect(matchesSessionSummary(item, "project")).toBe(true);
    expect(matchesSessionSummary(item, "GPT-5")).toBe(true);
    expect(matchesSessionSummary(item, "正文里")).toBe(true);
    expect(matchesSessionSummary(item, "abc123")).toBe(true);
  });

  it("**全部词都要命中**（AND），与后端 `matches_session_terms` 一致", () => {
    expect(matchesSessionSummary(item, "重构 project")).toBe(true);
    expect(matchesSessionSummary(item, "重构 不存在的词")).toBe(false);
  });

  it("空关键词视为命中（后端也是：空查询等价于列出全部）", () => {
    expect(matchesSessionSummary(item, "   ")).toBe(true);
  });

  it("摘要里没有的词不算命中——它只可能命中了正文", () => {
    expect(matchesSessionSummary(item, "只存在于完整正文里的词")).toBe(false);
  });
});
