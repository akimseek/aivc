import { useCallback, useEffect, useMemo, useState } from "react";

import { useToast } from "../components";
import { SessionsSidebar } from "../features/sessions/sessions-sidebar";
import type { SettingsTab } from "../features/settings/settings-center";
import { WorkbenchSidebar } from "../features/workbench/workbench-sidebar";
import * as api from "../ipc/commands";
import { ActivityBar } from "../shell/activity-bar";
import { AppShell } from "../shell/app-shell";
import { StatusBar } from "../shell/status-bar";
import { TitleBar } from "../shell/title-bar";
import type { AiSession, AiTarget, ApiVendor } from "../types";
import { AppOverlays } from "./overlay-layer";
import { supportsBranch, supportsDuplicate } from "./provider-capabilities";
import { planResume } from "./resume-session";
import { SessionWorkspace, canStartCodex } from "./session-workspace";
import { displayTitle } from "./session-display";
import { resolveStageContent } from "./stage-content";
import { nextPreference } from "./theme";
import { useConsole } from "./use-console";
import { useTerminals } from "./use-terminals";
import { useTheme } from "./use-theme";
import { useWorkspaceTabs } from "./use-workspace-tabs";

/**
 * 应用外壳。
 *
 * 布局是**标题栏 /〔活动栏 + 侧栏 + 工作区〕/ 状态栏**（spec §6）。
 *
 * # 这个文件只负责「接线」
 *
 * 它此前有 1460 行，其中绝大部分是**可以直接搬走的**：侧栏 260 行、
 * 工作区正文 240 行、状态栏 100 行、右键菜单 55 行、标签逻辑 120 行。
 * 现在它们分别在：
 *
 * | 内容 | 位置 |
 * | --- | --- |
 * | 骨架网格 | [`AppShell`](../shell/app-shell.tsx) |
 * | 标题栏（品牌 / 主题 / 铃铛） | [`TitleBar`](../shell/title-bar.tsx) |
 * | 活动栏（会话 / 工作台 / 设置） | [`ActivityBar`](../shell/activity-bar.tsx) |
 * | 状态栏 | [`StatusBar`](../shell/status-bar.tsx) |
 * | 侧栏（网关胶囊 / 筛选 chips / 会话卡片） | [`SessionsSidebar`](../features/sessions/sessions-sidebar.tsx) |
 * | 工作区（标签 + 终端栈 + 会话正文） | [`SessionWorkspace`](./session-workspace.tsx) |
 * | 右键菜单 | [`SessionContextMenu`](./session-context-menu.tsx) |
 * | 标签的增删与竞态 | [`useWorkspaceTabs`](./use-workspace-tabs.ts) |
 *
 * 留在这里的只有三类东西：**状态**（选中的标签、打开的弹层）、
 * **动作**（开终端、导出、改配置）、**组装**。
 */
export function App() {
  // 主题：把偏好写到 <html data-theme>，所有 token 随之整套切换。
  // 偏好与解析结果都要给标题栏（按钮上写的字与图标分别来自两者）。
  const theme = useTheme();

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
   * - `SettingsCenter` 拿到 `onNotify` 往下传给各面板（供应商 / 技能 / 环境 / 网关）。
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
   * 画布当前该显示什么：**终端优先**，终端不在了才回落到历史，都没有就是空。
   *
   * 判据是纯函数（`stage-content.ts`，逐条有测试）：它是「切画布」唯一的判据，
   * 错了整块区域就是错的——所以不在渲染里各写一遍。
   */
  const stage = resolveStageContent(workspace.activeTab, (id) =>
    terminals.terminals.some((item) => item.id === id)
  );

  /**
   * 标签条要的项目。
   *
   * 统一模型（Task 3/4）已经把界面需要的都算好了：`suffix`（退出码）、`tone`
   * （等待确认）、`providerId`（平台徽章）、`state`（状态点）。这里**直接透传**，
   * 不在渲染层再拼一遍——拼一次就多一份会漂移的规则。
   */
  const tabItems = workspace.tabItems;

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

  const [settingsOpen, setSettingsOpen] = useState(false);
  const [settingsTab, setSettingsTab] = useState<SettingsTab>("vendors");
  const [menu, setMenu] = useState<{ x: number; y: number; sessionId: string } | null>(null);

  /**
   * 应用版本。
   *
   * **只取一次**（挂载时）：版本是进程级常量，每个用到它的地方各取一次只是多几次
   * IPC，还会让「读不到」的兜底分散在几处。读不到时写死一句「无法读取」——
   * 界面上一片空白会被读成「这个产品没有版本号」。
   */
  const [appVersion, setAppVersion] = useState("");
  useEffect(() => {
    void api
      .getAppVersion()
      .then(setAppVersion)
      .catch(() => setAppVersion("无法读取"));
  }, []);

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
   * 「查找」计数：标签栏那个按钮把它加一。
   *
   * app 层**只转发这个计数**：真正的查找栏在 `TerminalPane` 里（只有那一层拿得到
   * xterm 的缓冲区，`readLines` 与滚动选中都在那里）。计数变化即「打开」，
   * 不需要事件总线——两个入口（按钮与 Ctrl+F）落在同一个组件里。
   */
  const [findSignal, setFindSignal] = useState(0);

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

  const openSettings = useCallback((tab: SettingsTab) => {
    setSettingsTab(tab);
    setSettingsOpen(true);
  }, []);

  /**
   * 打开「设置会话」对话框（WSL 的 CODEX_HOME 覆盖）。
   *
   * 两个入口共用这一处：菜单栏（Task 6 删除前）与环境面板的「WSL 会话目录」卡。
   * 非 WSL 目标没有可写的东西，直接返回，而不是弹一个填不了任何东西的对话框。
   */
  const openSessionSettings = useCallback(() => {
    if (selectedTarget?.kind !== "wsl") return;
    setSessionSettings({
      distro: selectedTarget.distro ?? "",
      path: selectedTarget.codexHome ?? ""
    });
  }, [selectedTarget]);

  /**
   * 窗口动作的统一收尾。
   *
   * 四条窗口命令都**不重试、不兜底**：窗口没最小化就是没最小化，假装成功只会让用户
   * 以为界面坏了。失败如实报在状态栏（`console.setError`）。
   */
  const runWindowAction = useCallback(
    (action: () => Promise<null>) => {
      void action().catch((error: unknown) =>
        console.setError(error instanceof Error ? error.message : String(error))
      );
    },
    // 依赖 `console.setError` 而不是整个 `console`：`useConsole()` 每次渲染都返回**新对象**，
    // 依赖它等于这里每次渲染都重建，`quit` / 四个窗口回调跟着一起失效（记忆化白写）。
    // `setError` 来自 `useState`，identity 恒定。
    [console.setError]
  );

  /**
   * 退出与打开日志目录。
   *
   * 各有两个入口（标题栏 / 关于面板），因此各定义一次：
   * - 退出走 `app:window-close`（与标题栏的关闭按钮**同一条命令**）——**避免两套退出
   *   语义**，后端在 `CloseRequested` 上仍然拦得住它并弹退出确认；
   * - 打开日志目录交给后端按运行根算路径（`app:command` 的 `openLogDir`），
   *   界面不需要知道它在哪。
   */
  const quit = useCallback(() => runWindowAction(api.windowClose), [runWindowAction]);
  const openLogDirectory = useCallback(() => void api.appCommand("openLogDir"), []);

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
   * **选中会话跟随当前标签。**
   *
   * 这是旧平台的规则（`app.tsx` 里那个依赖 `targetId` 的 effect）：
   *
   * ```js
   * const activeSessionForTarget = activeTab?.targetId === targetId ? activeTab.session : null;
   * setSelectedId(activeSessionForTarget?.id || "");
   * ```
   *
   * 也就是**跟着当前标签那条会话走；标签没有会话、或它不在这份列表里时就是不选**。
   * 从来不选「列表第一项」。
   *
   * 这条规则是「在终端里还能看到详情/从此处分支」的另一半：详情按钮在标签栏上、
   * 绑的是当前标签的会话，而选中项也要跟着走，否则右侧正文显示的是另一条会话。
   *
   * 统一模型下来判据不再分「终端标签 / 会话标签」：**激活任一挂着会话的标签**
   * 就把它设为选中项——切标签时左侧列表的高亮跟着走，靠的就是这一处；
   * 全新会话的标签还没有编号（`sessionId` 缺席），那时不动选中项，因为
   * 它对应的会话还不存在。**别在点击处各写一次。**
   */
  useEffect(() => {
    const sessionId = workspace.activeTab?.sessionId;
    if (!sessionId) return;
    console.setSelectedId(sessionId);
  }, [workspace.activeTab?.sessionId, console.setSelectedId]);
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
        workspace.attachTerminalTab(id, options?.sessionId, options?.title ?? "新会话");
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
   * **恢复一条历史会话并进入终端**（旧平台 `openResumeWithDirectory`）。
   *
   * 这是**显式动作**（标签栏的「继续会话」、右键菜单与详情里的分支跳转），
   * 不再由「点一下会话行」触发——单击只打开标签看历史。
   *
   * 不必再事先清掉这条会话上已经死掉的终端标签：`attachTerminal` 会把新终端挂到
   * 那张标签上并回报被替换的旧终端，由 `attachTerminalTab` 把它 `closeTerminal`
   * 收掉（这就是「再次进入被锁住」的修法）。
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
      /*
        先把选中态切过去（列表高亮、标签）。

        **标签要显式切**，不能只靠 `openAt` 写选中编号——理由与 `openSessionById`
        那句注释一样：编号没变时（这条会话本来就是选中项，而活动标签是另一路终端）
        标签的 effect 不会重跑，「原目录丢失」这一支尤其明显——它会弹出一个对话框，
        而对话框背后的画布还停在**别的**会话上，用户填工作目录时看不出这是给谁填的。
      */
      console.openAt(target, session);
      workspace.selectSession(session);

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
    [console, openTerminal, workspace.selectSession]
  );

  /**
   * 两点共用：按编号还原出完整会话（侧栏只回传编号，它不持有会话内容）。
   *
   * 「打开标签」与「继续会话」共用这一处判定，所以「会话已经不在列表里」
   * 这句提示只有一份——两处各写一遍迟早会不一致。
   */
  const sessionForAction = useCallback(
    (sessionId: string): AiSession | null => {
      const session = sessionById(sessionId) ?? console.selectedSession;
      if (!session || session.id !== sessionId) {
        // 会话可能刚被别处删掉。给一句明确的说明，而不是静默无反应。
        toast.show("该会话已不在当前列表里，请刷新后重试。");
        return null;
      }
      return session;
    },
    [sessionById, console.selectedSession, toast]
  );

  /** 继续会话动作用的入口（标签栏的「继续会话」）。 */
  const resumeById = useCallback(
    (target: AiTarget, sessionId: string) => {
      const session = sessionForAction(sessionId);
      if (session) void beginResumeSession(target, session);
    },
    [sessionForAction, beginResumeSession]
  );

  /**
   * 侧栏点会话行：**只打开/激活它的标签**，不再顺带起终端。
   *
   * 本批次的一处行为变化：单击的意图多半是「看一下这条会话」，而起一路 CLI
   * 进程既慢又意外；恢复终端改成显式动作（标签栏的「继续会话」）。
   *
   * 这里**显式**激活一次（`selectSession`），不再只依赖「选中项编号变了」那个
   * effect：点一条**本来就被选中**的会话（活动标签是一路没有会话编号的新会话
   * 终端）时，编号没变、effect 不跑，只写选中态的话标签不会被切过来——点击
   * 看起来毫无反应。effect 仍然保留，供列表刷新、搜索跳转等其它路径。
   */
  const openSessionById = useCallback(
    (target: AiTarget, sessionId: string) => {
      const session = sessionForAction(sessionId);
      if (!session) return;
      console.openAt(target, session);
      workspace.selectSession(session);
    },
    [sessionForAction, console.openAt, workspace.selectSession]
  );

  /**
   * 「继续会话」这一刻有没有意义。
   *
   * 两个条件都要：标签挂着**会话编号**（全新会话还没有编号，无从恢复），
   * 而且这张标签上的终端**没在跑**——正在跑时再点等于白起一路、再把它替换掉。
   * 异常退出或被关掉的终端（`running` 为假）仍然可恢复，这正是「退出后被锁住」
   * 那条路径的出口。
   */
  const canResumeSession = Boolean(workspace.activeTab?.sessionId) && !activeTerminal?.running;

  /** 「继续会话」：在当前标签的会话上再起一路终端（带 `--resume`）。 */
  const resumeActiveSession = useCallback(() => {
    const sessionId = workspace.activeTab?.sessionId;
    if (selectedTarget && sessionId) resumeById(selectedTarget, sessionId);
  }, [workspace.activeTab?.sessionId, selectedTarget, resumeById]);

  /* ── 导出 ─────────────────────────────────────────────── */

  /**
   * 导出：后端决定落点，这里只负责把「去了哪」告诉用户。
   *
   * `session` 是**明确要导出的那一条**（会话右键菜单）。不传时导出当前选中项——
   * 菜单栏那个入口就是这样。带明确编号的那条路不能省：右键菜单在同一个事件里
   * 先调 `setSelectedId` 再导出，`setState` 要等下一次渲染才生效，只读选中值会
   * 导出**上一条**（什么都没选时则静默不导，那是最糟的一种「点了没反应」）。
   */
  const runExport = useCallback(
    async (format: "markdown" | "json" | "html", session?: AiSession) => {
      const result = await console.exportCurrent(format, session?.id);
      if (result) setExportedTo(result.filePath);
    },
    [console]
  );

  /* ── 组装 ─────────────────────────────────────────────── */

  const trash = console.view === "trash";

  /**
   * 当前标签那条会话。
   *
   * **当前标签优先**——这正是「在终端里还能查看详情」的关键：那时正文不渲染
   * （原来详情按钮就在正文头部，于是消失了），而选中项也未必等于这张标签所属的
   * 会话。旧平台同理：它绑的是 `activeSession`（活动标签的会话）。
   */
  const activeSession =
    sessionById(workspace.activeTab?.sessionId ?? "") ?? console.selectedSession;

  /**
   * **终端信息条**要用的会话：活动标签自己挂着的那条。
   *
   * 判据是**这张标签自己有没有会话编号**：全新会话的标签还没有编号（`sessionId` 缺席），
   * 此刻传给画布的必须是 `null`——信息条于是退回显示这一路终端自己的标题与编号。
   *
   * 这里**不能**直接用 `activeSession`（它带「找不到就回退到选中项」）：信息条讲的是
   * **这一路终端属于谁**，读选中项会让它顶到另一条会话的标题与编号上（T9 要修的场景）。
   *
   * 历史视图**不**吃这个值——它吃 `console.selectedSession`，与消息列表同源：
   * 两者一旦分叉（右键菜单的导出 / 打开目录 / 重命名 / 复制只改选中项、不动标签），
   * 历史视图就会出现「S 的标题 + R 的消息」。画布该跟标签还是跟选中，是更上一层
   * 的设计问题（批次二再议）。
   */
  const tabSession = workspace.activeTab?.sessionId ? activeSession : null;

  /**
   * 「消息与分支」：打开这条会话的详情（消息列表 + 分支关系都在那里）。
   *
   * 没有会话可看时传 `undefined`——按钮显示为禁用，而不是「点得动但什么都不发生」。
   */
  const openMessages = activeSession ? () => setDetailSession(activeSession) : undefined;

  /**
   * 「消息与分支」上的条数。
   *
   * 优先用**详情路径带回来的精确总数**（`console.messageTotal`）；它还没到或没取过时
   * 退回列表里的条数。两个都没有就传 `null`——那时不渲染徽章，不编数字。
   */
  const messageCount =
    console.messageTotal ?? console.selectedSession?.messageCount ?? null;

  /**
   * 输入模式与输入框可见性都在 `useTerminals` 里（状态栏也要读它）。
   * 状态栏讲的是**前台那一路**，因此先按活动标签的终端编号取。
   */
  const activeTerminalId = workspace.activeTab?.terminalId;
  const activeUi = terminals.uiStates[activeTerminalId ?? ""];

  /**
   * 活动栏当前亮着哪一个。
   *
   * **不另存一份状态**：它与侧栏说的是同一件事（`console.pane`）。存两份的话，
   * 用侧栏里的「工作台 / 会话」切过之后活动栏就会亮错——同一件事只有一处能是真相。
   */
  const activity: "sessions" | "workbench" =
    console.pane === "workbench" ? "workbench" : "sessions";

  /**
   * 有终端在跑的会话编号：卡片左上角那个点按它亮起来。
   *
   * **由标签反查**：终端自己不记会话编号，但一张标签同时挂着会话与终端
   * （统一标签模型），两边一对就有了。
   */
  const runningSessionIds = useMemo(
    () =>
      new Set(
        workspace.tabs
          .filter(
            (tab) =>
              tab.sessionId &&
              terminals.terminals.some((item) => item.id === tab.terminalId && item.running)
          )
          .map((tab) => tab.sessionId!)
      ),
    [workspace.tabs, terminals.terminals]
  );

  return (
    <>
      <AppShell
        titleBar={
          <TitleBar
            version={appVersion}
            preference={theme.preference}
            resolved={theme.resolved}
            onCycleTheme={() => theme.setPreference(nextPreference(theme.preference))}
            attention={terminals.terminals
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
              // 标签的 key 在统一模型里**优先是会话编号**，所以要按 terminalId 反查
              // 标签——直接拿 terminalId 当 key 会切到一张不存在的标签上。
              const tab = workspace.tabs.find((item) => item.terminalId === terminalId);
              workspace.setActiveKey(tab?.key ?? terminalId);
              // 切过去就算看过了，清掉标记——否则铃铛上的数字一直不减。
              terminals.clearAttention(terminalId);
            }}
            onDismissAttention={(terminalId) => terminals.clearAttention(terminalId)}
            // 无边框窗口的窗口控制：系统框已经没有了，这四个动作是唯一的入口。
            // 关闭与「关于面板的退出」共用同一个回调——两套退出语义是不行的。
            onMinimize={() => runWindowAction(api.windowMinimize)}
            onToggleMaximize={() => runWindowAction(api.windowToggleMaximize)}
            onStartDrag={() => runWindowAction(api.windowStartDrag)}
            onClose={quit}
          />
        }
        activityBar={
          <ActivityBar
            activity={activity}
            /*
              活动栏切的是**整块侧栏**（工作台 / 会话两种形态），因此直接改 `pane`。
              折叠按钮在侧栏头里，折起来之后侧栏整个消失——**活动栏是唯一能把它叫回来的
              地方**，所以点这里顺带展开（否则那一下会像「点了没反应」）。
            */
            onActivity={(next) => {
              console.setPane(next);
              setSidebarCollapsed(false);
            }}
            // 设置默认开供应商页：它是这个入口最常被用来找的东西（spec §11）。
            onOpenSettings={() => openSettings("vendors")}
          />
        }
        sidebar={
          console.pane === "workbench" ? (
            sidebarCollapsed ? (
              /* 折叠时节点仍要渲染、而且必须在流内（0 宽的网格项，占住第 2 列）：
                 网格里 `display: none` 的项不参与放置，主区会被自动放进第 2 列
                 （`auto`），第 3 列的 `1fr` 空着——右侧留一条空白。
                 机制在 `.sidebar.hidden`（宽度归零），见 `app.css`。 */
              <aside className="sidebar hidden" aria-label="侧栏（已折叠）" />
            ) : (
              /* 工作台**整块替换**会话侧栏——活动栏切的是「哪一处地方」。
                 `WorkbenchSidebar` 自带 `<aside className="sidebar workbench-pane">`
                 与滚动容器（R24），这里不再包一层 <aside>。
                 类名不是 `workbench`——那是会话头用量胶囊的类，理由见组件头部注释。 */
              <WorkbenchSidebar
                provider={console.providers.find((item) => item.id === console.providerId)}
                target={selectedTarget}
                session={console.selectedSession}
                gateway={console.gateway}
                vendors={sidebarVendors}
                lastSwitch={lastRouteSwitch}
                refreshToken={workbenchToken}
                // 关闭 = 回会话：与活动栏的 onActivity 同一个语义（含把侧栏叫回来）。
                onClose={() => {
                  console.setPane("sessions");
                  setSidebarCollapsed(false);
                }}
                onOpenUsage={() => openSettings("usage")}
                onOpenFailures={() => openSettings("failures")}
              />
            )
          ) : (
            <SessionsSidebar
              console={console}
              selectedTarget={selectedTarget}
              onOpen={openSessionById}
              onResume={resumeById}
              onMenu={(x, y, sessionId) => setMenu({ x, y, sessionId })}
              /*
                「新建会话」与工作区那个 +Codex 同一个入口；
                「消息与分支」打开会话详情——两者都在侧栏里，动作本身留在这一层。
              */
              onNewSession={beginNewSession}
              onDetail={(session) => setDetailSession(session)}
              // 网关胶囊：设置中心的网关页（端口、启停、健康都在那里）。
              onOpenGateway={() => openSettings("gateway")}
              providers={console.providers}
              gateway={console.gateway}
              runningSessionIds={runningSessionIds}
              collapsed={sidebarCollapsed}
              onToggleCollapsed={() => setSidebarCollapsed((value) => !value)}
            />
          )
        }
        main={
          /*
            工作区外面这一层包裹是必须的：`.app-main` 是内容列，`.workarea`
            让「标签 + 终端栈 + 会话正文」这一整块吃掉列高（`flex: 1`），
            停靠面板若直接挂到 `.app-shell` 上会落到状态栏**下面**。
          */
          <div className="workarea">
            <SessionWorkspace
              tabItems={tabItems}
              activeKey={workspace.activeKey}
              stage={stage}
              terminalItems={terminals.terminals}
              // 输入框可见性在 `useTerminals` 里：这里只是把它按编号透传下去。
              // （输入模式不再往下传：切换入口在状态栏，输入区已经没有那个按钮。）
              uiStates={terminals.uiStates}
              onOutput={terminals.noteOutput}
              registerSink={terminals.registerSink}
              unregisterSink={terminals.unregisterSink}
              onError={console.setError}
              onActivate={(key) => {
                workspace.setActiveKey(key);
                // 看过就算处理过：清掉这一路的提醒标记，否则小点一直亮着，
                // 用户会以为还有事没处理。提醒是按**终端编号**记的，标签 key 未必是它。
                const terminalId = workspace.tabs.find((tab) => tab.key === key)?.terminalId;
                if (terminalId) terminals.clearAttention(terminalId);
              }}
              onClose={workspace.closeTab}
              onTabContextMenu={(key, x, y) => setTabMenu({ key, x, y })}
              // 新建会话走对话框（不是「点一下直接进终端」），入口在 `beginNewSession`。
              onNewSession={beginNewSession}
              /*
                查找：只转发计数，真正的查找栏在 `TerminalPane` 里（见上面的
                `findSignal`）。画布不是终端时没有可查找的缓冲区，因此按钮禁用。
              */
              onFind={
                stage.kind === "terminal"
                  ? () => setFindSignal((value) => value + 1)
                  : undefined
              }
              onOpenMessages={openMessages}
              // `＋` 的可用性：目标进得去才有意义（与侧栏那个入口同一条判据）。
              canCreate={canStartCodex(selectedTarget)}
              messageCount={messageCount}
              /*
                平台徽章上的名字来自后端（`ai:list-providers` 的 `label`）：
                界面不自己维护一份平台名表——它会与后端漂移。
              */
              providerLabel={(providerId) =>
                console.providers.find((item) => item.id === providerId)?.label ?? providerId
              }
              findSignal={findSignal}
              // xterm 的主题跟解析后的明暗走（它读不到 CSS 变量）。
              resolvedTheme={theme.resolved}
              target={selectedTarget}
              gateway={console.gateway}
              // 供应商编号 → 名字要一份清单；它已经为工作台取过了，复用同一份。
              vendors={sidebarVendors}
              /*
                「继续会话」：点会话行不再起终端之后，这是**显式的**恢复入口
                （`beginResumeSession` 原样保留：本目录还在就直接进终端，
                不在就弹「设置工作目录」）。`undefined` = 这一刻没有可恢复的东西。
              */
              onResumeSession={canResumeSession ? resumeActiveSession : undefined}
              /*
                会话有两个来源，**不能像从前那样合成一个**：

                - 历史视图（含它下面的消息列表）跟**选中项**走——标题与正文必须同源，
                  否则右键菜单那几条只改选中项的动作会让画布出「S 的标题 + R 的消息」；
                - 终端信息条跟**标签**走（`tabSession`）——它讲的是这一路终端属于谁。

                消息类 props（`messageTotal` / `messages` / `hasEarlier` / …）在
                `use-console` 里就是按选中项取的，因此与 `session` 天然同源。
              */
              session={console.selectedSession}
              infoSession={tabSession}
              messageTotal={console.messageTotal}
              messages={console.messages}
              hasEarlier={console.hasEarlier}
              loadingMore={console.loadingMore}
              onLoadEarlier={() => void console.loadEarlierMessages()}
              /*
                分支：**带上标签那条会话的编号**。历史视图讲的是活动标签的会话，
                而它未必等于列表里的选中项（例如切到没有会话编号的新标签之后，
                选中项还停在旧会话上），只读选中值会分到别的会话上去。
              */
              onBranch={(sourceLine, sessionId) =>
                void console.branchFromMessage(sourceLine, sessionId)
              }
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
        }
        statusBar={
          <StatusBar
            busy={console.busy}
            searching={console.searching}
            error={console.error}
            session={console.selectedSession}
            target={selectedTarget}
            terminals={terminals}
            activeTerminalId={activeTerminalId}
            gateway={console.gateway}
            // 还没有输入侧状态时按初值显示：新会话就是「自管输入」。
            inputMode={activeUi?.inputMode ?? "composer"}
            /*
              切输入模式。切到「终端原生输入」时**焦点也会跟回终端**——那件事由
              `setInputMode` 一并做（`focusRequest`），这里只决定切到哪一边：
              焦点规则留在状态层，往后多一个入口也不会漏掉它。
            */
            onToggleInputMode={() => {
              if (!activeTerminalId) return;
              terminals.setInputMode(
                activeTerminalId,
                activeUi?.inputMode === "terminal" ? "composer" : "terminal"
              );
            }}
            onNotify={toast.show}
          />
        }
      />

      <AppOverlays
        state={{
          toastMessage: toast.message,
          exportedTo,
          sessionSettings,
          newSession,
          menu,
          tabMenu,
          titleDialog,
          detailSession,
          settingsOpen,
          settingsTab
        }}
        actions={{
          closeToast: toast.dismiss,
          closeExported: () => setExportedTo(""),
          openExported: () => void api.openPath(exportedTo),
          changeSessionSettingsPath: (path) =>
            setSessionSettings((current) => (current ? { ...current, path } : current)),
          closeSessionSettings: () => setSessionSettings(null),
          // 环境面板的「WSL 会话目录」卡与菜单栏的「设置会话」共用这一个入口。
          openSessionSettings,
          // 关于面板承接的两个菜单动作（spec §11）：打开日志目录与退出。
          quit,
          openLogDirectory,
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
          exportSession: (session, format) => {
            // 先把选中态切到这一条：导出的落点提示、之后的「打开」都以它为上下文。
            console.setSelectedId(session.id);
            void runExport(format, session);
          },
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
          branchFromMessage: (sourceLine, sessionId) =>
            void console.branchFromMessage(sourceLine, sessionId),
          // 消息弹框里导出的必须是**弹框里那条**：`runExport` 显式带编号，
          // 否则它会去读「当前选中」，而选中项未必是弹框正开着的那条会话。
          exportDetail: (session, format) => void runExport(format, session),
          closeSettings: () => setSettingsOpen(false),
          reportError: console.setError,
          notify: toast.show
        }}
        providers={console.providers}
        providerId={console.providerId}
        appVersion={appVersion}
        targetId={console.targetId}
        targets={console.targets}
        selectedTarget={selectedTarget}
        trash={trash}
        // 能力判定只在这里算一次：`provider-capabilities.ts` 是唯一的判据来源。
        canDuplicate={supportsDuplicate(console.providers, console.providerId)}
        canBranch={supportsBranch(console.providers, console.providerId)}
        resolveSession={sessionById}
        tabLabel={workspace.tabItems.find((item) => item.key === tabMenu?.key)?.label ?? "标签"}
        closableTabCount={workspace.tabs.length}
      />
    </>
  );
}
