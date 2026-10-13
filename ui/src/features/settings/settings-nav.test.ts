import { describe, expect, it } from "vitest";

import { SETTINGS_GROUPS, SETTINGS_TAB_IDS, settingsTabLabel } from "./settings-nav";
import type { SettingsTab } from "./settings-nav";

/**
 * 导航模型。两条：**每个面板恰好出现一次**（漏一个等于那个面板没有入口，
 * 重复一个会出现两个激活态）与**分组顺序固定**（spec §10 是验收依据）。
 *
 * 名单**逐字写在这里**，不与 `SETTINGS_GROUPS` 互比——两者同源的话断言恒真：
 * 把某个面板从分组里删掉、或在两个分组里各注册一次，互比都照旧通过。
 * 字面量是这份名单的**独立来源**，它红了才是真的漏注册 / 多注册 / 换序。
 */
const TAB_IDS: SettingsTab[] = [
  "vendors",
  "gateway",
  "usage",
  "failures",
  "logs",
  "skills",
  "environment",
  "presets",
  "about"
];

/** 每页的标题。 */
const TAB_LABELS = [
  "供应商管理",
  "网关",
  "用量报表",
  "异常诊断",
  "请求日志",
  "平台技能 (Skills)",
  "运行时环境",
  "工作区预设",
  "关于控制台"
];

describe("设置中心导航", () => {
  it("九个面板恰好各出现一次，顺序与设置中心名单一致", () => {
    expect(SETTINGS_TAB_IDS).toEqual(TAB_IDS);
  });

  it("分组顺序与服务/观测/扩展/关于一致", () => {
    expect(SETTINGS_GROUPS.map((group) => group.label)).toEqual([
      "服务与路由",
      "观测与审计",
      "环境与扩展",
      "系统与关于"
    ]);
  });

  it("每一页的标题都解析得出来，而不是兜底返回的编号", () => {
    expect(TAB_IDS.map((id) => settingsTabLabel(id))).toEqual(TAB_LABELS);
    // 兜底分支 `return tab` 会让「每一页都有标题」恒真（没有注册的编号也有「标题」，
    // 就是编号自己）。这一条把兜底显式排掉，并给出点名的失败信息。
    for (const id of TAB_IDS) {
      expect(settingsTabLabel(id), `${id} 落到了兜底分支`).not.toBe(id);
    }
  });
});
