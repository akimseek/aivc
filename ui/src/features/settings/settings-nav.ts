import type { IconName } from "../../components";

/** 设置中心的面板编号。与 spec §10 的表一一对应。 */
export type SettingsTab =
  | "vendors"
  | "gateway"
  | "usage"
  | "failures"
  | "logs"
  | "skills"
  | "environment"
  | "presets"
  | "diagnostics"
  | "about";

export type SettingsGroup = {
  id: string;
  label: string;
  icon: IconName;
  tabs: Array<{ id: SettingsTab; label: string }>;
};

/**
 * 分组不是装饰：十个扁平标签在 224px 的导航列里读不出结构，
 * 而「服务与路由」（改行为）与「观测与审计」（只看不改）是两类动作。
 */
export const SETTINGS_GROUPS: SettingsGroup[] = [
  {
    id: "service",
    label: "服务与路由",
    icon: "globe",
    tabs: [
      { id: "vendors", label: "供应商管理" },
      { id: "gateway", label: "本地网关 & 路由" }
    ]
  },
  {
    id: "observe",
    label: "观测与审计",
    icon: "chart",
    tabs: [
      { id: "usage", label: "用量报表" },
      { id: "failures", label: "异常诊断" },
      { id: "logs", label: "请求日志" }
    ]
  },
  {
    id: "extend",
    label: "环境与扩展",
    icon: "wrench",
    tabs: [
      { id: "skills", label: "平台技能 (Skills)" },
      { id: "environment", label: "运行时环境" },
      { id: "presets", label: "工作区预设" },
      { id: "diagnostics", label: "系统诊断" }
    ]
  },
  {
    id: "about",
    label: "系统与关于",
    icon: "info",
    tabs: [{ id: "about", label: "关于控制台" }]
  }
];

export const SETTINGS_TAB_IDS: SettingsTab[] = SETTINGS_GROUPS.flatMap((group) =>
  group.tabs.map((tab) => tab.id)
);

export function settingsTabLabel(tab: SettingsTab): string {
  for (const group of SETTINGS_GROUPS) {
    const found = group.tabs.find((item) => item.id === tab);
    if (found) return found.label;
  }
  return tab;
}
