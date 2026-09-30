import { useCallback, useEffect, useState } from "react";

import { Button, ToolbarSpacer, useToast } from "../components";
import * as api from "../ipc/commands";
import type { AiSession, AiTarget, ApiVendor } from "../types";
import { AppMenubar } from "./app-menubar";
import type { ConfigTab } from "./config-overlay";
import { AppOverlays } from "./overlay-layer";
import { supportsBranch, supportsDuplicate } from "./provider-capabilities";
import { planResume } from "./resume-session";
import { SessionWorkspace, WorkspaceTrailing, canStartCodex } from "./session-workspace";
import { displayTitle } from "./session-display";
import { Sidebar } from "./sidebar";
import { StatusBar } from "./status-bar";
import { useConsole } from "./use-console";
import { useTerminals } from "./use-terminals";
import { useTheme } from "./use-theme";
import { useWorkspaceTabs } from "./use-workspace-tabs";

/**
 * 应用外壳。
 *
 * 布局沿用旧平台的形状：**菜单栏 /〔侧栏 + 工作区〕/ 状态栏**。
 *
 * # 这个文件只负责「接线」
 *
 * 它此前有 1460 行，其中绝大部分是**可以直接搬走的**：侧栏 260 行、
 * 工作区正文 240 行、状态栏 100 行、右键菜单 55 行、标签逻辑 120 行。
 * 现在它们分别在：
 *
 * | 内容 | 位置 |
 * | --- | --- |
 * | 侧栏（平台/目标/两级导航/三种列表） | [`Sidebar`](./sidebar.tsx) |
 * | 工作区（标签 + 终端栈 + 会话正文） | [`SessionWorkspace`](./session-workspace.tsx) |
 * | 状态栏 | [`StatusBar`](./status-bar.tsx) |
 * | 右键菜单 | [`SessionContextMenu`](./session-context-menu.tsx) |
 * | 标签的增删与竞态 | [`useWorkspaceTabs`](./use-workspace-tabs.ts) |
 *
 * 留在这里的只有三类东西：**状态**（选中的标签、打开的弹层）、
 * **动作**（开终端、导出、改配置）、**组装**。
 */
export function App() {
  // 主题：把偏好写到 <html data-theme>，所有 token 随之整套切换。
  // 返回值这一轮还无人消费，裸调用即可。
  useTheme();

  /**
   * 一句话反馈**唯一**的出口。
   *
   * # 为什么收敛成一个通道
   *
   * 旧平台**每个动作都有一句结果回应**（改名、复制、删除、恢复、收藏、保存供应商、
   * 启用 Skill、装完 CLI…）。新平台此前只有零星几处，因为每一处都要自己拿一个
   * `useToast`——散在十几个组件里，补齐过程必然漏。
   *
   * 现在分两层：
   * - `useConsole({ notify })` 报**会话与设置**类动作（它们都在那个 hook 里）；
   * - `ConfigOverlay` 拿到 `onNotify` 往下传给各面板（供应商 / 技能 / 环境 / 网关）。
   *
   * 于是「做完一件事要说一声」只有一处实现，新加动作时也只需要调一次 `notify`。
   *
   * # 为什么必须自动消失
   *
   * 常驻的条幅会把正文往下压，而这种消息过几秒就没有价值了。
   * **静默完成与没执行在用户看来是一样的**——这才是要补它的理由。
   *
   * **必须在 `useConsole` 之前**：那个 hook 要拿 `show` 当通知回调。
   */
  const toast = useToast();
  const console = useConsole({ notify: toast.show });
  const terminals = useTerminals();
  const { selectedTarget } = console;

  const workspace = useWorkspaceTabs(terminals, console.selectedSession);

  /**
   * 从会话列表里按编号找回完整会话——右键菜单要它的文件路径，
   * 分支面板要它的父会话。
   *
   * 返回 `undefined` 而不是 `null`：调用方（`branchParent`）的契约就是
   * 「找不到就没有」，而 `null` 会在类型上要求每个调用点各写一次转换。
   */
  const sessionById = useCallback(
    (id: string) => console.sessions.find((item) => item.id === id),
    [console.sessions]
  );

  /* ── 弹层 ─────────────────────────────────────────────── */

  const [configOpen, setConfigOpen] = useState(false);
  const [configTab, setConfigTab] = useState<ConfigTab>("vendors");
  const [aboutOpen, setAboutOpen] = useState(false);
  const [appVersion, setAppVersion] = useState("");
  const [menu, setMenu] = useState<{ x: number; y: number; sessionId: string } | null>(null);

  /**
   * 「设置会话」对话框的草稿；`null` 表示没打开。
   *
   * 只对 **WSL 目标**有意义：本机目标的会话目录就是用户目录，不需要手动设置。
   */
  const [sessionSettings, setSessionSettings] = useState<{ distro: string; path: string } | null>(
    null
  );

  /** 最近一次导出的实际落点，显示出来让用户知道东西去哪了。 */
  const [exportedTo, setExportedTo] = useState("");

  /**
   * 会话详情弹层。
   *
   * 与主区那条线程**并存**：那条是「随列表走的浏览」，这条是「停下来研究一条」。
   * 旧平台就是这么分的（`session-detail-modal.tsx`），两者是两种动作而不是二选一。
   */
  const [detailSession, setDetailSession] = useState<AiSession | null>(null);
  /** 标签右键菜单的位置与目标标签。`null` 表示没打开。 */
  const [tabMenu, setTabMenu] = useState<{ key: string; x: number; y: number } | null>(null);
  /**
   * 从右键菜单打开的重命名 / 复制对话框。
   *
   * 与主区那条内联输入框**并存**：那一处是「我正在看的这条」，这一处是
   * 「列表里的某一条」——两种上下文，因此两个入口。
   */
  const [titleDialog, setTitleDialog] = useState<
    { kind: "rename" | "duplicate"; session: AiSession } | null
  >(null);
  /**
   * 侧栏是否折起来。
   *
   * 折起来把宽度全让给终端——终端排版按列数算，多 200px 就是多十几列。
   * 状态**不持久化**：它是一次会话里的临时调整，与窗口大小同理。
   */
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);

  /**
   * 新会话对话框的内容；`null` 表示没打开。
   *
   * 四个字段都是**受控**的：对话框自己不保存，确认时才把工作目录与 CLI 参数
   * 交给 `openTerminal`（旧平台也是这个形状）。
   */
  const [newSession, setNewSession] = useState<{
    cwd: string;
    title: string;
    prompt: string;
    cliArgs: string;
    /**
     * 非空表示这是**恢复**历史会话（`sessionId` 会被变成 `--resume <id>`）。
     *
     * `missingCwd` 只在**确认**原工作目录已不在时才有值——那时对话框切成
     * `设置工作目录` / `继续打开` 形态。判断不出来就保持 `undefined`：
     * **不知道不等于丢了**。
     */
    resume?: { sessionId: string; missingCwd?: string };
  } | null>(null);

  const openConfig = useCallback((tab: ConfigTab) => {
    setConfigTab(tab);
    setConfigOpen(true);
  }, []);

  /* ── 工作台要的数据 ───────────────────────────────────── */

  /**
   * 供应商清单。
   *
   * 工作台的「最近路由变更」要把供应商编号翻成名字——只显示一串 `vendor-3f2a`
   * 用户认不出那是谁。**失败时留空数组**：工作台是概览，不该因为供应商列表读不到
   * 就整块变成错误（编号仍然显示得出来）。
   */
  const [sidebarVendors, setSidebarVendors] = useState<ApiVendor[]>([]);
  useEffect(() => {
    if (!console.providerId) {
      setSidebarVendors([]);
      return;
    }
    let cancelled = false;
    void api
      .listVendors(console.providerId)
      .then((list) => {
        if (!cancelled) setSidebarVendors(list);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [console.providerId]);

  /**
   * 最近一次供应商路由变更。
   *
   * 从**当前前台终端**上取：路由是终端的属性，把所有终端的变更混在一起会让人
   * 以为那是当前这一路发生的事。
   */
  const activeTerminal = terminals.terminals.find(
    (item) => item.id === workspace.activeTab?.terminalId
  );

  /**
   * **选中会话跟随当前终端。**
   *
   * 这是旧平台的规则（`app.tsx` 里那个依赖 `targetId` 的 effect）：
   *
   * ```js
   * const activeSessionForTarget = activeTab?.targetId === targetId ? activeTab.session : null;
   * setSelectedId(activeSessionForTarget?.id || "");
   * ```
   *
   * 也就是**跟着当前终端那条会话走；终端没开、或它不属于当前目标时就是不选**。
   * 从来不选「列表第一项」。
   *
   * 这条规则是「点历史会话进入终端后还能看到详情/从此处分支」的另一半：
   * 详情按钮在标签栏上、绑的是当前终端的会话，而选中项也要跟着走，
   * 否则右侧正文显示的是另一条会话。
   *
   * 只在**当前标签确实是终端**时同步。看会话正文（`kind === "session"`）时不动选中项
   * ——那时用户是在列表里挑，选中项就是他的选择。
   */
  useEffect(() => {
    if (workspace.activeTab?.kind !== "terminal") return;
    const sessionId = workspace.activeTab.sessionId;
    if (!sessionId) return;
    console.setSelectedId(sessionId);
  }, [workspace.activeTab?.kind, workspace.activeTab?.sessionId, console]);
  /**
   * 最近一次路由变更。
   *
   * `VendorSwitch` 里**没有时间戳**——后端的事件也不带。因此时间用「现在」：
   * 这个值是在收到事件时算出来的，它就是那一刻。编一个过去的时间会比不显示更糟。
   */
  const lastRouteSwitch =
    activeTerminal?.vendorId && activeTerminal.lastSwitch
      ? {
          vendorId: activeTerminal.lastSwitch.vendorId,
          reason: activeTerminal.lastSwitch.reason,
          at: new Date().toISOString()
        }
      : undefined;

  /**
   * 工作台的刷新计数器。
   *
   * 后端每转发一个请求就推一条事件；工作台据此重新取用量与异常。
   * **合并刷新**：一个终端重试时一秒能推好几条，而每次刷新要拉两个接口。
   */
  const [workbenchToken, setWorkbenchToken] = useState(0);
  useEffect(() => {
    let timer: number | undefined;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void api
      .listen(api.CHANNEL_GATEWAY_REQUEST_RECORDED, () => {
        if (timer !== undefined) window.clearTimeout(timer);
        timer = window.setTimeout(() => {
          timer = undefined;
          setWorkbenchToken((value) => value + 1);
        }, 300);
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
      if (timer !== undefined) window.clearTimeout(timer);
    };
  }, []);

  /* ── 终端 ─────────────────────────────────────────────── */

  /**
   * 起一路 Codex 终端。
   *
   * `options` 是新会话对话框确认后传来的：**工作目录与 CLI 参数**。
   * 不传时沿用「当前选中会话的目录」，也就是旧行为。
   */
  const openTerminal = useCallback(
    async (options?: {
      cwd?: string;
      cliArgs?: string;
      prompt?: string;
      sessionId?: string;
      title?: string;
    }) => {
      try {
        const id = await terminals.openTerminal(
          selectedTarget,
          options?.cwd ?? console.selectedSession?.cwd,
          options?.cliArgs,
          options?.prompt,
          options?.sessionId
        );
        workspace.addTerminalTab(id, options?.title ?? "终端", options?.sessionId);
      } catch (error) {
        console.setError(error instanceof Error ? error.message : String(error));
      }
    },
    [terminals, selectedTarget, console, workspace]
  );

  /**
   * 打开「新会话」对话框。
   *
   * 旧平台的流程是**先在这个对话框里定下工作目录、会话标题、启动提示词与 CLI 参数，
   * 再打开**。此前这里是「点一下直接进终端」——用户报的 H2 说的就是这件事：
   * 「**默认不选择平台、目标**，选中后直接进入终端会话」。
   */
  const beginNewSession = useCallback(() => {
    setNewSession({
      cwd: console.selectedSession?.cwd ?? "",
      title: "",
      prompt: "",
      cliArgs: ""
    });
  }, [console.selectedSession?.cwd]);

  /**
   * 点历史会话：**恢复那次会话并进入终端**（旧平台 `openResumeWithDirectory`）。
   *
   * 三条分支，与旧平台一致：
   *
   * | 情况 | 行为 |
   * | --- | --- |
   * | 原工作目录还在 | **直接进终端**（带 `--resume`），不弹对话框 |
   * | 原目录已不在 | 弹 `设置工作目录` 形态，按钮是 `继续打开` |
   * | 判断不出来（如 WSL 上那条检查未接线） | 按「还在」处理——**不知道不等于丢了** |
   *
   * 为什么恢复必须带 `cwd`：旧平台有一句要紧的注释——「Qoder 按**项目目录**定位
   * `--resume` 的历史文件，恢复时必须保留原始工作目录」。
   */
  const beginResumeSession = useCallback(
    async (target: AiTarget, session: AiSession) => {
      // 先清掉这条会话上已经死掉的终端标签——否则下一步会切到那一张死标签上，
      // 用户看到的是「再次进入被锁住」。
      workspace.clearDeadTabs(session.id);

      // 先把选中态切过去（列表高亮、标签）。
      console.openAt(target, session);

      const cwd = session.cwd ?? "";
      // 决策本身在 `resume-session.ts` 里，**有四条分支**且逐条有测试。
      const plan = await planResume(cwd || undefined, (path) => api.pathExists(path));
      if (plan.kind === "dialog") {
        setNewSession({
          cwd: plan.cwd,
          title: session.title ?? "",
          prompt: "",
          cliArgs: "",
          resume: { sessionId: session.id, missingCwd: plan.missingCwd }
        });
        return;
      }
      void openTerminal({
        cwd: plan.cwd,
        sessionId: session.id,
        // 标签直接叫这条会话的名字：用户点的是「会话甲」，标签上却写「终端」会很怪。
        title: displayTitle(session)
      });
    },
    [console, openTerminal, workspace]
  );

  /** 侧栏回传的是编号（它不持有会话内容），这里还原成完整会话。 */
  const resumeById = useCallback(
    (target: AiTarget, sessionId: string) => {
      const session = sessionById(sessionId) ?? console.selectedSession;
      if (!session || session.id !== sessionId) {
        // 会话可能刚被别处删掉。给一句明确的说明，而不是静默无反应。
        toast.show("该会话已不在当前列表里，请刷新后重试。");
        return;
      }
      void beginResumeSession(target, session);
    },
    [sessionById, console.selectedSession, beginResumeSession, toast]
  );

  /* ── 导出 ─────────────────────────────────────────────── */

  /** 导出：后端决定落点，这里只负责把「去了哪」告诉用户。 */
  const runExport = useCallback(
    async (format: "markdown" | "json" | "html") => {
      const result = await console.exportCurrent(format);
      if (result) setExportedTo(result.filePath);
    },
    [console]
  );

  /* ── 组装 ─────────────────────────────────────────────── */

  const trash = console.view === "trash";

  return (
    <div className="shell">
      <AppMenubar
        selectedTarget={selectedTarget}
        hasActiveSession={workspace.tabs.some((tab) => tab.kind === "session")}
        attentionEntries={terminals.terminals
          .filter((item) => terminals.attention[item.id])
          .map((item) => {
            const attention = terminals.attention[item.id]!;
            return {
              terminalId: item.id,
              title: item.title,
              provider: item.providerId,
              kind: attention.kind,
              detail: attention.detail,
              updatedAt: attention.updatedAt
            };
          })}
        onSelectAttention={(terminalId) => {
          workspace.setActiveKey(terminalId);
          // 切过去就算看过了，清掉标记——否则铃铛上的数字一直不减。
          terminals.clearAttention(terminalId);
        }}
        onDismissAttention={(terminalId) => terminals.clearAttention(terminalId)}
        actions={{
          manageSkills: () => openConfig("skills"),
          openGatewaySettings: () => openConfig("gateway"),
          manageVendors: () => openConfig("vendors"),
          // 「设置会话」：只有 WSL 目标有 `distro` 可写，因此这里也按 WSL 判定。
          openSessionSettings:
            selectedTarget?.kind === "wsl" && selectedTarget.distro
              ? () =>
                  setSessionSettings({
                    distro: selectedTarget.distro ?? "",
                    path: selectedTarget.codexHome ?? ""
                  })
              : undefined,
          installCli: () => openConfig("environment"),
          exportSession: (format) => void runExport(format),
          showAbout: () => {
            setAboutOpen(true);
            void api
              .getAppVersion()
              .then(setAppVersion)
              .catch(() => setAppVersion("无法读取"));
          },
          // 打开日志目录：后端按运行根算出 `log_dir()` 并在文件管理器里打开它
          // （`app:command` 的 `openLogDir`）。界面不需要知道路径。
          openLogDirectory: () => void api.appCommand("openLogDir"),
          // 清理 Gateway 日志：做完要说一声，否则「静默完成」与「没执行」看起来一样。
          clearGatewayLogs: () => {
            void api
              .clearGatewayLogs()
              .then((removed) => toast.show(`已清理 ${removed} 条 Gateway 日志。`))
              .catch((error: unknown) =>
                console.setError(error instanceof Error ? error.message : String(error))
              );
          },
          // 退出走浏览器窗口关闭，与标题栏的关闭同一条路，避免两套退出语义。
          quit: () => window.close()
        }}
      />

      <AppOverlays
        state={{
          aboutOpen,
          appVersion,
          toastMessage: toast.message,
          exportedTo,
          sessionSettings,
          newSession,
          menu,
          tabMenu,
          titleDialog,
          detailSession,
          configOpen,
          configTab
        }}
        actions={{
          closeAbout: () => setAboutOpen(false),
          closeToast: toast.dismiss,
          closeExported: () => setExportedTo(""),
          openExported: () => void api.openPath(exportedTo),
          changeSessionSettingsPath: (path) =>
            setSessionSettings((current) => (current ? { ...current, path } : current)),
          closeSessionSettings: () => setSessionSettings(null),
          // 恢复自动探测 = **清掉人工设置**，不是保存空值——这两件事不能合。
          restoreSessionSettings: (distro) => {
            setSessionSettings(null);
            void api
              .clearWslCodexHome(distro)
              .then(() => console.reloadSessions())
              .catch((error: unknown) =>
                console.setError(error instanceof Error ? error.message : String(error))
              );
          },
          saveSessionSettings: (distro, path) => {
            setSessionSettings(null);
            void api
              .setWslCodexHome(distro, path)
              .then(() => console.reloadSessions())
              .catch((error: unknown) =>
                console.setError(error instanceof Error ? error.message : String(error))
              );
          },
          changeNewSession: (patch) =>
            setNewSession((current) => (current ? { ...current, ...patch } : current)),
          chooseNewSessionDirectory: () =>
            void api.chooseDirectory().then((picked: string | null) => {
              if (picked) {
                setNewSession((current) => (current ? { ...current, cwd: picked } : current));
              }
            }),
          closeNewSession: () => setNewSession(null),
          confirmNewSession: () => {
            const draft = newSession;
            setNewSession(null);
            if (!draft) return;
            void openTerminal({
              cwd: draft.cwd || undefined,
              cliArgs: draft.cliArgs,
              prompt: draft.prompt,
              // 恢复形态：把会话编号递下去，后端会变成 `--resume <id>`。
              sessionId: draft.resume?.sessionId
            });
          },
          closeMenu: () => setMenu(null),
          openSessionFolder: (session) => {
            // 后端的「打开当前会话目录」读的是选中状态，因此先把它选中。
            console.setSelectedId(session.id);
            void console.openCurrentFolder();
          },
          toggleFavorite: (session) => void console.toggleFavorite(session),
          copyText: (text) => void api.copyText(text),
          removeSession: (session) => void console.removeSession(session, trash),
          restoreSession: (session) => void console.restoreOne(session),
          startRename: (session) => setTitleDialog({ kind: "rename", session }),
          startDuplicate: (session) => setTitleDialog({ kind: "duplicate", session }),
          closeTitleDialog: () => setTitleDialog(null),
          confirmRename: (session, title) => {
            console.setSelectedId(session.id);
            void console.renameSession(title);
            setTitleDialog(null);
          },
          confirmDuplicate: (session, title) => {
            console.setSelectedId(session.id);
            void console.duplicateCurrent(title);
            setTitleDialog(null);
          },
          closeTabMenu: () => setTabMenu(null),
          closeTabThis: workspace.closeTab,
          closeTabOthers: workspace.closeOtherTabs,
          closeTabAll: workspace.closeAllTabs,
          closeDetail: () => setDetailSession(null),
          resumeSession: (session) => {
            if (selectedTarget) void beginResumeSession(selectedTarget, session);
          },
          branchFromMessage: (sourceLine) => void console.branchFromMessage(sourceLine),
          closeConfig: () => setConfigOpen(false),
          reportError: console.setError,
          notify: toast.show
        }}
        providers={console.providers}
        providerId={console.providerId}
        targetId={console.targetId}
        targets={console.targets}
        selectedTarget={selectedTarget}
        trash={trash}
        // 能力判定只在这里算一次：`provider-capabilities.ts` 是唯一的判据来源。
        canDuplicate={supportsDuplicate(console.providers, console.providerId)}
        canBranch={supportsBranch(console.providers, console.providerId)}
        resolveSession={sessionById}
        tabLabel={workspace.tabItems.find((item) => item.key === tabMenu?.key)?.label ?? "标签"}
        closableTabCount={workspace.closableKeys().length}
      />

      <div className="main">
        <Sidebar
          console={console}
          selectedTarget={selectedTarget}
          onResume={resumeById}
          onMenu={(x, y, sessionId) => setMenu({ x, y, sessionId })}
          providers={console.providers}
          vendors={sidebarVendors}
          gateway={console.gateway}
          lastSwitch={lastRouteSwitch}
          workbenchToken={workbenchToken}
          collapsed={sidebarCollapsed}
          onToggleCollapsed={() => setSidebarCollapsed((value) => !value)}
        />

        {/*
          工作区与底部停靠面板**同一列上下排**。
          这一层包裹是必须的：`.main` 是「侧栏 | 内容」两列的网格，
          把停靠面板直接挂到 `.shell` 上会落到状态栏**下面**——
          而它必须留在侧栏右侧、状态栏之上。
        */}
        <div className="workarea">
          <SessionWorkspace
          tabItems={workspace.tabItems}
          activeKey={workspace.activeKey}
          activeTab={workspace.activeTab}
          terminalItems={terminals.terminals}
          registerSink={terminals.registerSink}
          unregisterSink={terminals.unregisterSink}
          onError={console.setError}
          onActivate={(key) => {
            workspace.setActiveKey(key);
            // 看过就算处理过：清掉这一路的提醒标记，否则小点一直亮着，
            // 用户会以为还有事没处理。
            if (workspace.tabs.find((tab) => tab.key === key)?.kind === "terminal") {
              terminals.clearAttention(key);
            }
          }}
          onClose={workspace.closeTab}
          onTabContextMenu={(key, x, y) => setTabMenu({ key, x, y })}
          trailing={
            <>
              <WorkspaceTrailing
                canStartCodex={canStartCodex(selectedTarget)}
                onNewCodex={beginNewSession}
                /*
                  「详情」要看的会话。

                  **当前终端那条会话优先**——这正是「点历史会话进入终端后还能查看详情」
                  的关键：那时正文不渲染（原来详情按钮就在正文头部，于是消失了），
                  而选中项也未必等于这条终端所属的会话。旧平台同理：它绑的是
                  `activeSession`（活动标签的会话）。
                */
                detailSession={
                  sessionById(workspace.activeTab?.sessionId ?? "") ?? console.selectedSession
                }
                onOpenDetail={(session) => setDetailSession(session)}
              />
              <ToolbarSpacer />
            </>
          }
          session={console.selectedSession}
          messageTotal={console.messageTotal}
          messages={console.messages}
          hasEarlier={console.hasEarlier}
          loadingMore={console.loadingMore}
          onLoadEarlier={() => void console.loadEarlierMessages()}
          onBranch={(sourceLine) => void console.branchFromMessage(sourceLine)}
          onOpenSession={(session) => {
            const target =
              console.targets.find(
                (item) => item.id === session.metadata?.branch?.parentTargetId
              ) ?? selectedTarget;
            if (target) void beginResumeSession(target, session);
          }}
          branchParent={(sessionId) => sessionById(sessionId)}
          branchChildren={(parentSessionId) =>
            api.listSessionChildren(parentSessionId, selectedTarget?.id)
          }
        />
        </div>
      </div>

      {exportedTo && (
        <div className="export-hint">
          已导出到 <code>{exportedTo}</code>
          <Button small onClick={() => void api.openPath(exportedTo)}>
            打开
          </Button>
          <Button small onClick={() => setExportedTo("")}>
            知道了
          </Button>
        </div>
      )}

      <StatusBar
        busy={console.busy}
        searching={console.searching}
        session={console.selectedSession}
        targetCount={console.targets.length}
        sessionCount={console.sessions.length}
        terminalCount={terminals.terminals.length}
        activeTerminalId={workspace.activeTab?.terminalId}
        terminals={terminals}
        error={console.error}
        gateway={console.gateway}
      />
    </div>
  );
}
