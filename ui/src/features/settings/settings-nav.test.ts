import { describe, expect, it } from "vitest";

import { SETTINGS_GROUPS, SETTINGS_TAB_IDS, settingsTabLabel } from "./settings-nav";

/**
 * 导航模型。两条：**每个面板恰好出现一次**（漏一个等于那个面板没有入口，
 * 重复一个会出现两个激活态）与**分组顺序固定**（spec §10 是验收依据）。
 */
describe("设置中心导航", () => {
  it("十个面板恰好各出现一次", () => {
    const ids = SETTINGS_GROUPS.flatMap((group) => group.tabs.map((tab) => tab.id));
    expect(ids).toHaveLength(10);
    expect(new Set(ids).size).toBe(10);
    expect([...ids].sort()).toEqual([...SETTINGS_TAB_IDS].sort());
  });

  it("分组顺序与服务/观测/扩展/关于一致", () => {
    expect(SETTINGS_GROUPS.map((group) => group.label)).toEqual([
      "服务与路由",
      "观测与审计",
      "环境与扩展",
      "系统与关于"
    ]);
  });

  it("每一页都有可读标题", () => {
    for (const id of SETTINGS_TAB_IDS) {
      expect(settingsTabLabel(id).length).toBeGreaterThan(0);
    }
    expect(settingsTabLabel("about")).toBe("关于控制台");
  });
});
