import { describe, expect, it } from "vitest";

import { resolveSidebarListMode } from "./sidebar-search";

/**
 * 列表区的四种内容与它们的优先级。
 *
 * # 为什么值得单测
 *
 * 判定顺序就是优先级，而「谁先谁后」是**有理由的**（见 `resolveSidebarListMode`
 * 的注释）。写成渲染里的四个 `if` 时，顺序错了界面也照常渲染，只是内容不对——
 * 那种错很难在图上看出来。
 *
 * 这里此前还有一组「摘要命中」的断言，服务于一个已删除的「搜索消息正文」开关：
 * 关掉时它把跨目标搜索结果按本地重算的摘要命中再筛一遍，而那个筛子**恒真**
 * （后端第二阶段命中正文时返回完整解析结果，命中词必然在 `preview` 里）。
 * 开关与筛子已按裁定 R23 一起删除，断言随实现一起走。
 */

const base = { view: "active" as const, query: "", globalSearching: false, globalQuery: "" };

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
