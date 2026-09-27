import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from "react";

import { AppMenuBar } from "./app-menu-bar";
import { BranchPanel } from "./branch-panel";
import { createAppMenus } from "./app-menus";
import { NewSessionDialog } from "./new-session-dialog";
import { planResume } from "./resume-session";

import {
  Button,
  ChipGroup,
  CommandPalette,
  EmptyState,
  Field,
  Notice,
  SearchInput,
  Select,
  Tabs,
  TextInput,
  Toolbar,
  ToolbarSpacer
} from "../components";
import type { Command, TabItem } from "../components";
import type { AiMessage, AiProviderId, AiSession, AiTarget, SessionQuickAccessItem } from "../types";
import * as api from "../ipc/commands";
import { ConfigOverlay } from "./config-overlay";
import { ExitConfirm } from "./exit-confirm";
import { displayTitle, formatWhen, messageCountLabel, pickPreview } from "./session-display";
import { useConsole } from "./use-console";
import { useTerminals } from "./use-terminals";
import { VendorRoutePicker } from "./vendor-route-picker";

/**
 * 终端**按需加载**。
 *
 * xterm 大约 340 KB，而它只在用户真的打开终端时才需要。静态引入会让每一个用户
 * 在首屏就为它付出代价——实测把主包从 235 KB 顶到 573 KB。
 */
const TerminalPane = lazy(() =>
  import("./terminal-pane").then((module) => ({ default: module.TerminalPane }))
);

/** 工作区的一个标签：会话或终端。 */
type WorkspaceTab = {
  key: string;
  kind: "session" | "terminal";
  title: string;
  /** `kind === "session"` 时是会话编号。 */
  sessionId?: string;
  /** `kind === "terminal"` 时是终端编号（同时用作 key）。 */
  terminalId?: string;
};

/**
 * 应用外壳。
 *
 * 布局沿用旧平台的形状：**菜单栏 /〔侧栏 360px + 工作区〕/ 状态栏**。
 * 取数顺序与竞态处理在 [`useConsole`]；终端在 [`useTerminals`] 与 [`TerminalPane`]。
 */
export function App() {
  const console = useConsole();
  const terminals = useTerminals();
  const { selectedTarget } = console;
  const trash = console.view === "trash";
  const selectedCount = console.selectedIds.length;

  const [tabs, setTabs] = useState<WorkspaceTab[]>([]);
  const [activeKey, setActiveKey] = useState("");
  /** 正在重命名的草稿；`null` 表示不在重命名。 */
  const [renameDraft, setRenameDraft] = useState<string | null>(null);
  /** 最近一次导出的实际落点，显示出来让用户知道东西去哪了。 */
  const [exportedTo, setExportedTo] = useState("");
  const [configOpen, setConfigOpen] = useState(false);
  const [configTab, setConfigTab] = useState<"vendors" | "skills" | "gateway" | "environment" | "diagnostics">("vendors");
  const [paletteOpen, setPaletteOpen] = useState(false);
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
     * `设置工作目录` / `继续打开` 形态。判断不出来（例如 WSL 目标上那条检查没接线）
     * 就保持 `undefined`：**不知道不等于丢了**。
     */
    resume?: { sessionId: string; missingCwd?: string };
  } | null>(null);
  /** 右键菜单的位置与目标会话。`null` 表示没打开。 */
  const [menu, setMenu] = useState<{ x: number; y: number; session: AiSession } | null>(null);

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

  /* 选中的会话自动成为当前标签。
   *
   * 由 `selectedId` 驱动而不是在每个点击处手写一次：列表里点击、搜索后跳转、
   * 批量操作后刷新——每条路径都会改 `selectedId`，只在一处响应才不会漏。 */
  useEffect(() => {
    if (!console.selectedSession) return;
    setTabs((current) => {
      if (current.some((tab) => tab.key === console.selectedSession!.id)) return current;
      return [
        ...current,
        {
          key: console.selectedSession!.id,
          kind: "session",
          title: displayTitle(console.selectedSession!),
          sessionId: console.selectedSession!.id
        }
      ];
    });
    setActiveKey(console.selectedSession.id);
  }, [console.selectedSession]);

  /**
   * 起一路 Codex 终端。
   *
   * `options` 是新会话对话框确认后传来的：**工作目录与 CLI 参数**。
   * 不传时沿用「当前选中会话的目录」，也就是旧行为。
   */
  const openTerminal = useCallback(
    async (options?: { cwd?: string; cliArgs?: string; prompt?: string; sessionId?: string }) => {
      try {
        const id = await terminals.openTerminal(
          selectedTarget,
          options?.cwd ?? console.selectedSession?.cwd,
          options?.cliArgs,
          options?.prompt,
          options?.sessionId
        );
        setTabs((current) => [
          ...current,
          { key: id, kind: "terminal", title: "终端", terminalId: id }
        ]);
        setActiveKey(id);
      } catch (error) {
        console.setError(error instanceof Error ? error.message : String(error));
      }
    },
    [terminals, selectedTarget, console]
  );

  /**
   * 打开「新会话」对话框。
   *
   * 旧平台的流程是**先在这个对话框里定下工作目录、会话标题、启动提示词与 CLI 参数，
   * 再打开**（`app.tsx:407` 的 `onCreate`）。此前这里是「点一下直接进终端」——
   * 用户报的 H2 说的就是这件事：「**默认不选择平台、目标**，选中后直接进入终端会话」。
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
      // 先把选中态切过去（列表高亮、标签），与从前一致。
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
      void openTerminal({ cwd: plan.cwd, sessionId: session.id });
    },
    [console, openTerminal]
  );

  const openSystemTerminal = useCallback(async () => {
    try {
      // 系统终端不依赖目标：没选目标时也能开一个本机 shell。
      const id = await terminals.openSystemTerminal(selectedTarget, console.selectedSession?.cwd);
      setTabs((current) => [
        ...current,
        { key: id, kind: "terminal", title: "系统终端", terminalId: id }
      ]);
      setActiveKey(id);
    } catch (error) {
      console.setError(error instanceof Error ? error.message : String(error));
    }
  }, [terminals, selectedTarget, console]);

  const closeTab = useCallback(
    (key: string) => {
      const closing = tabs.find((tab) => tab.key === key);
      setTabs((current) => {
        const next = current.filter((tab) => tab.key !== key);
        setActiveKey((currentKey) =>
          currentKey === key ? (next.at(-1)?.key ?? "") : currentKey
        );
        return next;
      });
      if (closing?.terminalId) void terminals.closeTerminal(closing.terminalId);
    },
    [tabs, terminals]
  );

  const tabItems: TabItem[] = useMemo(
    () =>
      tabs.map((tab) => {
        // 终端进程结束后必须在标签上看得出来。
        //
        // 这一条此前**永远不会出现**：`terminal:exit` 只有在我们主动关闭终端时才发得出来
        // ——ConPTY 下子进程自己退出不会让读取线程收到 EOF——所以 `running` 一直是 true，
        // 标签也就一直没有任何变化。用户敲了 `exit`，界面看起来跟还在运行一模一样。
        //
        // `running` 这个状态应用一直记着，却从来没有显示过：它此前只被用来「不在运行时
        // 跳过 resize」。记着而不显示，等于没记。
        const exited =
          tab.kind === "terminal" &&
          terminals.terminals.some((item) => item.id === tab.terminalId && !item.running);
        return {
          key: tab.key,
          label: exited ? `${tab.title}（已退出）` : tab.title,
          // 等待确认的终端在标签上打一个点。这正是「关注中心」要解决的问题：
          // 用户切到别的标签后，那一路还在等他，而界面上没有任何提示。
          tone: terminals.attention[tab.key] ? ("attention" as const) : ("normal" as const)
        };
      }),
    [tabs, terminals.attention, terminals.terminals]
  );

  const activeTab = tabs.find((tab) => tab.key === activeKey) ?? null;
  const sessionTabs = tabs.filter((tab) => tab.kind === "session");

  /**
   * 正在等待用户确认的终端。
   *
   * 标签上的小点只在**看得到那条标签时**才有用——而「程序在等你确认」恰恰常常发生在
   * 标签被滚出视野、或者用户在看另一个会话的时候。所以还需要一处汇总。
   */
  const waitingTerminals = terminals.terminals.filter((item) => terminals.attention[item.id]);

  /** 导出：后端决定落点，这里只负责把「去了哪」告诉用户。 */
  const runExport = useCallback(
    async (format: "markdown" | "json" | "html") => {
      const result = await console.exportCurrent(format);
      if (result) setExportedTo(result.filePath);
    },
    [console]
  );

  /**
   * 另存到…：先用**原生目录选择框**问用户放哪，再导出到那里。
   *
   * 与「一键导出」并存：默认落点让导出立即可用，指定目录让用户能放到自己想要的地方。
   */
  const runExportTo = useCallback(
    async (format: "markdown" | "json" | "html") => {
      const directory = await api.chooseDirectory("选择导出到的目录");
      // `null` 表示用户取消——取消是正常操作，什么都不做。
      if (!directory) return;
      const separator = directory.includes("\\") ? "\\" : "/";
      const extension = format === "markdown" ? "md" : format;
      const path = `${directory}${directory.endsWith(separator) ? "" : separator}session-${console.selectedId.slice(0, 8)}.${extension}`;
      const result = await console.exportCurrentTo(format, path);
      if (result) setExportedTo(result.filePath);
    },
    [console]
  );

  const openConfig = useCallback(
    (tab: "vendors" | "skills" | "gateway" | "environment" | "diagnostics") => {
      setConfigTab(tab);
      setConfigOpen(true);
    },
    []
  );

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
    [console, openTerminal, openSystemTerminal, runExport, openConfig]
  );

  return (
    <div className="shell">
      {/*
        菜单栏。「文件」「帮助」两个按钮**曾经没有 `onClick`**——点了没反应，是死按钮。
        死按钮比没有按钮更糟：它让用户以为自己点错了。

        这里**先移除**而不是假装接上：真实的「文件 / 视图 / 帮助」分组菜单需要按 `commands`
        的 group 生成下拉并管理开合与定位，是一件独立的事（也是 L16「命令面板复用菜单
        action」的前提）。在它做出来之前，菜单栏只留**确实能用的入口**。
      */}
      <div className="menubar">
        <span className="brand">AI 控制台</span>
        {/*
          分组菜单。此前这里只有「配置」与命令面板两个按钮——上面那段注释说明了
          「文件 / 帮助」曾经是**没有 onClick 的死按钮**，因此被移除；那个判断是对的。

          现在补上真正的分组菜单（结构在 `app-menus.ts`，数据与渲染分开、可单测）。
          **动作缺失的项一律显示为禁用**（见 `disabledWhenMissing`）：
          面还没实现的会显示成「这里有个东西、现在还不能用」，而不是点了没反应的死按钮。
        */}
        <AppMenuBar
          menus={createAppMenus(
            {
              supportsSkills: Boolean(selectedTarget),
              // 「设置会话」只对 WSL 目标可用。当前一律按「不是 WSL」处理：
              // 那个面（M5）还没实现，且 `openSessionSettings` 也没接，
              // 这一项无论如何都是禁用的。**不猜 `AiTarget` 上的字段名** ——
              // 等做 M5 时再按真实字段判定。
              isWslTarget: false,
              hasActiveSession: tabs.some((tab) => tab.kind === "session")
            },
            {
              openCommandPalette: () => setPaletteOpen(true),
              manageSkills: () => {
                setConfigTab("skills");
                setConfigOpen(true);
              },
              openGatewaySettings: () => {
                setConfigTab("gateway");
                setConfigOpen(true);
              },
              manageVendors: () => {
                setConfigTab("vendors");
                setConfigOpen(true);
              },
              installCli: () => {
                setConfigTab("environment");
                setConfigOpen(true);
              },
              exportSession: (format) => runExport(format),
              openSystemTerminal: () => void openSystemTerminal(),
              // 退出走浏览器窗口关闭，与标题栏的关闭同一条路，避免两套退出语义。
              quit: () => window.close()
              // 暂不接的：`openSessionSettings`（M5 未实现）、`showAbout`（M6 未实现）、
              // `openLogDirectory`、`clearGatewayLogs`（需要先确认日志目录与清空动作的
              // 现有入口在哪）。它们会以**禁用**呈现，而不是死按钮。
            }
          )}
        />
        <button type="button" onClick={() => setConfigOpen(true)}>
          配置
        </button>
        <ToolbarSpacer />
        <button type="button" onClick={() => setPaletteOpen(true)}>
          命令面板 ⌘K
        </button>
      </div>

      {paletteOpen && (
        <CommandPalette commands={commands} onClose={() => setPaletteOpen(false)} />
      )}

      {/*
        新会话对话框。旧平台的流程是**先定工作目录 / 标题 / 启动提示词 / CLI 参数，再打开**
        （`app.tsx:407` 的 `onCreate`）。此前这里是「点一下直接进终端」——
        用户报的 H2 说的就是这件事。

        `prompt`（启动提示词）暂时**不接**：它是渲染层的事，必须等会话可交互再写进 PTY，
        否则 ConPTY 还没准备好读输入时会丢字。那是单独一件事，见 docs/rust-rewrite/33 §5.5.1。
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

      {/* 右键菜单：铺一层透明背景接管「点别处关闭」。
          不用全局 click 监听——那样点菜单自身也会被当成「别处」。 */}
      {menu && (
        <div className="menu-backdrop" onClick={() => setMenu(null)} onContextMenu={(event) => event.preventDefault()}>
          <div className="context-menu" style={{ left: menu.x, top: menu.y }}>
            <button
              type="button"
              onClick={() => {
                console.setSelectedId(menu.session.id);
                void console.openCurrentFolder();
                setMenu(null);
              }}
            >
              打开所在目录
            </button>
            <button
              type="button"
              onClick={() => {
                void console.toggleFavorite(menu.session);
                setMenu(null);
              }}
            >
              {menu.session.metadata?.favorite ? "取消收藏" : "收藏"}
            </button>
            <button
              type="button"
              onClick={() => {
                // 复制**编号**而不是标题：编号是稳定的标识，粘到别处能用来定位；
                // 标题可能重复，也常常很长。
                void api.copyText(menu.session.id);
                setMenu(null);
              }}
            >
              复制会话编号
            </button>
            <button
              type="button"
              onClick={() => {
                void api.copyText(menu.session.filePath);
                setMenu(null);
              }}
            >
              复制文件路径
            </button>
            <button
              type="button"
              className="danger"
              onClick={() => {
                void console.removeSession(menu.session, trash);
                setMenu(null);
              }}
            >
              {trash ? "彻底删除" : "移入回收站"}
            </button>
          </div>
        </div>
      )}

      {/* 退出确认由**后端**发起（它拦得住窗口关闭），这里只负责显示与回答。 */}
      <ExitConfirm onError={console.setError} />

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
        <aside className="sidebar">
          <div className="sidebar-head">
            <div className="title">
              <span className={`status-dot${console.busy ? " off" : ""}`} />
              <h1>会话</h1>
            </div>
          </div>

          <div className="pickers">
            <Field label="平台">
              <Select
                value={console.providerId}
                onChange={(event) =>
                  console.selectProvider(event.target.value as AiProviderId | "")
                }
              >
                {console.providers.length === 0 && <option value="">加载中…</option>}
                {console.providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.label}
                  </option>
                ))}
              </Select>
            </Field>

            <Field label="目标">
              <Select
                value={console.targetId}
                disabled={console.targets.length === 0}
                onChange={(event) => console.selectTarget(event.target.value)}
              >
                {console.targets.length === 0 && <option value="">暂无可用目标</option>}
                {console.targets.map((target) => (
                  <option key={target.id} value={target.id}>
                    {/* 不可用的目标在选择列表里就标出来——等选中了才发现，用户已经先点了一次。 */}
                    {target.available ? target.label : `${target.label}（不可用）`}
                  </option>
                ))}
              </Select>
            </Field>

            {/* 目标状态说明。后端给出了四态 detail，界面必须显示：
                否则用户只看到「列表是空的」，分不清是**没有会话**还是**这个目标进不去**。 */}
            {selectedTarget && (
              <div className={`target-detail${selectedTarget.available ? "" : " bad"}`}>
                <span title={selectedTarget.detail ?? ""}>
                  {selectedTarget.detail ?? "（无状态说明）"}
                </span>
              </div>
            )}
          </div>

          {selectedTarget && !selectedTarget.available && (
            <div className="sidebar-notice">
              <Notice
                message={selectedTarget.detail || "此目标当前不可用。"}
                actions={
                  <>
                    <Button small onClick={() => void console.reloadTargets()}>
                      重新探测
                    </Button>
                    {selectedTarget.kind === "wsl" && selectedTarget.distro && (
                      <WslHomeForm
                        distro={selectedTarget.distro}
                        onSave={(home) =>
                          void console.configureWslCodexHome(selectedTarget.distro!, home)
                        }
                        onClear={() =>
                          void console.clearWslCodexHomeOverride(selectedTarget.distro!)
                        }
                      />
                    )}
                  </>
                }
              />
            </div>
          )}

          <div className="list-head">
            <Toolbar>
              {/* 一级导航。**包一层**是为了在 CSS 里和二级拉开轻重——两级用的是同一个
                  `ChipGroup`，不区分的话看上去完全一样，用户分不出哪层是哪层。 */}
              <div className="pane-nav">
                <ChipGroup
                  value={console.pane}
                  onChange={(value) => console.setPane(value as "sessions" | "quick" | "search")}
                  options={[
                    { value: "sessions", label: "会话" },
                    { value: "quick", label: "快捷" },
                    { value: "search", label: "搜索" }
                  ]}
                />
              </div>
              <ToolbarSpacer />
              <span className="row-meta">
                {console.pane === "sessions"
                  ? `${console.sessions.length} 条`
                  : console.pane === "quick"
                    ? `${console.quickAccess.favorites.length + console.quickAccess.recent.length} 条`
                    : `${console.globalResults.length} 条`}
              </span>
            </Toolbar>

            {console.pane === "sessions" && (
              <Toolbar>
                {/* 二级视图。比一级轻一档：它是**同一批内容的筛选**，不是另一处地方。 */}
                <div className="view-nav">
                  <ChipGroup
                    value={console.view}
                    onChange={(value) => console.setView(value === "trash" ? "trash" : "active")}
                    options={[
                      { value: "active", label: "活跃" },
                      { value: "trash", label: "回收站" }
                    ]}
                  />
                </div>
              </Toolbar>
            )}

            {console.pane === "sessions" && (
              <SearchInput
                value={console.query}
                onChange={console.setQuery}
                placeholder="搜索标题、编号、正文…"
                busy={console.searching}
              />
            )}

            {console.pane === "search" && (
              <>
                <SearchInput
                  value={console.globalQuery}
                  onChange={console.setGlobalQuery}
                  placeholder="在所有目标里搜索…"
                  busy={console.globalSearching}
                />
                {/* 范围收窄：不只是少显示结果，而是**少发请求**——
                    全量搜完再在前端过滤掉，那些请求已经付过代价了。 */}
                <Field label="搜索范围">
                  <Select
                    value={console.searchTargetId}
                    onChange={(event) => console.setSearchTargetId(event.target.value)}
                  >
                    <option value="">全部目标（{console.targets.length}）</option>
                    {console.targets.map((target) => (
                      <option key={target.id} value={target.id}>
                        {target.available ? target.label : `${target.label}（不可用）`}
                      </option>
                    ))}
                  </Select>
                </Field>
              </>
            )}
          </div>

          {console.pane === "sessions" && selectedCount > 0 && (
            <div className="batchbar">
              <span>已选 {selectedCount} 项</span>
              <ToolbarSpacer />
              <Button small onClick={console.toggleSelectAll}>
                全选
              </Button>
              {trash ? (
                <>
                  <Button small onClick={() => void console.restoreSelected()}>
                    还原
                  </Button>
                  <Button small variant="danger" onClick={() => void console.purgeSelected()}>
                    彻底删除
                  </Button>
                </>
              ) : (
                <Button small variant="danger" onClick={() => void console.deleteSelected()}>
                  移入回收站
                </Button>
              )}
            </div>
          )}

          <div className="rows">
            {console.pane === "sessions" &&
              console.sessions.map((session) => (
                <SessionRow
                  key={session.id}
                  session={session}
                  active={session.id === console.selectedId}
                  checked={console.selectedIds.includes(session.id)}
                  onOpen={() => console.setSelectedId(session.id)}
                  onToggle={() => console.toggleSelected(session.id)}
                  onFavorite={() => void console.toggleFavorite(session)}
                  onMenu={(x, y) => setMenu({ x, y, session })}
                />
              ))}

            {console.pane === "quick" && (
              <>
                <QuickGroup
                  title="收藏"
                  items={console.quickAccess.favorites}
                  empty="还没有收藏。在会话行上点 ☆ 收藏它。"
                  onOpen={beginResumeSession}
                />
                <QuickGroup
                  title="最近打开"
                  items={console.quickAccess.recent}
                  empty="还没有打开过会话。"
                  onOpen={beginResumeSession}
                />
              </>
            )}

            {console.pane === "search" &&
              console.globalResults.map((result) => (
                <div
                  key={`${result.target.id}:${result.session.id}`}
                  className="row"
                  onClick={() => void beginResumeSession(result.target, result.session)}
                  title={displayTitle(result.session)}
                >
                  <div className="row-line">
                    <span className="row-title">{displayTitle(result.session)}</span>
                  </div>
                  {/* 跨平台搜索必须标出**这条属于哪个目标**——否则同名会话分不清。 */}
                  <div className="row-meta">
                    <span>{result.target.label}</span>
                    
                  </div>
                </div>
              ))}

            {console.pane === "sessions" && console.sessions.length === 0 && (
              <EmptyState
                message={
                  console.query.trim()
                    ? "没有匹配的会话。"
                    : emptyMessage(console.targetId, selectedTarget, trash)
                }
              />
            )}
            {console.pane === "search" && console.globalResults.length === 0 && (
              <EmptyState
                message={
                  console.globalQuery.trim() ? "没有匹配的会话。" : "输入关键词，在所有目标里搜索。"
                }
                hint={
                  console.globalQuery.trim()
                    ? "后端会先匹配摘要，摘要不中再读整个文件——因此结果可能稍慢。"
                    : undefined
                }
              />
            )}
          </div>
        </aside>

        <section className="workspace">
          <Tabs
            items={tabItems}
            activeKey={activeKey}
            onActivate={(key) => {
              setActiveKey(key);
              // 看过就算处理过：清掉这一路的提醒标记，否则小点一直亮着，
              // 用户会以为还有事没处理。
              if (tabs.find((tab) => tab.key === key)?.kind === "terminal") {
                terminals.clearAttention(key);
              }
            }}
            onClose={closeTab}
            trailing={
              <>
                <Button
                  small
                  disabled={!selectedTarget}
                  onClick={beginNewSession}
                  title="在目标里启动 Codex"
                >
                  + Codex
                </Button>
                <Button
                  small
                  onClick={() => void openSystemTerminal()}
                  title="启动系统 shell（WSL 目标会开该发行版的 shell）"
                >
                  + Shell
                </Button>
                {activeTab?.kind === "session" && (
                  <Button small onClick={() => void console.reloadSessions()}>
                    刷新
                  </Button>
                )}
              </>
            }
          />

          <div className="workspace-body">
            {/* 关注中心：把「在等你的终端」汇总到一处，并且**一键切过去**。
                只看小点的话，用户还得自己在标签里找是哪一条。 */}
            {waitingTerminals.length > 0 && (
              <div className="attention-bar">
                <Notice
                  message={`${waitingTerminals.length} 个终端在等你确认。`}
                  actions={waitingTerminals.map((item) => (
                    <Button
                      key={item.id}
                      small
                      variant="primary"
                      onClick={() => {
                        setActiveKey(item.id);
                        // 切过去就算看过了，清掉标记——否则小点一直亮着。
                        terminals.clearAttention(item.id);
                      }}
                    >
                      {terminals.attention[item.id]?.title ?? "切过去"}
                    </Button>
                  ))}
                />
              </div>
            )}
            {/* 终端**全部保持挂载**，非前台的只是隐藏——卸载会丢掉 PTY 的输出。
                会话区只画当前那一个标签。 */}
            {terminals.terminals.length > 0 && (
              <div className={`terminal-stack${activeTab?.kind === "terminal" ? "" : " hidden"}`}>
                <Suspense fallback={<div className="terminal-loading">正在加载终端…</div>}>
                  {terminals.terminals.map((item) => (
                    <TerminalPane
                      key={item.id}
                      terminalId={item.id}
                      active={activeTab?.kind === "terminal" && activeTab.terminalId === item.id}
                      running={item.running}
                      registerSink={terminals.registerSink}
                      unregisterSink={terminals.unregisterSink}
                      onError={console.setError}
                    />
                  ))}
                </Suspense>
              </div>
            )}

            {activeTab?.kind === "terminal" ? null : sessionTabs.length === 0 ? (
              <EmptyState
                message="没有打开的会话。"
                hint="从左侧选择一个会话，或新建一个终端。"
                action={
                  <Button disabled={!selectedTarget} onClick={beginNewSession}>
                    新建终端
                  </Button>
                }
              />
            ) : (
              <>
                {/* 会话级操作。放在内容上方而不是右键菜单里：这些是**常用**动作，
                    藏进右键菜单会让它们难以发现。 */}
                <div className="session-head">
                  {renameDraft === null ? (
                    <h2 title={console.selectedSession?.title}>
                      {console.selectedSession ? displayTitle(console.selectedSession) : "未选择会话"}
                    </h2>
                  ) : (
                    <input
                      className="rename-input"
                      autoFocus
                      value={renameDraft}
                      onChange={(event) => setRenameDraft(event.target.value)}
                      onKeyDown={(event) => {
                        if (event.key === "Enter") {
                          void console.renameSession(renameDraft);
                          setRenameDraft(null);
                        }
                        if (event.key === "Escape") setRenameDraft(null);
                      }}
                      onBlur={() => setRenameDraft(null)}
                    />
                  )}
                  <span className="meta">
                    {/* 优先用**详情路径**带回来的精确总数；它还没到时回退到列表的条数，
                        并如实标注那是下界。 */}
                    {console.selectedSession
                      ? messageCountLabel(
                          console.messageTotal ?? console.selectedSession.messageCount,
                          console.messageTotal === null
                        )
                      : ""}
                  </span>
                  {/* 工作台要点：上下文占用、累计 token、模型。全都在会话自带的数据里，
                      此前界面上一个都没显示。 */}
                  {console.selectedSession?.usage && (
                    <span className="workbench">
                      {console.selectedSession.usage.contextPercent !== undefined && (
                        <span title="上下文占用">
                          上下文 <b>{console.selectedSession.usage.contextPercent}%</b>
                        </span>
                      )}
                      {console.selectedSession.usage.total?.totalTokens !== undefined && (
                        <span title="累计 token">
                          <b>{formatTokens(console.selectedSession.usage.total.totalTokens)}</b> token
                        </span>
                      )}
                      {console.selectedSession.model && (
                        <span title="模型">{console.selectedSession.model}</span>
                      )}
                    </span>
                  )}
                  <ToolbarSpacer />
                  <Button
                    small
                    disabled={!console.selectedSession}
                    onClick={() => setRenameDraft(console.selectedSession?.title ?? "")}
                  >
                    重命名
                  </Button>
                  <Button small disabled={!console.selectedSession} onClick={() => void console.duplicateCurrent()}>
                    复制
                  </Button>
                  <Button small disabled={!console.selectedSession} onClick={() => void runExport("markdown")}>
                    导出 MD
                  </Button>
                  <Button small disabled={!console.selectedSession} onClick={() => void runExport("json")}>
                    导出 JSON
                  </Button>
                  <Button small disabled={!console.selectedSession} onClick={() => void runExportTo("markdown")}>
                    另存到…
                  </Button>
                </div>

                {exportedTo && (
                  <div className="export-hint">
                    已导出到 <code>{exportedTo}</code>
                    <Button small onClick={() => void api.openPath(exportedTo)}>
                      打开
                    </Button>
                  </div>
                )}

                {/* 分支关系。放在正文**上方**、会话头下方：
                    它讲的是「这条会话从哪来、分出去了哪些」，属于这条会话的元信息，
                    而不是正文的一部分。没有分支时它自己整块不渲染。 */}
                {console.selectedSession && (
                  <BranchPanel
                    session={console.selectedSession}
                    resolveParent={(sessionId) =>
                      console.sessions.find((item) => item.id === sessionId)
                    }
                    listChildren={(parentSessionId) =>
                      api.listSessionChildren(parentSessionId, selectedTarget?.id)
                    }
                    onOpen={(session) => {
                      const target =
                        console.targets.find((item) => item.id === session.metadata?.branch?.parentTargetId) ??
                        selectedTarget;
                      if (target) void beginResumeSession(target, session);
                    }}
                  />
                )}

                <div className="thread">
                  {/* 往前翻的入口放在**顶部**：更早的消息在视觉上就在上面。 */}
                  {console.hasEarlier && (
                    <div className="load-earlier">
                      <Button
                        small
                        disabled={console.loadingMore}
                        onClick={() => void console.loadEarlierMessages()}
                      >
                        {console.loadingMore ? "加载中…" : "加载更早的消息"}
                      </Button>
                    </div>
                  )}

                  {console.messages.map((message, index) => (
                    <Message
                      key={`${message.sourceLine ?? index}`}
                      message={message}
                      onBranch={
                        // 只有带原始行号的消息才能分支——后端要靠行号定位到原始记录，
                        // 拿可见序号去截会截错位置。
                        message.sourceLine === undefined
                          ? undefined
                          : () => void console.branchFromMessage(message.sourceLine!)
                      }
                    />
                  ))}

                  {console.messages.length === 0 && (
                    <EmptyState
                      message={
                        console.selectedId ? "这个会话没有可显示的消息。" : "从左侧选择一个会话。"
                      }
                      hint={
                        console.selectedId
                          ? "文件可能是空的，或消息都被解析规则过滤掉了。"
                          : undefined
                      }
                    />
                  )}
                </div>
              </>
            )}
          </div>
        </section>
      </div>

      <div className="statusbar">
        <span>{console.busy ? "处理中…" : console.searching ? "搜索中…" : "就绪"}</span>
        {/* 会话上下文：**现在在哪**。
            只有计数（目标/会话/终端）回答不了这个问题——用户切了几个会话之后，
            最常问的是「我现在看的是哪一个、在哪个目录」。L7 要的就是这一组。 */}
        <span className="status-session">
          会话{" "}
          <code title={console.selectedSession?.id ?? undefined}>
            {console.selectedSession?.id ?? "-"}
          </code>
        </span>
        <span className="status-cwd" title={console.selectedSession?.cwd ?? undefined}>
          目录 <b>{console.selectedSession?.cwd ?? "-"}</b>
        </span>
        <span className="status-updated">
          更新 {formatWhen(console.selectedSession?.updatedAt) || "-"}
        </span>
        <span className="status-target">
          目标 <b>{console.targets.length}</b>
        </span>
        <span className="status-sessions">
          会话 <b>{console.sessions.length}</b>
        </span>
        <span className="status-terminals">
          终端 <b>{terminals.terminals.length}</b>
        </span>
        {/* 当前终端由谁服务。**必须常驻可见**：走网关之后供应商可能被自动换掉，
            用户没有做任何操作，界面不显示就会在账单上才发现。 */}
        <VendorRoutePicker
          terminal={terminals.terminals.find((item) => item.id === activeTab?.terminalId)}
          onSwitch={terminals.switchVendor}
          onSetMode={terminals.setRouteMode}
        />
        <ToolbarSpacer />
        {console.error && <span className="err">{console.error}</span>}
        <span>
          网关{" "}
          <b>
            {console.gateway?.enabled
              ? `:${console.gateway.activePort || console.gateway.configuredPort}`
              : "未启用"}
          </b>
        </span>
      </div>
    </div>
  );
}

/**
 * 快捷访问里的一组（收藏 / 最近）。
 *
 * 每项都显示**它属于哪个目标**——快捷访问是跨目标的，不标出来就分不清同名会话。
 */
function QuickGroup({
  title,
  items,
  empty,
  onOpen
}: {
  title: string;
  items: SessionQuickAccessItem[];
  empty: string;
  onOpen: (target: AiTarget, session: AiSession) => void;
}) {
  return (
    /*
     * 每组**各自封顶、独立滚动**（L3）。
     *
     * 不包这一层时，收藏与最近两组共用外层的**同一个滚动区**：翻收藏的时候最近区会
     * 跟着一起跑，用户没法在某一组里连续浏览。旧平台是每组单独封顶的。
     */
    <div className="quick-group">
      <div className="group-title">{title}</div>
      {items.length === 0 && <div className="group-empty">{empty}</div>}
      {items.map((item) => (
        <div
          key={`${item.target.id}:${item.session.id}`}
          className="row"
          onClick={() => onOpen(item.target, item.session)}
          title={displayTitle(item.session)}
        >
          <div className="row-line">
            <span className="row-title">
              {item.favorite ? "★ " : ""}
              {displayTitle(item.session)}
            </span>
          </div>
          <div className="row-meta">
            <span>{item.target.label}</span>
            <span>{formatWhen(item.lastOpenedAt ?? item.session.updatedAt)}</span>
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * 手动指定 WSL 内的 Codex 目录。
 *
 * # 为什么它必须出现在「目标不可用」的告警里
 *
 * 发行版进不去时，自动探测**注定失败**；而用户往往知道自己的 Codex 装在哪。
 * 这条路径此前只藏在菜单里（甚至压根没有入口），于是「目标不可用」变成了死路。
 *
 * 保存后必须**重新探测**——否则界面还停在旧结果上，用户会以为没生效。
 */
function WslHomeForm({
  distro,
  onSave,
  onClear
}: {
  distro: string;
  onSave: (codexHome: string) => void;
  onClear: () => void;
}) {
  const [draft, setDraft] = useState("");
  return (
    <div className="wsl-home">
      <TextInput
        value={draft}
        placeholder={`${distro} 内的路径，例如 /home/你/.codex`}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && draft.trim()) onSave(draft.trim());
        }}
      />
      <div className="wsl-home-actions">
        <Button small disabled={!draft.trim()} onClick={() => onSave(draft.trim())}>
          指定并重新探测
        </Button>
        <Button small onClick={onClear} title="清除人工指定，回到自动探测">
          清除
        </Button>
      </div>
    </div>
  );
}

function SessionRow({
  session,
  active,
  checked,
  onOpen,
  onToggle,
  onFavorite,
  onMenu
}: {
  session: AiSession;
  active: boolean;
  checked: boolean;
  onOpen: () => void;
  onToggle: () => void;
  onFavorite: () => void;
  onMenu: (x: number, y: number) => void;
}) {
  const preview = pickPreview(session);
  const favorite = session.metadata?.favorite === true;
  // 标题退化成了编号时显示「目录 · 时间」——列表要能区分会话，UUID 不能。
  const title = displayTitle(session);
  return (
    <div
      className={`row${active ? " on" : ""}`}
      onClick={onOpen}
      title={title}
      onContextMenu={(event) => {
        event.preventDefault();
        onMenu(event.clientX, event.clientY);
      }}
    >
      <div className="row-line">
        <input
          type="checkbox"
          checked={checked}
          aria-label="选择会话"
          // 勾选不该顺带把会话也打开——那是两件事。
          onClick={(event) => event.stopPropagation()}
          onChange={onToggle}
        />
        <span className="row-title">{title}</span>
        <button
          type="button"
          className={`star${favorite ? " on" : ""}`}
          aria-label={favorite ? "取消收藏" : "收藏"}
          onClick={(event) => {
            event.stopPropagation();
            onFavorite();
          }}
        >
          {favorite ? "★" : "☆"}
        </button>
      </div>
      {preview && <div className="row-preview">{preview}</div>}
      <div className="row-meta">
        <span>{formatWhen(session.updatedAt ?? session.createdAt)}</span>
        
        {session.model && <span>{session.model}</span>}
      </div>
    </div>
  );
}

function Message({ message, onBranch }: { message: AiMessage; onBranch?: () => void }) {
  const who =
    message.role === "user"
      ? "你"
      : message.role === "assistant"
        ? "助手"
        : message.role === "tool"
          ? "工具"
          : message.role === "system"
            ? "系统"
            : "记录";
  return (
    <div className={`message ${message.role}`}>
      <div className="who">
        <span>{who}</span>
        {onBranch && (
          <button type="button" className="branch-action" onClick={onBranch}>
            从此处分支
          </button>
        )}
      </div>
      <div className="text">{message.text}</div>
    </div>
  );
}

/**
 * 会话区为空时**必须说清是哪一种空**。
 *
 * 几种完全不同的处境此前长得一模一样，而用户需要做不同的事。
 */
function emptyMessage(targetId: string, target: AiTarget | null, trash: boolean): string {
  if (!targetId) return "请先在上面选择目标。";
  if (target && !target.available) return target.detail || "此目标当前不可用。";
  return trash ? "回收站为空。" : "未找到会话。";
}

/** 把 token 数显示成 `12.4k` 这样的紧凑形式——六位数在窄条里读不出来。 */
function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return String(value);
}
