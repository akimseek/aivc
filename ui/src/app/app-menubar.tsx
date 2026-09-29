import type { AiTarget } from "../types";
import { AppMenuBar } from "./app-menu-bar";
import { createAppMenus, type AppMenuActions } from "./app-menus";
import { AttentionCenter } from "./attention-center";
import type { AttentionEntry } from "./attention-center";

/**
 * 顶部菜单栏。
 *
 * # 为什么把它抽出来
 *
 * 这一块在 `app.tsx` 里有 **87 行**：品牌、四个下拉菜单、右侧的提醒铃铛。
 * 它与主视图**没有任何耦合**——菜单项要的动作由 `app-menus.ts` 定义（纯数据、可单测），
 * 提醒铃铛要的数据只在 `useTerminals` 里。留在 `app.tsx` 里只是把「外壳的接线」
 * 埋在了菜单的细节下面。
 *
 * # 三个条件必须由调用方算好再传进来
 *
 * `supportsSkills` / `isWslTarget` / `hasActiveSession` 决定哪些菜单项可用。
 * 它们**不由本组件从 target 猜**：猜的话「WSL 目标才有的『设置会话』」
 * 这类规则就会有两个真相（菜单一处、日历一处），迟早不一致。
 */
export function AppMenubar({
  selectedTarget,
  hasActiveSession,
  attentionEntries,
  onSelectAttention,
  onDismissAttention,
  actions
}: {
  selectedTarget: AiTarget | null;
  /** 有没有能导出的活动会话。决定「导出」整组是否禁用。 */
  hasActiveSession: boolean;
  attentionEntries: AttentionEntry[];
  onSelectAttention: (terminalId: string) => void;
  onDismissAttention: (terminalId: string) => void;
  /** 菜单项要触发的动作。缺失的项会被渲染成**禁用**，而不是死按钮。 */
  actions: AppMenuActions;
}) {
  const menus = createAppMenus(
    {
      supportsSkills: Boolean(selectedTarget),
      // 「设置会话」只对 WSL 目标可用。**按真实字段判定**
      // （`AiTarget.kind === "wsl"`），而不是写死。
      isWslTarget: selectedTarget?.kind === "wsl",
      hasActiveSession
    },
    actions
  );

  return (
    <div className="menubar">
      <span className="brand">AI 控制台</span>

      {/*
        分组菜单。此前这里只有「配置」与命令面板两个按钮——
        「文件 / 帮助」曾经是**没有 onClick 的死按钮**，因此被移除；那个判断是对的。
        现在补上真正的分组菜单（结构在 `app-menus.ts`，数据与渲染分开、可单测）。
        **动作缺失的项一律显示为禁用**（见 `disabledWhenMissing`）。
      */}
      <AppMenuBar menus={menus} />

      {/* 「配置」与「命令面板」两个按钮**删掉了**：它们与菜单完全重复。
          旧平台的顶栏只有 `AppMenuBar` + 提醒铃铛，命令面板靠 Ctrl+K 或菜单打开。
          重复的入口不只是多余——它们让「功能到底从哪进」变得含糊。 */}

      {/* 会话提醒：位置与旧平台一致（菜单栏右侧）。没有提醒时它自己不渲染。 */}
      <AttentionCenter
        entries={attentionEntries}
        onSelect={onSelectAttention}
        onDismiss={onDismissAttention}
      />
    </div>
  );
}
