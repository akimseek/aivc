import type { ReactNode } from "react";

import { Button } from "../components";
import type { AiProviderId, AiProviderSummary, AiSession, AiTarget } from "../types";
import { AboutDialog } from "./about-dialog";
import { ConfigOverlay, type ConfigTab } from "./config-overlay";
import { ExitConfirm } from "./exit-confirm";
import { NewSessionDialog } from "./new-session-dialog";
import { SessionContextMenu } from "./session-context-menu";
import { SessionDetailContainer } from "./session-detail-container";
import { SessionDuplicateDialog, SessionRenameDialog } from "./session-title-dialogs";
import { SessionSettingsDialog } from "./session-settings-dialog";
import { TabContextMenu } from "./tab-context-menu";

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
  aboutOpen: boolean;
  /** 版本号；还没读到是空串，对话框会显示「正在读取」。 */
  appVersion: string;
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
  configOpen: boolean;
  configTab: ConfigTab;
};

/**
 * 全部弹层的**动作**。
 *
 * # 每一项都是必填的
 *
 * 写这个文件时我一度给 `onCopyId` / `onRemove` 填过 `() => undefined` 的占位——
 * 那样「复制编号」与「删除」会**静默失效**：按钮点得下去、没有任何反应、也不报错。
 * 因此这里的每一项都必填，漏接会被类型检查挡住。
 */
export type OverlaysActions = {
  closeAbout: () => void;
  closeToast: () => void;
  closeExported: () => void;
  openExported: () => void;
  changeSessionSettingsPath: (path: string) => void;
  closeSessionSettings: () => void;
  restoreSessionSettings: (distro: string) => void;
  saveSessionSettings: (distro: string, path: string) => void;
  changeNewSession: (patch: Partial<NewSessionDraft>) => void;
  chooseNewSessionDirectory: () => void;
  closeNewSession: () => void;
  confirmNewSession: () => void;
  closeMenu: () => void;
  openSessionFolder: (session: AiSession) => void;
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
  branchFromMessage: (sourceLine: number) => void;
  closeConfig: () => void;
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
 * 哪些标签可关、标题是什么。在这里猜（例如拿 key 去查会话）对**终端标签**
 * 是错的：终端标签的 key 是终端编号，查不到任何会话。
 */
export function AppOverlays({
  state,
  actions,
  providers,
  providerId,
  targetId,
  targets,
  selectedTarget,
  trash,
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
  targetId: string;
  targets: AiTarget[];
  selectedTarget: AiTarget | null;
  /** 在回收站视图里时，删除项变成「彻底删除」。 */
  trash: boolean;
  /**
   * 当前平台的能力。
   *
   * 由调用方算好传进来，不在这里查：`AppOverlays` 拿不到 `providers` 与当前平台的
   * 对应关系之外的东西，而且**能力判定只该有一处**（`provider-capabilities.ts`）。
   */
  canDuplicate: boolean;
  canBranch: boolean;
  /** 按编号找会话——右键菜单要它的文件路径。 */
  resolveSession: (sessionId: string) => AiSession | undefined;
  /** 标签菜单显示的那张标签的标题。 */
  tabLabel: string;
  /** 一共关得掉几张标签（会话标签不可关）。 */
  closableTabCount: number;
}) {
  const menuSession = state.menu ? resolveSession(state.menu.sessionId) : undefined;

  return (
    <>

      {state.aboutOpen && <AboutDialog version={state.appVersion} onClose={actions.closeAbout} />}

      {/*
        两条一句话提示，形状相同：都要能被看见、也都能收掉。
        合成一个渲染分支是为了让「提示只有一个地方定义」——
        分成两处迟早会长得不一样。
      */}
      {state.toastMessage && (
        <HintBar message={state.toastMessage} onDismiss={actions.closeToast} />
      )}
      {state.exportedTo && (
        <HintBar
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
        「设置会话」：手动指定 WSL 里的 Codex 会话目录。自动探测受发行版、用户、
        nvm/home 布局影响，探测不到时没有这个入口那台目标就等于用不了。
      */}
      {state.sessionSettings && (
        <SessionSettingsDialog
          currentPath={selectedTarget?.codexHome}
          wslPath={state.sessionSettings.path}
          isWsl={selectedTarget?.kind === "wsl"}
          supportsSessionSettings
          onChange={actions.changeSessionSettingsPath}
          onClose={actions.closeSessionSettings}
          onRestore={() => actions.restoreSessionSettings(state.sessionSettings!.distro)}
          onSave={() =>
            actions.saveSessionSettings(state.sessionSettings!.distro, state.sessionSettings!.path)
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
          supportsCustomCwd
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

      {/* 退出确认由**后端**发起（它拦得住窗口关闭），这里只负责显示与回答。 */}
      <ExitConfirm onError={actions.reportError} />

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

      {/* 会话详情：把一条会话**停下来看**。分页与取数都在那个容器里，
          这里只持有「要看哪一条」——关掉就回到原来的视图。 */}
      <SessionDetailContainer
        session={state.detailSession}
        target={targets.find((item) => item.id === targetId) ?? null}
        onClose={actions.closeDetail}
        resolveParent={resolveSession}
        onBranch={actions.branchFromMessage}
        canBranch={canBranch}
        onOpenSession={actions.resumeSession}
        onError={actions.reportError}
      />

      {state.configOpen && (
        <ConfigOverlay
          providerId={providerId}
          providers={providers}
          targetId={targetId}
          targets={targets}
          initialTab={state.configTab}
          onClose={actions.closeConfig}
          onError={actions.reportError}
          onNotify={actions.notify}
        />
      )}
    </>
  );
}

/**
 * 一句话提示条。
 *
 * 抽出来是因为「已导出到…」与「已清理 N 条日志」此前是两段各自手写的 JSX，
 * 形状一样、位置一样，迟早会长出差别。
 */
function HintBar({
  message,
  onDismiss,
  extra
}: {
  message: ReactNode;
  onDismiss: () => void;
  /** 「打开」这类附加动作。 */
  extra?: ReactNode;
}) {
  return (
    <div className="export-hint" role="status">
      {message}
      {extra}
      <Button small onClick={onDismiss}>
        知道了
      </Button>
    </div>
  );
}
