import { useCallback, useEffect, useMemo, useState } from "react";

import {
  Button,
  CommandPalette,
  ToolbarSpacer,
  useToast,
  type Command
} from "../components";
import * as api from "../ipc/commands";
import type { AiSession, AiTarget, ApiVendor } from "../types";
import { AboutDialog } from "./about-dialog";
import { AppMenuBar } from "./app-menu-bar";
import { AttentionCenter } from "./attention-center";
import { createAppMenus } from "./app-menus";
import { ConfigOverlay, type ConfigTab } from "./config-overlay";
import { ExitConfirm } from "./exit-confirm";
import { NewSessionDialog } from "./new-session-dialog";
import { planResume } from "./resume-session";
import { SessionContextMenu } from "./session-context-menu";
import { SessionActions, exportToDirectory } from "./session-actions";
import { SessionDetailContainer } from "./session-detail-container";
import { displayTitle } from "./session-display";
import { SessionSettingsDialog } from "./session-settings-dialog";
import { SessionWorkspace, WorkspaceTrailing, canStartCodex } from "./session-workspace";
import { Sidebar } from "./sidebar";
import { StatusBar } from "./status-bar";
import { useConsole } from "./use-console";
import { useTerminals } from "./use-terminals";
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
  const console = useConsole();
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
  const [paletteOpen, setPaletteOpen] = useState(false);
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

  /**
   * 一句话的操作结果提示。
   *
   * 菜单里的动作（清理 Gateway 日志）没有自己的界面，做完必须说一声——
   * **静默完成与没执行在用户看来是一样的**。用 [`useToast`] 是因为它要自动消失：
   * 常驻的条幅会把正文往下压，而这条消息过几秒就没有价值了。
   */
  const toast = useToast();

  /** 最近一次导出的实际落点，显示出来让用户知道东西去哪了。 */
  const [exportedTo, setExportedTo] = useState("");

  /**
   * 会话详情弹层。
   *
   * 与主区那条线程**并存**：那条是「随列表走的浏览」，这条是「停下来研究一条」。
   * 旧平台就是这么分的（`session-detail-modal.tsx`），两者是两种动作而不是二选一。
   */
  const [detailSession, setDetailSession] = useState<AiSession | null>(null);

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

  /** ⌘K / Ctrl+K 打开命令面板。 */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setPaletteOpen((current) => !current);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
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

  const openSystemTerminal = useCallback(async () => {
    try {
      // 系统终端不依赖目标：没选目标时也能开一个本机 shell。
      const id = await terminals.openSystemTerminal(selectedTarget, console.selectedSession?.cwd);
      workspace.addTerminalTab(id, "系统终端");
    } catch (error) {
      console.setError(error instanceof Error ? error.message : String(error));
    }
  }, [terminals, selectedTarget, console, workspace]);

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

  /* ── 命令面板 ─────────────────────────────────────────── */

  /**
   * 命令面板的命令表。
   *
   * 这里**只放界面上确实存在的动作**：面板不是第二套实现，它只是同一批动作的另一个
   * 入口。多出一条「看起来能用但其实没接线」的命令，比少一条更糟。
   */
  const commands = useMemo<Command[]>(
    () => [
      { id: "pane.sessions", group: "视图", label: "显示会话列表", run: () => console.setPane("sessions") },
      { id: "pane.quick", group: "视图", label: "显示快捷访问", run: () => console.setPane("quick") },
      { id: "pane.search", group: "视图", label: "显示全局搜索", run: () => console.setPane("search") },
      { id: "view.active", group: "视图", label: "切换到活跃会话", run: () => { console.setPane("sessions"); console.setView("active"); } },
      { id: "view.trash", group: "视图", label: "切换到回收站", run: () => { console.setPane("sessions"); console.setView("trash"); } },
      { id: "term.codex", group: "终端", label: "新建 Codex 终端", run: beginNewSession },
      { id: "term.shell", group: "终端", label: "新建系统终端", run: () => void openSystemTerminal() },
      { id: "targets.reload", group: "动作", label: "重新探测目标", run: () => void console.reloadTargets() },
      { id: "sessions.reload", group: "动作", label: "刷新会话列表", run: () => void console.reloadSessions() },
      { id: "export.md", group: "动作", label: "导出当前会话为 Markdown", run: () => void runExport("markdown") },
      { id: "export.json", group: "动作", label: "导出当前会话为 JSON", run: () => void runExport("json") },
      { id: "config.vendors", group: "配置", label: "供应商", run: () => openConfig("vendors") },
      { id: "config.skills", group: "配置", label: "技能", run: () => openConfig("skills") },
      { id: "config.gateway", group: "配置", label: "网关", run: () => openConfig("gateway") },
      { id: "config.environment", group: "配置", label: "CLI 环境", run: () => openConfig("environment") },
      { id: "config.diagnostics", group: "配置", label: "诊断（全部目标状态）", run: () => openConfig("diagnostics") }
    ],
    [console, beginNewSession, openSystemTerminal, runExport, openConfig]
  );

  /* ── 组装 ─────────────────────────────────────────────── */

  const menuSession = menu ? sessionById(menu.sessionId) : null;
  const trash = console.view === "trash";

  return (
    <div className="shell">
      {/*
        菜单栏。「文件」「帮助」两个按钮**曾经没有 `onClick`**——点了没反应，是死按钮。
        死按钮比没有按钮更糟：它让用户以为自己点错了。

        现在补上真正的分组菜单（结构在 `app-menus.ts`，数据与渲染分开、可单测）。
        **动作缺失的项一律显示为禁用**（见 `disabledWhenMissing`）：
        面还没实现的会显示成「这里有个东西、现在还不能用」，而不是点了没反应的死按钮。
      */}
      <div className="menubar">
        <span className="brand">AI 控制台</span>
        <AppMenuBar
          menus={createAppMenus(
            {
              supportsSkills: Boolean(selectedTarget),
              // 「设置会话」只对 WSL 目标可用。**按真实字段判定**
              // （`AiTarget.kind === "wsl"`），而不是写死。
              isWslTarget: selectedTarget?.kind === "wsl",
              hasActiveSession: workspace.tabs.some((tab) => tab.kind === "session")
            },
            {
              openCommandPalette: () => setPaletteOpen(true),
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
              openSystemTerminal: () => void openSystemTerminal(),
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
            }
          )}
        />
        {/* 「配置」与「命令面板」两个按钮**删掉了**：它们与菜单完全重复。
            旧平台的顶栏只有 `AppMenuBar` + 提醒铃铛，命令面板靠 Ctrl+K 或菜单打开。
            重复的入口不只是多余——它们让「功能到底从哪进」变得含糊。 */}
        {/* 会话提醒：位置与旧平台一致（菜单栏右侧）。没有提醒时它自己不渲染。 */}
        <AttentionCenter
          entries={terminals.terminals
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
          onSelect={(terminalId) => {
            workspace.setActiveKey(terminalId);
            // 切过去就算看过了，清掉标记——否则铃铛上的数字一直不减。
            terminals.clearAttention(terminalId);
          }}
          onDismiss={(terminalId) => terminals.clearAttention(terminalId)}
        />
      </div>

      {paletteOpen && (
        <CommandPalette commands={commands} onClose={() => setPaletteOpen(false)} />
      )}

      {aboutOpen && <AboutDialog version={appVersion} onClose={() => setAboutOpen(false)} />}

      {/* 菜单动作与侧栏提示共用的条幅。与「已导出到…」同一形状：都要能被看见、也都能收掉。 */}
      {toast.message && (
        <div className="export-hint" role="status">
          {toast.message}
          <Button small onClick={toast.dismiss}>
            知道了
          </Button>
        </div>
      )}

      {/*
        「设置会话」：手动指定 WSL 里的 Codex 会话目录。自动探测受发行版、用户、
        nvm/home 布局影响，探测不到时没有这个入口那台目标就等于用不了。
      */}
      {sessionSettings && (
        <SessionSettingsDialog
          currentPath={selectedTarget?.codexHome}
          wslPath={sessionSettings.path}
          isWsl={selectedTarget?.kind === "wsl"}
          supportsSessionSettings
          onChange={(value) =>
            setSessionSettings((current) => (current ? { ...current, path: value } : current))
          }
          onClose={() => setSessionSettings(null)}
          onRestore={() => {
            const draft = sessionSettings;
            setSessionSettings(null);
            void api
              .clearWslCodexHome(draft.distro)
              .then(() => console.reloadSessions())
              .catch((error: unknown) =>
                console.setError(error instanceof Error ? error.message : String(error))
              );
          }}
          onSave={() => {
            const draft = sessionSettings;
            setSessionSettings(null);
            void api
              .setWslCodexHome(draft.distro, draft.path)
              .then(() => console.reloadSessions())
              .catch((error: unknown) =>
                console.setError(error instanceof Error ? error.message : String(error))
              );
          }}
        />
      )}

      {/*
        新会话对话框。旧平台的流程是**先定工作目录 / 标题 / 启动提示词 / CLI 参数，再打开**。
        此前这里是「点一下直接进终端」——用户报的 H2 说的就是这件事。
      */}
      {newSession && (
        <NewSessionDialog
          pendingResume={
            newSession.resume?.missingCwd ? { missingCwd: newSession.resume.missingCwd } : null
          }
          supportsCustomCwd
          cwd={newSession.cwd}
          title={newSession.title}
          prompt={newSession.prompt}
          cliArgs={newSession.cliArgs}
          onChooseDirectory={() =>
            void api.chooseDirectory().then((picked: string | null) => {
              if (picked) {
                setNewSession((current) => (current ? { ...current, cwd: picked } : current));
              }
            })
          }
          onTitleChange={(value) =>
            setNewSession((current) => (current ? { ...current, title: value } : current))
          }
          onPromptChange={(value) =>
            setNewSession((current) => (current ? { ...current, prompt: value } : current))
          }
          onCliArgsChange={(value) =>
            setNewSession((current) => (current ? { ...current, cliArgs: value } : current))
          }
          onClose={() => setNewSession(null)}
          onConfirm={() => {
            const draft = newSession;
            setNewSession(null);
            void openTerminal({
              cwd: draft.cwd || undefined,
              cliArgs: draft.cliArgs,
              prompt: draft.prompt,
              // 恢复形态：把会话编号递下去，后端会变成 `--resume <id>`。
              sessionId: draft.resume?.sessionId
            });
          }}
        />
      )}

      {menu && menuSession && (
        <SessionContextMenu
          x={menu.x}
          y={menu.y}
          session={menuSession}
          trash={trash}
          onClose={() => setMenu(null)}
          onOpenFolder={() => {
            console.setSelectedId(menuSession.id);
            void console.openCurrentFolder();
          }}
          onToggleFavorite={() => void console.toggleFavorite(menuSession)}
          onCopyId={() => void api.copyText(menuSession.id)}
          onCopyPath={() => void api.copyText(menuSession.filePath)}
          onRemove={() => void console.removeSession(menuSession, trash)}
        />
      )}

      {/* 退出确认由**后端**发起（它拦得住窗口关闭），这里只负责显示与回答。 */}
      <ExitConfirm onError={console.setError} />

      {/* 会话详情：把一条会话**停下来看**。分页与取数都在那个容器里，
          这里只持有「要看哪一条」——关掉就回到原来的视图。 */}
      <SessionDetailContainer
        session={detailSession}
        target={selectedTarget}
        onClose={() => setDetailSession(null)}
        resolveParent={(sessionId) => sessionById(sessionId)}
        onBranch={(sourceLine) => void console.branchFromMessage(sourceLine)}
        onOpenSession={(session) => {
          if (selectedTarget) void beginResumeSession(selectedTarget, session);
        }}
        onError={console.setError}
      />

      {configOpen && (
        <ConfigOverlay
          providerId={console.providerId}
          targetId={console.targetId}
          targets={console.targets}
          initialTab={configTab}
          onClose={() => setConfigOpen(false)}
          onError={console.setError}
        />
      )}

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
        />

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
          trailing={
            <>
              <WorkspaceTrailing
                canStartCodex={canStartCodex(selectedTarget)}
                onNewCodex={beginNewSession}
                onNewShell={() => void openSystemTerminal()}
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
          sessionActions={
            <SessionActions
              session={console.selectedSession}
              onOpenDetail={() => setDetailSession(console.selectedSession)}
              onRename={(title) => void console.renameSession(title)}
              onDuplicate={() => void console.duplicateCurrent()}
              onExport={(format, toDirectory) => {
                if (!toDirectory) {
                  void runExport(format);
                  return;
                }
                void exportToDirectory(format, console.selectedId, (fmt, path) =>
                  console.exportCurrentTo(fmt, path)
                ).then((path) => {
                  if (path) setExportedTo(path);
                });
              }}
            />
          }
        />
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
