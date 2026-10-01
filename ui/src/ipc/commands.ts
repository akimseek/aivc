import { invoke } from "./client";
import type {
  AiProviderSummary,
  AiSession,
  AiTarget,
  ApiVendor,
  CliEnvironmentStatus,
  ExportFormat,
  ExternalApiStatus,
  FailureDiagnosticsPage,
  FailoverRule,
  FailoverRuleScope,
  GatewayLogEntry,
  GatewayLogsPage,
  GatewayPortStatus,
  GatewayVendorHealth,
  SessionMessagePage,
  SessionMetadata,
  SessionMutationRef,
  SessionQuickAccess,
  SessionView,
  SkillEntry,
  UsageReport,
  UsageSummary,
  VendorBalanceResult,
  VendorConfigFile,
  VendorModel,
  WorkspacePreset
} from "../types";

// 事件订阅与命令调用同属 IPC 面，调用方从这一层统一取，
// 不必知道「命令在 commands、事件在 client」这种内部分工。
export { listen, onDragDrop, onDragHover, IpcError } from "./client";

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

/**
 * 在系统文件管理器里打开某个会话所在的目录。
 *
 * 命令名必须与后端注册表逐字一致（这里是 `shell:` 前缀，不是 `codex:`）：
 * 两边都编译得过，调错只会得到一个运行时的「未知命令」。
 */
export const openSessionFolder = (targetId: string, sessionId: string) =>
  invoke<unknown>("shell:open-session-folder", { targetId, sessionId });

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

/**
 * 列出供应商。
 *
 * `providerId` **可以省略**：省略或传空串时后端列出**全部平台**的供应商
 * （`vendor:list` 只在参数非空时才过滤）。供应商管理页需要这一条——
 * 它是「所有厂商的供应商放在一起管」，不是「当前平台的供应商」。
 *
 * # 这里不能用 `ProviderId::from_target_id` 那种宽松解析
 *
 * 后端的注释写得很明确：宽松解析对没有前缀的裸字符串（如 `"claude"`）
 * 会**兜底成 Codex**，于是「查询 Claude 供应商」返回的是 Codex 的结果——
 * 静默出错，比报错更糟。所以传值必须是真的平台编号。
 */
export const listVendors = (providerId?: string) =>
  invoke<ApiVendor[]>("vendor:list", providerId ? { providerId } : {});

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
 * **启用**供应商：把它的配置模板写进 CLI 的配置文件（写前备份）。
 *
 * # 与 `setVendorEnabled` 的区别就是「写不写文件」
 *
 * | 命令 | 做什么 |
 * | --- | --- |
 * | `vendor:set-enabled` | **只翻数据库里的启用位** |
 * | `vendor:enable` | 写文件（带备份）**然后**才翻启用位 |
 *
 * 旧平台的「启用」是后者：供应商被启用的含义就是「CLI 的配置指向它」。
 * 只翻标志位等于界面显示已启用、而 CLI 实际还用着旧配置——那是这类功能里
 * 最难排查的一种不一致。
 *
 * 顺序固定为**先写文件、再改数据库**，中途失败会如实报告已经写了哪些
 * （返回值里的 `written`），已写入的都有备份。
 */
export const enableVendor = (id: string) =>
  invoke<{
    vendorId: string;
    /** 实际写入的配置文件名。 */
    written: string[];
    /** 备份根目录（`vendor-backups/<供应商名>/<时间戳>/`）。 */
    backupRoot: string;
    lastEnabledAt?: string;
  }>("vendor:enable", { id });

/**
 * 保存供应商（新建或更新）。
 *
 * 后端把**整个参数对象**反序列化成 `ApiVendorInput`，字段名必须逐个对上：
 * `providerId` / `name` / `apiKey` / `apiBaseUrl` / `sort?` / `enabled?` / `id?`。
 *
 * `apiKey` **留空表示沿用旧值**（领域层的归一化语义），因此编辑时不必重新输入密钥。
 */
/**
 * 新建或更新一个供应商。
 *
 * 字段名与后端 `ApiVendorInput`（camelCase）逐字对应。`pricing` / `balanceQuery` /
 * `modelQuery` 都是**可选**的：不传表示不设置，传空对象则是「设置了但什么都没有」，
 * 两者不是一回事。
 */
export const saveVendor = (request: {
  id?: string;
  providerId: string;
  name: string;
  apiKey: string;
  apiBaseUrl: string;
  sort?: number;
  enabled?: boolean;
  /** 费率（每百万 Token，美元）。 */
  pricing?: { inputPerMillionUsd?: number; outputPerMillionUsd?: number };
  /** 余额查询配置（后端 `BalanceQueryConfig` 的结构化对象）。 */
  balanceQuery?: unknown;
  /** 模型查询配置（后端 `ModelQueryConfig` 的结构化对象）。 */
  modelQuery?: unknown;
}) => invoke<ApiVendor>("vendor:save", request);

/** 删除供应商。 */
export const deleteVendor = (id: string) => invoke<boolean>("vendor:delete", { id });

/**
 * 向供应商的 HTTP 接口查可用模型。
 *
 * 这是**发起真实网络请求**的命令，因此它会慢、会失败，也可能因为 base_url 写错而超时。
 * 界面必须给出「正在查询」与失败原因，而不是静默转圈。
 */
export const listVendorModels = (vendorId: string) =>
  invoke<VendorModel[]>("vendor:list-models", { vendorId });

/**
 * 列出某个目标下 **CLI 自己**能识别的模型（目前只有 Qoder 支持）。
 *
 * 与 `listVendorModels` 是两件事：那个查供应商的 HTTP 接口（当前路由到的那家有什么模型），
 * 这个查 CLI 自己的清单（`qodercn --list-models`），选中后靠 `/model <id>` 切。
 *
 * 后端只认 `qoder:` 开头的目标，其余目标**显式报错**而不是返回空列表——否则界面上
 * 「不支持」与「一个模型都没有」长得一模一样（见 `aivc-ipc/src/router.rs` 的 `dispatch_models`）。
 */
export const listModels = (targetId: string) =>
  invoke<Array<{ id: string }>>("models:list", { targetId });

/**
 * 查询一个供应商的余额。
 *
 * 单家与批量是**两条命令**：批量那条会对十几家并发（仓储自己分批，
 * 避免给中转站造成瞬时压力）。界面上的「刷新全部」用批量，行内那个用单家。
 */
export const refreshVendorBalance = (vendorId: string) =>
  invoke<VendorBalanceResult>("vendor:refresh-balance", { vendorId });

export const refreshAllVendorBalances = () =>
  invoke<VendorBalanceResult[]>("vendor:refresh-balances");

/**
 * 读取若干配置文件的内容。
 *
 * **路径白名单在后端强制**：不加限制的话这条命令就成了「读取任意文件」的入口，
 * 而配置文件里通常有 API Key。界面只应传后端已知的那几个路径。
 */
export const readVendorConfigs = (paths: string[]) =>
  invoke<VendorConfigFile[]>("vendor:read-configs", { paths });

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

/** 用量总览。**区间必填**——后端 `period()` 在缺 `periodStart`/`periodEnd` 时直接报错。 */
export const getUsageSummary = (periodStart: string, periodEnd: string) =>
  invoke<UsageSummary>("gateway:get-usage-summary", { periodStart, periodEnd });

/** 用量报表（总览 + 按供应商 + 按模型）。 */
export const getUsageReport = (periodStart: string, periodEnd: string) =>
  invoke<UsageReport>("gateway:get-usage-report", { periodStart, periodEnd });

/**
 * 失败诊断分页。
 *
 * 页码从 **1** 起；`pageSize` 由仓储钳制在上限内。筛选条件全部可选，
 * 但不传**不等于传空字符串**：后端把空串当作「不筛选」，
 * 因此这里显式省略掉空值，避免多传一堆无用参数。
 */
export const getFailureDiagnostics = (request: {
  page: number;
  pageSize: number;
  vendorId?: string;
  providerId?: string;
  outcome?: "error" | "timeout";
  periodStart?: string;
  periodEnd?: string;
}) => invoke<FailureDiagnosticsPage>("gateway:get-failure-diagnostics", request);

/**
 * 查可清理的日志条目。
 *
 * # 这个函数此前传错了参数
 *
 * 它原本传的是 `{ page, pageSize }`，而后端的 `query_logs` 用的是
 * `filter(args)`——只读 `vendorId` / `outcome` / `periodStart` / `periodEnd`。
 * 也就是说那两个参数**被静默忽略了**，函数看起来「分页」，实际每次都是全量。
 * 现在后端真的支持分页了，这里把参数接回去。
 *
 * # 分页可选，且参数决定返回形状
 *
 * - **不带** `page`/`pageSize`：后端返回裸数组（全量）——保留为兼容路径，
 *   但本仓界面已无人走它：「最近请求」也从它改走了分页（全量裸数组在有流量时
 *   payload 无界，每刷新都要整段解析）；
 * - **带上**（必须两个一起给）：后端返回 `{ items, total, page, pageSize }`
 *   （与先例 `gateway:get-failure-diagnostics` 逐字同形）。日志页 20/页；
 *   「最近请求」（`GatewayInsights`）固定首页 50 条——它是概览不是审计。
 *
 * 两种形状用重载分开声明，调用方不必自己 `Array.isArray` 收窄；
 * 只给一个参数在**编译期**就通不过（后端也会报「参数无效」——静默忽略参数
 * 正是这个函数踩过的坑）。
 *
 * 页码从 **1** 起；`pageSize` 由仓储钳制在上限（50）内。
 */
export function queryGatewayLogs(request: {
  page: number;
  pageSize: number;
  vendorId?: string;
  outcome?: "ok" | "error" | "timeout" | "client-aborted";
  periodStart?: string;
  periodEnd?: string;
}): Promise<GatewayLogsPage>;
export function queryGatewayLogs(request?: {
  vendorId?: string;
  outcome?: "ok" | "error" | "timeout" | "client-aborted";
  periodStart?: string;
  periodEnd?: string;
}): Promise<GatewayLogEntry[]>;
export function queryGatewayLogs(
  request: {
    page?: number;
    pageSize?: number;
    vendorId?: string;
    outcome?: "ok" | "error" | "timeout" | "client-aborted";
    periodStart?: string;
    periodEnd?: string;
  } = {}
) {
  return invoke<GatewayLogEntry[] | GatewayLogsPage>("gateway:query-logs", request);
}

/** 删除指定请求编号的日志，返回删除条数。 */
export const deleteGatewayLogEntries = (requestIds: string[]) =>
  invoke<number>("gateway:delete-log-entries", { requestIds });

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

/**
 * 试一条**还没保存**的规则会不会命中。
 *
 * # 为什么必须有这条命令
 *
 * 匹配规则是「状态码精确匹配 **或** 错误文本包含匹配」。这件事**只有规则引擎自己
 * 说了算**——界面自己实现一遍的话，两份逻辑迟早分叉，表现是「测试说会命中、
 * 实际没切换」，而那是最难查的一类偏差。
 *
 * 返回 `matchedBy` 而不是只回布尔值：用户看到「没命中」需要知道该改状态码
 * 还是该改文本。
 *
 * 与保存**走同一套校验**，因此测试被拒的规则，保存时也会被拒。
 */
export const testFailoverRule = (request: {
  /** 规则本体，与 `saveFailoverRule` 同形状（`id` 可省）。 */
  rule: {
    id?: string;
    scope: FailoverRuleScope;
    providerId?: string;
    vendorId?: string;
    pattern: string;
    statusCodes?: number[];
    customResponseStatus?: number;
    customResponseBody?: string;
    enabled?: boolean;
  };
  /** 模拟的上游状态码。 */
  statusCode?: number;
  /** 模拟的上游错误文本。 */
  message?: string;
}) =>
  invoke<{ matched: boolean; matchedBy: "status" | "pattern" | "none" }>(
    "gateway:test-failover-rule",
    request
  );

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

/* ── 窗口控制 ─────────────────────────────────────────
 *
 * 窗口是无边框的（`tauri_app.rs` 的 `.decorations(false)`），系统框不再提供
 * 最小化 / 最大化 / 关闭，应用内标题栏必须自己补齐这四条命令。
 *
 * 拖拽也走命令（`app:window-start-drag`）而不是 `-webkit-app-region: drag`：
 * 前端只允许经**单一命令面**与后端通信，且那个 CSS 属性在各版本 WebView2 上
 * 行为不一，还会吃掉 mousedown。参数一律空对象——后端不读任何入参。
 */

/** 最小化窗口。 */
export const windowMinimize = () => invoke<null>("app:window-minimize", {});

/** 最大化 ↔ 还原（由后端按当前状态取反，界面不维护镜像状态）。 */
export const windowToggleMaximize = () => invoke<null>("app:window-toggle-maximize", {});

/** 开始拖动窗口。由拖拽区的 `mousedown` 发起。 */
export const windowStartDrag = () => invoke<null>("app:window-start-drag", {});

/**
 * 关闭窗口。
 *
 * **不是**直接退出：后端会在 `CloseRequested` 上拦下来弹退出确认
 * （应用里可能跑着终端与本地网关），因此这条命令同时也是「退出」的唯一入口。
 */
export const windowClose = () => invoke<null>("app:window-close", {});

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
/**
 * 弹出原生**文件**选择框。
 *
 * `kind` 决定过滤器：`"image"` 只列图片，`"skill"` 只列 `.md`，其余不加过滤器。
 * **此前过滤器写死是 `.md`**，于是自管输入的「添加文件」实际上只挑得到 Markdown。
 */
export const chooseFile = (title?: string, kind?: "skill" | "image" | "any") =>
  invoke<string | null>("dialog:choose-file", {
    ...(title ? { title } : {}),
    ...(kind ? { kind } : {})
  });

/** 弹出可**多选**的文件选择框。取消时返回空数组（不是错误）。 */
export const chooseFiles = (title?: string, kind?: "image" | "any") =>
  invoke<string[]>("dialog:choose-files", {
    ...(title ? { title } : {}),
    ...(kind ? { kind } : {})
  });

/**
 * 读一张本地图片，返回可直接放进 `<img src>` 的 data URL。
 *
 * 界面上预览本地图片不能直接写 `file://`（webview 拦掉），也不该为此开放
 * asset 协议（那等于把「读任意文件」给整个页面）。这条命令只读白名单后缀、
 * 只读 10 MB 以内。
 */
export const readImage = (path: string) =>
  invoke<{ dataUrl: string; name: string; size: number }>("shell:read-image", { path });

/**
 * 把一段 `data:image/png;base64,…` 落盘，返回路径。
 *
 * 从剪贴板粘进来的图片在 webview 里只是一个 `Blob`、**没有路径**，而附件交给 CLI
 * 靠的是 `@<路径>`，所以必须先落盘。前端负责编码（canvas 本来就会），后端只负责写。
 */
export const saveImage = (dataUrl: string) =>
  invoke<{ path: string; name: string; size: number }>("shell:save-image", { dataUrl });

/** 把一张图**按文件路径**放进系统剪贴板。预览里的「复制」用它。 */
export const copyImage = (path: string) => invoke<null>("clipboard:write-image", { path });

/** 发一条系统通知。权限缺失或系统静音时静默失败。 */
export const notify = (title: string, detail?: string) =>
  invoke<null>("terminal:attention-notification", {
    title,
    ...(detail ? { detail } : {})
  }).catch(() => undefined);

/** 网关端口状态。 */
export const getGatewayPort = () => invoke<GatewayPortStatus>("gateway:get-port");

/* ── 网关韧性阈值 ───────────────────────────────────────
 *
 * 三条命令**已经在后端完整实现**（含边界校验与错误文案），此前只是没登记进命令表，
 * 因此命令面里查不到、界面也就没法调这三个设置。
 *
 * 三个 setter 都返回**完整的 `GatewayPortStatus`**（不是只回显那一个数），
 * 与 `setGatewayPort` / `setGatewayEnabled` 一致——界面可以整体替换状态，
 * 不会因为「只回了一部分字段」把别的抹掉。
 */

/** 异常切换阈值（1–10）。 */
export const setGatewayFailureThreshold = (value: number) =>
  invoke<GatewayPortStatus>("gateway:set-failure-threshold", { value });

/** 熔断次数（1–20）。 */
export const setGatewayCircuitFailureThreshold = (value: number) =>
  invoke<GatewayPortStatus>("gateway:set-circuit-failure-threshold", { value });

/** 熔断持续时间（10–86400 秒）。 */
export const setGatewayCircuitDurationSeconds = (value: number) =>
  invoke<GatewayPortStatus>("gateway:set-circuit-duration-seconds", { value });

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
 * 路径是否存在。
 *
 * 带上 `targetId`：后端**按目标分流**，WSL 目标的路径在本机看来毫无意义，
 * 它会显式拒绝而不是按本机路径误判成「不存在」。`require_local` 认得的形制是后端
 * 自己发出的 `codex:wsl:<distro>` 等带前缀编号，加上旧缓存里的裸 `wsl:<distro>`
 * （见 `aivc-core/src/target.rs` 的 `wsl_distro_of`）。不传就按本机处理。
 */
export const pathExists = (path: string, targetId?: string) =>
  invoke<boolean>("shell:path-exists", {
    path,
    ...(targetId ? { targetId } : {})
  });

/**
 * 列出某个会话的**子分支**。
 *
 * 后端一直有这个命令（`codex:list-session-children`），只是界面此前没有入口——
 * 「分支关系」面板要靠它。
 */
export const listSessionChildren = (parentSessionId: string, targetId?: string) =>
  invoke<AiSession[]>("codex:list-session-children", {
    parentSessionId,
    ...(targetId ? { targetId } : {})
  });


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
