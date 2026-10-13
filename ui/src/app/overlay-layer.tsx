import { Button, HintBar } from "../components";
import { MessagesModalContainer } from "../features/workspace/messages-modal";
import { SettingsCenter, type SettingsTab } from "../features/settings/settings-center";
import * as api from "../ipc/commands";
import type { AiProviderId, AiProviderSummary, AiSession, AiTarget, ExportFormat } from "../types";
import { ExitConfirm } from "./exit-confirm";
import { NewSessionDialog } from "../features/sessions/new-session-dialog";
import { SessionContextMenu } from "../features/sessions/session-context-menu";
import { SessionDuplicateDialog, SessionRenameDialog } from "../features/sessions/session-title-dialogs";
import { SessionSettingsDialog } from "../features/sessions/session-settings-dialog";
import { TabContextMenu } from "../features/workspace/tab-context-menu";
import { supportsBranch } from "./provider-capabilities";

/** 新会话对话框的草稿。 */
export type NewSessionDraft = {
  cwd: string;
  title: string;
  prompt: string;
  cliArgs: string;
  /** 非空表示这是**恢复**历史会话（`sessionId` 会被变成 `--resume <id>`）。 */
  resume?: { sessionId: string; missingCwd?: string };
};

/** 全部弹层的**状态**。收成一个对象，调用方一眼能看出这些是弹层的开关。 */
export type OverlaysState = {
  /** 菜单动作的一句话结果。空串表示不显示。 */
  toastMessage: string;
  /** 最近一次导出的落点。空串表示不显示。 */
  exportedTo: string;
  sessionSettings: { distro: string; path: string } | null;
  newSession: NewSessionDraft | null;
  menu: { x: number; y: number; sessionId: string } | null;
  tabMenu: { key: string; x: number; y: number } | null;
  titleDialog: { kind: "rename" | "duplicate"; session: AiSession } | null;
  detailSession: AiSession | null;
  detailTarget?: AiTarget | null;
  settingsOpen: boolean;
  settingsTab: SettingsTab;
};

/**
 * 全部弹层的**动作**。
 *
 * # 每一项都是必填的（`openSessionSettings` 是唯一例外，见该项说明）
 *
 * 写这个文件时我一度给 `onCopyId` / `onRemove` 填过 `() => undefined` 的占位——
 * 那样「复制编号」与「删除」会**静默失效**：按钮点得下去、没有任何反应、也不报错。
 * 因此这里的每一项都必填，漏接会被类型检查挡住。
 */
export type OverlaysActions = {
  closeToast: () => void;
  closeExported: () => void;
  openExported: () => void;
  changeSessionSettingsPath: (path: string) => void;
  closeSessionSettings: () => void;
  restoreSessionSettings: (distro: string) => void;
  saveSessionSettings: (distro: string, path: string) => void;
  /**
   * 打开「设置会话」对话框（WSL 的 CODEX_HOME 覆盖）。
   *
   * **唯一允许缺省的动作**：平台不支持会话设置（claude/qoder）时上层不构造它
   * （`undefined`），消费方（环境卡）按「缺入口不渲染」处理。这不是上面说的
   * 「占位静默失效」——静默失效是「平台支持但没接」，这里是能力不支持的显式表达。
   */
  openSessionSettings?: () => void;
  /** 退出控制台：走窗口关闭，后端的退出确认流程仍然拦得住。 */
  quit: () => void;
  /** 在文件管理器里打开日志目录（关于面板的入口）。 */
  openLogDirectory: () => void;
  changeNewSession: (patch: Partial<NewSessionDraft>) => void;
  chooseNewSessionDirectory: () => void;
  closeNewSession: () => void;
  confirmNewSession: () => void;
  closeMenu: () => void;
  openSessionFolder: (session: AiSession) => void;
  /**
   * 导出**某一条**会话。
   *
   * 带 `session` 而不是只带格式：右键菜单里导出的是**被右键的那一条**，
   * 而它与「当前选中」未必是同一条（详见 `exportCurrent` 的说明）。
   */
  exportSession: (session: AiSession, format: ExportFormat) => void;
  toggleFavorite: (session: AiSession) => void;
  copyText: (text: string) => void;
  removeSession: (session: AiSession) => void;
  /** 从回收站恢复一条。 */
  restoreSession: (session: AiSession) => void;
  startRename: (session: AiSession) => void;
  startDuplicate: (session: AiSession) => void;
  closeTitleDialog: () => void;
  confirmRename: (session: AiSession, title: string) => void;
  confirmDuplicate: (session: AiSession, title: string | undefined) => void;
  closeTabMenu: () => void;
  closeTabThis: (key: string) => void;
  closeTabOthers: (key: string) => void;
  closeTabAll: () => void;
  closeDetail: () => void;
  resumeSession: (session: AiSession) => void;
  /**
   * 从某条消息处创建分支。
   *
   * 带 `sessionId`：消息弹框开着的未必是**当前选中**的那一条（侧栏卡片上的
   * 「消息与分支」能为非选中会话开弹框），只读选中值会把分支分到别的会话上。
   */
  branchFromMessage: (anchor: number, sessionId: string, targetId?: string) => Promise<boolean>;
  /**
   * 导出**消息弹框里那一条**会话。
   *
   * 与 `exportSession` 分开：右键菜单那条会**先改选中态**（它要就着被右键的会话），
   * 而消息弹框是「停下来看」——导出不该把背后的视图换到别的会话上去。
   * 两处都必须显式带上会话，只读「当前选中」会导出别的会话。
   */
  exportDetail: (session: AiSession, format: ExportFormat) => void;
  closeSettings: () => void;
  reportError: (message: string) => void;
  /**
   * 一句话**成功**反馈。
   *
   * 与 `reportError` 分开而不是合并：失败要**留着**（用户得看清哪里错了），
   * 成功几秒后自动消失。两条路的呈现方式不同，因此不能共用一个回调。
   */
  notify: (message: string) => void;
};

/**
 * 全部弹层。
 *
 * # 它只负责「渲染」，状态与动作全在外面
 *
 * 这些弹层与主视图**互相牵连**：右键菜单点「重命名」要先选中那条会话；
 * 详情弹层里点分支要关掉自己再去开终端；新会话确认后要开一路终端。
 * 把状态一起搬进来，就得把这些关系反向注入回去——那只是把复杂度挪个位置。
 *
 * 所以它是**纯展示**组件：自己不持有状态，也不认识 `useConsole`。
 * 好处是 `app.tsx` 少了 190 行 JSX 的噪音，而行为一字未改。
 *
 * # 标签菜单的两项数据由调用方算好
 *
 * `tabLabel` 与 `closableTabCount` 来自 `useWorkspaceTabs`——那里才知道
 * 哪些标签可关、标题是什么。在这里猜（例如拿 key 去查会话）是错的：
 * 统一模型里标签的 key 可能是终端编号（全新会话），查不到任何会话。
 */
export function AppOverlays({
  state,
  actions,
  providers,
  providerId,
  appVersion,
  targetId,
  targets,
  selectedTarget,
  trash,
  canCustomCwd,
  canSessionSettings,
  canDuplicate,
  canBranch,
  resolveSession,
  tabLabel,
  closableTabCount
}: {
  state: OverlaysState;
  actions: OverlaysActions;
  /** 全部平台：供应商页要用它显示厂商名。 */
  providers: AiProviderSummary[];
  providerId: AiProviderId | "";
  /**
   * 应用版本。
   *
   * 由 `app.tsx` 取一次后传下来（设置中心的关于面板要用它）：版本是进程级的常量，
   * 在这里自己取只是多几次 IPC，还会让「读不到」的兜底分散在几处。
   */
  appVersion: string;
  targetId: string;
  targets: AiTarget[];
  selectedTarget: AiTarget | null;
  /** 在回收站视图里时，删除项变成「彻底删除」。 */
  trash: boolean;
  /**
   * 当前平台的四项能力：自定义工作目录 / 会话设置 / 复制 / 分支。
   *
   * 由调用方按 `provider-capabilities.ts` 算好传进来，这里只做转发：
   * 能力键名与「缺键 = 不支持」的约定只在那一个模块里解释一次。
   *
   * `canCustomCwd` 与 `canSessionSettings` 曾以 JSX 布尔简写恒真传入，
   * 于是 claude/qoder 的 WSL 目标也能打开「设置会话」，而保存写的是 Codex 的
   * 全局设置——恒真等于「入口在、点得动、必然失败」，正是本组件的类型层面要堵的事。
   */
  canCustomCwd: boolean;
  canSessionSettings: boolean;
  canDuplicate: boolean;
  canBranch: boolean;
  /** 按编号找会话——右键菜单要它的文件路径。 */
  resolveSession: (sessionId: string) => AiSession | undefined;
  /** 标签菜单显示的那张标签的标题。 */
  tabLabel: string;
  /** 一共关得掉几张标签（统一模型里每张都可关，等于标签总数）。 */
  closableTabCount: number;
}) {
  const menuSession = state.menu ? resolveSession(state.menu.sessionId) : undefined;

  return (
    <>

      {/*
        两条一句话提示，形状相同：都要能被看见、也都能收掉。
        都走共享的 `HintBar`（浮层形态）——分成两处写迟早会长得不一样。
      */}
      {state.toastMessage && (
        <HintBar floating message={state.toastMessage} onDismiss={actions.closeToast} />
      )}
      {state.exportedTo && (
        <HintBar
          floating
          message={
            <>
              已导出到 <code>{state.exportedTo}</code>
            </>
          }
          onDismiss={actions.closeExported}
          extra={
            <Button small onClick={actions.openExported}>
              打开
            </Button>
          }
        />
      )}

      {/*
        新会话对话框。旧平台的流程是**先定工作目录 / 标题 / 启动提示词 / CLI 参数，再打开**。
        此前这里是「点一下直接进终端」——用户报的 H2 说的就是这件事。
      */}
      {state.newSession && (
        <NewSessionDialog
          pendingResume={
            state.newSession.resume?.missingCwd
              ? { missingCwd: state.newSession.resume.missingCwd }
              : null
          }
          supportsCustomCwd={canCustomCwd}
          cwd={state.newSession.cwd}
          title={state.newSession.title}
          prompt={state.newSession.prompt}
          cliArgs={state.newSession.cliArgs}
          onChooseDirectory={actions.chooseNewSessionDirectory}
          onTitleChange={(value) => actions.changeNewSession({ title: value })}
          onPromptChange={(value) => actions.changeNewSession({ prompt: value })}
          onCliArgsChange={(value) => actions.changeNewSession({ cliArgs: value })}
          onClose={actions.closeNewSession}
          onConfirm={actions.confirmNewSession}
        />
      )}

      {state.menu && menuSession && (
        <SessionContextMenu
          x={state.menu.x}
          y={state.menu.y}
          session={menuSession}
          trash={trash}
          canDuplicate={canDuplicate}
          onClose={actions.closeMenu}
          onOpenFolder={() => actions.openSessionFolder(menuSession)}
          onToggleFavorite={() => actions.toggleFavorite(menuSession)}
          // 复制**编号**而不是标题：编号是稳定的标识，粘到别处能用来定位。
          onCopyId={() => actions.copyText(menuSession.id)}
          onCopyPath={() => actions.copyText(menuSession.filePath)}
          // 导出被右键的那一条：菜单栏里的「导出」只能导出选中项，这里能就着右键的对象来。
          onExport={(format) => actions.exportSession(menuSession, format)}
          onRemove={() => actions.removeSession(menuSession)}
          onRestore={() => actions.restoreSession(menuSession)}
          onRename={() => actions.startRename(menuSession)}
          onDuplicate={() => actions.startDuplicate(menuSession)}
        />
      )}

      {/* 右键菜单来的重命名 / 复制。复制**要问名字**：不问的话列表里立刻出现
          两条一模一样的标题，而用户复制它通常正是为了改出个不同的名字。 */}
      {state.titleDialog?.kind === "rename" && (
        <SessionRenameDialog
          session={state.titleDialog.session}
          onClose={actions.closeTitleDialog}
          onConfirm={(title) => actions.confirmRename(state.titleDialog!.session, title)}
        />
      )}
      {state.titleDialog?.kind === "duplicate" && (
        <SessionDuplicateDialog
          onClose={actions.closeTitleDialog}
          onConfirm={(title) => actions.confirmDuplicate(state.titleDialog!.session, title)}
        />
      )}

      {/* 标签右键菜单：开了七八路终端之后，一个一个点 ✕ 很难受。 */}
      {state.tabMenu && (
        <TabContextMenu
          x={state.tabMenu.x}
          y={state.tabMenu.y}
          tabKey={state.tabMenu.key}
          tabLabel={tabLabel}
          closableCount={closableTabCount}
          onCloseThis={actions.closeTabThis}
          onCloseOthers={actions.closeTabOthers}
          onCloseAll={actions.closeTabAll}
          onCloseMenu={actions.closeTabMenu}
        />
      )}

      {/* 消息与分支：把一条会话**停下来看**。分页、取数与竞态保护都在
          `useSessionDetail` 里（`MessagesModalContainer`），这里只持有
          「要看哪一条」——关掉就回到原来的视图。 */}
      <MessagesModalContainer
        session={state.detailSession}
        target={state.detailTarget ?? targets.find((item) => item.id === targetId) ?? null}
        canBranch={state.detailTarget ? supportsBranch(providers, state.detailTarget.provider) : canBranch}
        resolveParent={resolveSession}
        listChildren={(parentSessionId) => api.listSessionChildren(parentSessionId, state.detailTarget?.id ?? targetId)}
        onClose={actions.closeDetail}
        onBranch={actions.branchFromMessage}
        onOpenSession={actions.resumeSession}
        onExport={(format) => {
          if (state.detailSession) actions.exportDetail(state.detailSession, format);
        }}
      />

      {/*
        设置中心取代了旧的扁平九标签覆盖层。菜单栏被移除后，
        「关于」「打开日志目录」「退出」「设置会话」这些动作都从这里进（spec §11）。
      */}
      {state.settingsOpen && (
        <SettingsCenter
          providerId={providerId}
          providers={providers}
          targetId={targetId}
          selectedTarget={selectedTarget}
          initialTab={state.settingsTab}
          version={appVersion}
          onOpenSessionSettings={actions.openSessionSettings}
          onQuit={actions.quit}
          onOpenLogDirectory={actions.openLogDirectory}
          onClose={actions.closeSettings}
          onError={actions.notify}
          onNotify={actions.notify}
        />
      )}

      {/*
        「设置会话」：手动指定 WSL 里的 Codex 会话目录。自动探测受发行版、用户、
        nvm/home 布局影响，探测不到时没有这个入口那台目标就等于用不了。

        # 为什么它排在设置中心**之后**

        `SettingsCenter` 的运行时环境页正是打开它的地方（`onOpenSessionSettings`），
        而这两层同属 `--ui-z-overlay=60` 这一档：**同一档里 DOM 顺序决定谁盖住谁**。
        它此前排在设置中心之前，于是从这里点开之后**什么都看不见**——框就压在下面
        等着填，用户的结论只能是「按钮坏了」。排到设置中心与消息弹框之后才看得见。
        （同形态的第一处是退出确认框，见文件末尾。）
      */}
      {state.sessionSettings && (
        <SessionSettingsDialog
          currentPath={selectedTarget?.codexHome}
          wslPath={state.sessionSettings.path}
          isWsl={selectedTarget?.kind === "wsl"}
          supportsSessionSettings={canSessionSettings}
          onChange={actions.changeSessionSettingsPath}
          onClose={actions.closeSessionSettings}
          onRestore={() => actions.restoreSessionSettings(state.sessionSettings!.distro)}
          onSave={() =>
            actions.saveSessionSettings(state.sessionSettings!.distro, state.sessionSettings!.path)
          }
        />
      )}

      {/*
        退出确认框**必须排在最后**，由**后端**发起（它拦得住窗口关闭），
        这里只负责显示与回答。

        # 为什么它的位置是有讲究的

        这几个弹层同属 `--ui-z-overlay=60` 这一档：**同一档里 DOM 顺序决定谁盖住谁**。
        它此前排在设置中心与消息弹框之前，于是那两个一开就把它压在下面——
        用户点了窗口关闭，看到的却是一个看不见的问题：「后端是不是没反应？」
        而框就在下面等着回答，后端一直等。
        排到最后就等于「它在最上面」——这正是它该有的位置：它问的是**要不要退出整个
        应用**，比任何一个面板都更需要被看见。
      */}
      <ExitConfirm onError={actions.reportError} />
    </>
  );
}
