//! Codex `rollout-*.jsonl` 解析。
//!
//! # 两种解析模式
//!
//! | 模式 | 输入 | 预览 | `updatedAt` |
//! | --- | --- | --- | --- |
//! | [`ParseMode::List`] | 文件头部 64 KB | 最多 8 条 | **文件 mtime** |
//! | [`ParseMode::Full`] | 全量流式 | 全部可见消息 | 最后一条记录的时间戳 |
//!
//! 列表模式用 mtime 作 `updatedAt` 是现网行为，也是列表排序的依据：列表页只读头部，
//! 拿不到最后一条记录的时间戳，用 mtime 是唯一可得且单调的近似。
//!
//! # 记录类型
//!
//! - `session_meta`：`id` / `timestamp` / `cwd` / `model` / `model_provider` / `cli_version`
//! - `event_msg`：`payload.message` 携带正文；`payload.type` 决定角色
//!   （`user_message` → user、`agent_message` → assistant、其余 → unknown）；
//!   其中 `token_count` 提供用量
//! - `response_item`：仅 `payload.type == "message"` 时取 `payload.content` 里的
//!   `text` / `input_text` / `output_text`
//! - `turn_context`：提供 `model` / `effort` / `summary` / `collaboration_mode.settings`
//!
//! # 不设置 `source_line`
//!
//! 现网的共享解析器**不**填 `sourceLine`，它由分页层在逐行读取时按行号设置
//! （`codex-targets.ts` 的 `getSessionMessagesPage`）。这里保持一致：解析器只负责
//! 「这一行是不是可见消息」，行号由调用方补。

use serde::Deserialize;
use serde_json::Value;

use aivc_core::error::Result;
use aivc_core::session::{
    Message, ModelStatus, RateLimitUsage, RateLimits, Role, Session, SessionUsage, TokenUsage,
    UsageSource,
};
use aivc_core::text::{clamp_title, is_synthetic_user_block, should_keep_message};
use aivc_core::time::Timestamp;

pub use crate::common::SessionFileInfo;

/// 列表模式的预览条数上限。
pub const LIST_PREVIEW_LIMIT: usize = 8;

/// 解析模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMode {
    /// 列表：只保留前 [`LIST_PREVIEW_LIMIT`] 条预览，但计数全量。
    List,
    /// 详情：保留全部可见消息。
    Full,
}

/// 一行原始记录。
#[derive(Debug, Deserialize)]
struct RawLine {
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    payload: Option<Value>,
}

/// 流式会话解析器。
///
/// 逐行喂入即可，不要求整文件在内存中；列表模式只读头部时行为与全量读取一致
/// （计数仍是全量——只要把全部行都喂进来）。
#[derive(Debug)]
pub struct SessionParser {
    info: SessionFileInfo,
    mode: ParseMode,
    id: String,
    created_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    cwd: Option<String>,
    model: Option<String>,
    model_status: Option<ModelStatus>,
    cli_version: Option<String>,
    usage: Option<SessionUsage>,
    messages: Vec<Message>,
    message_count: u64,
    previous: Option<(Role, String)>,
    /// 标题候选（第一条合格的 user 消息，已截断）。
    ///
    /// **独立于 `messages` 保存**：`messages` 在列表模式下有 8 条上限（省内存），
    /// 而标题只需要**扫描**，不需要保留整条消息。两者此前共用一个缓冲，于是头部
    /// 8 条内没有合格候选时标题会退化——而且退化得毫无提示。
    title_candidate: Option<String>,
}

impl SessionParser {
    /// 构造。
    #[must_use]
    pub fn new(info: SessionFileInfo, mode: ParseMode) -> Self {
        // 文件名为 `rollout-<时间戳>-<uuid>.jsonl`：先按文件名取出 uuid 作为兜底编号，
        // 若 `session_meta` 提供了 id 则以后者为准。
        let id = session_id_from_file_path(&info.file_path);
        Self {
            info,
            mode,
            id,
            created_at: None,
            updated_at: None,
            cwd: None,
            model: None,
            model_status: None,
            cli_version: None,
            usage: None,
            messages: Vec::new(),
            message_count: 0,
            previous: None,
            title_candidate: None,
        }
    }

    /// 喂入一行。`line_number` 为 1 起的原始行号（本解析器不记录它，仅供调用方对齐）。
    pub fn push(&mut self, line: &str, _line_number: u64) {
        if line.is_empty() {
            return;
        }
        let Ok(raw) = serde_json::from_str::<RawLine>(line) else {
            // 损坏行直接跳过：单个文件异常不应让整个会话列表打不开。
            return;
        };

        if let Some(timestamp) = raw.timestamp.as_deref().filter(|value| !value.is_empty()) {
            self.updated_at = Some(Timestamp::from_raw(timestamp));
            if self.created_at.is_none() {
                self.created_at = Some(Timestamp::from_raw(timestamp));
            }
        }

        let kind = raw.kind.as_deref().unwrap_or_default();
        let payload = raw.payload.clone().unwrap_or(Value::Null);

        if kind == "session_meta" {
            let object = payload.as_object();
            if let Some(object) = object {
                if let Some(value) = string_field(object.get("id"))
                    && self.id.is_empty()
                {
                    self.id = value;
                }
                if let Some(value) = string_field(object.get("timestamp")) {
                    self.created_at = Some(Timestamp::from_raw(value));
                }
                if let Some(value) = string_field(object.get("cwd")) {
                    self.cwd = Some(value);
                }
                if let Some(value) = string_field(object.get("model")) {
                    self.model = Some(value);
                }
                if let Some(value) = string_field(object.get("cli_version")) {
                    self.cli_version = Some(value);
                }
            }
            self.merge_model_status(&payload);
            return;
        }

        self.merge_model_status(&payload);
        if let Some(model) = self
            .model_status
            .as_ref()
            .and_then(|status| status.model.clone())
        {
            self.model = Some(model);
        }
        if let Some(usage) = extract_usage(&payload, raw.timestamp.as_deref(), kind) {
            self.usage = Some(usage);
        }

        let Some(message) = extract_message(&payload, kind) else {
            return;
        };
        if !should_keep_message(&message.text) {
            return;
        }
        // 连续同 role + 同正文去重：Codex 会把同一条消息同时写进 `event_msg` 与
        // `response_item`，不去重会让每条消息出现两次。
        if let Some((role, text)) = &self.previous
            && *role == message.role
            && text == &message.text
        {
            return;
        }
        self.previous = Some((message.role, message.text.clone()));
        self.message_count += 1;

        let visible = Message {
            timestamp: raw.timestamp.map(Timestamp::from_raw),
            ..message
        };

        // 标题候选在**预览上限之外**单独记录：上限是「省内存」，标题是「扫一遍」。
        // 只取第一条，之后不再覆盖——标题应当稳定，不该随解析进度变来变去。
        if self.title_candidate.is_none()
            && visible.role == Role::User
            && is_title_candidate(&visible.text)
        {
            // 去掉标记后可能什么都不剩（整行都是 `---` 之类）。这时**不记录**，
            // 继续往下找——记录空标题会让界面显示一片空白，比编号更糟。
            let candidate = clamp_title(&visible.text);
            if !candidate.is_empty() {
                self.title_candidate = Some(candidate);
            }
        }

        match self.mode {
            ParseMode::List => {
                if self.messages.len() < LIST_PREVIEW_LIMIT {
                    self.messages.push(visible);
                }
            }
            ParseMode::Full => self.messages.push(visible),
        }
    }

    /// 结束解析并产出会话；无法确定编号时返回 [`None`]。
    #[must_use]
    pub fn finish(self) -> Option<Session> {
        let file_name = file_name_of(&self.info.file_path);
        if self.id.is_empty() || self.id == file_name {
            return None;
        }

        // 标题取第一条合格的 user 消息；没有则回退为编号。
        //
        // 注意这里读的是**独立记录的候选**，不是 `self.messages`：列表模式下后者只有
        // 前 8 条，真正的用户提示词常常在它之外。
        let title = match self.title_candidate {
            Some(candidate) => candidate,
            None => self.id.clone(),
        };

        // 列表模式用 mtime 作 updatedAt（只读头部，拿不到最后一条记录的时间戳）。
        let updated_at = match self.mode {
            ParseMode::List => Some(Timestamp::from_unix_millis(self.info.mtime_ms as i64)),
            ParseMode::Full => self.updated_at,
        };

        Some(Session {
            id: self.id.into(),
            title,
            source_title: None,
            cwd: self.cwd,
            created_at: self.created_at,
            updated_at,
            model: self.model,
            model_status: self.model_status,
            cli_version: self.cli_version,
            file_path: self.info.file_path,
            file_mtime_ms: Some(self.info.mtime_ms),
            file_change_ms: self.info.change_ms,
            file_size: Some(self.info.size),
            message_count: self.message_count,
            // **parser 不知道**自己拿到的是不是整个文件（`List` 模式收的是头部窗口，
            // 但那由调用方决定）。因此这里一律 `false`，由**读了头部窗口的调用方**置位
            // ——见 `listing.rs`。
            message_count_is_lower_bound: false,
            preview: self.messages,
            preview_offset: None,
            usage: self.usage,
            metadata: None,
        })
    }

    /// 合并 `session_meta` / `turn_context` 提供的模型状态。
    fn merge_model_status(&mut self, payload: &Value) {
        let Some(next) = extract_model_status(payload) else {
            return;
        };
        match self.model_status.as_mut() {
            Some(current) => current.merge(&next),
            None => self.model_status = Some(next),
        }
    }
}

/// 从文件路径提取会话编号（`rollout-<时间戳>-<uuid>.jsonl`）。
#[must_use]
pub fn session_id_from_file_path(file_path: &str) -> String {
    let name = file_name_of(file_path);
    let Some(rest) = name
        .strip_prefix("rollout-")
        .and_then(|value| value.strip_suffix(".jsonl"))
    else {
        return String::new();
    };
    // 取最后一个 `-` 之后的部分，并要求它是 36 字符的 UUID 形状。
    let Some(index) = rest.rfind('-') else {
        return String::new();
    };
    // UUID 自身含 4 个短横线，因此从末尾回溯 36 个字符再校验形状。
    if rest.len() < 36 {
        return String::new();
    }
    let candidate = &rest[rest.len() - 36..];
    let _ = index;
    if is_uuid_shape(candidate) {
        candidate.to_owned()
    } else {
        String::new()
    }
}

/// 判断是否为 UUID 形状（8-4-4-4-12，十六进制）。
fn is_uuid_shape(value: &str) -> bool {
    let groups: Vec<&str> = value.split('-').collect();
    let expected = [8usize, 4, 4, 4, 12];
    groups.len() == expected.len()
        && groups.iter().zip(expected).all(|(group, width)| {
            group.len() == width && group.chars().all(|c| c.is_ascii_hexdigit())
        })
}

/// 路径的文件名。
fn file_name_of(file_path: &str) -> String {
    file_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_path)
        .to_owned()
}

/// 标题候选：归一化后非空且不是合成块。
fn is_title_candidate(text: &str) -> bool {
    !text.trim().is_empty() && !is_synthetic_user_block(text)
}

/// 读取非空字符串字段。
fn string_field(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// 读取有限数值字段。
fn number_field(value: Option<&Value>) -> Option<f64> {
    let number = value?.as_f64()?;
    number.is_finite().then_some(number)
}

/// 读取对象字段。
fn object_field(value: Option<&Value>) -> Option<&serde_json::Map<String, Value>> {
    value?.as_object()
}

/// 从一条记录提取可见消息。
#[must_use]
pub fn extract_message(payload: &Value, kind: &str) -> Option<Message> {
    let object = payload.as_object()?;

    if kind == "event_msg"
        && let Some(text) = object.get("message").and_then(Value::as_str)
    {
        return Some(Message {
            role: event_role(
                object
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            text: text.to_owned(),
            timestamp: None,
            source_line: None,
        });
    }

    if kind == "response_item" && object.get("type").and_then(Value::as_str) == Some("message") {
        let text = extract_content_text(object.get("content"));
        if text.is_empty() {
            return None;
        }
        return Some(Message {
            role: message_role(object.get("role")),
            text,
            timestamp: None,
            source_line: None,
        });
    }

    None
}

/// `event_msg` 的角色映射。
fn event_role(kind: &str) -> Role {
    match kind {
        "user_message" => Role::User,
        "agent_message" => Role::Assistant,
        _ => Role::Unknown,
    }
}

/// `response_item` 的角色映射。
fn message_role(value: Option<&Value>) -> Role {
    match value.and_then(Value::as_str) {
        Some("user") => Role::User,
        Some("assistant") => Role::Assistant,
        Some("system") => Role::System,
        Some("tool") => Role::Tool,
        _ => Role::Unknown,
    }
}

/// 从 `content` 数组提取正文：依次取 `text` / `input_text` / `output_text`，用换行拼接。
fn extract_content_text(content: Option<&Value>) -> String {
    let Some(parts) = content.and_then(Value::as_array) else {
        return String::new();
    };
    let joined: Vec<&str> = parts
        .iter()
        .filter_map(|part| {
            let object = part.as_object()?;
            ["text", "input_text", "output_text"]
                .into_iter()
                .find_map(|key| object.get(key).and_then(Value::as_str))
        })
        .filter(|text| !text.is_empty())
        .collect();
    joined.join("\n").trim().to_owned()
}

/// 从一条记录提取模型状态。
#[must_use]
pub fn extract_model_status(payload: &Value) -> Option<ModelStatus> {
    let object = payload.as_object()?;
    // `session_meta` 的模型状态由调用方直接读取顶层字段，这里只处理 `turn_context`
    // 与 `session_meta` 的 `model_provider`。
    let settings = object_field(object.get("collaboration_mode"))
        .and_then(|mode| object_field(mode.get("settings")));

    let reasoning = string_field(object.get("effort"))
        .or_else(|| settings.and_then(|settings| string_field(settings.get("reasoning_effort"))));
    let model = string_field(object.get("model"))
        .or_else(|| settings.and_then(|settings| string_field(settings.get("model"))));
    let summaries = string_field(object.get("summary"));
    let model_provider = string_field(object.get("model_provider"));

    let status = ModelStatus {
        model,
        reasoning,
        summaries,
        model_provider,
    };
    status.has_any().then_some(status)
}

/// 从 `event_msg.token_count` 提取用量。
#[must_use]
pub fn extract_usage(payload: &Value, timestamp: Option<&str>, kind: &str) -> Option<SessionUsage> {
    if kind != "event_msg" || payload.get("type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let info = object_field(payload.get("info"));
    let total = normalize_token_usage(info.and_then(|info| info.get("total_token_usage")));
    let last = normalize_token_usage(info.and_then(|info| info.get("last_token_usage")));
    let context_window =
        number_field(info.and_then(|info| info.get("model_context_window"))).map(as_u64);
    // 上下文占用以**最近一轮**的输入 token 计量，而不是累计值。
    let context_used_tokens = last.and_then(|usage| usage.input_tokens);

    let context_percent = match (context_used_tokens, context_window) {
        (Some(used), Some(window)) if window > 0 => {
            // 保留 1 位小数，并封顶 100。
            let raw = (used as f64 / window as f64) * 1000.0;
            Some((raw.round() / 10.0).min(100.0))
        }
        _ => None,
    };
    let context_left_percent =
        context_percent.map(|percent| ((100.0 - percent) * 10.0).round() / 10.0);

    let rate_limits = object_field(payload.get("rate_limits"));
    let primary = rate_limits.and_then(|limits| normalize_rate_limit(limits.get("primary")));
    let secondary = rate_limits.and_then(|limits| normalize_rate_limit(limits.get("secondary")));

    if total.is_none()
        && last.is_none()
        && context_window.is_none()
        && primary.is_none()
        && secondary.is_none()
    {
        return None;
    }

    Some(SessionUsage {
        total,
        last,
        context_window,
        context_used_tokens,
        context_percent,
        context_left_percent,
        rate_limits: (primary.is_some() || secondary.is_some())
            .then_some(RateLimits { primary, secondary }),
        updated_at: timestamp.map(Timestamp::from_raw),
        source: UsageSource::CodexTokenCount,
    })
}

/// 归一化 token 用量。
fn normalize_token_usage(value: Option<&Value>) -> Option<TokenUsage> {
    let object = value?.as_object()?;
    let usage = TokenUsage {
        input_tokens: number_field(object.get("input_tokens")).map(as_u64),
        cached_input_tokens: number_field(object.get("cached_input_tokens")).map(as_u64),
        output_tokens: number_field(object.get("output_tokens")).map(as_u64),
        reasoning_output_tokens: number_field(object.get("reasoning_output_tokens")).map(as_u64),
        total_tokens: number_field(object.get("total_tokens")).map(as_u64),
    };
    usage.has_any().then_some(usage)
}

/// 归一化速率限制窗口。
fn normalize_rate_limit(value: Option<&Value>) -> Option<RateLimitUsage> {
    let object = value?.as_object()?;
    let usage = RateLimitUsage {
        used_percent: number_field(object.get("used_percent")),
        window_minutes: number_field(object.get("window_minutes")).map(as_u64),
        resets_at: string_field(object.get("resets_at")).map(Timestamp::from_raw),
    };
    usage.has_any().then_some(usage)
}

/// `f64` → `u64`：负数与超界按 0 处理。
///
/// 上方已确认数值有限且非负，因此截断转换不会丢符号。
#[allow(clippy::cast_sign_loss)]
fn as_u64(value: f64) -> u64 {
    if value.is_finite() && value >= 0.0 {
        value as u64
    } else {
        0
    }
}

/// 便捷入口：解析整段文本。
pub fn parse_content(info: SessionFileInfo, content: &str, mode: ParseMode) -> Option<Session> {
    let mut parser = SessionParser::new(info, mode);
    for (index, line) in content.split('\n').enumerate() {
        parser.push(line.trim_end_matches('\r'), index as u64 + 1);
    }
    parser.finish()
}

/// 便捷入口：只解析头部若干字节（列表模式）。
pub fn parse_head(info: SessionFileInfo, head: &str) -> Option<Session> {
    parse_content(info, head, ParseMode::List)
}

/// 解析结果包装为 [`Result`]，便于在上层统一错误处理。
pub fn parse_head_result(info: SessionFileInfo, head: &str) -> Result<Option<Session>> {
    Ok(parse_head(info, head))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(path: &str, mtime: f64) -> SessionFileInfo {
        SessionFileInfo {
            file_path: path.to_owned(),
            mtime_ms: mtime,
            change_ms: Some(mtime),
            size: 100,
        }
    }

    fn meta_line(id: &str, timestamp: &str, cwd: &str) -> String {
        format!(
            r#"{{"timestamp":"{timestamp}","type":"session_meta","payload":{{"id":"{id}","timestamp":"{timestamp}","cwd":"{cwd}","model":"gpt-5","cli_version":"0.154.0"}}}}"#
        )
    }

    fn event_line(timestamp: &str, kind: &str, message: &str) -> String {
        format!(
            r#"{{"timestamp":"{timestamp}","type":"event_msg","payload":{{"type":"{kind}","message":"{message}"}}}}"#
        )
    }

    #[test]
    fn session_id_is_extracted_from_the_rollout_file_name() {
        assert_eq!(
            session_id_from_file_path(
                "/home/a/.codex/sessions/2026/09/16/rollout-2026-09-16T12-34-56-0191f2a3-0000-7000-8000-000000000000.jsonl"
            ),
            "0191f2a3-0000-7000-8000-000000000000"
        );
        // 非 rollout 文件、形状不对的 uuid 都返回空串。
        assert_eq!(session_id_from_file_path("/a/other.jsonl"), "");
        assert_eq!(session_id_from_file_path("/a/rollout-x-short.jsonl"), "");
        assert_eq!(
            session_id_from_file_path("/a/rollout-x-zzzzzzzz-0000-7000-8000-000000000000.jsonl"),
            "",
            "非十六进制不算 uuid"
        );
    }

    #[test]
    fn full_parse_extracts_metadata_and_messages() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/work"),
            event_line("2026-09-16T12:00:01.000Z", "user_message", "帮我重构"),
            event_line("2026-09-16T12:00:02.000Z", "agent_message", "好的"),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1_000.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.id.as_str(), "s1");
        assert_eq!(session.title, "帮我重构");
        assert_eq!(session.cwd.as_deref(), Some("/work"));
        assert_eq!(session.model.as_deref(), Some("gpt-5"));
        assert_eq!(session.cli_version.as_deref(), Some("0.154.0"));
        assert_eq!(session.message_count, 2);
        assert_eq!(session.preview.len(), 2);
        assert_eq!(session.preview[0].role, Role::User);
        assert_eq!(session.preview[1].role, Role::Assistant);
        // 详情模式的 updatedAt 是最后一条记录的时间戳。
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("2026-09-16T12:00:02.000Z")
        );
    }

    #[test]
    fn list_mode_truncates_preview_but_keeps_the_full_count() {
        let mut lines = vec![meta_line("s1", "2026-09-16T12:00:00.000Z", "/w")];
        for index in 0..20 {
            lines.push(event_line(
                "2026-09-16T12:00:01.000Z",
                "user_message",
                &format!("消息{index}"),
            ));
        }
        let content = lines.join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 5_000.0),
            &content,
            ParseMode::List,
        )
        .expect("session");
        assert_eq!(session.preview.len(), LIST_PREVIEW_LIMIT);
        assert_eq!(session.message_count, 20, "计数必须是全量");
        // 列表模式用 mtime 作 updatedAt。
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("1970-01-01T00:00:05.000Z")
        );
    }

    #[test]
    fn list_mode_finds_a_title_candidate_beyond_the_preview_limit() {
        // 预览缓冲只保留前 8 条，而真正的用户提示词可能远在其后——Codex 会话开头
        // 常常是一串系统注入的助手记录。
        //
        // 标题查找**不能复用预览缓冲**。此前它正是这么做的：缓冲里没有合格的 user
        // 消息就回退成会话编号，于是列表里 5 条会话的标题全是 UUID，而且**没有任何
        // 提示**说这是退化结果。
        let mut lines = vec![meta_line("s1", "2026-09-16T12:00:00.000Z", "/w")];
        for index in 0..LIST_PREVIEW_LIMIT + 2 {
            lines.push(event_line(
                "2026-09-16T12:00:01.000Z",
                "agent_message",
                &format!("助手第{index}条"),
            ));
        }
        lines.push(event_line(
            "2026-09-16T12:00:02.000Z",
            "user_message",
            "把登录改成 SSO",
        ));
        let content = lines.join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 5_000.0),
            &content,
            ParseMode::List,
        )
        .expect("session");

        assert_eq!(
            session.preview.len(),
            LIST_PREVIEW_LIMIT,
            "预览仍受上限约束"
        );
        assert_eq!(
            session.title, "把登录改成 SSO",
            "标题必须找到预览上限之外的候选，而不是退化成编号"
        );
    }

    #[test]
    fn a_markdown_heavy_first_message_becomes_a_readable_title() {
        // 真实数据里第一条用户消息常常是这种结构。直接截断会把标记带进列表：
        //   `# 工作状态摘要 ## 当前目标 - [ ] 完成…`
        // 标题需要的是**词**，不是标记。
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line(
                "2026-09-16T12:00:01.000Z",
                "user_message",
                "# 工作状态摘要 ## 当前目标 - [ ] 收口性能优化",
            ),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 5_000.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");

        assert!(
            session.title.contains("工作状态摘要"),
            "实际：{}",
            session.title
        );
        assert!(!session.title.contains('#'), "标记不该出现在标题里");
        assert!(!session.title.contains('['), "复选框不该出现在标题里");
    }

    #[test]
    fn a_first_line_made_only_of_markers_does_not_become_an_empty_title() {
        // 去掉标记后什么都不剩时，**不能**记一个空标题——那会让列表显示一片空白，
        // 比显示编号更糟。应当继续往下找。
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line("2026-09-16T12:00:01.000Z", "user_message", "---\n##\n"),
            event_line("2026-09-16T12:00:01.000Z", "agent_message", "收到"),
            event_line(
                "2026-09-16T12:00:02.000Z",
                "user_message",
                "真正的标题在这里",
            ),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 5_000.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");

        assert_eq!(session.title, "真正的标题在这里");
    }

    #[test]
    fn consecutive_duplicate_messages_are_deduplicated() {
        // Codex 会把同一条消息同时写进 event_msg 与 response_item。
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line("2026-09-16T12:00:01.000Z", "user_message", "重复"),
            event_line("2026-09-16T12:00:01.000Z", "user_message", "重复"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"重复"}]}}"#.to_owned(),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.message_count, 1, "连续同 role+正文必须去重");
    }

    #[test]
    fn synthetic_blocks_are_filtered_out() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line(
                "2026-09-16T12:00:01.000Z",
                "user_message",
                "<environment_context>\\n<shell>bash</shell>",
            ),
            event_line("2026-09-16T12:00:02.000Z", "user_message", "真实输入"),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.preview[0].text, "真实输入");
        // 标题必须跳过合成块。
        assert_eq!(session.title, "真实输入");
    }

    #[test]
    fn response_item_content_concatenates_parts_with_newlines() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"第一段"},{"type":"output_text","text":"第二段"}]}}"#.to_owned(),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.preview[0].text, "第一段\n第二段");
        assert_eq!(session.preview[0].role, Role::Assistant);
    }

    #[test]
    fn unknown_event_types_map_to_the_unknown_role() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line("2026-09-16T12:00:01.000Z", "something_else", "内容"),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.preview[0].role, Role::Unknown);
    }

    #[test]
    fn token_count_records_produce_usage_with_context_math() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"output_tokens":200,"total_tokens":1200},"last_token_usage":{"input_tokens":250,"output_tokens":50},"model_context_window":1000},"rate_limits":{"primary":{"used_percent":12.5,"window_minutes":300,"resets_at":"2026-09-16T13:00:00.000Z"}}}}"#.to_owned(),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.source, UsageSource::CodexTokenCount);
        assert_eq!(usage.total.expect("total").input_tokens, Some(1000));
        assert_eq!(usage.total.expect("total").total_tokens, Some(1200));
        assert_eq!(usage.last.expect("last").input_tokens, Some(250));
        assert_eq!(usage.context_window, Some(1000));
        // 上下文占用取**最近一轮**的输入 token：250/1000 = 25%。
        assert_eq!(usage.context_used_tokens, Some(250));
        assert_eq!(usage.context_percent, Some(25.0));
        assert_eq!(usage.context_left_percent, Some(75.0));
        let limits = usage.rate_limits.expect("rate limits");
        assert_eq!(limits.primary.expect("primary").used_percent, Some(12.5));
    }

    #[test]
    fn context_percent_is_capped_at_one_hundred() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":5000},"model_context_window":1000}}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.context_percent, Some(100.0));
        assert_eq!(usage.context_left_percent, Some(0.0));
    }

    #[test]
    fn zero_context_window_does_not_produce_percentages() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10},"model_context_window":0}}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.context_percent, None);
        assert_eq!(usage.context_left_percent, None);
    }

    #[test]
    fn turn_context_contributes_model_and_reasoning() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"turn_context","payload":{"model":"gpt-5.1","effort":"high","summary":"auto"}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        // turn_context 的模型覆盖 session_meta 的模型。
        assert_eq!(session.model.as_deref(), Some("gpt-5.1"));
        let status = session.model_status.expect("model status");
        assert_eq!(status.reasoning.as_deref(), Some("high"));
        assert_eq!(status.summaries.as_deref(), Some("auto"));
    }

    #[test]
    fn turn_context_falls_back_to_collaboration_mode_settings() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"turn_context","payload":{"collaboration_mode":{"settings":{"model":"gpt-5-mini","reasoning_effort":"low"}}}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        let status = session.model_status.expect("model status");
        assert_eq!(status.model.as_deref(), Some("gpt-5-mini"));
        assert_eq!(status.reasoning.as_deref(), Some("low"));
    }

    #[test]
    fn corrupt_lines_are_skipped_without_failing_the_parse() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            "not json at all".to_owned(),
            r#"{"type":"event_msg"}"#.to_owned(),
            event_line("2026-09-16T12:00:02.000Z", "user_message", "仍然解析"),
        ]
        .join("\n");

        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.preview[0].text, "仍然解析");
    }

    #[test]
    fn session_without_a_usable_id_is_rejected() {
        // 文件名不含 uuid 且没有 session_meta → 无法确定编号。
        let content = event_line("2026-09-16T12:00:00.000Z", "user_message", "hi");
        assert!(parse_content(info("/a/mystery.jsonl", 1.0), &content, ParseMode::Full).is_none());
    }

    #[test]
    fn title_falls_back_to_the_session_id_when_no_user_message_exists() {
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line("2026-09-16T12:00:01.000Z", "agent_message", "只有助手消息"),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert_eq!(session.title, "s1");
    }

    #[test]
    fn long_titles_are_clamped() {
        let long = "标".repeat(200);
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line("2026-09-16T12:00:01.000Z", "user_message", &long),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert!(session.title.ends_with("..."));
    }

    #[test]
    fn parser_output_does_not_set_source_line() {
        // sourceLine 由分页层按行号设置，解析器不得代填。
        let content = [
            meta_line("s1", "2026-09-16T12:00:00.000Z", "/w"),
            event_line("2026-09-16T12:00:01.000Z", "user_message", "hi"),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/rollout-x-s1.jsonl", 1.0),
            &content,
            ParseMode::Full,
        )
        .expect("session");
        assert!(session.preview[0].source_line.is_none());
    }
}
