/**
 * 与后端共享的领域类型（**只覆盖主流程用到的那部分**）。
 *
 * # 为什么手写而不是自动生成
 *
 * Rust 侧的命令面是 `aivc-ipc` 里的一张表，理论上可以生成 TS 类型。但生成器本身要维护，
 * 而现在真正需要对齐的只有主流程这几个形状。**先把类型写在这一处**，等命令面稳定了再
 * 上生成——反过来做，等于在会变的东西上盖楼。
 *
 * # 命名
 *
 * Rust 侧用 `#[serde(rename_all = "camelCase")]`，因此这里一律 camelCase，
 * 与 JSON 逐字一致。
 */

/** 支持的平台。 */
export type AiProviderId = "codex" | "gemini" | "claude" | "qoder";

/** 平台摘要（`ai:list-providers`）。 */
export type AiProviderSummary = {
  id: AiProviderId;
  label: string;
  /** 能力矩阵，键与 Rust 的 `ProviderCapabilities` 字段一致。 */
  capabilities: Record<string, boolean>;
};

/** 目标类型。 */
export type TargetKind = "local" | "wsl";

/**
 * 一个可操作的目标（`codex:list-targets`）。
 *
 * `available === false` **不代表这个目标不存在**，而代表它现在用不了（发行版进不去、
 * CLI 没装）。`detail` 说明原因，界面必须把它显示出来——否则用户只看到「列表是空的」。
 */
export type AiTarget = {
  id: string;
  provider: AiProviderId;
  label: string;
  kind: TargetKind;
  distro?: string;
  codexHome?: string;
  available: boolean;
  detail?: string;
};

/** 消息角色。 */
export type MessageRole = "user" | "assistant" | "system" | "tool" | "unknown";

/** 一条会话消息。 */
export type AiMessage = {
  role: MessageRole;
  text: string;
  timestamp?: string;
  /** 原始 JSONL 行号（1 起）；只有详情流式读取时会填。 */
  sourceLine?: number;
};

/** 一次 token 用量。 */
export type TokenUsage = {
  inputTokens?: number;
  cachedInputTokens?: number;
  outputTokens?: number;
  reasoningOutputTokens?: number;
  totalTokens?: number;
};

/**
 * 会话用量（工作台显示）。
 *
 * 字段**全部可选**：不同 CLI 记录的信息多寡不一样，缺字段是常态而不是异常——
 * 界面必须能显示「这一项没有数据」，而不是显示 `NaN` 或 `undefined`。
 */
export type SessionUsage = {
  total?: TokenUsage;
  last?: TokenUsage;
  contextWindow?: number;
  contextUsedTokens?: number;
  /** 上下文占用百分比（后端保留 1 位小数）。 */
  contextPercent?: number;
  contextLeftPercent?: number;
  /**
   * 速率限制窗口。
   *
   * 与后端的 `RateLimits` 一一对应：`primary` / `secondary` 各是一个窗口。
   * **字段全部可选**：只有部分 CLI 的会话摘要里带这个信息，缺了就是缺了——
   * 工作台要能显示「这一项没有数据」，而不是显示 `undefined%`。
   */
  rateLimits?: {
    primary?: RateLimitWindow;
    secondary?: RateLimitWindow;
  };
  /** 用量数据的来源。不同 CLI 的统计口径不同，必须留着它。 */
  updatedAt?: string;
  source: string;
};

/** 一个速率限制窗口。 */
export type RateLimitWindow = {
  /** 已用百分比。 */
  usedPercent?: number;
  /** 窗口时长（分钟）。 */
  windowMinutes?: number;
  /** 重置时间。 */
  resetsAt?: string;
};

/**
 * 分支来源。
 *
 * 与后端的 `aivc-core/src/session.rs` 的 `BranchMetadata` 一一对应。
 * 「从此处分支」的锚点是**一条消息**：既记可见序号，也记**原始 JSONL 行号**——
 * 后端要靠行号定位到原始记录，拿可见序号去截会截错位置。
 */
export type BranchMetadata = {
  parentTargetId?: string;
  parentSessionId?: string;
  /** 父会话中选中的可见消息序号（历史字段，保留以兼容既有数据）。 */
  parentMessageIndex?: number;
  /** 父会话中选中的原始 JSONL 行号。 */
  parentMessageLine?: number;
};

/** 用户级会话元数据。 */
export type SessionMetadata = {
  updatedAt?: string;
  customTitle?: string;
  /** 是否收藏。 */
  favorite?: boolean;
  lastOpenedAt?: string;
  /** 这个会话是「从某条消息分支出来」的时候才有。 */
  branch?: BranchMetadata;
};

/** 会话摘要或详情（`codex:list-sessions` / `codex:get-session`）。 */
export type AiSession = {
  id: string;
  title: string;
  sourceTitle?: string;
  cwd?: string;
  createdAt?: string;
  updatedAt?: string;
  model?: string;
  cliVersion?: string;
  filePath: string;
  messageCount: number;
  /**
   * `messageCount` 是**下界**而不是精确总数。
   *
   * 列表模式只读文件的头部窗口，数出来的条数是下界：数到 2 条，这个会话就**至少**
   * 有 2 条（它可能有 80 条）。只有详情路径（全量解析）才是精确值。
   *
   * 这是**数据自带**的解读方式，不是消费方的约定——见 `messageCountLabel`。
   */
  messageCountIsLowerBound?: boolean;
  preview: AiMessage[];
  metadata?: SessionMetadata;
  /** 用量。列表模式下可能没有（只有详情才带完整用量）。 */
  usage?: SessionUsage;
};

/** 一页消息（`codex:get-session-messages-page`）。 */
export type SessionMessagePage = {
  offset: number;
  messages: AiMessage[];
  hasMore: boolean;
  /** 会话的可见消息**总数**；`0` 表示本次无从得知。详情路径是全量解析，所以这里是精确值。 */
  messageCount: number;
};

/** 会话视图。 */
export type SessionView = "active" | "trash";

/** 导出格式。与 Rust 侧 `ExportFormat` 的字符串取值逐字对应。 */
export type ExportFormat = "markdown" | "json" | "html";

/** 导出结果。`filePath` 是**实际**落点——没指定路径时由后端决定。 */
export type ExportResult = {
  filePath: string;
  format: string;
};

/**
 * 变更类操作的会话引用。
 *
 * `filePath` 是提示：后端拿到它会先做目录围栏校验，校验不过再按编号回退查找。
 * 因此**传上它更准**，但传错也不会删错东西——围栏会拦住。
 */
export type SessionMutationRef = {
  id: string;
  filePath?: string;
};

/** 本地网关端口状态（`gateway:get-port`）。 */
export type GatewayPortStatus = {
  enabled: boolean;
  configuredPort: number;
  activePort: number;
};

/* ── 终端 ─────────────────────────────────────────────── */

/** 终端状态。与 Rust 侧 `TerminalStatus` 的 kebab-case 序列化逐字对应。 */
export type TerminalStatus =
  | "running"
  | "awaiting-confirmation"
  | "completed"
  | "error";

/** 需要提醒用户的事件。 */
export type TerminalAttention = {
  kind: TerminalStatus;
  title: string;
  detail?: string;
  /**
   * 这条提醒是什么时候来的（毫秒）。
   *
   * 界面自己盖的时间戳，后端不发这个字段——它只用于**排序**：
   * 「会话提醒」里最新的一条要在最上面，否则用户得自己找哪条是刚发生的。
   */
  updatedAt?: number;
};

/** `terminal:status` 事件的载荷。 */
export type TerminalStatusPayload = {
  terminalId: string;
  status: TerminalStatus;
  attention?: TerminalAttention;
};

/** `terminal:data` 事件的载荷：**原始字节**，不是文本。 */
export type TerminalDataPayload = {
  terminalId: string;
  data: number[];
};

/* ── 快捷访问 ─────────────────────────────────────────── */

/** 快捷访问里的一项：会话 + 它属于哪个目标。 */
export type SessionQuickAccessItem = {
  target: AiTarget;
  session: AiSession;
  favorite: boolean;
  lastOpenedAt?: string;
};

/**
 * 快捷访问：收藏与最近打开。
 *
 * 两项**互斥**（收藏的不再出现在「最近」里），后端已经这样算好——前端不要再去重，
 * 否则会掩盖后端语义变化。
 */
export type SessionQuickAccess = {
  favorites: SessionQuickAccessItem[];
  recent: SessionQuickAccessItem[];
};

/* ── 供应商 ───────────────────────────────────────────── */

/** 一个 API 供应商（`vendor:list`）。 */
export type ApiVendor = {
  id: string;
  providerId: AiProviderId;
  name: string;
  apiBaseUrl: string;
  /** 候选池排序。 */
  sort: number;
  enabled: boolean;
};

/* ── 技能 ─────────────────────────────────────────────── */

/**
 * 一个已安装技能（`skill:list`）。
 *
 * 后端返回的是**手拼的 JSON**（`aivc-ipc/src/skills.rs` 里的 `to_values`），不是
 * 某个 Rust 结构体序列化的结果。字段名因此没有类型层面上的约束，改动不会有编译错误
 * ——这一点值得记下：它意味着这份 TS 类型只能靠人对齐。
 */
export type SkillEntry = {
  name: string;
  description: string;
  path: string;
  enabled: boolean;
  sourceName?: string;
};

/* ── 工作区预设 ───────────────────────────────────────── */

/** 一个工作区预设（`workspace:list-presets`）。 */
export type WorkspacePreset = {
  id: string;
  name: string;
  cwd: string;
  targetKind?: TargetKind;
  prompt?: string;
  cliArgs?: string;
  updatedAt: string;
};

/* ── 网关 ─────────────────────────────────────────────── */

/** 供应商健康快照（`gateway:get-vendor-health`）。 */
export type GatewayVendorHealth = {
  vendorId: string;
  providerId: AiProviderId;
  status: string;
  failureCount: number;
  successCount: number;
  failureRate: number;
  lastFailureAt?: string;
  lastSuccessAt?: string;
  circuitUntil?: string;
  lastFailureReason?: string;
};

/** 用量总览（`gateway:get-usage-summary`）。 */
export type UsageSummary = {
  requestCount: number;
  successCount: number;
  failureCount: number;
  switchedCount: number;
  retryCount?: number;
  averageDurationMs?: number;
  /** token 分项。缺字段是常态——不同上游记录的信息多寡不一样。 */
  inputTokens?: number;
  outputTokens?: number;
  cachedInputTokens?: number;
  reasoningTokens?: number;
  totalTokens?: number;
  costUsd?: number;
  periodStart: string;
  periodEnd: string;
};

/**
 * 一个维度的用量汇总（`UsageReport.vendors` / `.models`）。
 *
 * `key` 是维度键（供应商编号 / 模型名），`label` 是展示名。二者**都要留着**：
 * 供应商可能已被删除，那时 `label` 是占位文案而 `key` 才是真实身份。
 */
export type UsageDimension = {
  key: string;
  label: string;
  providerId: AiProviderId;
  requestCount: number;
  successCount: number;
  failureCount: number;
  switchedCount: number;
  retryCount: number;
  averageDurationMs: number;
  inputTokens?: number;
  outputTokens?: number;
  cachedInputTokens?: number;
  reasoningTokens?: number;
  totalTokens?: number;
  costUsd?: number;
};

/** 用量报表（`gateway:get-usage-report`）：总览 + 按供应商 + 按模型。 */
export type UsageReport = {
  summary: UsageSummary;
  vendors: UsageDimension[];
  models: UsageDimension[];
};

/** 一条失败诊断（`gateway:get-failure-diagnostics`）。 */
export type FailureDiagnostic = {
  vendorId: string;
  providerId: AiProviderId;
  outcome: string;
  upstreamStatus?: number;
  retryCount: number;
  durationMs: number;
  errorCode?: string;
  errorMessage?: string;
  createdAt: string;
};

/** 诊断分页。页码从 1 起。 */
export type FailureDiagnosticsPage = {
  items: FailureDiagnostic[];
  total: number;
  page: number;
  pageSize: number;
};

/**
 * 余额快照（`vendor:refresh-balance(s)` 的 `balance` 字段）。
 *
 * 全部可空：上游各式各样，很多中转站根本不返回余额。缺字段要显示「未提供」，
 * 而不是 `0`——那会让人以为余额真的用光了。
 */
export type VendorBalance = {
  remaining?: number;
  total?: number;
  used?: number;
  unit?: string;
  planName?: string;
  isValid?: boolean;
};

/** 一个供应商的余额结果。失败时带 `error`（批量刷新**不会**因为一家失败就整体失败）。 */
export type VendorBalanceResult = {
  vendorId: string;
  balance?: VendorBalance;
  error?: string;
};

/** 一个可查询的模型（`vendor:list-models`）。 */
export type VendorModel = {
  id: string;
  name?: string;
};

/** 读取到的一份配置文件（`vendor:read-configs`）。 */
export type VendorConfigFile = {
  path: string;
  content: string;
};

/** 一条用量区间选择。界面提供几个常用区间，也允许自定义。 */
export type UsageRangeKey = "today" | "7d" | "30d" | "all" | "custom";

/**
 * 一条网关请求日志（`gateway:query-logs`）。
 *
 * 后端**刻意**用显式映射固定了字段名（见 `aivc-ipc/src/gateway.rs` 的
 * `log_entry_value` 注释：避免以后给仓储读取模型加 `derive` 就悄悄改变契约）。
 * 这里只声明界面要用的字段；多出来的字段在运行时存在但被忽略。
 */
export type GatewayLogEntry = {
  requestId: string;
  createdAt: string;
  vendorId: string;
  providerId: AiProviderId;
  outcome: string;
  upstreamStatus?: number;
  /**
   * 错误代码。
   *
   * 后端**特意**加了这一段（见 `log_entry_value` 的注释）：「路由不存在 / 鉴权失败」
   * 记的是 `outcome: ok`（那不是供应商的错），于是它与一次真正转发成功的记录
   * 只差这一个字段。少了它，查「CLI 为什么失败」的人在日志里只能看到一串 `ok`。
   */
  errorCode?: string;
};

/** 故障规则作用域。 */
export type FailoverRuleScope = "global" | "provider" | "vendor";

/**
 * 一条故障识别规则（`gateway:list-failover-rules`）。
 *
 * `pattern` 是**不区分大小写的包含匹配**，不是正则——界面提示里要写清楚，
 * 否则用户会往里写正则然后疑惑为什么匹配不上。
 */
export type FailoverRule = {
  id: string;
  scope: FailoverRuleScope;
  providerId?: AiProviderId;
  vendorId?: string;
  pattern: string;
  statusCodes?: number[];
  customResponseStatus?: number;
  customResponseBody?: string;
  enabled: boolean;
  createdAt: string;
  updatedAt: string;
};

/** 外部 API 状态（`gateway:get-external-api-status`）。 */
export type ExternalApiStatus = {
  enabled: boolean;
  /** 只暴露「是否已设置」，**后端绝不返回令牌本身**。 */
  hasToken: boolean;
};

/* ── CLI 环境 ─────────────────────────────────────────── */

/** CLI 环境检测结果（`cli:check-environment`）。 */
export type CliEnvironmentStatus = {
  providerId: AiProviderId;
  scope: "local" | "wsl";
  hasNode: boolean;
  hasNpm: boolean;
  hasCli: boolean;
  nodeMeetsMinimum: boolean;
  canInstallCli: boolean;
  canInstallNodeWithNvm: boolean;
  nodeVersion?: string;
  /** 后端给出的**可执行建议**，直接显示即可。 */
  messages: string[];
};
