import { invoke } from "./client";
import type {
  AiProviderSummary,
  AiSession,
  AiTarget,
  ApiVendor,
  CliEnvironmentStatus,
  ExportFormat,
  ExternalApiStatus,
  FailoverRule,
  FailoverRuleScope,
  GatewayLogEntry,
  GatewayPortStatus,
  GatewayVendorHealth,
  SessionMessagePage,
  SessionMetadata,
  SessionMutationRef,
  SessionQuickAccess,
  SessionView,
  SkillEntry,
  UsageSummary,
  WorkspacePreset
} from "../types";

// 事件订阅与命令调用同属 IPC 面，调用方从这一层统一取，
// 不必知道「命令在 commands、事件在 client」这种内部分工。
export { listen, IpcError } from "./client";

/**
 * 命令面（**只列已接线、且主流程要用的**）。
 *
 * 每个函数就是一行 `invoke`：这一层的价值不在逻辑，而在**把命令名与参数形状收在一处**。
 * 别处只调这些函数，不出现命令字符串。
 */

/** 平台清单。 */
export const listProviders = () =>
  invoke<AiProviderSummary[]>("ai:list-providers");

/** 探测某平台的目标（会真的去碰文件系统与 WSL，可能慢）。 */
export const listTargets = (providerId: string) =>
  invoke<AiTarget[]>("codex:list-targets", { providerId });

/** 读目标缓存（不探测，首屏用它立刻出内容）。 */
export const listCachedTargets = (providerId: string) =>
  invoke<AiTarget[]>("codex:list-cached-targets", { providerId });

/** 列出某目标的会话。 */
export const listSessions = (targetId: string) =>
  invoke<AiSession[]>("codex:list-sessions", { targetId });

/** 列出回收站里的会话。 */
export const listTrashSessions = (targetId: string) =>
  invoke<AiSession[]>("codex:list-trash-sessions", { targetId });

/** 读会话缓存（同样用于首屏）。 */
export const listCachedSessions = (targetId: string, view: SessionView) =>
  invoke<AiSession[]>("codex:list-cached-sessions", { targetId, view });

/**
 * 按关键词搜索会话。
 *
 * 后端会先匹配摘要，摘要不中再**读整个文件**二次匹配——因此它比列列表重得多，
 * 界面必须给出「搜索中」而不是让它静默地卡住。
 */
export const searchSessions = (targetId: string, view: SessionView, query: string) =>
  invoke<AiSession[]>("codex:search-sessions", { targetId, view, query });

/** 取一页消息。`offset = -1` 表示最后一页。 */
export const getSessionMessagesPage = (
  targetId: string,
  sessionId: string,
  offset: number,
  limit: number
) =>
  invoke<SessionMessagePage>("codex:get-session-messages-page", {
    targetId,
    sessionId,
    offset,
    limit
  });

/** 设置收藏。返回更新后的元数据。 */
export const setSessionFavorite = (targetId: string, sessionId: string, favorite: boolean) =>
  invoke<SessionMetadata>("codex:set-session-favorite", { targetId, sessionId, favorite });

/** 把会话移入回收站（批量）。 */
export const deleteSessions = (targetId: string, sessions: SessionMutationRef[]) =>
  invoke<unknown>("codex:delete-sessions", { targetId, sessions });

/** 从回收站还原。 */
export const restoreSession = (targetId: string, sessionId: string) =>
  invoke<unknown>("codex:restore-session", { targetId, sessionId });

/** 从回收站彻底删除（批量）。 */
export const purgeSessions = (targetId: string, sessions: SessionMutationRef[]) =>
  invoke<unknown>("codex:purge-sessions", { targetId, sessions });

/**
 * 从某条消息处创建分支。
 *
 * `messageLine` 是**原始 JSONL 行号**（不是过滤后的可见序号）：后端要靠它精确定位到
 * 原始记录，用可见序号会截错位置。
 */
export const branchSession = (targetId: string, sessionId: string, messageLine: number) =>
  invoke<AiSession>("codex:branch-session", { targetId, sessionId, messageLine });

/** 复制一份会话（可选给副本一个标题）。 */
export const duplicateSession = (targetId: string, sessionId: string, title?: string) =>
  invoke<AiSession>("codex:duplicate-session", {
    targetId,
    sessionId,
    ...(title ? { title } : {})
  });

/** 设置自定义标题。 */
export const setSessionCustomTitle = (targetId: string, sessionId: string, title: string) =>
  invoke<SessionMetadata>("codex:set-session-custom-title", { targetId, sessionId, title });

/**
 * 导出会话到指定文件。
 *
 * **没有原生保存对话框**（那需要 Tauri 的 dialog 插件，离线装不上），因此目标路径由
 * 界面决定并显示给用户——见 `exportTargetPath`。
 */
export const exportSession = (
  targetId: string,
  sessionId: string,
  format: ExportFormat,
  filePath: string
) =>
  invoke<{ filePath: string; format: string }>("codex:export-session", {
    targetId,
    sessionId,
    format,
    filePath
  });

/** 用系统默认程序打开一个路径（导出后「打开所在目录」用它）。 */
export const openPath = (path: string, targetId?: string) =>
  invoke<unknown>("shell:open-path", { path, ...(targetId ? { targetId } : {}) });

/** 在系统文件管理器里打开某个会话所在的目录。 */
export const openSessionFolder = (targetId: string, sessionId: string) =>
  invoke<unknown>("codex:open-session-folder", { targetId, sessionId });

/** 单个会话移入回收站。 */
export const deleteSession = (targetId: string, sessionId: string, filePath?: string) =>
  invoke<unknown>("codex:delete-session", {
    targetId,
    sessionId,
    ...(filePath ? { ref: filePath } : {})
  });

/** 单个会话从回收站彻底删除。 */
export const purgeSession = (targetId: string, sessionId: string, filePath?: string) =>
  invoke<unknown>("codex:purge-session", {
    targetId,
    sessionId,
    ...(filePath ? { ref: filePath } : {})
  });

/**
 * 手动指定某 WSL 发行版内的 Codex 目录。
 *
 * **这是发行版进不去时唯一还能走的路**（见 `docs/rust-rewrite/09`）：自动探测必然
 * 失败，但用户知道自己的 Codex 装在哪。后端会校验目录存在并取真实路径。
 */
export const setWslCodexHome = (distro: string, codexHome: string) =>
  invoke<{ distro: string; codexHome: string }>("codex:set-wsl-codex-home", {
    distro,
    codexHome
  });

/** 清除人工覆盖，回到自动探测。 */
export const clearWslCodexHome = (distro: string) =>
  invoke<{ distro: string }>("codex:clear-wsl-codex-home", { distro });

/**
 * 快捷访问：收藏 + 最近打开。
 *
 * **只读缓存，不触发探测**——它要能在界面首屏立刻返回。后端因此不会去碰文件系统。
 */
export const listSessionQuickAccess = () =>
  invoke<SessionQuickAccess>("codex:list-session-quick-access");

/** 标记会话已打开（用于「最近打开」）。 */
export const markSessionOpened = (targetId: string, sessionId: string) =>
  invoke<unknown>("codex:mark-session-opened", { targetId, sessionId });

/* ── 供应商 ─────────────────────────────────────────── */

/** 列出某平台的供应商。 */
export const listVendors = (providerId: string) =>
  invoke<ApiVendor[]>("vendor:list", { providerId });

/**
 * 把一路终端的路由切到指定供应商。
 *
 * `routeId` 就是 `terminalId`：路由是按终端创建的，两者在本实现里是同一个值。
 *
 * 返回 `{ routeId, vendorId }`。**切换失败时后端返回的是错误**（不是 `switched: false`），
 * 因此调用方必须处理异常——供应商被禁用、配置不全、或平台不匹配都会失败。
 */
export const switchVendorRoute = (routeId: string, vendorId: string) =>
  invoke<{ routeId: string; vendorId: string }>("vendor:route-switch", { routeId, vendorId });

/**
 * 设置一路终端的路由模式。
 *
 * `locked` 表示**不自动故障转移**：只用手动切换改变供应商。用户为某台终端锁定了
 * 一家供应商时用它。
 */
export const setVendorRouteMode = (routeId: string, mode: "dynamic" | "locked") =>
  invoke<{ routeId: string; mode: "dynamic" | "locked" }>("vendor:route-mode", {
    routeId,
    mode
  });

/** 启用/禁用供应商。 */
export const setVendorEnabled = (id: string, enabled: boolean) =>
  invoke<ApiVendor>("vendor:set-enabled", { id, enabled });

/**
 * 保存供应商（新建或更新）。
 *
 * 后端把**整个参数对象**反序列化成 `ApiVendorInput`，字段名必须逐个对上：
 * `providerId` / `name` / `apiKey` / `apiBaseUrl` / `sort?` / `enabled?` / `id?`。
 *
 * `apiKey` **留空表示沿用旧值**（领域层的归一化语义），因此编辑时不必重新输入密钥。
 */
export const saveVendor = (request: {
  id?: string;
  providerId: string;
  name: string;
  apiKey: string;
  apiBaseUrl: string;
  sort?: number;
  enabled?: boolean;
}) => invoke<ApiVendor>("vendor:save", request);

/** 删除供应商。 */
export const deleteVendor = (id: string) => invoke<boolean>("vendor:delete", { id });

/* ── 技能 ───────────────────────────────────────────── */

/** 列出某目标下的技能。 */
export const listSkills = (targetId: string) => invoke<SkillEntry[]>("skill:list", { targetId });

/** 启用/停用技能。 */
export const setSkillEnabled = (targetId: string, name: string, enabled: boolean) =>
  invoke<{ renamedTo: string }>("skill:set-enabled", { targetId, name, enabled });

/** 在系统文件管理器里打开技能目录。 */
export const openSkillFolder = (targetId: string) =>
  invoke<unknown>("skill:open-folder", { targetId });

/** 列出回收站里的技能。 */
export const listTrashSkills = (targetId: string) =>
  invoke<SkillEntry[]>("skill:list-trash", { targetId });

/** 从回收站还原技能。 */
export const restoreSkill = (targetId: string, name: string) =>
  invoke<unknown>("skill:restore", { targetId, name });

/** 从回收站彻底删除技能。 */
export const purgeSkill = (targetId: string, name: string) =>
  invoke<unknown>("skill:purge", { targetId, name });

/**
 * 导入技能。
 *
 * `source` 是**本地路径**，由界面用 `dialog:choose-directory` 取到——`aivc-ipc`
 * 不依赖 tauri，取不到平台对话框。
 *
 * `overwrite` **默认为 false**：覆盖不可逆，因此默认拒绝，由界面在冲突时再问一次。
 */
export const importSkill = (targetId: string, source: string, overwrite = false) =>
  invoke<{ skillName: string; destinationPath: string }>("skill:import", {
    targetId,
    source,
    overwrite
  });

/* ── 工作区预设 ─────────────────────────────────────── */

/** 列出全部工作区预设。 */
export const listPresets = () => invoke<WorkspacePreset[]>("workspace:list-presets");

/**
 * 保存预设（新建或更新）。
 *
 * `cwd` 必填——空工作目录的预设没有意义，领域层也会拒绝。名称留空时由领域层回退到
 * `cwd`，因此这里不强制非空。
 */
export const savePreset = (request: {
  id?: string;
  name?: string;
  cwd: string;
  targetKind?: string;
  prompt?: string;
  cliArgs?: string;
}) => invoke<string>("workspace:save-preset", request);

/** 删除预设。 */
export const deletePreset = (id: string) =>
  invoke<boolean>("workspace:delete-preset", { id });

/* ── 网关 ───────────────────────────────────────────── */

/** 打开/关闭本地网关。 */
export const setGatewayEnabled = (enabled: boolean) =>
  invoke<GatewayPortStatus>("gateway:set-enabled", { enabled });

/** 设置网关端口；`0` 表示自动分配。 */
export const setGatewayPort = (port: number) =>
  invoke<GatewayPortStatus>("gateway:set-port", { port });

/** 各供应商的健康快照。 */
export const getVendorHealth = () =>
  invoke<GatewayVendorHealth[]>("gateway:get-vendor-health");

/** 用量总览。 */
export const getUsageSummary = () => invoke<UsageSummary>("gateway:get-usage-summary");

/** 查请求日志（按时间倒序的一页）。 */
export const queryGatewayLogs = (page = 1, pageSize = 20) =>
  invoke<GatewayLogEntry[]>("gateway:query-logs", { page, pageSize });

/** 清空日志。 */
export const clearGatewayLogs = () => invoke<number>("gateway:clear-logs", {});

/* ── 失败转移规则 ───────────────────────────────────── */

/** 列出全部故障识别规则。 */
export const listFailoverRules = () =>
  invoke<FailoverRule[]>("gateway:list-failover-rules");

/**
 * 保存规则（新建或更新）。
 *
 * 后端把**整个参数对象**反序列化成 `FailoverRuleInput`。
 * `statusCodes` 是「命中即视为故障的 HTTP 状态码」；`pattern` 是不区分大小写的
 * **包含匹配**，不是正则。
 */
export const saveFailoverRule = (request: {
  id?: string;
  scope: FailoverRuleScope;
  providerId?: string;
  vendorId?: string;
  pattern: string;
  statusCodes?: number[];
  customResponseStatus?: number;
  customResponseBody?: string;
  enabled?: boolean;
}) => invoke<FailoverRule>("gateway:save-failover-rule", request);

/** 删除规则。 */
export const deleteFailoverRule = (id: string) =>
  invoke<boolean>("gateway:delete-failover-rule", { id });

/** 启用/停用规则。 */
export const setFailoverRuleEnabled = (id: string, enabled: boolean) =>
  invoke<boolean>("gateway:set-failover-rule-enabled", { id, enabled });

/* ── 外部 API ───────────────────────────────────────── */

/** 外部 API 状态。 */
export const getExternalApiStatus = () =>
  invoke<ExternalApiStatus>("gateway:get-external-api-status");

/** 开关外部 API。 */
export const setExternalApiEnabled = (enabled: boolean) =>
  invoke<ExternalApiStatus>("gateway:set-external-api-enabled", { enabled });

/**
 * 轮换外部 API 令牌。
 *
 * 返回值里的令牌**只在这里出现一次**——后端只存哈希，之后再也取不回来。
 * 因此界面必须让用户当场复制走。
 */
export const rotateExternalApiToken = () =>
  invoke<{ token: string }>("gateway:rotate-external-api-token", {});

/* ── CLI 环境 ───────────────────────────────────────── */

/**
 * 检测某目标下的 CLI 环境。
 *
 * 返回的 `messages` 是后端算好的**可执行建议**——界面直接显示即可，
 * 不要在前后端各写一份判断逻辑。
 */
export const checkCliEnvironment = (providerId: string, targetId?: string) =>
  invoke<CliEnvironmentStatus>("cli:check-environment", {
    providerId,
    ...(targetId ? { targetId } : {})
  });

/** 应用版本。 */
export const getAppVersion = () => invoke<string>("app:get-version");

/**
 * 安装 CLI。
 *
 * `ensureNode` 为真时先确保 Node——后端会根据环境检测结果决定要不要带它。
 * 安装过程**可能很久**（下载 + npm 安装），界面必须给出「进行中」而不是让它静默卡住。
 */
export const installCli = (request: {
  providerId: string;
  targetId?: string;
  nodeMajor?: number;
  ensureNode?: boolean;
}) => invoke<unknown>("cli:install", request);

/* ── 平台能力（剪贴板 / 对话框 / 通知）──────────────────
 *
 * 这三组命令**不在命令路由器里**：它们要用 Tauri 的平台插件，而 `aivc-ipc` 刻意
 * 不依赖 tauri。它们在 `aivc-app` 的 `aivc_invoke` 里被拦下处理。
 * 对前端来说没有区别——同样是一个命令名。
 */

/** 写入系统剪贴板。 */
export const copyText = (text: string) => invoke<null>("clipboard:copy-text", { text });

/** 读取系统剪贴板。 */
export const readText = () => invoke<string>("clipboard:read-text");

/**
 * 弹出原生目录选择框。
 *
 * 用户取消时返回 `null`——**取消是正常操作，不是错误**，调用方要能区分。
 */
export const chooseDirectory = (title?: string) =>
  invoke<string | null>("dialog:choose-directory", title ? { title } : {});

/**
 * 弹出原生**文件**选择框（限定 `.md`）。
 *
 * 技能既可以是目录，也可以是单个 `SKILL.md` —— 后端的 `import_from_path` 按
 * `path.is_dir()` 自动判断，两种都支持。取消同样返回 `null`。
 */
export const chooseFile = (title?: string) =>
  invoke<string | null>("dialog:choose-file", title ? { title } : {});

/** 发一条系统通知。权限缺失或系统静音时静默失败。 */
export const notify = (title: string, detail?: string) =>
  invoke<null>("terminal:attention-notification", {
    title,
    ...(detail ? { detail } : {})
  }).catch(() => undefined);

/** 网关端口状态。 */
export const getGatewayPort = () => invoke<GatewayPortStatus>("gateway:get-port");

/* ── 终端 ─────────────────────────────────────────────
 *
 * 后端的事件通道名与命令名一样是**契约**，因此也放在这一层：
 *
 * - `terminal:data` 载荷 `{ terminalId, data: number[] }`——**原始字节**，
 *   交互式程序会发出光标控制序列，不能当作文本处理；
 * - `terminal:exit` 载荷 `{ terminalId, exitCode }`——`exitCode` 为 `null` 表示
 *   **不是进程自己结束的**（用户主动关闭了终端），此时按「正常结束」处理。
 */
export const CHANNEL_TERMINAL_DATA = "terminal:data";
export const CHANNEL_TERMINAL_EXIT = "terminal:exit";
export const CHANNEL_TERMINAL_STATUS = "terminal:status";

/**
 * 网关把某一路终端切到了另一个供应商。
 *
 * 三种来源都会发：用户手动切换（`manual`）、路由模式变化（`mode`）、
 * **以及自动故障转移（`failure`）**。最后一种是这个事件存在的**主要原因**——
 * 没有它，界面会一直显示一个已经不是实际服务方的供应商：请求正由第二家服务，
 * 状态栏却写着第一家，用户看不到任何异常，只有账单对不上。
 *
 * 载荷：`{ terminalId, routeId, vendorId, reason, mode }`。
 * `terminalId` 与 `routeId` 是同一个值，前端用哪个都行。
 */
export const CHANNEL_GATEWAY_VENDOR_SWITCHED = "gateway:vendor-switched";

/**
 * 网关**每转发一个请求**就推一条。
 *
 * 工作台的请求视图靠它实时更新，**不轮询**（后端注释里就是这么写的）。
 * 载荷：`{ requestId, providerId, vendorId, outcome, upstreamStatus, durationMs, switched }`。
 *
 * 注意频率：一个终端在重试循环里一秒能发好几条，而刷新一次要拉三个接口。
 * 因此订阅方**必须合并刷新**——见 `app/refresh-coalescer.ts`。
 */
export const CHANNEL_GATEWAY_REQUEST_RECORDED = "gateway:request-recorded";

/**
 * 重置供应商健康。
 *
 * `vendorId` 省略表示重置全部。**它同时清掉内存里正在生效的熔断状态**——
 * 只清数据库那一份时，按下去行为完全不变。
 */
export const resetVendorHealth = (vendorId?: string) =>
  invoke<number>("gateway:reset-vendor-health", vendorId ? { vendorId } : {});


/**
 * 启动一路终端。
 *
 * `targetId` 与 `vendorId` **都要传**，它们不是可有可无的：
 *
 * - `targetId` 决定命令是「本机形态」还是「WSL 形态」，同时决定走网关时的上游地址
 *   要不要换成发行版内可达的宿主地址。不传它，WSL 目标会被当成本机目标。
 * - `vendorId` 决定这次终端绑到哪个供应商。不传时后端取该平台候选池里的第一个。
 *
 * 返回值里的 `vendorId` / `vendorMode` 是**后端实际选定的**那一家——界面不在启动时
 * 挑供应商（它还不知道哪家合适），而是绑定后端选出来的这个，之后靠
 * `gateway:vendor-switched` 更新。没有可用供应商时两者都缺席，表示直连官方 API。
 */
export const startTerminal = (request: {
  terminalId: string;
  providerId: string;
  targetId?: string;
  vendorId?: string;
  sessionId?: string;
  cwd?: string;
  codexHome?: string;
  cliArgs?: string;
  useCodexCwdFlag?: boolean;
  cols?: number;
  rows?: number;
}) =>
  invoke<{
    terminalId: string;
    cols: number;
    rows: number;
    vendorId?: string;
    vendorMode?: "dynamic" | "locked";
  }>("terminal:start", request);

/**
 * 启动一路**系统终端**：跑用户本来就在用的那个 shell，而不是 Codex。
 *
 * 它**不注入任何环境变量**——注入会让它不再是用户平时那个 shell。
 */
export const startSystemTerminal = (request: {
  terminalId: string;
  kind?: string;
  targetId?: string;
  cwd?: string;
  cols?: number;
  rows?: number;
}) => invoke<{ terminalId: string; cols: number; rows: number }>("terminal:start-system", request);

/** 把输入写进 PTY。xterm 对 `ESC[6n` 等查询的应答正是走这里回去的。 */
export const writeTerminal = (terminalId: string, data: string) =>
  invoke<void>("terminal:write", { terminalId, data });

/** 调整终端尺寸（行列数）。 */
export const resizeTerminal = (terminalId: string, cols: number, rows: number) =>
  invoke<void>("terminal:resize", { terminalId, cols, rows });

/** 停止一路终端。 */
export const stopTerminal = (terminalId: string) =>
  invoke<{ stopped: boolean }>("terminal:stop", { terminalId });

/* ── 应用与退出 ───────────────────────────────────────
 *
 * 退出是**后端发起**的：用户点窗口关闭按钮（或托盘「退出」）时，后端先拦住关闭，
 * 再发 `app:request-exit` 让界面弹确认框。界面选完之后回调 `app:exit-choice`。
 *
 * 方向反过来做（前端自己监听窗口关闭）在 Tauri 里拿不到「可以阻止关闭」的钩子，
 * 而**必须**能阻止——应用里可能跑着终端和本地网关。
 */

/** 后端请求界面弹出退出确认框。 */
export const CHANNEL_REQUEST_EXIT = "app:request-exit";

/** 用户在退出确认框里的选择。 */
export type ExitChoice = "minimize" | "quit" | "cancel";

/**
 * 回答退出询问。
 *
 * **必须回答**：后端会一直等着，直到窗口被关掉或收到选择。选 `cancel` 时后端会把
 * 「正在询问」的状态清掉，下次关闭会重新问。
 */
export const chooseExitAction = (choice: ExitChoice) =>
  invoke<void>("app:exit-choice", { choice });

/** 应用级命令。`quit` 走退出确认（**不是**立刻退出），与现网菜单一致。 */
export type AppCommand = "quit" | "openLogDir";

/** 执行一条应用级命令。 */
export const appCommand = (command: AppCommand) => invoke<void>("app:command", { command });

/**
 * 上报一次界面操作用时。
 *
 * **失败不抛**：埋点是观测手段，它自己出错不该影响用户的正常操作。
 */
export const logPerformance = (
  label: string,
  durationMs: number,
  status?: string
): Promise<void> =>
  invoke<void>("performance:log", { label, durationMs, status }).catch(() => undefined);
