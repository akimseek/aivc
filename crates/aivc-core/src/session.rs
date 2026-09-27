//! 会话资产领域模型。
//!
//! 字段名与现网 `CodexSession` / `CodexMessage` 等类型一一对应（JSON 为 camelCase），
//! 因为整个结构会经 IPC 直接送到渲染层并被写入 SQLite 的 `session_json` 列。
//!
//! # 列表与详情共用同一类型
//!
//! 现网列表项与详情项都是 `CodexSession`，区别只在 `preview` 的长度（列表最多 8 条）
//! 与 `messageCount`（列表为全量计数）。这里刻意保持单一类型，把差异收敛到解析模式
//! （`aivc-provider` 的 `ParseMode`），避免在 wire 层引入两种形状。

use serde::{Deserialize, Serialize};

use crate::id::{SessionId, TargetId};
use crate::time::Timestamp;

/// 消息角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// 用户。
    User,
    /// 助手。
    Assistant,
    /// 系统。
    System,
    /// 工具调用。
    Tool,
    /// 无法归类（现网 `unknown`，常见于 Codex 事件记录）。
    Unknown,
}

/// 一条会话消息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// 角色。
    pub role: Role,
    /// 正文。
    pub text: String,
    /// 该条记录的时间戳。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<Timestamp>,
    /// 原始 JSONL 行号（1 起）。
    ///
    /// 只在详情流式读取时填充；「从此处分支」依赖它精确定位到原始记录，
    /// 因此**不得**用过滤后的可见序号替代。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_line: Option<u64>,
}

/// Token 用量。
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    /// 输入 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// 命中缓存的输入 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u64>,
    /// 输出 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// 推理输出 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_output_tokens: Option<u64>,
    /// 总 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl TokenUsage {
    /// 是否至少有一个字段有值。
    #[must_use]
    pub const fn has_any(&self) -> bool {
        self.input_tokens.is_some()
            || self.cached_input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.reasoning_output_tokens.is_some()
            || self.total_tokens.is_some()
    }
}

/// 速率限制窗口用量。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitUsage {
    /// 已用百分比。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    /// 窗口时长（分钟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u64>,
    /// 重置时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<Timestamp>,
}

impl RateLimitUsage {
    /// 是否至少有一个字段有值。
    #[must_use]
    pub fn has_any(&self) -> bool {
        self.used_percent.is_some() || self.window_minutes.is_some() || self.resets_at.is_some()
    }
}

/// 主 / 次速率限制窗口。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimits {
    /// 主窗口。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary: Option<RateLimitUsage>,
    /// 次窗口。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<RateLimitUsage>,
}

/// 用量数据来源。不同 CLI 的统计口径不同，必须标注来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageSource {
    /// Codex 的 `event_msg.token_count`。
    CodexTokenCount,
    /// Gemini 的消息内 token 字段。
    GeminiMessageTokens,
    /// Claude Code 的消息 `usage` 字段。
    ClaudeMessageUsage,
    /// Qoder CN 的消息 `usage` 字段。
    QoderMessageUsage,
}

/// 会话用量汇总。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsage {
    /// 累计用量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<TokenUsage>,
    /// 最近一轮用量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<TokenUsage>,
    /// 上下文窗口大小。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// 已用上下文 token。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_used_tokens: Option<u64>,
    /// 上下文占用百分比（保留 1 位小数）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_percent: Option<f64>,
    /// 上下文剩余百分比（保留 1 位小数）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_left_percent: Option<f64>,
    /// 速率限制窗口。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<RateLimits>,
    /// 用量最后更新时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
    /// 数据来源。
    pub source: UsageSource,
}

/// 模型与推理配置状态。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    /// 模型名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 推理强度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
    /// 摘要模式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summaries: Option<String>,
    /// 模型提供方。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
}

impl ModelStatus {
    /// 用后出现的值逐字段补齐当前值（现网 `mergeModelStatus` 语义：新值优先，缺省回退旧值）。
    pub fn merge(&mut self, next: &Self) {
        if next.model.is_some() {
            self.model.clone_from(&next.model);
        }
        if next.reasoning.is_some() {
            self.reasoning.clone_from(&next.reasoning);
        }
        if next.summaries.is_some() {
            self.summaries.clone_from(&next.summaries);
        }
        if next.model_provider.is_some() {
            self.model_provider.clone_from(&next.model_provider);
        }
    }

    /// 是否至少有一个字段有值。
    #[must_use]
    pub const fn has_any(&self) -> bool {
        self.model.is_some()
            || self.reasoning.is_some()
            || self.summaries.is_some()
            || self.model_provider.is_some()
    }
}

/// 分支来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BranchOrigin {
    /// 由「从此处分支」创建。
    Branch,
    /// 手工创建。
    Manual,
}

/// 分支元数据。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchMetadata {
    /// 父会话所属目标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_target_id: Option<TargetId>,
    /// 父会话编号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    /// 父会话中选中的可见消息序号（历史字段，保留以兼容既有数据）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_index: Option<u64>,
    /// 父会话中选中的原始 JSONL 行号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_message_line: Option<u64>,
    /// 创建方式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<BranchOrigin>,
}

/// 用户级会话元数据（与 CLI 私有存储无关，只存在本应用库中）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    /// 元数据最后更新时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
    /// 用户自定义标题。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_title: Option<String>,
    /// 分支来源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<BranchMetadata>,
    /// 是否收藏。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    /// 最近打开时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_opened_at: Option<Timestamp>,
}

/// 一个会话（列表项与详情项共用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    /// 会话编号。
    pub id: SessionId,
    /// 展示标题（用户自定义标题优先，其次首条合格用户消息，最后回退编号）。
    pub title: String,
    /// CLI 自己写入的原始标题（未被自定义标题覆盖时保留）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
    /// 工作目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// 创建时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamp>,
    /// 最后更新时间。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
    /// 模型名。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// 模型与推理配置状态。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_status: Option<ModelStatus>,
    /// CLI 版本。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_version: Option<String>,
    /// 会话文件绝对路径。
    pub file_path: String,
    /// 文件修改时间（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_mtime_ms: Option<f64>,
    /// 文件内容变更时间（毫秒）。
    ///
    /// 与 `file_mtime_ms` 一起构成缓存失效键：Windows/WSL 之间复制文件会保留 mtime 与
    /// 大小，只有 ctime 能反映内容已变，因此**两者加 size 必须同时比对**。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_change_ms: Option<f64>,
    /// 文件大小（字节）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<u64>,
    /// 可见消息总数（列表模式下为全量计数，即使 `preview` 被截断）。
    pub message_count: u64,
    /// `message_count` 是**下界**而不是精确总数。
    ///
    /// # 为什么这件事必须进数据
    ///
    /// 列表模式只读文件的**头部窗口**，数出来的条数是一个**真实的下界**：数到 2 条，
    /// 这个会话就**至少**有 2 条（它可能有 80 条）。
    ///
    /// 曾经把列表模式的计数一律置 `0` 表示「未知」，界面显示「—」。那修掉的是一句假话
    /// （更早的时候把 80 条显示成「2 条」），但代价是把信息也一起丢了——「≥2 条」不撒谎，
    /// 而且比「—」有用。
    ///
    /// **但「它是下界」不能只写在渲染层的注释里。** 那样线上传的是一个光看数字分不出
    /// 是下界还是总数的值，而约定只存在于人的记忆里——本项目里每一个这样的地方都出过
    /// 问题。所以它是一个**字段**：消费方读到的数字自带「怎么解读它」。
    ///
    /// 见 docs/rust-rewrite/20 第 7 节。
    #[serde(default, skip_serializing_if = "is_false")]
    pub message_count_is_lower_bound: bool,
    /// 预览消息；列表模式最多 8 条，详情模式为当前页。
    #[serde(default)]
    pub preview: Vec<Message>,
    /// 预览起始的绝对消息序号。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_offset: Option<u64>,
    /// 用量。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<SessionUsage>,
    /// 用户级元数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SessionMetadata>,
}

/// 供 `skip_serializing_if` 使用：`false` 时不写进线上载荷。
///
/// 绝大多数会话的计数都是精确的（详情路径永远是全量解析），为它们省掉一个恒为 `false`
/// 的字段。
///
/// 收引用不是笔误：serde 的 `skip_serializing_if` 只接受 `fn(&T) -> bool`。
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl Session {
    /// 展示标题：自定义标题优先，其次 CLI 原始标题，最后回退会话编号。
    #[must_use]
    pub fn effective_title(&self) -> &str {
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.custom_title.as_deref())
            .filter(|title| !title.is_empty())
            .or_else(|| {
                self.source_title
                    .as_deref()
                    .filter(|title| !title.is_empty())
            })
            .unwrap_or(self.title.as_str())
    }
}

/// 会话视图：活跃会话或回收站。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionView {
    /// 活跃会话。
    Active,
    /// 回收站。
    Trash,
}

impl SessionView {
    /// 视图短名，用于缓存键与日志。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Trash => "trash",
        }
    }
}

/// 会话文件的指纹，用于缓存失效判断与消息索引定位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFile {
    /// 文件绝对路径。
    pub file_path: String,
    /// 修改时间（毫秒）。
    pub mtime_ms: f64,
    /// 内容变更时间（毫秒）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_ms: Option<f64>,
    /// 文件大小（字节）。
    pub size: u64,
}

/// 消息分页游标。
///
/// 现网用一个 `number` 承载两种语义：`-1` 表示「取末尾一页」，`>= 0` 表示绝对序号。
/// 这里把它显式建模为枚举，既保留线上格式（仍序列化为数字），又消除哨兵值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageCursor {
    /// 首屏：取末尾 `limit` 条，并回填其真实绝对偏移。
    Latest,
    /// 从指定绝对消息序号开始向后读取。
    Offset(u64),
}

impl PageCursor {
    /// 现网哨兵值：`offset === -1` 表示首屏。
    pub const LATEST_SENTINEL: i64 = -1;

    /// 由现网的 `number` 参数构造。
    ///
    /// 任意负数（含哨兵 `-1`）都视为首屏请求，因此不需要把负数转成 `u64`。
    #[must_use]
    pub fn from_wire(offset: i64) -> Self {
        match u64::try_from(offset) {
            Ok(value) => Self::Offset(value),
            Err(_) => Self::Latest,
        }
    }

    /// 转为现网的 `number` 参数。
    ///
    /// 偏移量实际远小于 `i64::MAX`；真出现溢出时饱和到上限，而不是回绕成负数
    /// （回绕会让调用方把它误读成首屏哨兵）。
    #[must_use]
    pub fn to_wire(self) -> i64 {
        match self {
            Self::Latest => Self::LATEST_SENTINEL,
            Self::Offset(value) => i64::try_from(value).unwrap_or(i64::MAX),
        }
    }

    /// 是否为首屏请求。
    #[must_use]
    pub const fn is_latest(self) -> bool {
        matches!(self, Self::Latest)
    }
}

impl Serialize for PageCursor {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_i64(self.to_wire())
    }
}

impl<'de> Deserialize<'de> for PageCursor {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::from_wire(i64::deserialize(deserializer)?))
    }
}

/// 会话变更请求中的单个引用。
///
/// `file_path` 是**快路径提示**：渲染层在列表里已经拿到路径，带上它可以省掉一次全量
/// 扫描。但它来自渲染层，使用前必须重新校验是否落在期望目录内——现网踩过这个坑，
/// 因此校验逻辑写在 Provider 里而不是这里。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMutationRef {
    /// 会话编号。
    pub id: SessionId,
    /// 渲染层提供的文件路径提示。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

/// 批量变更结果中的单条。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMutationEntry {
    /// 会话编号。
    pub id: SessionId,
    /// 变更前的文件路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
    /// 移入回收站后的路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    /// 永久删除的路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<String>,
}

/// 批量变更结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionBatchResult {
    /// 逐条结果，顺序与请求一致。
    #[serde(default)]
    pub processed: Vec<SessionMutationEntry>,
}

/// 单条删除结果。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDeleteResult {
    /// 移入回收站后的路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved_to: Option<String>,
    /// 文件已不存在（陈旧列表项）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
}

/// 会话导出格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    /// Markdown。
    Markdown,
    /// JSON。
    Json,
    /// HTML。
    Html,
}

/// 导出结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    /// 写入的文件路径。
    pub file_path: String,
    /// 使用的格式。
    pub format: ExportFormat,
}

/// 一页会话消息。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    /// 本页首条消息的绝对序号（首屏请求时为回填的真实偏移）。
    pub offset: u64,
    /// 本页消息。
    pub messages: Vec<Message>,
    /// 是否还有更多（向更新的方向）。
    pub has_more: bool,
    /// 该会话的可见消息**总数**；`0` 表示本次无从得知。
    ///
    /// # 为什么总数要跟着每一页回来
    ///
    /// 界面上的会话头需要它。而会话头拿到的那个 `Session` 来自**列表**，那里的条数是
    /// **下界**（只读了头部窗口）。真正精确的总数只有详情路径有——它做的是全量解析。
    /// 让它随分页响应一起回来，界面就不必去猜、也不必另发一次请求。
    ///
    /// 与列表路径的 `message_count_is_lower_bound` 相对：**这里是精确值**，
    /// 拿不到时是 `0` 而不是某个近似数字。
    #[serde(default)]
    pub message_count: u64,
}

/// 消息分页锚点：把「可见消息序号」映射到文件内的位置。
///
/// # 为什么同时记录字节偏移与行号
///
/// 两者职责不同，缺一不可：
///
/// - **`byte_offset` 用于定位**：本地文件可 `seek` 直接跳到该处，分页从 O(n) 降到 O(1)。
/// - **`line_number` 是消息的身份**：详情返回的 `source_line` 必须是**原始 JSONL 的
///   绝对行号**，「从此处分支」依赖它定位轮次。若从字节偏移起读后把行号按相对值
///   计数，分页越靠后行号偏移越大，分支位置就会漂移。
///
/// 现网只有行号（没有字节偏移），因此定位是线性的；这里两者都存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageAnchor {
    /// 该锚点对应的可见消息绝对序号。
    pub message_offset: u64,
    /// 该消息在文件中的起始字节偏移。
    ///
    /// **`0` 表示「未知」**：WSL 侧只能按行号读取（`tail -n +N`），拿不到绝对字节偏移，
    /// 因此那里写入 0。消费方必须把 0 当作「不可用于 seek」而不是「文件开头」——
    /// 可见消息永远不可能是文件的第一行（首行是 `session_meta`），但依赖这一点太脆弱，
    /// 所以用显式哨兵。
    pub byte_offset: u64,
    /// 该消息所在原始行的绝对行号（1 起）。
    pub line_number: u64,
    /// 解析到该锚点时已知的可见消息总数。
    pub message_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(role: Role, text: &str, source_line: Option<u64>) -> Message {
        Message {
            role,
            text: text.to_owned(),
            timestamp: None,
            source_line,
        }
    }

    #[test]
    fn page_cursor_keeps_the_negative_one_sentinel_on_the_wire() {
        // 渲染层仍然发送 -1，必须能被正确解释为首屏请求。
        assert_eq!(PageCursor::from_wire(-1), PageCursor::Latest);
        assert_eq!(PageCursor::from_wire(0), PageCursor::Offset(0));
        assert_eq!(PageCursor::from_wire(42), PageCursor::Offset(42));
        // 任意负数都按首屏处理，避免越界。
        assert_eq!(PageCursor::from_wire(-99), PageCursor::Latest);

        assert_eq!(PageCursor::Latest.to_wire(), -1);
        assert_eq!(PageCursor::Offset(42).to_wire(), 42);
        assert!(PageCursor::Latest.is_latest());
        assert!(!PageCursor::Offset(0).is_latest());
    }

    #[test]
    fn page_cursor_serializes_as_a_plain_number() {
        assert_eq!(serde_json::to_string(&PageCursor::Latest).unwrap(), "-1");
        assert_eq!(serde_json::to_string(&PageCursor::Offset(7)).unwrap(), "7");
        let parsed: PageCursor = serde_json::from_str("-1").unwrap();
        assert_eq!(parsed, PageCursor::Latest);
    }

    #[test]
    fn message_omits_absent_optional_fields_like_typescript_undefined() {
        let json = serde_json::to_value(message(Role::User, "hi", None)).unwrap();
        assert_eq!(json["role"], "user");
        assert_eq!(json["text"], "hi");
        // TS 的 `?:` 字段在渲染层读到 undefined；序列化时省略最接近该语义。
        assert!(json.get("timestamp").is_none());
        assert!(json.get("sourceLine").is_none());
    }

    #[test]
    fn message_serializes_source_line_in_camel_case() {
        let json = serde_json::to_value(message(Role::Assistant, "ok", Some(42))).unwrap();
        assert_eq!(json["sourceLine"], 42);
    }

    #[test]
    fn message_accepts_null_for_optional_fields_from_the_frontend() {
        // 渲染层可能显式发送 null，必须能解析而不是报错。
        let parsed: Message =
            serde_json::from_str(r#"{"role":"user","text":"hi","sourceLine":null}"#).unwrap();
        assert_eq!(parsed.source_line, None);
    }

    #[test]
    fn token_usage_reports_whether_it_carries_any_value() {
        assert!(!TokenUsage::default().has_any());
        let usage = TokenUsage {
            input_tokens: Some(0),
            ..TokenUsage::default()
        };
        // 0 是有效值，不能被当成「无数据」。
        assert!(usage.has_any());
    }

    #[test]
    fn model_status_merge_prefers_new_values_and_keeps_old_ones() {
        let mut current = ModelStatus {
            model: Some("gpt-5".into()),
            reasoning: Some("high".into()),
            ..ModelStatus::default()
        };
        // 新记录只带 model，reasoning 应保留旧值。
        current.merge(&ModelStatus {
            model: Some("gpt-5.1".into()),
            ..ModelStatus::default()
        });
        assert_eq!(current.model.as_deref(), Some("gpt-5.1"));
        assert_eq!(current.reasoning.as_deref(), Some("high"));
        assert!(current.has_any());
    }

    #[test]
    fn session_serializes_with_camel_case_and_skips_absent_optionals() {
        let session = Session {
            id: SessionId::from("abc"),
            title: "标题".into(),
            source_title: None,
            cwd: Some("/work".into()),
            created_at: None,
            updated_at: None,
            model: None,
            model_status: None,
            cli_version: None,
            file_path: "/home/a/.codex/sessions/x.jsonl".into(),
            file_mtime_ms: Some(1_700_000_000_000.0),
            file_change_ms: None,
            file_size: Some(1234),
            message_count: 3,
            message_count_is_lower_bound: false,
            preview: vec![message(Role::User, "hi", None)],
            preview_offset: None,
            usage: None,
            metadata: None,
        };
        let json = serde_json::to_value(&session).unwrap();
        assert_eq!(json["filePath"], "/home/a/.codex/sessions/x.jsonl");
        assert_eq!(json["fileMtimeMs"], 1_700_000_000_000.0);
        assert_eq!(json["messageCount"], 3);
        assert!(json.get("sourceTitle").is_none());
        assert!(json.get("fileChangeMs").is_none());
    }

    #[test]
    fn effective_title_prefers_custom_then_source_then_id() {
        let mut session = Session {
            id: SessionId::from("abc"),
            title: "首条用户消息".into(),
            source_title: Some("CLI 标题".into()),
            cwd: None,
            created_at: None,
            updated_at: None,
            model: None,
            model_status: None,
            cli_version: None,
            file_path: "x".into(),
            file_mtime_ms: None,
            file_change_ms: None,
            file_size: None,
            message_count: 0,
            message_count_is_lower_bound: false,
            preview: Vec::new(),
            preview_offset: None,
            usage: None,
            metadata: None,
        };
        assert_eq!(session.effective_title(), "CLI 标题");

        session.metadata = Some(SessionMetadata {
            custom_title: Some("我的标题".into()),
            ..SessionMetadata::default()
        });
        assert_eq!(session.effective_title(), "我的标题");

        // 空字符串的自定义标题视为未设置，继续回退。
        session.metadata = Some(SessionMetadata {
            custom_title: Some(String::new()),
            ..SessionMetadata::default()
        });
        assert_eq!(session.effective_title(), "CLI 标题");
    }

    #[test]
    fn session_view_serializes_as_lowercase() {
        assert_eq!(
            serde_json::to_string(&SessionView::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&SessionView::Trash).unwrap(),
            "\"trash\""
        );
        assert_eq!(SessionView::Trash.as_str(), "trash");
    }

    #[test]
    fn usage_source_serializes_as_kebab_case_like_existing_contract() {
        assert_eq!(
            serde_json::to_string(&UsageSource::CodexTokenCount).unwrap(),
            "\"codex-token-count\""
        );
        assert_eq!(
            serde_json::to_string(&UsageSource::ClaudeMessageUsage).unwrap(),
            "\"claude-message-usage\""
        );
    }

    #[test]
    fn rate_limit_reports_whether_it_carries_any_value() {
        assert!(!RateLimitUsage::default().has_any());
        let limit = RateLimitUsage {
            used_percent: Some(0.0),
            ..RateLimitUsage::default()
        };
        assert!(limit.has_any());
    }
}
