//! Claude Code `~/.claude/projects/<编码后的工作目录>/<会话编号>.jsonl` 解析。
//!
//! # 记录类型
//!
//! - `type: "user" | "assistant"`：正文在 `message.content`，角色在 `message.role`
//! - `type: "last-prompt"`：`lastPrompt` 提供**兜底标题**（用户最后一条输入）
//! - 其余类型一律忽略
//!
//! `isMeta === true` 的记录必须跳过：它们是 Claude 自己注入的元信息，不是对话内容。
//!
//! # 与 Codex 的关键差异
//!
//! | 维度 | Codex | Claude |
//! | --- | --- | --- |
//! | 会话编号 | 文件名里的 uuid，可被 `session_meta.id` 覆盖 | 记录里的 `sessionId`，回退文件名 |
//! | `updatedAt` | 列表模式用文件 mtime；详情用最后一条记录 | **两者取较晚者**（`latestIsoTimestamp`） |
//! | 用量 | 单条 `token_count` 记录给出全量 | 逐条 `message.usage` **累加** |
//! | 上下文窗口 | 记录里直接给 | 由模型名**推断** |
//!
//! # 上下文窗口为什么靠推断
//!
//! Claude 的 JSONL 不含上下文窗口大小，而 `contextLeftPercent` 是界面上的核心指标。
//! 现网按模型名推断：含 `[1m]` 或 `opus-4-8` 视为 100 万，`claude-` 开头视为 20 万，
//! 其余不给。更精确的值来自 telemetry 文件（由 Provider 层补上）。

use serde_json::Value;

use aivc_core::session::{Message, Role, Session, SessionUsage, TokenUsage, UsageSource};
use aivc_core::text::normalize_whitespace;
use aivc_core::time::Timestamp;

use crate::common::SessionFileInfo;

/// 列表模式的预览条数上限。
pub const LIST_PREVIEW_LIMIT: usize = 8;
/// 默认上下文窗口（20 万）。
pub const DEFAULT_CONTEXT_WINDOW: u64 = 200_000;
/// 长上下文窗口（100 万）。
pub const ONE_MILLION_CONTEXT_WINDOW: u64 = 1_000_000;
/// 标题的最大长度。
pub const TITLE_MAX_LENGTH: usize = 160;

/// 逐条累加的用量。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageAccumulator {
    /// 累计用量。
    pub total: TokenUsage,
    /// 最后一条助手消息的用量。
    pub last: Option<TokenUsage>,
    /// 最后一次用量的时间戳。
    pub updated_at: Option<Timestamp>,
}

impl UsageAccumulator {
    /// 累加一条助手消息的用量。
    ///
    /// 三个输入维度相加：`input_tokens` + `cache_creation_input_tokens` +
    /// `cache_read_input_tokens`，再加 `output_tokens`。缓存命中与新建缓存都是输入，
    /// 只是计费不同——漏掉任一项都会让 Token 统计偏低。
    pub fn accumulate(&mut self, usage: Option<&Value>, timestamp: Option<&Timestamp>) {
        let Some(object) = usage.and_then(Value::as_object) else {
            return;
        };
        let input = read_number(object.get("input_tokens"));
        let cache_creation = read_number(object.get("cache_creation_input_tokens"));
        let cache_read = read_number(object.get("cache_read_input_tokens"));
        let output = read_number(object.get("output_tokens"));
        let cached = cache_creation + cache_read;
        let total = input + cached + output;
        if total == 0 {
            return;
        }

        self.total.input_tokens = Some(self.total.input_tokens.unwrap_or(0) + input);
        self.total.cached_input_tokens = Some(self.total.cached_input_tokens.unwrap_or(0) + cached);
        self.total.output_tokens = Some(self.total.output_tokens.unwrap_or(0) + output);
        self.total.total_tokens = Some(self.total.total_tokens.unwrap_or(0) + total);
        self.last = Some(TokenUsage {
            input_tokens: Some(input),
            cached_input_tokens: Some(cached),
            output_tokens: Some(output),
            reasoning_output_tokens: None,
            total_tokens: Some(total),
        });
        if let Some(timestamp) = timestamp {
            self.updated_at = Some(timestamp.clone());
        }
    }

    /// 是否累计到了任何用量。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total.total_tokens.unwrap_or(0) == 0
    }
}

/// 构造会话用量。
///
/// `context_used_tokens` 取**最近一轮**的输入（含缓存），而不是累计输入——上下文占用
/// 是「当前窗口里有多少」，累计值会随时间单调增长到远超窗口大小。
#[must_use]
pub fn build_usage(
    accumulator: &UsageAccumulator,
    context_window: Option<u64>,
) -> Option<SessionUsage> {
    if accumulator.is_empty() {
        return None;
    }
    let context_used_tokens = accumulator.last.as_ref().map_or(0, |last| {
        last.input_tokens.unwrap_or(0) + last.cached_input_tokens.unwrap_or(0)
    });
    // 百分比按整数四舍五入（现网 `Math.round`），与 Codex 的 1 位小数不同。
    let context_percent = match (context_window, context_used_tokens) {
        (Some(window), used) if window > 0 && used > 0 => {
            Some(((used as f64 / window as f64) * 100.0).round().min(100.0))
        }
        _ => None,
    };
    Some(SessionUsage {
        total: Some(accumulator.total),
        last: accumulator.last,
        context_window,
        context_used_tokens: (context_used_tokens > 0).then_some(context_used_tokens),
        context_percent,
        context_left_percent: context_percent.map(|percent| (100.0 - percent).max(0.0)),
        rate_limits: None,
        updated_at: accumulator.updated_at.clone(),
        source: UsageSource::ClaudeMessageUsage,
    })
}

/// 按模型名推断上下文窗口。
#[must_use]
pub fn infer_context_window(model: &str) -> Option<u64> {
    let normalized = model.to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if normalized.contains("[1m]") || normalized.contains("opus-4-8") {
        return Some(ONE_MILLION_CONTEXT_WINDOW);
    }
    if normalized.starts_with("claude-") {
        return Some(DEFAULT_CONTEXT_WINDOW);
    }
    None
}

/// 按 telemetry 信息推断上下文窗口（beta 标记优先）。
#[must_use]
pub fn infer_telemetry_context_window(model: &str, betas: &str) -> Option<u64> {
    let normalized_model = model.to_lowercase();
    let normalized_betas = betas.to_lowercase();
    if normalized_model.contains("[1m]") || normalized_betas.contains("context-1m") {
        return Some(ONE_MILLION_CONTEXT_WINDOW);
    }
    None
}

/// 用已知的上下文窗口重算百分比。
#[must_use]
pub fn apply_context_window(usage: &SessionUsage, context_window: u64) -> SessionUsage {
    let context_used_tokens = usage.context_used_tokens.unwrap_or_else(|| {
        usage.last.as_ref().map_or(0, |last| {
            last.input_tokens.unwrap_or(0) + last.cached_input_tokens.unwrap_or(0)
        })
    });
    let context_percent = if context_used_tokens > 0 && context_window > 0 {
        Some(
            ((context_used_tokens as f64 / context_window as f64) * 100.0)
                .round()
                .min(100.0),
        )
    } else {
        None
    };
    SessionUsage {
        context_window: Some(context_window),
        context_used_tokens: (context_used_tokens > 0).then_some(context_used_tokens),
        context_percent,
        context_left_percent: context_percent.map(|percent| (100.0 - percent).max(0.0)),
        ..usage.clone()
    }
}

/// 标题压缩：折叠空白并截断。
///
/// 用**纯截断**而不是 `clamp_raw`：后者会补省略号，而 Claude 的标题上限是按字符数
/// 硬截的，补省略号会让结果超出上限。
#[must_use]
pub fn compact_title(value: &str) -> String {
    normalize_whitespace(value)
        .chars()
        .take(TITLE_MAX_LENGTH)
        .collect()
}

/// 取两个 ISO 时间戳中较晚的一个。
#[must_use]
pub fn latest_iso_timestamp(
    left: Option<&Timestamp>,
    right: Option<&Timestamp>,
) -> Option<Timestamp> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let left_ms = left.to_unix_millis();
            let right_ms = right.to_unix_millis();
            match (left_ms, right_ms) {
                (Some(left_ms), Some(right_ms)) => Some(if right_ms > left_ms {
                    right.clone()
                } else {
                    left.clone()
                }),
                // 解析失败时退回另一个（现网的 `Number.isFinite` 判断）。
                (None, Some(_)) => Some(right.clone()),
                (Some(_), None) => Some(left.clone()),
                (None, None) => None,
            }
        }
        (Some(value), None) | (None, Some(value)) => Some(value.clone()),
        (None, None) => None,
    }
}

/// 从 `message.content` 提取正文。
///
/// `content` 既可能是字符串，也可能是数组；数组元素可能是字符串，也可能是带 `text`
/// 或 `content` 字段的对象。四种形态都要处理——Claude 在不同版本里都用过。
#[must_use]
pub fn extract_content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_owned(),
        Some(Value::Array(items)) => {
            let parts: Vec<&str> = items
                .iter()
                .filter_map(|item| match item {
                    Value::String(text) => Some(text.as_str()),
                    Value::Object(object) => object
                        .get("text")
                        .and_then(Value::as_str)
                        .or_else(|| object.get("content").and_then(Value::as_str)),
                    _ => None,
                })
                .filter(|text| !text.is_empty())
                .collect();
            parts.join("\n").trim().to_owned()
        }
        _ => String::new(),
    }
}

/// 读取有限数值；非法值按 0 处理（现网 `readNumber`）。
///
/// 过滤器已确认数值有限且非负，因此截断转换不会丢符号。
#[allow(clippy::cast_sign_loss)]
fn read_number(value: Option<&Value>) -> u64 {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite() && *number >= 0.0)
        .map_or(0, |number| number as u64)
}

/// 判断一条记录是否是**可计数的可见消息**。
///
/// 与解析时的判据保持一致：分支截断依赖它决定「保留到第几条消息」。
#[must_use]
pub fn is_countable_message(record: &Value) -> bool {
    let Some(object) = record.as_object() else {
        return false;
    };
    if object.get("type").and_then(Value::as_str) != Some("user")
        && object.get("type").and_then(Value::as_str) != Some("assistant")
    {
        return false;
    }
    if object.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return false;
    }
    let Some(message) = object.get("message").and_then(Value::as_object) else {
        return false;
    };
    message_role(message.get("role")).is_some()
        && !extract_content_text(message.get("content")).is_empty()
}

/// `message.role` 映射。
fn message_role(value: Option<&Value>) -> Option<Role> {
    match value.and_then(Value::as_str) {
        Some("assistant") => Some(Role::Assistant),
        Some("user") => Some(Role::User),
        _ => None,
    }
}

/// 流式 Claude 会话解析器。
#[derive(Debug)]
pub struct ClaudeParser {
    info: SessionFileInfo,
    max_messages: Option<usize>,
    resolved_session_id: String,
    cwd: Option<String>,
    model: Option<String>,
    created_at: Option<Timestamp>,
    updated_at: Option<Timestamp>,
    fallback_title: String,
    user_title: String,
    usage: UsageAccumulator,
    messages: Vec<Message>,
    message_count: u64,
    previous: Option<(Role, String)>,
}

impl ClaudeParser {
    /// 构造。`max_messages` 为 [`None`] 时保留全部消息。
    #[must_use]
    pub fn new(info: SessionFileInfo, max_messages: Option<usize>) -> Self {
        Self {
            info,
            max_messages,
            resolved_session_id: String::new(),
            cwd: None,
            model: None,
            created_at: None,
            updated_at: None,
            fallback_title: String::new(),
            user_title: String::new(),
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

        if self.resolved_session_id.is_empty()
            && let Some(value) = object.get("sessionId").and_then(Value::as_str)
            && !value.is_empty()
        {
            value.clone_into(&mut self.resolved_session_id);
        }
        let timestamp = object
            .get("timestamp")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(Timestamp::from_raw);
        if self.created_at.is_none() {
            self.created_at.clone_from(&timestamp);
        }
        if timestamp.is_some() {
            self.updated_at.clone_from(&timestamp);
        }
        if self.cwd.is_none()
            && let Some(value) = object.get("cwd").and_then(Value::as_str)
        {
            self.cwd = Some(value.to_owned());
        }

        // 兜底标题：用户最后一次输入。
        if object.get("type").and_then(Value::as_str) == Some("last-prompt")
            && let Some(value) = object.get("lastPrompt").and_then(Value::as_str)
        {
            value.clone_into(&mut self.fallback_title);
            return;
        }

        let kind = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if kind != "user" && kind != "assistant" {
            return;
        }
        let Some(message) = object.get("message").and_then(Value::as_object) else {
            return;
        };
        let Some(role) = message_role(message.get("role")) else {
            return;
        };
        // 元信息记录不是对话内容。
        if object.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return;
        }

        if role == Role::Assistant
            && self.model.is_none()
            && let Some(value) = message.get("model").and_then(Value::as_str)
        {
            self.model = Some(value.to_owned());
        }
        if role == Role::Assistant {
            self.usage
                .accumulate(message.get("usage"), timestamp.as_ref());
        }

        let text = extract_content_text(message.get("content"));
        if text.is_empty() {
            return;
        }
        // 连续同 role + 同正文去重。
        if let Some((previous_role, previous_text)) = &self.previous
            && *previous_role == role
            && previous_text == &text
        {
            return;
        }
        self.previous = Some((role, text.clone()));
        self.message_count += 1;

        let retain = self
            .max_messages
            .is_none_or(|limit| self.messages.len() < limit);
        // 标题必须取**第一条**用户消息，而它可能落在预览窗口之外，因此即使不保留
        // 消息也要把它记下来。
        let needs_title = role == Role::User && self.user_title.is_empty();
        if !retain && !needs_title {
            return;
        }
        if needs_title {
            self.user_title.clone_from(&text);
        }
        if retain {
            self.messages.push(Message {
                role,
                text,
                timestamp,
                source_line: None,
            });
        }
    }

    /// 结束并产出会话。
    #[must_use]
    pub fn finish(self) -> Option<Session> {
        let file_name_id = session_id_from_file_path(&self.info.file_path);
        if self.messages.is_empty() && self.resolved_session_id.is_empty() {
            return None;
        }
        let mtime = Timestamp::from_unix_millis(self.info.mtime_ms as i64);
        // 与 Codex 不同：这里取「最后一条记录的时间」与「文件 mtime」中**较晚者**。
        // 只取记录时间会让刚写入但还没刷盘的时间戳落后；只取 mtime 会在文件被复制
        // 或恢复后把时间推后到不真实的未来。
        let effective_updated_at = latest_iso_timestamp(self.updated_at.as_ref(), Some(&mtime));
        let title_source = if !self.user_title.is_empty() {
            self.user_title.clone()
        } else if !self.fallback_title.is_empty() {
            self.fallback_title.clone()
        } else if !self.resolved_session_id.is_empty() {
            self.resolved_session_id.clone()
        } else {
            file_name_id.clone()
        };
        let context_window = infer_context_window(self.model.as_deref().unwrap_or_default());
        let usage = build_usage(&self.usage, context_window);
        let id = if self.resolved_session_id.is_empty() {
            file_name_id
        } else {
            self.resolved_session_id
        };
        if id.is_empty() {
            return None;
        }

        Some(Session {
            id: id.into(),
            title: compact_title(&title_source),
            source_title: None,
            cwd: self.cwd,
            created_at: self.created_at.or_else(|| Some(mtime.clone())),
            updated_at: effective_updated_at.or(Some(mtime)),
            model: self.model,
            model_status: None,
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
            usage,
            metadata: None,
        })
    }
}

/// 从文件路径提取会话编号（文件名去掉 `.jsonl`）。
#[must_use]
pub fn session_id_from_file_path(file_path: &str) -> String {
    let name = file_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(file_path)
        .to_owned();
    name.strip_suffix(".jsonl").unwrap_or(&name).to_owned()
}

/// 便捷入口：解析整段文本。
#[must_use]
pub fn parse_content(
    info: SessionFileInfo,
    content: &str,
    max_messages: Option<usize>,
) -> Option<Session> {
    let mut parser = ClaudeParser::new(info, max_messages);
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

    fn user_line(timestamp: &str, text: &str) -> String {
        format!(
            r#"{{"type":"user","sessionId":"sess-1","timestamp":"{timestamp}","cwd":"/work","message":{{"role":"user","content":"{text}"}}}}"#
        )
    }

    fn assistant_line(timestamp: &str, text: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"sess-1","timestamp":"{timestamp}","message":{{"role":"assistant","model":"claude-sonnet-4","content":[{{"type":"text","text":"{text}"}}],"usage":{{"input_tokens":100,"cache_creation_input_tokens":10,"cache_read_input_tokens":20,"output_tokens":30}}}}}}"#
        )
    }

    #[test]
    fn session_id_comes_from_the_record_and_falls_back_to_the_file_name() {
        assert_eq!(session_id_from_file_path("/a/b/abc-123.jsonl"), "abc-123");
        assert_eq!(session_id_from_file_path("C:\\a\\abc.jsonl"), "abc");
        assert_eq!(session_id_from_file_path("/a/no-extension"), "no-extension");
    }

    #[test]
    fn parsing_extracts_messages_title_and_metadata() {
        let content = [
            user_line("2026-09-16T12:00:00.000Z", "帮我看看"),
            assistant_line("2026-09-16T12:00:01.000Z", "好的"),
        ]
        .join("\n");
        let session =
            parse_content(info("/a/sess-1.jsonl", 0.0), &content, Some(8)).expect("session");

        assert_eq!(session.id.as_str(), "sess-1");
        assert_eq!(session.title, "帮我看看");
        assert_eq!(session.cwd.as_deref(), Some("/work"));
        assert_eq!(session.model.as_deref(), Some("claude-sonnet-4"));
        assert_eq!(session.message_count, 2);
        assert_eq!(session.preview.len(), 2);
        assert_eq!(session.preview[0].role, Role::User);
        assert_eq!(session.preview[1].role, Role::Assistant);
    }

    #[test]
    fn usage_is_accumulated_across_assistant_messages() {
        let content = [
            assistant_line("2026-09-16T12:00:01.000Z", "第一次"),
            assistant_line("2026-09-16T12:00:02.000Z", "第二次"),
        ]
        .join("\n");
        let session = parse_content(info("/a/sess-1.jsonl", 0.0), &content, None).expect("session");
        let usage = session.usage.expect("usage");

        // 每条：input 100 + cache 30 + output 30 = 160；两条共 320。
        assert_eq!(usage.total.as_ref().unwrap().total_tokens, Some(320));
        assert_eq!(usage.total.as_ref().unwrap().input_tokens, Some(200));
        assert_eq!(usage.total.as_ref().unwrap().cached_input_tokens, Some(60));
        assert_eq!(usage.total.as_ref().unwrap().output_tokens, Some(60));
        // last 只反映最后一条。
        assert_eq!(usage.last.as_ref().unwrap().total_tokens, Some(160));
        assert_eq!(usage.source, UsageSource::ClaudeMessageUsage);
    }

    #[test]
    fn cached_input_counts_both_creation_and_read() {
        // 缓存新建与缓存命中都是输入，只是计费不同；漏掉任一项统计都会偏低。
        let line = r#"{"type":"assistant","sessionId":"s","message":{"role":"assistant","content":"x","usage":{"input_tokens":1,"cache_creation_input_tokens":100,"cache_read_input_tokens":200,"output_tokens":3}}}"#;
        let session = parse_content(info("/a/s.jsonl", 0.0), line, None).expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.total.as_ref().unwrap().cached_input_tokens, Some(300));
        assert_eq!(usage.total.as_ref().unwrap().total_tokens, Some(304));
    }

    #[test]
    fn context_percent_uses_the_last_turn_and_rounds_to_an_integer() {
        // 最近一轮输入 = 100 + 30 = 130；窗口 20 万 → 0.065% → 四舍五入为 0。
        let session = parse_content(
            info("/a/sess-1.jsonl", 0.0),
            &assistant_line("2026-09-16T12:00:01.000Z", "x"),
            None,
        )
        .expect("session");
        let usage = session.usage.expect("usage");
        assert_eq!(usage.context_window, Some(DEFAULT_CONTEXT_WINDOW));
        assert_eq!(usage.context_used_tokens, Some(130));
        assert_eq!(usage.context_percent, Some(0.0));
        assert_eq!(usage.context_left_percent, Some(100.0));
    }

    #[test]
    fn last_prompt_provides_a_fallback_title() {
        let content = [
            r#"{"type":"last-prompt","sessionId":"sess-1","lastPrompt":"最后的问题"}"#.to_owned(),
            assistant_line("2026-09-16T12:00:01.000Z", "回答"),
        ]
        .join("\n");
        let session = parse_content(info("/a/sess-1.jsonl", 0.0), &content, None).expect("session");
        // 没有用户消息时用 lastPrompt。
        assert_eq!(session.title, "最后的问题");
    }

    #[test]
    fn a_user_message_outranks_last_prompt() {
        let content = [
            r#"{"type":"last-prompt","sessionId":"sess-1","lastPrompt":"最后的问题"}"#.to_owned(),
            user_line("2026-09-16T12:00:00.000Z", "第一条用户消息"),
            assistant_line("2026-09-16T12:00:01.000Z", "回答"),
        ]
        .join("\n");
        let session = parse_content(info("/a/sess-1.jsonl", 0.0), &content, None).expect("session");
        assert_eq!(session.title, "第一条用户消息");
    }

    #[test]
    fn meta_records_are_skipped() {
        let content = [
            user_line("2026-09-16T12:00:00.000Z", "正常"),
            r#"{"type":"user","sessionId":"sess-1","isMeta":true,"message":{"role":"user","content":"注入的元信息"}}"#.to_owned(),
        ]
        .join("\n");
        let session = parse_content(info("/a/sess-1.jsonl", 0.0), &content, None).expect("session");
        assert_eq!(session.message_count, 1);
        assert_eq!(session.preview[0].text, "正常");
    }

    #[test]
    fn content_can_be_a_string_or_an_array_of_parts() {
        let string_content = r#"{"type":"user","sessionId":"s","message":{"role":"user","content":"  直接字符串  "}}"#;
        let session =
            parse_content(info("/a/s.jsonl", 0.0), string_content, None).expect("session");
        assert_eq!(session.preview[0].text, "直接字符串");

        let array_content = r#"{"type":"user","sessionId":"s","message":{"role":"user","content":[{"type":"text","text":"第一段"},{"content":"第二段"},"第三段"]}}"#;
        let session = parse_content(info("/a/s.jsonl", 0.0), array_content, None).expect("session");
        assert_eq!(session.preview[0].text, "第一段\n第二段\n第三段");
    }

    #[test]
    fn the_title_is_captured_even_when_it_falls_outside_the_preview_window() {
        // 预览只留 1 条，但标题必须来自**第一条**用户消息。
        let content = [
            user_line("2026-09-16T12:00:00.000Z", "第一条"),
            assistant_line("2026-09-16T12:00:01.000Z", "回答"),
            user_line("2026-09-16T12:00:02.000Z", "第三条"),
        ]
        .join("\n");
        let session =
            parse_content(info("/a/sess-1.jsonl", 0.0), &content, Some(1)).expect("session");
        assert_eq!(session.preview.len(), 1);
        assert_eq!(session.preview[0].text, "第一条");
        assert_eq!(session.title, "第一条");
        assert_eq!(session.message_count, 3, "计数必须是全量");
    }

    #[test]
    fn consecutive_duplicates_are_deduplicated() {
        let content = [
            user_line("2026-09-16T12:00:00.000Z", "重复"),
            user_line("2026-09-16T12:00:00.000Z", "重复"),
        ]
        .join("\n");
        let session = parse_content(info("/a/sess-1.jsonl", 0.0), &content, None).expect("session");
        assert_eq!(session.message_count, 1);
    }

    #[test]
    fn updated_at_is_the_later_of_the_record_time_and_the_file_mtime() {
        // 记录时间早于 mtime → 取 mtime。
        let session = parse_content(
            info("/a/sess-1.jsonl", 1_800_000_000_000.0),
            &user_line("2026-09-16T12:00:00.000Z", "x"),
            None,
        )
        .expect("session");
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("2027-01-15T08:00:00.000Z")
        );

        // 记录时间晚于 mtime → 取记录时间。
        let session = parse_content(
            info("/a/sess-1.jsonl", 0.0),
            &user_line("2026-09-16T12:00:00.000Z", "x"),
            None,
        )
        .expect("session");
        assert_eq!(
            session.updated_at.as_ref().map(Timestamp::as_str),
            Some("2026-09-16T12:00:00.000Z")
        );
    }

    #[test]
    fn the_id_falls_back_to_the_file_name() {
        let line = r#"{"type":"user","timestamp":"2026-09-16T12:00:00.000Z","message":{"role":"user","content":"x"}}"#;
        let session =
            parse_content(info("/a/from-file-name.jsonl", 0.0), line, None).expect("session");
        assert_eq!(session.id.as_str(), "from-file-name");
    }

    #[test]
    fn a_session_with_neither_id_nor_messages_is_rejected() {
        assert!(parse_content(info("/a/x.jsonl", 0.0), "", None).is_none());
    }

    #[test]
    fn context_window_inference_follows_the_model_name() {
        assert_eq!(
            infer_context_window("claude-sonnet-4"),
            Some(DEFAULT_CONTEXT_WINDOW)
        );
        assert_eq!(
            infer_context_window("claude-opus-4-8"),
            Some(ONE_MILLION_CONTEXT_WINDOW)
        );
        assert_eq!(
            infer_context_window("claude-sonnet-4[1m]"),
            Some(ONE_MILLION_CONTEXT_WINDOW)
        );
        assert_eq!(infer_context_window("gpt-5"), None);
        assert_eq!(infer_context_window(""), None);
    }

    #[test]
    fn telemetry_betas_can_upgrade_the_context_window() {
        assert_eq!(
            infer_telemetry_context_window("claude-sonnet-4", "context-1m-2025-08-07"),
            Some(ONE_MILLION_CONTEXT_WINDOW)
        );
        assert_eq!(infer_telemetry_context_window("claude-sonnet-4", ""), None);
    }

    #[test]
    fn applying_a_context_window_recomputes_the_percentages() {
        let mut usage = build_usage(
            &UsageAccumulator {
                total: TokenUsage {
                    total_tokens: Some(1000),
                    ..Default::default()
                },
                last: Some(TokenUsage {
                    input_tokens: Some(50_000),
                    cached_input_tokens: Some(0),
                    ..Default::default()
                }),
                updated_at: None,
            },
            None,
        )
        .expect("usage");
        usage = apply_context_window(&usage, 100_000);
        assert_eq!(usage.context_window, Some(100_000));
        assert_eq!(usage.context_used_tokens, Some(50_000));
        assert_eq!(usage.context_percent, Some(50.0));
        assert_eq!(usage.context_left_percent, Some(50.0));
    }

    #[test]
    fn titles_are_compacted_and_truncated() {
        assert_eq!(compact_title("  多个   空白\n字符  "), "多个 空白 字符");
        let long = "字".repeat(300);
        assert_eq!(compact_title(&long).chars().count(), TITLE_MAX_LENGTH);
    }

    #[test]
    fn countable_message_matches_the_parser_rules() {
        let ok: Value = serde_json::from_str(&user_line("2026-09-16T12:00:00.000Z", "x")).unwrap();
        assert!(is_countable_message(&ok));

        let meta: Value = serde_json::from_str(
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"x"}}"#,
        )
        .unwrap();
        assert!(!is_countable_message(&meta), "isMeta 记录不可计数");

        let empty: Value =
            serde_json::from_str(r#"{"type":"user","message":{"role":"user","content":""}}"#)
                .unwrap();
        assert!(!is_countable_message(&empty), "空正文不可计数");

        let other: Value = serde_json::from_str(r#"{"type":"summary"}"#).unwrap();
        assert!(!is_countable_message(&other));
    }

    #[test]
    fn corrupt_lines_are_skipped() {
        let content = [
            "not json".to_owned(),
            user_line("2026-09-16T12:00:00.000Z", "仍然解析"),
        ]
        .join("\n");
        let session = parse_content(info("/a/sess-1.jsonl", 0.0), &content, None).expect("session");
        assert_eq!(session.message_count, 1);
    }
}
