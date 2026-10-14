//! 错误分类与用户可见消息。
//!
//! # 为什么文案是契约
//!
//! 渲染层直接展示 `error.message`，因此这些中文字符串是**功能一致的一部分**，
//! 必须与 TypeScript 版逐字一致。任何改动都等同于改变用户可见行为。
//! `tests/error_messages.rs` 用 `insta` 快照固化全部文案，防止无意漂移。
//!
//! # 分层
//!
//! 每个业务域一个子枚举（[`SessionError`] / [`GatewayError`] / [`WslError`] …），
//! 由顶层 [`Error`] 统一收敛。库 crate 一律返回 [`Result`]，禁止 `unwrap`；
//! 只有 `aivc-app` 这一层才允许使用 `anyhow`。

use std::path::PathBuf;

/// 本工程的统一结果别名。
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// 顶层错误：按业务域收敛全部失败路径。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// IPC 入参校验失败。
    #[error(transparent)]
    Ipc(#[from] IpcError),

    /// 会话资产相关失败。
    #[error(transparent)]
    Session(#[from] SessionError),

    /// 目标（本机 / WSL 发行版）解析与探测失败。
    #[error(transparent)]
    Target(#[from] TargetError),

    /// 配置与运行根派生失败。
    #[error(transparent)]
    Config(#[from] ConfigError),

    /// 持久化失败。
    #[error(transparent)]
    Database(#[from] DatabaseError),

    /// WSL 子进程执行失败。
    #[error(transparent)]
    Wsl(#[from] WslError),

    /// 本地文件系统失败。
    #[error(transparent)]
    Io(#[from] std::io::Error),

    /// 本地 Gateway 失败。
    #[error(transparent)]
    Gateway(#[from] GatewayError),

    /// 终端运行时失败。
    #[error(transparent)]
    Terminal(#[from] TerminalError),

    /// Skill 管理失败。
    #[error(transparent)]
    Skill(#[from] SkillError),

    /// CLI 环境探测与安装失败。
    #[error(transparent)]
    CliEnv(#[from] CliEnvError),

    /// 供应商配置失败。
    #[error(transparent)]
    Vendor(#[from] VendorError),

    /// 平台不支持该能力。
    ///
    /// 对应 TypeScript 版的「保留显式拒绝实现」约定：能力子 trait 在编译期
    /// 已排除该路径，但对外仍需给出与现网一致的拒绝语义与文案。
    #[error("{provider} 不支持{capability}。")]
    Unsupported {
        /// 平台标识，例如 `Qoder CN`。
        provider: String,
        /// 能力名称，例如「分支」。
        capability: String,
    },
}

/// 会话资产相关失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SessionError {
    /// 活跃会话中找不到指定编号。
    #[error("未找到会话：{id}")]
    NotFound {
        /// 会话编号。
        id: String,
    },

    /// 回收站中找不到指定编号。
    #[error("未在回收站找到会话：{id}")]
    NotFoundInTrash {
        /// 会话编号。
        id: String,
    },

    /// 提供的文件路径不在期望目录内。
    #[error("提供的会话文件路径不在允许目录内")]
    PathNotAllowed,

    /// 提供的文件路径不在期望目录内（携带平台专属文案）。
    ///
    /// 各平台的目录名不同（`sessions` / `projects` / `tmp`），因此拒绝文案必须由调用方
    /// 给出。把文案做成参数而不是为每个平台加一个变体，避免错误枚举随平台数量膨胀。
    #[error("{message}")]
    SessionPathNotAllowed {
        /// 面向用户的拒绝文案。
        message: String,
    },

    /// 试图把 `sessions` 目录之外的文件移入回收站。
    #[error("拒绝将 sessions 目录之外的文件移动到回收站")]
    DeleteOutsideSessionsRoot,

    /// 试图恢复回收站目录之外的文件。
    #[error("拒绝恢复回收站目录之外的文件")]
    RestoreOutsideTrashRoot,

    /// 试图永久删除回收站目录之外的文件。
    #[error("拒绝删除回收站目录之外的文件")]
    PurgeOutsideTrashRoot,

    /// 试图读取 Codex 会话目录之外的文件。
    #[error("拒绝读取 Codex 会话目录之外的文件。")]
    ReadOutsideCodexSessionsRoot,

    /// 恢复目标位置已存在同名文件。
    #[error("原位置已存在同名会话文件，无法恢复。")]
    RestoreConflict,

    /// 文件内容与声明的会话编号不匹配。
    #[error("会话文件与会话编号不匹配：{id}")]
    SessionIdMismatch {
        /// 会话编号。
        id: String,
    },

    /// 会话文件超过一次性读取上限。
    #[error("会话文件过大，拒绝一次性读取：{name}")]
    FileTooLarge {
        /// 文件名（不含目录）。
        name: String,
    },

    /// 分支位置不是可见会话消息。
    #[error("分支位置不是可见会话消息。")]
    BranchAnchorNotMessage,

    /// 分支位置已失效（找不到所属外层轮次）。
    #[error("分支位置已失效，请重新打开会话详情后再试。")]
    BranchAnchorStale,

    /// 所选消息不是已完成轮次的最后一条助手回复。
    #[error("只能从已完成轮次的最后一条助手回复创建分支，请重新加载消息。")]
    BranchAnchorNotTurnEnd,

    /// 无法确认官方创建的分支边界。保留原生分支供用户恢复。
    #[error(
        "Codex 创建了新会话 {id}，但未能确认分支轮次边界。请升级该目标的 Codex CLI 后重试，或在历史会话中检查这个新会话。"
    )]
    ForkBoundaryUnconfirmed {
        /// 已创建的原生分支编号。
        id: String,
    },

    /// 未选择任何前置上下文。
    #[error("请选择至少一条前置上下文后再创建分支。")]
    BranchNeedsContext,

    /// 分支只能从会话目录或回收站内的会话生成。
    #[error("分支只能从会话目录或回收站内的会话生成。")]
    BranchSourceNotAllowed,

    /// Codex 官方服务未返回可恢复的分支线程。
    #[error("Codex 未返回可恢复的分支线程。")]
    ForkUnavailable,

    /// Codex 返回的分支编号无效。
    #[error("Codex 返回的分支会话编号无效，已停止打开分支。")]
    ForkIdInvalid,

    /// Codex 返回的分支文件不在当前会话目录内。
    #[error("Codex 返回的分支文件不在当前会话目录内，已停止打开分支。")]
    ForkPathOutsideRoot,

    /// Codex 返回的分支文件与原会话相同。
    #[error("Codex 返回的分支文件与原会话相同，已停止打开分支。")]
    ForkSameAsParent,

    /// 复制会话失败（无法解析出结果）。
    #[error("复制会话失败。")]
    DuplicateFailed,

    /// Claude 分支失败（无法解析出结果）。
    #[error("创建 Claude 分支失败。")]
    ClaudeBranchFailed,

    /// Claude 分支可保留的消息不足。
    #[error("创建 Claude 分支失败：原始 jsonl 中可保留消息不足。")]
    BranchInsufficientMessages,

    /// Claude 复制失败（无法解析出结果）。
    #[error("复制 Claude Code 会话失败。")]
    ClaudeDuplicateFailed,

    /// Claude 复制失败：源文件缺少会话编号字段。
    #[error("复制 Claude Code 会话失败：缺少会话编号。")]
    DuplicateMissingSessionId,

    /// Gemini 源会话缺少元数据记录。
    #[error("Gemini 源会话缺少 session 元数据。")]
    GeminiMissingMetadata,

    /// Gemini 源会话的可见消息不足。
    #[error("Gemini 源会话上下文不足，无法创建分支。")]
    GeminiInsufficientContext,

    /// Gemini 复制失败：源文件缺少元数据记录。
    #[error("复制 Gemini 会话失败：缺少会话元数据。")]
    GeminiDuplicateMissingMetadata,

    /// 移动会话时目标位置已存在。
    #[error("目标会话已存在，已拒绝覆盖。")]
    MoveTargetExists,

    /// Qoder CLI 不支持该操作。
    ///
    /// 与其他平台不同：Qoder 的部分能力**必须**在它自己的 CLI 内完成，应用只做只读
    /// 展示与回收站管理。因此这里给出的是引导性提示而不是「不支持」。
    #[error("Qoder CLI 当前不支持在本应用中{action}，请在 Qoder CLI 内执行。")]
    QoderUnsupported {
        /// 操作名（如「分支会话」）。
        action: String,
    },

    /// 复制会话时源文件缺少 `session_meta` 记录。
    #[error("复制会话失败：缺少 session_meta 记录。")]
    DuplicateMissingMeta,

    /// 同一历史会话已存在写入者。
    #[error("该历史会话正在打开，请勿重复操作。")]
    ResumeInFlight,
}

/// 目标解析与探测失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TargetError {
    /// 未知 AI 平台标识。
    #[error("未知 AI 平台：{id}")]
    UnknownProvider {
        /// 平台标识。
        id: String,
    },

    /// 未知终端平台标识。
    #[error("未知终端平台：{id}")]
    UnknownTerminalProvider {
        /// 目标标识。
        id: String,
    },

    /// 缺少 WSL 发行版信息。
    #[error("缺少 WSL 发行版。")]
    MissingWslDistro,

    /// 目标编号无法解析为可用目标。
    ///
    /// 与 [`TargetError::UnknownProvider`] 的区别：那个是平台标识非法，这个是**编号
    /// 合法但当前不存在**（例如对应的 WSL 发行版已被删除）。
    #[error("未解析到目标：{id}")]
    NotResolved {
        /// 目标编号。
        id: String,
    },
}

/// 配置与路径派生失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// 运行根尚未初始化。
    #[error("应用运行根未初始化。")]
    RuntimeRootUnset,

    /// 数据库路径尚未初始化。
    #[error("会话数据库路径未初始化。")]
    DatabasePathUnset,

    /// 工作目录为空。
    #[error("工作目录不能为空。")]
    EmptyWorkingDirectory,

    /// 供应商配置文件路径不在允许范围。
    #[error("配置文件路径不在允许范围：{path}")]
    ConfigPathNotAllowed {
        /// 被拒绝的路径。
        path: String,
    },

    /// CLI 参数模板存在未闭合引号。
    #[error("CLI 参数模板存在未闭合引号。")]
    UnclosedCliArgQuote,
}

/// 持久化失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DatabaseError {
    /// SQLite 驱动错误。
    #[error("数据库操作失败：{message}")]
    Sqlite {
        /// 底层错误消息。
        message: String,
    },

    /// 迁移失败。
    #[error("数据库迁移失败（版本 {version}）：{message}")]
    Migration {
        /// 迁移版本号。
        version: u32,
        /// 失败原因。
        message: String,
    },

    /// 写入队列已关闭。
    #[error("数据库写入通道已关闭。")]
    WriterClosed,
}

/// WSL 子进程执行失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WslError {
    /// 未找到 `wsl.exe`。
    #[error("未找到 wsl.exe。")]
    ExecutableNotFound,

    /// 子进程超时被强制终止。
    #[error("WSL 操作超时（{timeout_ms} ms）：{label}")]
    Timeout {
        /// 超时上限（毫秒）。
        timeout_ms: u64,
        /// 操作描述。
        label: String,
    },

    /// 输出超过上限。
    #[error("WSL 命令输出超过限制（{limit} bytes）：{distro}")]
    OutputTooLarge {
        /// 上限（字节）。
        limit: usize,
        /// 发行版名称。
        distro: String,
    },

    /// 读取文件超过上限。
    #[error("WSL 文件超过读取限制（{limit} bytes）：{path}")]
    FileTooLarge {
        /// 上限（字节）。
        limit: usize,
        /// 文件路径。
        path: String,
    },

    /// 命令以非零码退出。
    #[error("{message}")]
    CommandFailed {
        /// 已清洗的失败说明。
        message: String,
    },

    /// 持久 worker 协议异常。
    #[error("WSL 持久 worker 协议异常：{message}")]
    WorkerProtocol {
        /// 异常说明。
        message: String,
    },
}

/// 本地 Gateway 失败。
///
/// 其中「面向 CLI 客户端的英文响应体」与「面向用户的中文提示」刻意分开：
/// 前者由 [`GatewayError::client_response`] 提供，后者是 `Display` 输出。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GatewayError {
    /// 网关已关闭。
    #[error("本地 Gateway 已关闭，请先在网关设置中启用。")]
    Disabled,

    /// 未返回有效监听地址。
    #[error("本地 Gateway 未返回有效监听地址。")]
    NoListenAddress,

    /// 固定端口被占用且不允许回退。
    #[error("本地 Gateway 端口被占用：{port}")]
    PortInUse {
        /// 配置端口。
        port: u16,
    },

    /// 仍有活动路由，拒绝停止常驻监听。
    #[error("仍有终端正在使用网关，请先关闭终端。")]
    RoutesActive,

    /// 路由不存在。
    #[error("gateway route not found")]
    RouteNotFound,

    /// 本地令牌校验失败。
    #[error("gateway unauthorized")]
    Unauthorized,

    /// 对外 API 未启用。
    #[error("gateway external API is disabled")]
    ExternalApiDisabled,

    /// 没有可用供应商。
    #[error("没有可用的供应商。")]
    NoAvailableVendor,

    /// 供应商不可用（未启用 / 缺 Key / 缺地址）。
    #[error("gateway vendor unavailable")]
    VendorUnavailable,

    /// 供应商协议与终端类型不匹配。
    #[error("供应商协议与终端类型不匹配。")]
    VendorProviderMismatch,

    /// 供应商地址非法（协议、查询参数或片段）。
    #[error("{message}")]
    InvalidVendorUrl {
        /// 具体原因。
        message: String,
    },

    /// 供应商地址指向本地 Gateway 自身。
    #[error("供应商地址不能指向本地 Gateway。")]
    VendorUrlIsLocalGateway,

    /// 上游请求失败。
    #[error("gateway upstream request failed")]
    UpstreamRequestFailed,

    /// 上游响应超时。
    #[error("上游响应超时。")]
    UpstreamTimeout,

    /// 请求体超过本地限制。
    #[error("请求体超过本地 Gateway 限制。")]
    BodyTooLarge,

    /// 通用网关失败。
    #[error("Gateway 请求失败。")]
    GenericFailure,

    /// 故障规则不存在。
    #[error("Gateway 故障规则不存在。")]
    FailoverRuleNotFound,

    /// 故障规则数量超过上限。
    #[error("Gateway 故障规则最多保存 {limit} 条。")]
    FailoverRuleLimit {
        /// 条数上限。
        limit: usize,
    },

    /// 故障规则作用域无效。
    #[error("Gateway 故障规则作用域无效。")]
    FailoverRuleScopeInvalid,

    /// 故障规则缺少匹配条件。
    #[error("请至少配置错误文本或 HTTP 状态码。")]
    FailoverRuleNeedsCondition,

    /// 故障规则模式过长。
    #[error("Gateway 故障规则不能超过 {limit} 个字符。")]
    FailoverRulePatternTooLong {
        /// 字符上限。
        limit: usize,
    },

    /// 故障规则状态码非法。
    #[error("HTTP 状态码必须是 400 到 599 之间的整数。")]
    FailoverRuleStatusCodeInvalid,

    /// 自定义响应的状态码与正文未同时配置。
    #[error("自定义返回状态码和内容必须同时配置。")]
    FailoverRuleCustomResponseIncomplete,

    /// 自定义响应状态码非法。
    #[error("自定义返回状态码必须是 100 到 599 之间的整数。")]
    FailoverRuleCustomStatusInvalid,

    /// 自定义响应正文过大。
    #[error("自定义返回内容不能超过 {limit_kb} KB。")]
    FailoverRuleCustomBodyTooLarge {
        /// 上限（KB）。
        limit_kb: usize,
    },

    /// 自定义响应正文不是合法 JSON。
    #[error("自定义返回内容必须是合法 JSON。")]
    FailoverRuleCustomBodyInvalidJson,

    /// Provider 级与供应商级规则缺少 Provider。
    #[error("Provider 级和供应商级规则必须指定 Provider。")]
    FailoverRuleProviderRequired,

    /// 供应商级规则缺少供应商。
    #[error("供应商级规则必须指定供应商。")]
    FailoverRuleVendorRequired,

    /// 全局规则不应指定 Provider 或供应商。
    #[error("全局规则不能指定 Provider 或供应商。")]
    FailoverRuleScopeConflict,

    /// Provider 级规则不应指定供应商。
    #[error("Provider 级规则不能指定供应商。")]
    FailoverRuleProviderScopeConflict,

    /// 规则 ID 为空。
    #[error("规则 ID 不能为空。")]
    FailoverRuleIdEmpty,
}

impl GatewayError {
    /// 面向 CLI 客户端的响应体文本（与 TypeScript 版 `GATEWAY_ERROR_MESSAGES` 一致）。
    ///
    /// 与 `Display`（面向用户的中文提示）刻意区分：CLI 客户端解析的是前者。
    #[must_use]
    pub fn client_response(&self) -> &'static str {
        match self {
            Self::RouteNotFound => "gateway route not found",
            Self::Unauthorized => "gateway unauthorized",
            Self::ExternalApiDisabled => "gateway external API is disabled",
            Self::VendorUnavailable => "gateway vendor unavailable",
            Self::UpstreamTimeout => "上游响应超时。",
            Self::NoAvailableVendor => "没有可用的供应商。",
            Self::GenericFailure => "Gateway 请求失败。",
            Self::VendorUrlIsLocalGateway => "供应商地址不能指向本地 Gateway。",
            Self::BodyTooLarge => "请求体超过本地 Gateway 限制。",
            // 其余分支（含 UpstreamRequestFailed 与本地配置类错误）统一回退到
            // 通用上游失败文案，避免调用方拿到空响应体。
            _ => "gateway upstream request failed",
        }
    }

    /// 该错误对应的 HTTP 状态码。
    #[must_use]
    pub fn http_status(&self) -> u16 {
        match self {
            Self::RouteNotFound => 404,
            Self::Unauthorized => 401,
            Self::ExternalApiDisabled | Self::VendorUnavailable | Self::NoAvailableVendor => 503,
            Self::UpstreamTimeout => 504,
            _ => 502,
        }
    }
}

/// 终端运行时失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TerminalError {
    /// 未找到 Codex CLI。
    #[error("未找到 Codex CLI。请在新电脑安装 Codex CLI，并重启应用使 PATH 生效。")]
    CodexCliNotFound,

    /// PTY 创建失败。
    #[error("创建终端失败：{message}")]
    SpawnFailed {
        /// 底层错误说明。
        message: String,
    },

    /// 终端会话不存在。
    #[error("终端会话不存在：{id}")]
    NotFound {
        /// 终端编号。
        id: String,
    },

    /// 向终端写入失败（通常是进程已退出）。
    #[error("写入终端失败：{message}")]
    WriteFailed {
        /// 底层错误说明。
        message: String,
    },

    /// 调整终端尺寸失败。
    #[error("调整终端尺寸失败：{message}")]
    ResizeFailed {
        /// 底层错误说明。
        message: String,
    },

    /// 终止终端进程失败。
    #[error("终止终端失败：{message}")]
    KillFailed {
        /// 底层错误说明。
        message: String,
    },
}

/// Skill 管理失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SkillError {
    /// 源目录缺少 `SKILL.md`。
    #[error("未找到 SKILL.md。")]
    ManifestMissing,

    /// `SKILL.md` 超过上限。
    #[error("SKILL.md 过大，拒绝导入。")]
    ManifestTooLarge,

    /// `SKILL.md` 缺少 YAML frontmatter。
    #[error("SKILL.md 必须以 YAML frontmatter 开头。")]
    ManifestFrontmatterMissing,

    /// `SKILL.md` frontmatter 缺少 `name`。
    #[error("SKILL.md 缺少 frontmatter 字段：name。")]
    ManifestNameMissing,

    /// `SKILL.md` frontmatter 缺少 `description`。
    #[error("SKILL.md 缺少 frontmatter 字段：description。")]
    ManifestDescriptionMissing,

    /// 目标目录已存在。
    #[error("目标目录已存在：{path}")]
    DestinationExists {
        /// 目标路径。
        path: PathBuf,
    },
}

/// CLI 环境探测与安装失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CliEnvError {
    /// 命令执行失败。
    #[error("命令执行失败（{command}）：{message}")]
    CommandFailed {
        /// 被执行的命令。
        command: String,
        /// 已清洗的失败说明。
        message: String,
    },

    /// 命令执行超时。
    #[error("命令执行超时（{timeout_ms} ms）：{command}")]
    Timeout {
        /// 超时上限（毫秒）。
        timeout_ms: u64,
        /// 被执行的命令。
        command: String,
    },

    /// 目标环境不满足安装前提。
    #[error("当前环境无法安装 CLI：{message}")]
    PrerequisiteUnmet {
        /// 具体原因。
        message: String,
    },
}

/// 供应商配置失败。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum VendorError {
    /// 供应商名称为空。
    #[error("供应商名称不能为空。")]
    NameRequired,

    /// API Key 为空（新建供应商时必填）。
    #[error("API Key 不能为空。")]
    ApiKeyRequired,

    /// API 地址为空。
    #[error("API 请求地址不能为空。")]
    ApiBaseUrlRequired,

    /// 费率非法。
    ///
    /// `label` 为「输入费率」或「输出费率」——现网把字段名拼进消息，因此必须保留。
    #[error("{label}必须是非负数，且最多保留 2 位小数。")]
    PricingInvalid {
        /// 字段展示名。
        label: &'static str,
    },

    /// 查询配置的 JSON 体积超过上限。
    #[error("供应商查询配置过大。")]
    QueryConfigTooLarge,

    /// 自定义认证 Header 名称非法。
    #[error("自定义认证 Header 名称无效。")]
    AuthHeaderNameInvalid,

    /// 查询认证方式非法。
    #[error("查询认证方式无效。")]
    AuthModeInvalid,

    /// 自定义请求头非法。
    #[error("自定义请求头无效。")]
    HeaderInvalid,

    /// 余额查询模板非法。
    #[error("余额查询模板无效。")]
    BalanceTemplateInvalid,

    /// 余额换算倍率越界。
    #[error("余额换算倍率必须大于 0 且不超过 1000000。")]
    BalanceMultiplierInvalid,

    /// 同一平台内供应商重名。
    #[error("供应商名称已存在：{name}")]
    DuplicateName {
        /// 冲突的供应商名称。
        name: String,
    },

    /// 同一平台内排序值冲突。
    #[error("排序值 {sort} 已被占用。")]
    DuplicateSort {
        /// 冲突的排序值。
        sort: i64,
    },

    /// 供应商地址无法解析。
    #[error("供应商地址无效。")]
    BaseUrlInvalid,

    /// 供应商地址的协议不是 HTTP/HTTPS。
    ///
    /// 其他协议会让转发变成任意协议请求，是一个典型的 SSRF 面。
    #[error("供应商地址必须使用 HTTP 或 HTTPS。")]
    BaseUrlSchemeInvalid,

    /// 供应商地址携带了查询参数或片段。
    ///
    /// 拼接上游 URL 时它们会被覆盖，静默丢弃用户配置的参数。
    #[error("供应商地址不能包含查询参数或片段。")]
    BaseUrlHasQueryOrFragment,

    /// 供应商不存在。
    #[error("供应商不存在。")]
    NotFound,

    /// 没有启用任何配置文件。
    #[error("请至少启用一个配置文件。")]
    NoEnabledConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_error_messages_match_typescript_wording() {
        assert_eq!(
            SessionError::NotFound { id: "abc".into() }.to_string(),
            "未找到会话：abc"
        );
        assert_eq!(
            SessionError::DeleteOutsideSessionsRoot.to_string(),
            "拒绝将 sessions 目录之外的文件移动到回收站"
        );
        assert_eq!(
            SessionError::RestoreConflict.to_string(),
            "原位置已存在同名会话文件，无法恢复。"
        );
    }

    #[test]
    fn gateway_client_response_is_english_for_cli_facing_errors() {
        // CLI 客户端解析的是英文响应体；用户提示是中文，两者不能混用。
        assert_eq!(
            GatewayError::RouteNotFound.client_response(),
            "gateway route not found"
        );
        assert_eq!(GatewayError::RouteNotFound.http_status(), 404);
        assert_eq!(GatewayError::Unauthorized.http_status(), 401);
        assert_eq!(GatewayError::UpstreamTimeout.http_status(), 504);
        assert_eq!(GatewayError::VendorUnavailable.http_status(), 503);
    }

    #[test]
    fn domain_errors_convert_into_top_level_error() {
        let err: Error = SessionError::ResumeInFlight.into();
        assert_eq!(err.to_string(), "该历史会话正在打开，请勿重复操作。");
    }

    #[test]
    fn unsupported_error_keeps_explicit_rejection_semantics() {
        let err = Error::Unsupported {
            provider: "Qoder CN".into(),
            capability: "分支".into(),
        };
        assert_eq!(err.to_string(), "Qoder CN 不支持分支。");
    }
}

/// IPC 入参校验失败。
///
/// # 为什么文案原样保留
///
/// 这些消息会直接显示给用户，且现网已经把它们当作稳定契约（前端测试断言过）。
/// 因此不重新措辞、不统一标点——「参数无效：targetId」这样的半角冒号也是契约的一部分。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IpcError {
    /// 参数类型或取值非法。
    #[error("参数无效：{name}")]
    InvalidParameter {
        /// 参数名。
        name: String,
    },

    /// 带完整文案的参数错误（取值范围类）。
    #[error("{message}")]
    InvalidValue {
        /// 完整文案。
        message: String,
    },

    /// IPC 命令执行失败。
    #[error("{message}")]
    CommandFailed {
        /// 完整文案。
        message: String,
    },
}
