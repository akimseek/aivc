/**
 * 菜单栏的**结构定义**（纯数据）。
 *
 * # 为什么单独一个文件
 *
 * 旧平台就是这么分的（`app-menus.ts` 定义结构、`app-menu-bar.tsx` 负责渲染），
 * 而这样分的好处很实际：**结构可以直接单测**——菜单项、快捷键、禁用条件、
 * 分隔符、子菜单都能断言，不必渲染。
 *
 * # 条件启用也是要一致的部分
 *
 * 「设置会话」只对 **WSL 目标**可用、「导出」没有活动会话时禁用——
 * 这些不是美化，是旧平台写死在结构里的行为，照着抄。
 */

/** 一个菜单项。分隔符只有 `separator: true`，其余字段忽略。 */
export type AppMenuItem = {
  label: string;
  /** 显示在右侧的快捷键提示（**只显示**，按键本身在别处绑定）。 */
  shortcut?: string;
  disabled?: boolean;
  /** 危险动作（退出）用醒目样式。 */
  danger?: boolean;
  separator?: boolean;
  /** 子菜单。有它时点击不执行 `action`，而是展开。 */
  children?: AppMenuItem[];
  action?: () => void;
};

export type AppMenu = {
  id: string;
  label: string;
  items: AppMenuItem[];
};

/** 菜单项要触发的动作，由组装处注入。 */
export type AppMenuActions = {
  openCommandPalette: () => void;
  manageSkills: () => void;
  /** 设置会话（仅 WSL）。面还没实现时传 `undefined`，菜单项会显示为禁用。 */
  openSessionSettings?: () => void;
  openGatewaySettings: () => void;
  exportSession: (format: "markdown" | "json" | "html") => void;
  quit: () => void;
  manageVendors: () => void;
  openSystemTerminal: () => void;
  installCli: () => void;
  /** 打开日志目录。入口尚未接入 → 省略即为禁用。 */
  openLogDirectory?: () => void;
  /** 清理 Gateway 日志。入口尚未接入 → 省略即为禁用。 */
  clearGatewayLogs?: () => void;
  /** 关于。面还没实现时传 `undefined`。 */
  showAbout?: () => void;
};

/**
 * 取一项的动作；**没有动作就是禁用**。
 *
 * # 为什么要有这个约定
 *
 * 这个应用里已经有过一次教训：菜单栏里的「文件」「帮助」两个按钮**没有 `onClick`**，
 * 点了没反应——注释写着「**死按钮比没有按钮更糟**，它让用户以为自己点错了」。
 *
 * 所以这里把「有面但还不能用」与「点了没反应」分开：**缺动作的项一律显示为禁用**，
 * 用户看到的是「这里有个东西、现在还不能用」，而不是「我点错了」。
 */
function disabledWhenMissing(action: unknown): boolean {
  return !action;
}

export type AppMenuConditions = {
  /** 当前目标支不支持「管理 skill」（旧平台：`supportsSkills`）。 */
  supportsSkills: boolean;
  /** 当前目标是不是 WSL（旧平台：`isWslTarget`，同时决定「设置会话」）。 */
  isWslTarget: boolean;
  /** 有没有能导出的活动会话（旧平台：`hasActiveSession`）。 */
  hasActiveSession: boolean;
};

/**
 * 造出四个菜单。
 *
 * 顺序与旧平台一致：**文件 / 工具箱 / 终端 / 帮助**。
 */
export function createAppMenus(
  conditions: AppMenuConditions,
  actions: AppMenuActions
): AppMenu[] {
  const { supportsSkills, isWslTarget, hasActiveSession } = conditions;

  // 「导出」在没有活动会话时**连同子菜单三项一起禁用**（旧平台就是同步禁用的）。
  // 另外：动作本身缺失时也禁用——见 `disabledWhenMissing`。
  const exportDisabled = !hasActiveSession || disabledWhenMissing(actions.exportSession);

  return [
    {
      id: "file",
      label: "文件",
      items: [
        {
          label: "管理 skill",
          // 旧平台：`!supportsSkills || !hasTarget`。我这边的目标概念与之对应，
          // 用 `supportsSkills` 表达「当前目标能管 skill」。
          disabled: !supportsSkills || disabledWhenMissing(actions.manageSkills),
          action: actions.manageSkills
        },
        {
          label: "设置会话",
          // 旧平台：`!supportsSessionSettings || !isWslTarget`。
          // 我这个面（M5）还没实现，因此按「没有这个能力」处理——它会显示为禁用，
          // 而不是一个点了没反应的死按钮。
          disabled: !actions.openSessionSettings || !isWslTarget,
          action: actions.openSessionSettings
        },
        { separator: true, label: "" },
        {
          label: "导出",
          disabled: exportDisabled,
          children: [
            {
              label: "导出 Markdown",
              disabled: exportDisabled,
              action: () => actions.exportSession("markdown")
            },
            {
              label: "导出 JSON",
              disabled: exportDisabled,
              action: () => actions.exportSession("json")
            },
            {
              label: "导出 HTML",
              disabled: exportDisabled,
              action: () => actions.exportSession("html")
            }
          ]
        },
        { separator: true, label: "" },
        { label: "退出", danger: true, action: actions.quit }
      ]
    },
    {
      id: "toolbox",
      label: "工具箱",
      items: [
        { label: "供应商管理", action: actions.manageVendors },
        { label: "网关", action: actions.openGatewaySettings }
      ]
    },
    {
      id: "terminal",
      label: "终端",
      items: [{ label: "新建终端", action: actions.openSystemTerminal }]
    },
    {
      id: "help",
      label: "帮助",
      items: [
        {
          label: "命令面板",
          shortcut: "Ctrl + K",
          action: actions.openCommandPalette
        },
        { separator: true, label: "" },
        { label: "安装 CLI", action: actions.installCli },
        { separator: true, label: "" },
        {
          label: "打开日志目录",
          disabled: disabledWhenMissing(actions.openLogDirectory),
          action: actions.openLogDirectory
        },
        {
          label: "清理 Gateway 日志",
          disabled: disabledWhenMissing(actions.clearGatewayLogs),
          action: actions.clearGatewayLogs
        },
        {
          label: "关于",
          // 同「设置会话」：面（M6）没实现就显示为禁用。
          disabled: !actions.showAbout,
          action: actions.showAbout
        }
      ]
    }
  ];
}
