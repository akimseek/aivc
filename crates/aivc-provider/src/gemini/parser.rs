//! Gemini CLI `session-*.jsonl` 解析。
//!
//! # 三种记录形态
//!
//! Gemini 的会话文件是**增量补丁流**，一条记录可能是三种形态之一：
//!
//! | 形态 | 判据 | 含义 |
//! | --- | --- | --- |
//! | 元数据 | 有 `sessionId` | 会话起点：`startTime` / `lastUpdated` / `summary` / `kind` |
//! | 补丁 | 有 `$set` | 批量更新：`$set.messages` 数组、`$set.lastUpdated`、`$set.summary` |
//! | 单条 | 有 `type` | 一条消息（`user` / `gemini` / `info` / `warning` / `error`） |
//!
//! 同一份文件里三种形态交错出现，因此解析器必须按判据分派而不是按行号假设结构。
//!
//! # 子代理会话必须整体丢弃
//!
//! `kind === "subagent"` 的会话是主会话派生的内部会话，不出现在会话列表里。这个判断
//! 在**结束**时才生效（`kind` 可能在文件后段才出现），因此不能提前短路。
//!
//! # 计数与预览的过滤规则不同
//!
//! - **计数**用 [`should_keep_message`]（排除 `<session_context>` 合成块）；
//! - **预览**在结束前不再过滤，而是在 `finish` 时统一过滤。
//!
//! 这是现网的行为：`push` 不检查合成块，`finish` 才过滤。因此预览数组里可能短暂存在
//! 合成块，最终输出不会。这里保持一致，避免「看起来一样但边界行为不同」。
//!
//! # 标题优先级
//!
//! `summary` → 第一条用户消息 → 第一条可见用户消息 → 会话编号，最后截到 88 字符。

use serde_json::{Map, Value};

use aivc_core::session::{
    Message, ModelStatus, Role, Session, SessionUsage, TokenUsage, UsageSource,
};
use aivc_core::text::clamp_title;
use aivc_core::time::Timestamp;

use crate::common::SessionFileInfo;

/// 列表模式的预览条数上限。
pub const LIST_PREVIEW_LIMIT: usize = 8;
/// 会话文件名前缀。
pub const SESSION_FILE_PREFIX: &str = "session-";
/// 会话文件扩展名。
pub const SESSION_FILE_SUFFIX: &str = ".jsonl";
/// 默认上下文窗口（1 MiB）。
pub const DEFAULT_TOKEN_LIMIT: u64 = 1_048_576;
/// Gemma 模型的上下文窗口。
pub const GEMMA_TOKEN_LIMIT: u64 = 256_000;
/// 合成块前缀：以它开头的消息不计入消息数。
pub const SYNTHETIC_PREFIX: &str = "<session_context>";

/// 是否是应当丢弃的合成消息。
#[must_use]
pub fn should_keep_gemini_message(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && !trimmed.starts_with(SYNTHETIC_PREFIX)
}

/// 按模型名给出上下文窗口。
#[must_use]
pub fn token_limit(model: &str) -> u64 {
    let lower = model.to_lowercase();
    if lower.contains("gemma-4-31b-it") || lower.contains("gemma-4-26b-a4b-it") {
        return GEMMA_TOKEN_LIMIT;
    }
    DEFAULT_TOKEN_LIMIT
}

/// 从文件名提取会话编号（`session-<时间戳>-<8 位十六进制>.jsonl`）。
#[must_use]
pub fn session_id_from_file_path(file_path: &str) -> String {
    let name = file_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_path)
        .to_owned();
    let Some(rest) = name
        .strip_prefix(SESSION_FILE_PREFIX)
        .and_then(|value| value.strip_suffix(SESSION_FILE_SUFFIX))
    else {
        return String::new();
    };
    // 形状：`YYYY-MM-DDTHH-MM-<8 位十六进制>`。时间戳部分固定 16 字符。
    let Some((stamp, suffix)) = rest.rsplit_once('-') else {
        return String::new();
    };
    if suffix.len() != 8 || !suffix.chars().all(|c| c.is_ascii_hexdigit()) {
        return String::new();
    }
    if stamp.len() != 16 {
        return String::new();
    }
    suffix.to_lowercase()
}

/// 是否是 Gemini 会话文件。
#[must_use]
pub fn is_session_file_name(name: &str) -> bool {
    name.starts_with(SESSION_FILE_PREFIX)
        && name.to_ascii_lowercase().ends_with(SESSION_FILE_SUFFIX)
}

/// 角色映射。
#[must_use]
pub fn gemini_role(kind: &str) -> Option<Role> {
    match kind {
        "user" => Some(Role::User),
        "gemini" => Some(Role::Assistant),
        "info" | "warning" | "error" => Some(Role::System),
        _ => None,
    }
}

/// 从 `displayContent` / `content` 提取正文。
#[must_use]
pub fn extract_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(parts)) => {
            let joined: Vec<&str> = parts
                .iter()
                .filter_map(|part| match part {
                    Value::String(text) => Some(text.as_str()),
                    Value::Object(object) => ["text", "input_text", "output_text"]
                        .into_iter()
                        .find_map(|key| object.get(key).and_then(Value::as_str)),
                    _ => None,
                })
                .filter(|text| !text.is_empty())
                .collect();
            joined.join("\n").trim().to_owned()
        }
        _ => String::new(),
    }
}

/// 把一条记录转成消息（`type` 判据）。
#[must_use]
pub fn to_message(value: &Value) -> Option<Message> {
    let object = value.as_object()?;
    let role = gemini_role(
        object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )?;
    let text = extract_text(object.get("displayContent"));
    let text = if text.is_empty() {
        extract_text(object.get("content"))
    } else {
        text
    };
    if text.is_empty() {
        return None;
    }
    Some(Message {
        role,
        text,
        timestamp: string_field(object.get("timestamp")).map(Timestamp::from_raw),
        source_line: None,
    })
}

/// 提取模型名（仅 `type == "gemini"`）。
#[must_use]
pub fn extract_model(value: &Value) -> String {
    let Some(object) = value.as_object() else {
        return String::new();
    };
    if object.get("type").and_then(Value::as_str) != Some("gemini") {
        return String::new();
    }
    string_field(object.get("model")).unwrap_or_default()
}

/// 逐条累加的用量。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageAccumulator {
    /// 累计用量。
    pub total: Option<TokenUsage>,
    /// 最后一条的用量。
    pub last: Option<TokenUsage>,
    /// 最近一轮的输入 token（上下文占用）。
    pub context_used_tokens: Option<u64>,
    /// 上下文窗口。
    pub context_window: Option<u64>,
    /// 最后更新时间。
    pub updated_at: Option<Timestamp>,
}

impl UsageAccumulator {
    /// 从一条 `type == "gemini"` 记录收集用量。
    pub fn collect(&mut self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };
        if object.get("type").and_then(Value::as_str) != Some("gemini") {
            return;
        }
        let Some(tokens) = object.get("tokens").and_then(Value::as_object) else {
            return;
        };
        let Some(usage) = normalize_token_usage(tokens) else {
            return;
        };

        self.last = Some(usage);
        self.total = Some(add_token_usage(self.total.as_ref(), &usage));
        self.context_used_tokens = usage.input_tokens;
        self.context_window = Some(token_limit(
            object
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ));
        if let Some(timestamp) = string_field(object.get("timestamp")) {
            self.updated_at = Some(Timestamp::from_raw(timestamp));
        }
    }

    /// 构造会话用量。
    #[must_use]
    pub fn build(&self) -> Option<SessionUsage> {
        if self.total.is_none() && self.last.is_none() {
            return None;
        }
        let context_percent = match (self.context_used_tokens, self.context_window) {
            (Some(used), Some(window)) if window > 0 => {
                // 保留 1 位小数并封顶 100（现网 `Math.round(x * 1000) / 10`）。
                Some(
                    ((used as f64 / window as f64) * 1000.0)
                        .round()
                        .div_euclid(10.0)
                        .min(100.0),
                )
            }
            _ => None,
        };
        let context_left_percent =
            context_percent.map(|percent| ((100.0 - percent) * 10.0).round() / 10.0);
        Some(SessionUsage {
            total: self.total,
            last: self.last,
            context_window: self.context_window,
            context_used_tokens: self.context_used_tokens,
            context_percent,
            context_left_percent,
            rate_limits: None,
            updated_at: self.updated_at.clone(),
            source: UsageSource::GeminiMessageTokens,
        })
    }
}

/// 归一化 token 字段。
///
/// `total` 缺失时按 `input + cached + output + thoughts + tool` 求和——Gemini 在部分
/// 版本里不写 `total`，直接留空会让界面上显示 0。
#[must_use]
pub fn normalize_token_usage(tokens: &Map<String, Value>) -> Option<TokenUsage> {
    let output = number_field(tokens.get("output"));
    let thoughts = number_field(tokens.get("thoughts"));
    let mut usage = TokenUsage {
        input_tokens: number_field(tokens.get("input")),
        cached_input_tokens: number_field(tokens.get("cached")),
        output_tokens: output,
        reasoning_output_tokens: thoughts,
        total_tokens: number_field(tokens.get("total")),
    };
    if usage.total_tokens.is_none() {
        let total: u64 = [
            usage.input_tokens,
            usage.cached_input_tokens,
            output,
            thoughts,
            number_field(tokens.get("tool")),
        ]
        .into_iter()
        .flatten()
        .sum();
        if total > 0 {
            usage.total_tokens = Some(total);
        }
    }
    usage.has_any().then_some(usage)
}

/// 逐字段相加。
#[must_use]
pub fn add_token_usage(current: Option<&TokenUsage>, next: &TokenUsage) -> TokenUsage {
    let add = |left: Option<u64>, right: Option<u64>| match (left, right) {
        (None, None) => None,
        (left, right) => Some(left.unwrap_or(0) + right.unwrap_or(0)),
    };
    TokenUsage {
        input_tokens: add(
            current.and_then(|item| item.input_tokens),
            next.input_tokens,
        ),
        cached_input_tokens: add(
            current.and_then(|item| item.cached_input_tokens),
            next.cached_input_tokens,
        ),
        output_tokens: add(
            current.and_then(|item| item.output_tokens),
            next.output_tokens,
        ),
        reasoning_output_tokens: add(
            current.and_then(|item| item.reasoning_output_tokens),
            next.reasoning_output_tokens,
        ),
        total_tokens: add(
            current.and_then(|item| item.total_tokens),
            next.total_tokens,
        ),
    }
}

/// 读取有限非负数值。
///
/// 过滤器已确认数值有限且非负，因此截断转换不会丢符号。
#[allow(clippy::cast_sign_loss)]
fn number_field(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map(|number| number as u64)
}

/// 读取非空字符串字段。
fn string_field(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    (!text.is_empty()).then(|| text.to_owned())
}

/// 流式 Gemini 会话解析器。
#[derive(Debug)]
pub struct GeminiParser {
    info: SessionFileInfo,
    cwd: Option<String>,
    max_messages: Option<usize>,
    id: String,
    created_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    summary: Option<String>,
    user_title: String,
    kind: String,
    model: Option<String>,
    usage: UsageAccumulator,
    messages: Vec<Message>,
    message_count: u64,
    previous: Option<(Role, String)>,
}

impl GeminiParser {
    /// 构造。
    #[must_use]
    pub fn new(info: SessionFileInfo, cwd: Option<String>, max_messages: Option<usize>) -> Self {
        Self {
            info,
            cwd,
            max_messages,
            id: String::new(),
            created_at: None,
            updated_at: None,
            summary: None,
            user_title: String::new(),
            kind: "main".to_owned(),
            model: None,
            usage: UsageAccumulator::default(),
            messages: Vec::new(),
            message_count: 0,
            previous: None,
        }
    }

    /// 喂入一行。
    pub fn push(&mut self, line: &str) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return;
        }
        let Ok(item) = serde_json::from_str::<Value>(trimmed) else {
            return;
        };
        let Some(object) = item.as_object() else {
            return;
        };

        // 形态一：会话元数据。
        if let Some(session_id) = object.get("sessionId").and_then(Value::as_str) {
            if !session_id.is_empty() {
                session_id.clone_into(&mut self.id);
            }
            if let Some(value) = string_field(object.get("startTime")) {
                self.created_at = Some(Timestamp::from_raw(value));
            }
            if let Some(value) = string_field(object.get("lastUpdated")) {
                self.updated_at = Some(Timestamp::from_raw(value));
            }
            if let Some(value) = string_field(object.get("summary")) {
                self.summary = Some(value);
            }
            if let Some(value) = string_field(object.get("kind")) {
                self.kind = value;
            }
            return;
        }

        // 形态二：`$set` 补丁。
        if let Some(patch) = object.get("$set").and_then(Value::as_object) {
            if let Some(messages) = patch.get("messages").and_then(Value::as_array) {
                for message in messages {
                    self.count_message(message);
                    self.push_message(message);
                    self.usage.collect(message);
                    let model = extract_model(message);
                    if !model.is_empty() {
                        self.model = Some(model);
                    }
                }
            }
            if let Some(value) = string_field(patch.get("lastUpdated")) {
                self.updated_at = Some(Timestamp::from_raw(value));
            }
            if let Some(value) = string_field(patch.get("summary")) {
                self.summary = Some(value);
            }
        }

        // 形态三：单条消息。
        if object.get("type").and_then(Value::as_str).is_some() {
            self.count_message(&item);
            self.push_message(&item);
            self.usage.collect(&item);
            let model = extract_model(&item);
            if !model.is_empty() {
                self.model = Some(model);
            }
            if let Some(value) = string_field(object.get("timestamp")) {
                self.updated_at = Some(Timestamp::from_raw(value));
            }
        }
    }

    /// 计数：排除合成块，并对连续重复去重。
    fn count_message(&mut self, value: &Value) {
        let Some(message) = to_message(value) else {
            return;
        };
        // 必须用 Gemini 自己的合成块判据：Codex 的过滤器认的是 <environment_context>
        // 等前缀，认不出 <session_context>，会让合成块被计入消息数。
        if !should_keep_gemini_message(&message.text) {
            return;
        }
        if let Some((role, text)) = &self.previous
            && *role == message.role
            && text == &message.text
        {
            return;
        }
        self.previous = Some((message.role, message.text.clone()));
        self.message_count += 1;
    }

    /// 收集预览：注意这里**不**过滤合成块（与现网一致，由 `finish` 统一过滤）。
    fn push_message(&mut self, value: &Value) {
        let Some(object) = value.as_object() else {
            return;
        };
        let Some(role) = gemini_role(
            object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        ) else {
            return;
        };
        let retain = self
            .max_messages
            .is_none_or(|limit| self.messages.len() < limit);
        let needs_title = role == Role::User;
        if !retain && !needs_title {
            return;
        }
        let text = extract_text(object.get("displayContent"));
        let text = if text.is_empty() {
            extract_text(object.get("content"))
        } else {
            text
        };
        if text.is_empty() {
            return;
        }
        // 标题只取**第一条**用户消息，且即使超出预览窗口也要取到。
        if needs_title && self.user_title.is_empty() {
            self.user_title.clone_from(&text);
        }
        if !retain {
            return;
        }
        if let Some(previous) = self.messages.last()
            && previous.role == role
            && previous.text == text
        {
            return;
        }
        self.messages.push(Message {
            role,
            text,
            timestamp: string_field(object.get("timestamp")).map(Timestamp::from_raw),
            source_line: None,
        });
    }

    /// 结束并产出会话。
    #[must_use]
    pub fn finish(self) -> Option<Session> {
        // 子代理会话整体丢弃：它是主会话派生的内部会话，不属于会话列表。
        if self.kind == "subagent" {
            return None;
        }
        let id = if self.id.is_empty() {
            session_id_from_file_path(&self.info.file_path)
        } else {
            self.id
        };
        if id.is_empty() {
            return None;
        }

        let visible: Vec<Message> = self
            .messages
            .into_iter()
            .filter(|message| should_keep_gemini_message(&message.text))
            .collect();
        if visible.is_empty() {
            return None;
        }
        let first_visible_user = visible
            .iter()
            .find(|message| message.role == Role::User && !message.text.trim().is_empty());

        let mtime = Timestamp::from_unix_millis(self.info.mtime_ms as i64);
        let title_source = self
            .summary
            .clone()
            .filter(|value| !value.is_empty())
            .or_else(|| (!self.user_title.is_empty()).then(|| self.user_title.clone()))
            .or_else(|| first_visible_user.map(|message| message.text.clone()))
            .unwrap_or_else(|| id.clone());

        Some(Session {
            id: id.into(),
            title: clamp_title(&title_source),
            source_title: None,
            cwd: self.cwd,
            created_at: self.created_at,
            updated_at: self.updated_at.or(Some(mtime)),
            model: self.model.clone(),
            // Gemini 的模型状态没有独立记录，由解析出的模型名补上提供方。
            model_status: self.model.map(|model| ModelStatus {
                model: Some(model),
                reasoning: None,
                summaries: None,
                model_provider: Some("Gemini".to_owned()),
            }),
            cli_version: None,
            file_path: self.info.file_path,
            file_mtime_ms: Some(self.info.mtime_ms),
            file_change_ms: self.info.change_ms,
            file_size: Some(self.info.size),
            message_count: self.message_count,
            // 见 codex parser 的说明：置位归**读了头部窗口的调用方**。
            message_count_is_lower_bound: false,
            preview: visible,
            preview_offset: None,
            usage: self.usage.build(),
            metadata: None,
        })
    }
}

/// 便捷入口：解析整段文本。
#[must_use]
pub fn parse_content(
    info: SessionFileInfo,
    cwd: Option<String>,
    content: &str,
    max_messages: Option<usize>,
) -> Option<Session> {
    let mut parser = GeminiParser::new(info, cwd, max_messages);
    for line in content.split('\n') {
        parser.push(line.trim_end_matches('\r'));
    }
    parser.finish()
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

    fn meta(session_id: &str) -> String {
        format!(
            r#"{{"sessionId":"{session_id}","startTime":"2026-09-16T12:00:00.000Z","lastUpdated":"2026-09-16T12:05:00.000Z","summary":"会话摘要","kind":"main"}}"#
        )
    }

    fn user(text: &str) -> String {
        format!(r#"{{"type":"user","timestamp":"2026-09-16T12:00:01.000Z","content":"{text}"}}"#)
    }

    fn gemini(text: &str) -> String {
        format!(
            r#"{{"type":"gemini","timestamp":"2026-09-16T12:00:02.000Z","model":"gemini-2.5-pro","displayContent":[{{"text":"{text}"}}]}}"#
        )
    }

    #[test]
    fn session_id_is_extracted_from_the_file_name() {
        assert_eq!(
            session_id_from_file_path("/a/tmp/p/chats/session-2026-09-16T12-00-abcdef01.jsonl"),
            "abcdef01"
        );
        assert_eq!(session_id_from_file_path("/a/session-short.jsonl"), "");
        assert_eq!(session_id_from_file_path("/a/other.jsonl"), "");
        // 后缀必须恰好 8 位十六进制。
        assert_eq!(
            session_id_from_file_path("/a/session-2026-09-16T12-00-abcdef0123.jsonl"),
            ""
        );
        assert_eq!(
            session_id_from_file_path("/a/session-2026-09-16T12-00-zzzzzzzz.jsonl"),
            ""
        );
    }

    #[test]
    fn file_name_detection_matches_the_prefix_and_suffix() {
        assert!(is_session_file_name(
            "session-2026-09-16T12-00-abcdef01.jsonl"
        ));
        assert!(!is_session_file_name("other.jsonl"));
        assert!(!is_session_file_name("session-x.json"));
    }

    #[test]
    fn parsing_handles_all_three_record_shapes() {
        let content = [meta("sess-1"), user("第一条"), gemini("回答一")].join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            Some(8),
        )
        .expect("session");

        assert_eq!(session.id.as_str(), "sess-1");
        assert_eq!(session.title, "会话摘要");
        assert_eq!(session.message_count, 2);
        assert_eq!(session.preview.len(), 2);
        assert_eq!(session.preview[0].role, Role::User);
        assert_eq!(session.preview[1].role, Role::Assistant);
        assert_eq!(session.model.as_deref(), Some("gemini-2.5-pro"));
    }

    #[test]
    fn the_set_patch_shape_contributes_messages() {
        let content = [
            meta("sess-1"),
            r#"{"$set":{"messages":[{"type":"user","content":"补丁里的用户"},{"type":"gemini","model":"gemini-2.5-flash","content":"补丁里的回答"}],"summary":"补丁摘要"}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.message_count, 2);
        assert_eq!(session.title, "补丁摘要", "补丁的 summary 后写覆盖");
        assert_eq!(session.model.as_deref(), Some("gemini-2.5-flash"));
    }

    #[test]
    fn subagent_sessions_are_discarded_entirely() {
        let content = [
            r#"{"sessionId":"sess-1","kind":"subagent","summary":"子代理"}"#.to_owned(),
            user("内部消息"),
        ]
        .join("\n");
        assert!(
            parse_content(
                info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
                None,
                &content,
                None
            )
            .is_none(),
            "子代理会话不得出现在列表里"
        );
    }

    #[test]
    fn a_session_without_visible_messages_is_rejected() {
        let content = meta("sess-1");
        assert!(
            parse_content(
                info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
                None,
                &content,
                None
            )
            .is_none()
        );
    }

    #[test]
    fn the_id_falls_back_to_the_file_name() {
        let content = user("没有元数据");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.id.as_str(), "abcdef01");
    }

    #[test]
    fn synthetic_blocks_are_excluded_from_the_preview_and_the_count() {
        let content = [
            meta("sess-1"),
            r#"{"type":"user","content":"<session_context>注入"}"#.to_owned(),
            user("真实输入"),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.preview.len(), 1);
        assert_eq!(session.preview[0].text, "真实输入");
        // 该夹具带 summary，因此标题来自 summary；合成块只影响计数与预览。`n        assert_eq!(session.title, "会话摘要");
    }

    #[test]
    fn role_mapping_covers_every_known_type() {
        assert_eq!(gemini_role("user"), Some(Role::User));
        assert_eq!(gemini_role("gemini"), Some(Role::Assistant));
        assert_eq!(gemini_role("info"), Some(Role::System));
        assert_eq!(gemini_role("warning"), Some(Role::System));
        assert_eq!(gemini_role("error"), Some(Role::System));
        assert_eq!(gemini_role("unknown"), None);
    }

    #[test]
    fn usage_is_accumulated_and_context_uses_the_last_turn() {
        let content = [
            meta("sess-1"),
            r#"{"type":"gemini","model":"gemini-2.5-pro","timestamp":"2026-09-16T12:00:02.000Z","content":"回答一","tokens":{"input":100,"cached":10,"output":20,"thoughts":5,"total":135}}"#.to_owned(),
            r#"{"type":"gemini","model":"gemini-2.5-pro","timestamp":"2026-09-16T12:00:03.000Z","content":"回答二","tokens":{"input":200,"cached":0,"output":30,"thoughts":0,"total":230}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.source, UsageSource::GeminiMessageTokens);
        assert_eq!(usage.total.as_ref().unwrap().total_tokens, Some(365));
        assert_eq!(usage.total.as_ref().unwrap().input_tokens, Some(300));
        // 上下文占用取**最近一轮**的输入。
        assert_eq!(usage.context_used_tokens, Some(200));
        assert_eq!(usage.context_window, Some(DEFAULT_TOKEN_LIMIT));
    }

    #[test]
    fn missing_total_is_summed_from_the_other_dimensions() {
        let tokens: Map<String, Value> =
            serde_json::from_str(r#"{"input":10,"cached":5,"output":3,"thoughts":2,"tool":1}"#)
                .unwrap();
        let usage = normalize_token_usage(&tokens).expect("usage");
        assert_eq!(usage.total_tokens, Some(21), "10+5+3+2+1");
    }

    #[test]
    fn an_explicit_total_wins_over_the_sum() {
        let tokens: Map<String, Value> =
            serde_json::from_str(r#"{"input":10,"output":3,"total":999}"#).unwrap();
        assert_eq!(
            normalize_token_usage(&tokens).unwrap().total_tokens,
            Some(999)
        );
    }

    #[test]
    fn gemma_models_get_a_smaller_context_window() {
        assert_eq!(token_limit("gemini-2.5-pro"), DEFAULT_TOKEN_LIMIT);
        assert_eq!(token_limit("gemma-4-31b-it"), GEMMA_TOKEN_LIMIT);
        assert_eq!(token_limit("GEMMA-4-26B-A4B-IT"), GEMMA_TOKEN_LIMIT);
    }

    #[test]
    fn context_percent_keeps_one_decimal() {
        let mut accumulator = UsageAccumulator::default();
        let record: Value = serde_json::from_str(
            r#"{"type":"gemini","model":"gemma-4-31b-it","tokens":{"input":128000,"total":128000}}"#,
        )
        .unwrap();
        accumulator.collect(&record);
        let usage = accumulator.build().expect("usage");
        // 128000 / 256000 = 50%。
        assert_eq!(usage.context_percent, Some(50.0));
        assert_eq!(usage.context_left_percent, Some(50.0));
    }

    #[test]
    fn titles_fall_back_through_the_documented_chain() {
        // 无 summary → 用第一条用户消息。
        let content = [user("用户输入"), gemini("回答")].join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.title, "用户输入");

        // 完全没有用户消息 → 用编号。
        let content = gemini("只有助手");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.title, "abcdef01");
    }

    #[test]
    fn the_title_is_captured_outside_the_preview_window() {
        let content = [user("第一条"), gemini("回答"), user("第三条")].join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            Some(1),
        )
        .expect("session");
        assert_eq!(session.preview.len(), 1);
        assert_eq!(session.title, "第一条");
        assert_eq!(session.message_count, 3, "计数必须是全量");
    }

    #[test]
    fn consecutive_duplicates_are_deduplicated() {
        let content = [user("重复"), user("重复")].join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.message_count, 1);
    }

    #[test]
    fn updated_at_falls_back_to_the_file_mtime() {
        // 记录里没有任何时间戳时才回退到文件 mtime。
        let content = r#"{"type":"user","content":"无时间戳"}"#.to_owned();
        let session = parse_content(
            info(
                "/a/session-2026-09-16T12-00-abcdef01.jsonl",
                1_800_000_000_000.0,
            ),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("2027-01-15T08:00:00.000Z")
        );
    }

    #[test]
    fn model_status_carries_the_gemini_provider() {
        let content = [user("x"), gemini("回答")].join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        let status = session.model_status.expect("model status");
        assert_eq!(status.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(status.model_provider.as_deref(), Some("Gemini"));
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let content = ["not json".to_owned(), user("仍然解析")].join("\n");
        let session = parse_content(
            info("/a/session-2026-09-16T12-00-abcdef01.jsonl", 0.0),
            None,
            &content,
            None,
        )
        .expect("session");
        assert_eq!(session.message_count, 1);
    }
}
