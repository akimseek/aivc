//! Qoder CLI `*.jsonl` 解析。
//!
//! # 记录类型
//!
//! | `type` | 含义 |
//! | --- | --- |
//! | `user` / `assistant` | 对话消息，正文在 `message.content` |
//! | `ai-title` | 自动生成的标题（`aiTitle`） |
//! | `runtime-config` | 运行时配置，提供 `model` |
//! | 其余 | 忽略 |
//!
//! `isMeta` 与 `isSidechain` 为真时跳过：前者是注入的元信息，后者是子链（旁支）消息，
//! 都不属于主对话。
//!
//! # 时间戳只来自**消息**
//!
//! `createdAt` 取第一条消息的时间戳，`updatedAt` 取最后一条消息的时间戳——**不**取
//! 记录本身的时间戳。这是现网行为：`ai-title` / `runtime-config` 这类记录的时间戳
//! 描述的是「元信息何时写入」，不是「对话何时进行」，用它们会让会话时间线错位。
//!
//! # 用量来自助手消息
//!
//! 只有 `type == "assistant"` 的 `message.usage` 计入。`context_usage_ratio` 是**比例**
//! 而不是百分比：≤1 时乘 100，否则原样使用（不同版本给的单位不同）。

use serde_json::Value;

use aivc_core::session::{
    Message, ModelStatus, Role, Session, SessionUsage, TokenUsage, UsageSource,
};
use aivc_core::text::clamp_title;
use aivc_core::time::Timestamp;

use crate::common::SessionFileInfo;

/// 列表模式的预览条数上限。
pub const LIST_PREVIEW_LIMIT: usize = 8;
/// 会话文件扩展名。
pub const SESSION_FILE_SUFFIX: &str = ".jsonl";

/// 逐条累加的用量。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageAccumulator {
    /// 累计用量。
    pub total: Option<TokenUsage>,
    /// 最后一条助手消息的用量。
    pub last: Option<TokenUsage>,
    /// 上下文占用百分比（已归一化到 0–100）。
    pub context_percent: Option<f64>,
    /// 最后更新时间。
    pub updated_at: Option<Timestamp>,
}

impl UsageAccumulator {
    /// 从一条记录收集用量。
    pub fn collect(&mut self, record: &Value) {
        let Some(object) = record.as_object() else {
            return;
        };
        if object.get("type").and_then(Value::as_str) != Some("assistant") {
            return;
        }
        let raw = object
            .get("message")
            .and_then(Value::as_object)
            .and_then(|message| message.get("usage"))
            .and_then(Value::as_object);

        if let Some(raw) = raw {
            let next = TokenUsage {
                input_tokens: number_field(raw.get("input_tokens")),
                cached_input_tokens: number_field(raw.get("cache_read_input_tokens")),
                output_tokens: number_field(raw.get("output_tokens")),
                reasoning_output_tokens: None,
                total_tokens: None,
            };
            let total: u64 = [
                next.input_tokens,
                next.cached_input_tokens,
                next.output_tokens,
            ]
            .into_iter()
            .flatten()
            .sum();
            let next = TokenUsage {
                total_tokens: (total > 0).then_some(total),
                ..next
            };
            if next.has_any() {
                self.last = Some(next);
                self.total = Some(add_token_usage(self.total.as_ref(), &next));
            }
            // 比例而非百分比：≤1 视为比例，否则原样（不同版本单位不同）。
            if let Some(ratio) = raw.get("context_usage_ratio").and_then(Value::as_f64)
                && ratio.is_finite()
            {
                self.context_percent =
                    Some((if ratio <= 1.0 { ratio * 100.0 } else { ratio }).clamp(0.0, 100.0));
            }
        }
        if let Some(timestamp) = string_field(object.get("timestamp")) {
            self.updated_at = Some(Timestamp::from_raw(timestamp));
        }
    }

    /// 构造会话用量。
    #[must_use]
    pub fn build(&self) -> Option<SessionUsage> {
        if self.total.is_none() && self.last.is_none() && self.context_percent.is_none() {
            return None;
        }
        // 保留 1 位小数（现网 `Math.round(x * 10) / 10`）。
        let context_percent = self
            .context_percent
            .map(|value| (value * 10.0).round() / 10.0);
        Some(SessionUsage {
            total: self.total,
            last: self.last,
            context_window: None,
            context_used_tokens: None,
            context_percent,
            context_left_percent: context_percent
                .map(|percent| ((100.0 - percent).max(0.0) * 10.0).round() / 10.0),
            rate_limits: None,
            updated_at: self.updated_at.clone(),
            source: UsageSource::QoderMessageUsage,
        })
    }
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

/// 取数组里第一个字符串元素（`directories` 用）。
#[must_use]
pub fn first_string(value: Option<&Value>) -> Option<String> {
    let items = value?.as_array()?;
    items
        .iter()
        .find_map(|item| item.as_str())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// 提取助手正文：字符串，或 `{text}` 数组。
#[must_use]
pub fn extract_assistant_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(parts)) => {
            let joined: Vec<&str> = parts
                .iter()
                .filter_map(|part| part.as_object()?.get("text")?.as_str())
                .filter(|text| !text.is_empty())
                .collect();
            joined.join("\n").trim().to_owned()
        }
        _ => String::new(),
    }
}

/// 把一条记录转成消息。
#[must_use]
pub fn to_message(record: &Value) -> Option<Message> {
    let object = record.as_object()?;
    // 元信息与子链消息都不属于主对话。
    if object.get("isMeta").and_then(Value::as_bool) == Some(true)
        || object.get("isSidechain").and_then(Value::as_bool) == Some(true)
    {
        return None;
    }
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message = object.get("message").and_then(Value::as_object);
    let timestamp = string_field(object.get("timestamp")).map(Timestamp::from_raw);

    match kind {
        "user" => {
            let text = message
                .and_then(|item| item.get("content"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned();
            (!text.is_empty()).then_some(Message {
                role: Role::User,
                text,
                timestamp,
                source_line: None,
            })
        }
        "assistant" => {
            let text = extract_assistant_text(message.and_then(|item| item.get("content")));
            (!text.is_empty()).then_some(Message {
                role: Role::Assistant,
                text,
                timestamp,
                source_line: None,
            })
        }
        _ => None,
    }
}

/// 流式 Qoder 会话解析器。
#[derive(Debug)]
pub struct QoderParser {
    info: SessionFileInfo,
    max_messages: Option<usize>,
    id: String,
    title: String,
    cwd: Option<String>,
    model: Option<String>,
    created_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    first_user_text: String,
    usage: UsageAccumulator,
    messages: Vec<Message>,
    message_count: u64,
    previous: Option<(Role, String)>,
}

impl QoderParser {
    /// 构造。
    #[must_use]
    pub fn new(info: SessionFileInfo, max_messages: Option<usize>) -> Self {
        Self {
            info,
            max_messages,
            id: String::new(),
            title: String::new(),
            cwd: None,
            model: None,
            created_at: None,
            updated_at: None,
            first_user_text: String::new(),
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
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            return;
        };
        let Some(object) = record.as_object() else {
            return;
        };

        if self.id.is_empty()
            && let Some(value) = string_field(object.get("sessionId"))
        {
            self.id = value;
        }
        match object.get("type").and_then(Value::as_str) {
            Some("ai-title") => {
                if let Some(value) = string_field(object.get("aiTitle")) {
                    self.title = value;
                }
            }
            Some("runtime-config") => {
                if let Some(value) = string_field(object.get("model")) {
                    self.model = Some(value);
                }
            }
            _ => {}
        }
        if self.cwd.is_none() {
            self.cwd =
                string_field(object.get("cwd")).or_else(|| first_string(object.get("directories")));
        }

        let Some(message) = to_message(&record) else {
            return;
        };
        // 连续同 role + 同正文去重。
        if let Some((role, text)) = &self.previous
            && *role == message.role
            && text == &message.text
        {
            return;
        }
        self.previous = Some((message.role, message.text.clone()));

        // 时间线只由消息驱动：元信息记录的时间戳描述的是「何时写入元信息」。
        if self.created_at.is_none() {
            self.created_at.clone_from(&message.timestamp);
        }
        if message.timestamp.is_some() {
            self.updated_at.clone_from(&message.timestamp);
        }
        if message.role == Role::User && self.first_user_text.is_empty() {
            self.first_user_text.clone_from(&message.text);
        }
        self.message_count += 1;
        self.usage.collect(&record);
        if self
            .max_messages
            .is_none_or(|limit| self.messages.len() < limit)
        {
            self.messages.push(message);
        }
    }

    /// 结束并产出会话。
    #[must_use]
    pub fn finish(self) -> Option<Session> {
        // 没有编号或没有任何消息都不算会话。
        if self.id.is_empty() || self.message_count == 0 {
            return None;
        }
        let mtime = Timestamp::from_unix_millis(self.info.mtime_ms as i64);
        let title_source = if !self.title.is_empty() {
            self.title
        } else if !self.first_user_text.is_empty() {
            self.first_user_text
        } else {
            self.id.clone()
        };
        let model = self.model;
        Some(Session {
            id: self.id.into(),
            title: clamp_title(&title_source),
            source_title: None,
            cwd: self.cwd,
            created_at: self.created_at.or_else(|| Some(mtime.clone())),
            updated_at: self.updated_at.or(Some(mtime)),
            model: model.clone(),
            model_status: Some(ModelStatus {
                model,
                reasoning: None,
                summaries: None,
                model_provider: Some("Qoder".to_owned()),
            }),
            cli_version: None,
            file_path: self.info.file_path,
            file_mtime_ms: Some(self.info.mtime_ms),
            file_change_ms: self.info.change_ms,
            file_size: Some(self.info.size),
            message_count: self.message_count,
            // 见 codex parser 的说明：置位归**读了头部窗口的调用方**。
            message_count_is_lower_bound: false,
            preview: self.messages,
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
    content: &str,
    max_messages: Option<usize>,
) -> Option<Session> {
    let mut parser = QoderParser::new(info, max_messages);
    for line in content.split('\n') {
        parser.push(line.trim_end_matches('\r'));
    }
    parser.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(mtime: f64) -> SessionFileInfo {
        SessionFileInfo {
            file_path: "/a/projects/proj1/abc.jsonl".to_owned(),
            mtime_ms: mtime,
            change_ms: Some(mtime),
            size: 100,
        }
    }

    fn meta(session_id: &str) -> String {
        format!(
            r#"{{"type":"session-meta","sessionId":"{session_id}","cwd":"/work","timestamp":"2026-09-16T11:00:00.000Z"}}"#
        )
    }

    fn user(text: &str, timestamp: &str) -> String {
        format!(r#"{{"type":"user","timestamp":"{timestamp}","message":{{"content":"{text}"}}}}"#)
    }

    fn assistant(text: &str, timestamp: &str) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"{timestamp}","message":{{"content":[{{"text":"{text}"}}],"usage":{{"input_tokens":100,"cache_read_input_tokens":20,"output_tokens":30,"context_usage_ratio":0.25}}}}}}"#
        )
    }

    #[test]
    fn parsing_extracts_metadata_and_messages() {
        let content = [
            meta("sess-1"),
            r#"{"type":"ai-title","sessionId":"sess-1","aiTitle":"自动标题"}"#.to_owned(),
            r#"{"type":"runtime-config","sessionId":"sess-1","model":"qoder-pro"}"#.to_owned(),
            user("第一条", "2026-09-16T12:00:00.000Z"),
            assistant("回答", "2026-09-16T12:00:01.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, Some(8)).expect("session");

        assert_eq!(session.id.as_str(), "sess-1");
        assert_eq!(session.title, "自动标题");
        assert_eq!(session.cwd.as_deref(), Some("/work"));
        assert_eq!(session.model.as_deref(), Some("qoder-pro"));
        assert_eq!(session.message_count, 2);
        assert_eq!(session.preview.len(), 2);
        assert_eq!(session.preview[0].role, Role::User);
        assert_eq!(session.preview[1].role, Role::Assistant);
    }

    #[test]
    fn the_timeline_comes_from_messages_not_records() {
        // 元信息记录的时间戳（11:00）早于消息（12:00），不得影响会话时间线。
        let content = [
            meta("sess-1"),
            user("第一条", "2026-09-16T12:00:00.000Z"),
            assistant("回答", "2026-09-16T12:00:05.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(
            session.created_at.as_ref().map(Timestamp::as_str),
            Some("2026-09-16T12:00:00.000Z")
        );
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("2026-09-16T12:00:05.000Z")
        );
    }

    #[test]
    fn cwd_falls_back_to_the_first_directory_entry() {
        let content = [
            r#"{"type":"session-meta","sessionId":"s","directories":["/first","/second"]}"#
                .to_owned(),
            user("x", "2026-09-16T12:00:00.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.cwd.as_deref(), Some("/first"));
    }

    #[test]
    fn meta_and_sidechain_records_are_skipped() {
        let content = [
            meta("sess-1"),
            r#"{"type":"user","sessionId":"sess-1","isMeta":true,"message":{"content":"元信息"}}"#.to_owned(),
            r#"{"type":"assistant","sessionId":"sess-1","isSidechain":true,"message":{"content":"旁支"}}"#.to_owned(),
            user("真实", "2026-09-16T12:00:00.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.preview[0].text, "真实");
    }

    #[test]
    fn assistant_content_can_be_a_string_or_an_array() {
        let string_form = [
            meta("s"),
            r#"{"type":"assistant","timestamp":"2026-09-16T12:00:00.000Z","message":{"content":"  直接字符串  "}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &string_form, None).expect("session");
        assert_eq!(session.preview[0].text, "直接字符串");

        let array_form = [
            meta("s"),
            r#"{"type":"assistant","timestamp":"2026-09-16T12:00:00.000Z","message":{"content":[{"text":"第一段"},{"text":"第二段"}]}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &array_form, None).expect("session");
        assert_eq!(session.preview[0].text, "第一段\n第二段");
    }

    #[test]
    fn usage_accumulates_and_the_context_ratio_is_normalized() {
        let content = [
            meta("s"),
            user("问题", "2026-09-16T12:00:00.000Z"),
            assistant("回答一", "2026-09-16T12:00:01.000Z"),
            assistant("回答二", "2026-09-16T12:00:02.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.source, UsageSource::QoderMessageUsage);
        // 每条 100+20+30 = 150，两条共 300。
        assert_eq!(usage.total.as_ref().unwrap().total_tokens, Some(300));
        assert_eq!(usage.last.as_ref().unwrap().total_tokens, Some(150));
        // 比例 0.25 → 25%。
        assert_eq!(usage.context_percent, Some(25.0));
        assert_eq!(usage.context_left_percent, Some(75.0));
    }

    #[test]
    fn a_ratio_above_one_is_treated_as_a_percentage() {
        let content = [
            meta("s"),
            r#"{"type":"assistant","timestamp":"2026-09-16T12:00:00.000Z","message":{"content":"x","usage":{"context_usage_ratio":42}}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.usage.expect("usage").context_percent, Some(42.0));
    }

    #[test]
    fn the_title_falls_back_through_the_documented_chain() {
        // 无 ai-title → 第一条用户消息。
        let content = [meta("s"), user("用户输入", "2026-09-16T12:00:00.000Z")].join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.title, "用户输入");

        // 只有助手消息 → 会话编号。
        let content = [
            meta("sess-id"),
            assistant("只有助手", "2026-09-16T12:00:00.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.title, "sess-id");
    }

    #[test]
    fn a_session_without_an_id_or_messages_is_rejected() {
        // 有编号无消息。
        let content = meta("sess-1");
        assert!(parse_content(info(0.0), &content, None).is_none());

        // 有消息无编号。
        let content = user("x", "2026-09-16T12:00:00.000Z");
        assert!(parse_content(info(0.0), &content, None).is_none());
    }

    #[test]
    fn timestamps_fall_back_to_the_file_mtime() {
        let content = [
            meta("s"),
            r#"{"type":"user","message":{"content":"无时间戳"}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(info(1_800_000_000_000.0), &content, None).expect("session");
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("2027-01-15T08:00:00.000Z")
        );
    }

    #[test]
    fn consecutive_duplicates_are_deduplicated() {
        let content = [
            meta("s"),
            user("重复", "2026-09-16T12:00:00.000Z"),
            user("重复", "2026-09-16T12:00:01.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.message_count, 1);
    }

    #[test]
    fn the_preview_is_capped_but_the_count_is_not() {
        let mut lines = vec![meta("s")];
        for index in 0..20 {
            lines.push(user(
                &format!("消息{index}"),
                &format!("2026-09-16T12:00:{index:02}.000Z"),
            ));
        }
        let session =
            parse_content(info(0.0), &lines.join("\n"), Some(LIST_PREVIEW_LIMIT)).expect("session");
        assert_eq!(session.preview.len(), LIST_PREVIEW_LIMIT);
        assert_eq!(session.message_count, 20);
    }

    #[test]
    fn model_status_carries_the_qoder_provider() {
        let content = [
            meta("s"),
            r#"{"type":"runtime-config","sessionId":"s","model":"qoder-pro"}"#.to_owned(),
            user("x", "2026-09-16T12:00:00.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        let status = session.model_status.expect("model status");
        assert_eq!(status.model.as_deref(), Some("qoder-pro"));
        assert_eq!(status.model_provider.as_deref(), Some("Qoder"));
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let content = [
            "not json".to_owned(),
            meta("s"),
            user("仍然解析", "2026-09-16T12:00:00.000Z"),
        ]
        .join("\n");
        let session = parse_content(info(0.0), &content, None).expect("session");
        assert_eq!(session.message_count, 1);
    }
}
