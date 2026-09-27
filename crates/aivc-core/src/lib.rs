//! AI Visual Console —— 领域核心层。
//!
//! # 职责
//!
//! 本 crate 是整个重构工程的**最内层**：只放实体、值对象、错误分类与端口（trait），
//! 不含任何 I/O、平台调用或框架依赖。所有外部能力（文件系统、WSL、SQLite、HTTP、
//! PTY）都由外层基础设施 crate 实现端口，再注入到业务层。
//!
//! 依赖方向：
//!
//! ```text
//! aivc-app ──► aivc-ipc ──► { aivc-provider, aivc-gateway, aivc-terminal, … }
//!                                     │
//!                                     ▼
//!                        aivc-infra-{db,fs,wsl,http} ──► aivc-core
//! ```
//!
//! 任何反向依赖都会被 `cargo xtask check-layers` 在 CI 中拦下。
//!
//! # 功能一致的约束
//!
//! 本 crate 里的字符串与序列化形状**不是实现细节，而是对外契约**：
//!
//! - 错误消息会经 IPC 直接展示给用户（见 [`error`]）；
//! - 会话/目标结构会写入 SQLite 并送往渲染层（见 [`session`]、[`target`]）；
//! - targetId 语法被持久化在设置文件里（见 [`target`]）。
//!
//! 因此改动这些内容等同于改动产品行为，必须同步更新
//! `docs/rust-rewrite/03-parity-matrix-and-migration.md` 的对应条目。

#![forbid(unsafe_code)]

pub mod constants;
pub mod error;
pub mod gateway;
pub mod gateway_usage;
pub mod id;
pub mod metadata;
pub mod path;
pub mod ports;
pub mod preset;
pub mod session;
pub mod shell;
pub mod target;
pub mod text;
pub mod time;
pub mod vendor;

pub use constants::{
    GATEWAY_FAILOVER_DEFAULT_PATTERNS, GATEWAY_FAILOVER_DEFAULT_STATUS_CODES,
    GATEWAY_FAILOVER_RULE_LIMITS, GATEWAY_REQUEST_LIMITS, PAGINATION_DEFAULT_PAGE_SIZE,
    PAGINATION_MAX_PAGE_SIZE, PAGINATION_PAGE_SIZE_OPTIONS, RESPONSE_PROBE_MAX_BYTES,
};
pub use error::{Error, Result};
pub use gateway::{
    FailoverRule, FailoverRuleInput, FailoverRuleScope, GatewayOutcome, GatewayRequestRecord,
    GatewayUsage, GatewayVendorHealth, GatewayVendorHealthStatus, NormalizedFailoverRule,
    sanitize_error_message,
};
pub use gateway_usage::{
    FailureDiagnostic, FailureDiagnosticsPage, FailureOutcomeFilter, RecentFailure,
    UNLABELLED_MODEL, UsageDimension, UsageReport, UsageRow, UsageSummary, aggregate_usage,
    parse_gateway_usage, parse_stored_usage, parse_usage_from_chunk,
};
pub use id::{PresetId, RequestId, RouteId, SessionId, SkillName, TargetId, TerminalId, VendorId};
pub use metadata::{RawSessionMetadata, attach};
pub use path::{
    is_inside_local_path, is_inside_posix_dir, posix_normalize, sanitize_wsl_distro,
    wsl_mount_path_to_windows_path,
};
pub use ports::{
    BoxFuture, Clock, ConflictPolicy, LineControl, LineSink, ReadOptions, ReadStart, SessionStore,
    StoreKind, SystemClock,
};
pub use preset::{NormalizedPresetInput, WorkspacePreset, WorkspacePresetInput};
pub use session::{
    BranchMetadata, BranchOrigin, Message, MessageAnchor, MessagePage, ModelStatus, PageCursor,
    RateLimitUsage, RateLimits, Role, Session, SessionFile, SessionMetadata, SessionUsage,
    SessionView, TokenUsage, UsageSource,
};
pub use shell::{
    assert_allowed_config_path, build_cmd_command, build_posix_shell_command, cmd_quote,
    parse_cli_args, posix_shell_quote,
};
pub use target::{
    Platform, ProviderCapabilities, ProviderId, Target, TargetKind, cli_target_detail_config_only,
    cli_target_detail_found, codex_local_target_detail, codex_wsl_target_detail,
    local_target_label, wsl_target_label,
};
pub use text::{
    TITLE_MAX_LENGTH, clamp_raw, clamp_text, clamp_title, format_session_stamp,
    is_synthetic_user_block, matches_all_terms, matches_session_summary, normalize_search_terms,
    normalize_whitespace, should_keep_message,
};
pub use time::Timestamp;
pub use vendor::{
    ApiVendor, ApiVendorInput, BalanceQueryConfig, BalanceQueryTemplate, BalanceSnapshot,
    BalanceStatus, HttpMethod, ModelQueryConfig, NormalizedVendorInput, QueryAuthMode,
    VendorConfigTemplate, VendorModel, VendorPricing, normalize_vendor_name,
};
