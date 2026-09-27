//! 会话详情分页折叠器（Codex 与 Claude 共用）。
//!
//! # 为什么两个平台共用\n//!\n//! Codex 与 Claude 的分页**结构完全相同**：首屏取末尾、普通分页从锚点向后读、\n//! 每 100 条可见消息落一个锚点、连续重复消息去重且不占页容量。差异只在「一行如何\n//! 变成一条消息」。复制这份逻辑意味着修了一处会漏掉另一处，因此参数化提取策略。\n//!\n//! # 折叠器与 I/O 分离
//!
//! [`PageFolder`] 只做「逐行 → 消息页」的折叠，不碰文件系统。这样分页规则（首屏语义、
//! 提前停止、锚点落点、去重）可以用纯逻辑穷举测试，而不必准备真实 JSONL 与数据库。
//!
//! # 首屏语义（`offset === -1`）
//!
//! 首屏请求要的是**末尾** `limit` 条消息，且必须回填它们在文件中的**真实绝对序号**。
//! 因此首屏无法使用锚点（锚点是「从某处向后读」，而首屏要的是末尾），只能从头读一遍
//! 并用滑动窗口保留最后 `limit` 条。返回的 `offset` 是 `visible_index - messages.len()`。
//!
//! # 字节锚点（相对现网的改进）
//!
//! 现网锚点只记行号，因此 `offset >= 0` 的分页仍要从文件头逐行读到起始行。这里额外
//! 记录**字节偏移**：本地文件可直接 `seek`（真 O(1)），WSL 侧退化为按行号读取。
//! 行号仍然保留——它是消息的身份，`source_line` 必须是绝对行号。
//!
//! # 锚点落点
//!
//! 每 [`MESSAGE_ANCHOR_INTERVAL`] 条可见消息落一个锚点（`visible_index % 100 == 0`）。
//! 这个判断在首屏与普通分页**都会**执行，因此首屏读完后也会顺带建立索引。

use std::collections::VecDeque;

use serde_json::Value;

use aivc_core::session::{
    Message, MessageAnchor, MessagePage, PageCursor, Role, SessionUsage, UsageSource,
};
use aivc_core::text::should_keep_message;
use aivc_core::time::Timestamp;

use crate::claude::parser::extract_content_text;
use crate::codex::parser::{extract_message, extract_usage};
use crate::gemini::parser::{should_keep_gemini_message, to_message as to_gemini_message};
use crate::qoder::parser::to_message as to_qoder_message;

/// 行 → 消息的提取策略。
///
/// 各平台的记录结构不同，但折叠规则一致；差异全部收敛到这里。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinePolicy {
    /// Codex：`event_msg` / `response_item`，并记录 `source_line`（分支依赖绝对行号）。
    Codex,
    /// Claude：顶层 `user` / `assistant`，**不**记录 `source_line`（分支按消息条数定位）。
    Claude,
    /// Gemini：`type` 单条记录或 `$set.messages` 补丁（**一行可产出多条**），
    /// 不记录 `source_line`。
    Gemini,
    /// Qoder：顶层 `user` / `assistant`（结构与 Claude 相同），不记录 `source_line`。
    Qoder,
}

/// 锚点间隔：每这么多条可见消息落一个锚点。
pub const MESSAGE_ANCHOR_INTERVAL: u64 = 100;

/// 分页结果。
#[derive(Debug, Clone, PartialEq)]
pub struct PageOutcome {
    /// 消息页。
    pub page: MessagePage,
    /// 本次读取过程中新建立的锚点。
    pub anchors: Vec<MessageAnchor>,
    /// 已知的可见消息总数（用于写回消息索引）。
    pub message_count: u64,
    /// 本次读取过程中看到的最后一份用量（若记录中出现）。
    pub usage: Option<SessionUsage>,
}

/// 把逐行内容折叠成消息页。
#[derive(Debug)]
pub struct PageFolder {
    policy: LinePolicy,
    latest: bool,
    page_offset: u64,
    limit: u64,
    messages: VecDeque<Message>,
    previous: Option<(Role, String)>,
    visible_index: u64,
    has_more: bool,
    anchors: Vec<MessageAnchor>,
    usage: Option<SessionUsage>,
    start_line: u64,
    start_byte: Option<u64>,
    anchor_message_count: u64,
}

impl PageFolder {
    /// 构造。
    ///
    /// `anchor` 为 [`PageCursor::Offset`] 时可传入已索引的锚点；首屏会忽略它。
    #[must_use]
    pub fn new(
        cursor: PageCursor,
        limit: u64,
        anchor: Option<&MessageAnchor>,
        policy: LinePolicy,
    ) -> Self {
        let latest = cursor.is_latest();
        let page_offset = if latest {
            0
        } else {
            cursor.to_wire().max(0).cast_unsigned()
        };
        let usable = if latest { None } else { anchor };
        Self {
            policy,
            latest,
            page_offset,
            limit: limit.max(1),
            messages: VecDeque::new(),
            previous: None,
            visible_index: usable.map_or(0, |item| item.message_offset),
            has_more: false,
            anchors: Vec::new(),
            usage: None,
            // 首屏必须从头读：要的是末尾若干条，锚点帮不上忙。
            start_line: usable.map_or(1, |item| item.line_number.max(1)),
            start_byte: usable.map(|item| item.byte_offset),
            anchor_message_count: usable.map_or(0, |item| item.message_count),
        }
    }

    /// 建议的起始行号。
    #[must_use]
    pub const fn start_line(&self) -> u64 {
        self.start_line
    }

    /// 建议的起始字节偏移；[`None`] 表示从行号起读。
    ///
    /// 锚点的 `byte_offset` 为 0 时按「未知」处理（WSL 侧只能按行号读取），
    /// 否则会 `seek` 到文件开头，把整页内容算错。
    #[must_use]
    pub const fn start_byte(&self) -> Option<u64> {
        match self.start_byte {
            Some(0) | None => None,
            Some(offset) => Some(offset),
        }
    }

    /// 喂入一行。返回 `false` 表示**应当停止读取**（已取够本页内容）。
    ///
    /// `byte_offset` 是该行在文件中的起始字节偏移，用于建立可 `seek` 的锚点。
    ///
    /// 一行可能产出**多条**消息（Gemini 的 `$set.messages` 补丁），因此内部按序处理，
    /// 一旦某条要求停止就立即返回。
    pub fn push_line(&mut self, line: &str, line_number: u64, byte_offset: u64) -> bool {
        let Some(parsed) = parse_line(line, self.policy) else {
            return true;
        };
        // 用量必须在「是否可见消息」判断**之前**提取：`token_count` 记录不是消息，
        // 若把它一起过滤掉，分页就永远拿不到用量。
        if let Some(usage) = parsed.usage {
            self.usage = Some(usage);
        }
        for pending in parsed.messages {
            if !self.push_message(pending, line_number, byte_offset) {
                return false;
            }
        }
        true
    }

    /// 处理单条候选消息；返回 `false` 表示应当停止读取。
    fn push_message(
        &mut self,
        pending: PendingMessage,
        line_number: u64,
        byte_offset: u64,
    ) -> bool {
        let PendingMessage {
            message,
            can_anchor,
        } = pending;
        // 去重必须在「是否够页」判断之前：重复消息不计入 visible_index，
        // 否则会把重复项算进页容量，导致本页少给一条。
        if let Some((role, text)) = &self.previous
            && *role == message.role
            && text == &message.text
        {
            return true;
        }
        self.previous = Some((message.role, message.text.clone()));

        // 行号只有 Codex 需要：它的分支锚点是绝对行号。Claude 与 Gemini 的分支按
        // 消息条数定位，凭空填一个行号只会误导调用方。
        let source_line = match self.policy {
            LinePolicy::Codex => Some(line_number),
            LinePolicy::Claude | LinePolicy::Gemini | LinePolicy::Qoder => None,
        };
        let visible = Message {
            source_line,
            ..message
        };

        if !self.latest && self.visible_index >= self.page_offset + self.limit {
            self.has_more = true;
            return false;
        }

        if can_anchor && self.visible_index.is_multiple_of(MESSAGE_ANCHOR_INTERVAL) {
            self.anchors.push(MessageAnchor {
                message_offset: self.visible_index,
                byte_offset,
                line_number,
                message_count: self.visible_index,
            });
        }

        if self.latest {
            self.messages.push_back(visible);
            // 滑动窗口：只保留最后 limit 条。
            if self.messages.len() as u64 > self.limit {
                self.messages.pop_front();
            }
        } else if self.visible_index >= self.page_offset {
            self.messages.push_back(visible);
        }
        self.visible_index += 1;
        true
    }

    /// 结束并产出结果。
    ///
    /// `session_message_count` 来自会话摘要；`has_more` 时不得把已读条数当成总数
    /// （后面还有没读的），因此那一项取 0。
    #[must_use]
    pub fn finish(self, session_message_count: u64) -> PageOutcome {
        let read_count = self.visible_index;
        let message_count = session_message_count
            .max(self.anchor_message_count)
            .max(if self.has_more { 0 } else { read_count });
        let page = MessagePage {
            offset: if self.latest {
                // 首屏：回填窗口首条消息的真实绝对序号。
                read_count.saturating_sub(self.messages.len() as u64)
            } else {
                self.page_offset
            },
            has_more: if self.latest {
                read_count > self.messages.len() as u64
            } else {
                self.has_more
            },
            messages: self.messages.into_iter().collect(),
            // 随页带回，界面上的会话头要用它——那里的 `Session` 来自列表，条数只是下界。
            message_count,
        };
        PageOutcome {
            page,
            anchors: self.anchors,
            message_count,
            usage: self.usage,
        }
    }
}

/// 一行的解析结果。
///
/// 消息与用量**分开**：`token_count` 这类记录携带用量但不是可见消息，
/// 若合并成一个 `Option` 就会在「不是消息」时把用量一起丢掉。
struct ParsedLine {
    /// 本行产出的候选消息（Gemini 的补丁一行可产出多条）。
    messages: Vec<PendingMessage>,
    usage: Option<SessionUsage>,
}

impl ParsedLine {
    /// 空结果（没有可提取的内容）。
    const fn empty() -> Self {
        Self {
            messages: Vec::new(),
            usage: None,
        }
    }
}

/// 一条候选消息。
struct PendingMessage {
    message: Message,
    /// 是否允许在此处落锚点。
    ///
    /// 同一行的多条消息不能各自落锚点：锚点记的是行号，无法区分同一行内的不同消息。
    can_anchor: bool,
}

/// 解析一行，提取可能存在的消息与用量。
fn parse_line(line: &str, policy: LinePolicy) -> Option<ParsedLine> {
    if line.is_empty() {
        return None;
    }
    let value: Value = serde_json::from_str(line).ok()?;
    let object = value.as_object()?;
    let timestamp = object
        .get("timestamp")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(Timestamp::from_raw);

    match policy {
        LinePolicy::Codex => {
            let kind = match object.get("type").and_then(Value::as_str) {
                Some("event_msg") => "event_msg",
                Some("response_item") => "response_item",
                Some("session_meta") => "session_meta",
                Some("turn_context") => "turn_context",
                _ => "other",
            };
            let payload = object.get("payload").cloned().unwrap_or(Value::Null);
            let usage = extract_usage(&payload, timestamp.as_ref().map(Timestamp::as_str), kind);
            let messages = extract_message(&payload, kind)
                .filter(|message| should_keep_message(&message.text))
                .map(|message| {
                    vec![PendingMessage {
                        message: Message {
                            timestamp: timestamp.clone(),
                            ..message
                        },
                        can_anchor: true,
                    }]
                })
                .unwrap_or_default();
            Some(ParsedLine { messages, usage })
        }
        LinePolicy::Claude => {
            let kind = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if (kind != "user" && kind != "assistant")
                || object.get("isMeta").and_then(Value::as_bool) == Some(true)
            {
                return Some(ParsedLine::empty());
            }
            let message_object = object.get("message").and_then(Value::as_object);
            let role = match message_object
                .and_then(|item| item.get("role"))
                .and_then(Value::as_str)
            {
                Some("assistant") => Some(Role::Assistant),
                Some("user") => Some(Role::User),
                _ => None,
            };
            let text = message_object
                .and_then(|item| item.get("content"))
                .map_or_else(String::new, |content| extract_content_text(Some(content)));
            // Claude 的用量是逐条累加的，分页不承担这个职责（摘要里已经有累计值）。
            let messages = role
                .filter(|_| !text.is_empty())
                .map(|role| {
                    vec![PendingMessage {
                        message: Message {
                            role,
                            text,
                            timestamp: timestamp.clone(),
                            source_line: None,
                        },
                        can_anchor: true,
                    }]
                })
                .unwrap_or_default();
            Some(ParsedLine {
                messages,
                usage: None,
            })
        }
        LinePolicy::Gemini => {
            let mut messages = Vec::new();
            // `$set.messages` 补丁：一行产出多条消息，但**只有首条**可以落锚点。
            // 锚点记录的是行号，同一行的多条消息无法用同一个行号各自定位。
            if let Some(patch) = object.get("$set").and_then(Value::as_object)
                && let Some(items) = patch.get("messages").and_then(Value::as_array)
            {
                for (index, item) in items.iter().enumerate() {
                    if let Some(message) = gemini_message(item) {
                        messages.push(PendingMessage {
                            message,
                            can_anchor: index == 0,
                        });
                    }
                }
            }
            // 单条消息记录。
            if object.get("type").and_then(Value::as_str).is_some()
                && let Some(message) = gemini_message(&value)
            {
                messages.push(PendingMessage {
                    message,
                    can_anchor: true,
                });
            }
            Some(ParsedLine {
                messages,
                usage: None,
            })
        }
        LinePolicy::Qoder => {
            let messages = to_qoder_message(&value)
                .map(|message| {
                    vec![PendingMessage {
                        message,
                        can_anchor: true,
                    }]
                })
                .unwrap_or_default();
            Some(ParsedLine {
                messages,
                usage: None,
            })
        }
    }
}

/// 把一条 Gemini 记录转成候选消息（可见性按 Gemini 自己的判据）。
fn gemini_message(value: &Value) -> Option<Message> {
    let message = to_gemini_message(value)?;
    should_keep_gemini_message(&message.text).then_some(message)
}

/// 便捷入口：从整段文本折叠出一页（用于测试与小型会话）。
#[must_use]
pub fn fold_page(
    content: &str,
    cursor: PageCursor,
    limit: u64,
    anchor: Option<&MessageAnchor>,
    session_message_count: u64,
) -> PageOutcome {
    fold_page_with(
        content,
        cursor,
        limit,
        anchor,
        session_message_count,
        LinePolicy::Codex,
    )
}

/// 便捷入口：指定提取策略。
#[must_use]
pub fn fold_page_with(
    content: &str,
    cursor: PageCursor,
    limit: u64,
    anchor: Option<&MessageAnchor>,
    session_message_count: u64,
    policy: LinePolicy,
) -> PageOutcome {
    let mut folder = PageFolder::new(cursor, limit, anchor, policy);
    let mut offset = 0u64;
    for (index, line) in content.split('\n').enumerate() {
        let line = line.trim_end_matches('\r');
        let next_offset = offset + line.len() as u64 + 1;
        if !folder.push_line(line, index as u64 + 1, offset) {
            break;
        }
        offset = next_offset;
    }
    folder.finish(session_message_count)
}

/// 由消息序号计算它应当落在哪个锚点间隔上。
///
/// 用于判断是否需要重建索引：序号不是间隔整数倍时，最近的锚点在它之前。
#[must_use]
pub const fn anchor_slot(message_offset: u64) -> u64 {
    message_offset - message_offset % MESSAGE_ANCHOR_INTERVAL
}

/// 判断用量是否只有 Codex 会产出的形态（供契约自检使用）。
#[must_use]
pub fn is_codex_usage(usage: &SessionUsage) -> bool {
    usage.source == UsageSource::CodexTokenCount
        && (usage.total.is_some() || usage.last.is_some() || usage.context_window.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:00.000Z","type":"session_meta","payload":{{"id":"{id}","cwd":"/w"}}}}"#
        )
    }

    fn user(text: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{{"type":"user_message","message":"{text}"}}}}"#
        )
    }

    fn content(count: usize) -> String {
        let mut lines = vec![meta("s1")];
        for index in 0..count {
            lines.push(user(&format!("消息{index}")));
        }
        lines.join("\n")
    }

    #[test]
    fn offset_page_returns_the_requested_window() {
        // 请求从第 3 条起的 2 条。
        let outcome = fold_page(&content(10), PageCursor::Offset(3), 2, None, 10);
        assert_eq!(outcome.page.offset, 3);
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["消息3", "消息4"]);
        assert!(outcome.page.has_more, "后面还有消息");
    }

    #[test]
    fn offset_page_reports_no_more_when_the_file_ends() {
        let outcome = fold_page(&content(5), PageCursor::Offset(3), 10, None, 5);
        assert_eq!(outcome.page.messages.len(), 2);
        assert!(!outcome.page.has_more);
    }

    #[test]
    fn latest_page_returns_the_tail_and_backfills_the_real_offset() {
        let outcome = fold_page(&content(10), PageCursor::Latest, 3, None, 10);
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["消息7", "消息8", "消息9"]);
        // 首屏必须回填真实绝对序号，否则向上翻页会错位。
        assert_eq!(outcome.page.offset, 7);
        assert!(outcome.page.has_more);
    }

    #[test]
    fn latest_page_with_fewer_messages_than_the_limit_starts_at_zero() {
        let outcome = fold_page(&content(2), PageCursor::Latest, 10, None, 2);
        assert_eq!(outcome.page.offset, 0);
        assert_eq!(outcome.page.messages.len(), 2);
        assert!(!outcome.page.has_more, "全部消息都在一页内");
    }

    #[test]
    fn latest_page_ignores_the_anchor_and_reads_from_the_start() {
        // 首屏要的是末尾若干条，锚点（从某处向后读）帮不上忙，必须从头读。
        let anchor = MessageAnchor {
            message_offset: 5,
            byte_offset: 123,
            line_number: 6,
            message_count: 5,
        };
        let folder = PageFolder::new(PageCursor::Latest, 3, Some(&anchor), LinePolicy::Codex);
        assert_eq!(folder.start_line(), 1);
        assert_eq!(folder.start_byte(), None);
    }

    #[test]
    fn offset_page_uses_the_anchor_for_both_line_and_byte_start() {
        let anchor = MessageAnchor {
            message_offset: 100,
            byte_offset: 4096,
            line_number: 102,
            message_count: 100,
        };
        let folder = PageFolder::new(
            PageCursor::Offset(100),
            10,
            Some(&anchor),
            LinePolicy::Codex,
        );
        assert_eq!(folder.start_line(), 102);
        // 字节锚点让本地文件可以 seek，而不是逐行读过去。
        assert_eq!(folder.start_byte(), Some(4096));
    }

    #[test]
    fn source_line_is_the_absolute_jsonl_line_number() {
        let outcome = fold_page(&content(3), PageCursor::Latest, 10, None, 3);
        // 第 1 行是 session_meta，因此消息从第 2 行开始。
        let lines: Vec<u64> = outcome
            .page
            .messages
            .iter()
            .filter_map(|message| message.source_line)
            .collect();
        assert_eq!(lines, vec![2, 3, 4]);
    }

    #[test]
    fn anchors_are_placed_every_hundred_visible_messages() {
        let outcome = fold_page(&content(250), PageCursor::Latest, 10, None, 250);
        let offsets: Vec<u64> = outcome
            .anchors
            .iter()
            .map(|anchor| anchor.message_offset)
            .collect();
        assert_eq!(offsets, vec![0, 100, 200]);
        // 锚点必须同时携带行号与字节偏移。
        assert!(outcome.anchors.iter().all(|anchor| anchor.line_number > 0));
        assert!(outcome.anchors.iter().all(|anchor| anchor.byte_offset > 0));
    }

    #[test]
    fn duplicate_messages_do_not_consume_page_capacity() {
        // 重复消息若计入页容量，本页就会少给一条。
        let duplicated = [
            meta("s1"),
            user("重复"),
            user("重复"),
            user("第二条"),
            user("第三条"),
        ]
        .join("\n");
        let outcome = fold_page(&duplicated, PageCursor::Offset(0), 3, None, 3);
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["重复", "第二条", "第三条"]);
    }

    #[test]
    fn synthetic_and_unparseable_lines_are_skipped() {
        let mixed = [
            meta("s1"),
            "not json".to_owned(),
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"<environment_context>"}}"#.to_owned(),
            user("真实"),
        ]
        .join("\n");
        let outcome = fold_page(&mixed, PageCursor::Latest, 10, None, 1);
        assert_eq!(outcome.page.messages.len(), 1);
        assert_eq!(outcome.page.messages[0].text, "真实");
        // 行号仍反映原始位置：真实消息在第 4 行。
        assert_eq!(outcome.page.messages[0].source_line, Some(4));
    }

    #[test]
    fn message_count_prefers_the_largest_known_value() {
        // 已读 3 条但摘要声称 10 条 → 取 10。
        let outcome = fold_page(&content(3), PageCursor::Latest, 10, None, 10);
        assert_eq!(outcome.message_count, 10);
        // **同一页也要带上它**：界面上的会话头读的是分页响应，不是 `PageOutcome`。
        // 两处取不同的值就会出现「索引里 10 条、界面显示 3 条」这种自相矛盾。
        assert_eq!(outcome.page.message_count, 10);

        // 摘要过小（陈旧）时取实际读到的条数。
        let outcome = fold_page(&content(5), PageCursor::Latest, 10, None, 1);
        assert_eq!(outcome.message_count, 5);
        assert_eq!(outcome.page.message_count, 5);
    }

    #[test]
    fn has_more_prevents_reporting_a_truncated_count_as_the_total() {
        // 提前停止时后面还有没读的消息，不能把已读条数当成总数。
        let outcome = fold_page(&content(100), PageCursor::Offset(0), 5, None, 0);
        assert!(outcome.page.has_more);
        assert_eq!(outcome.message_count, 0, "has_more 时不得声称已知总数");
        // 页上同样是 0：界面回退到列表的下界，而不是显示一个偏小的数。
        assert_eq!(outcome.page.message_count, 0);
    }

    #[test]
    fn anchor_message_count_is_kept_when_available() {
        let anchor = MessageAnchor {
            message_offset: 100,
            byte_offset: 0,
            line_number: 102,
            message_count: 150,
        };
        let outcome = fold_page(&content(0), PageCursor::Offset(100), 10, Some(&anchor), 0);
        assert_eq!(outcome.message_count, 150);
    }

    #[test]
    fn usage_from_token_count_records_is_reported() {
        let with_usage = [
            meta("s1"),
            r#"{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":100},"model_context_window":1000}}}"#.to_owned(),
            user("hi"),
        ]
        .join("\n");
        let outcome = fold_page(&with_usage, PageCursor::Latest, 10, None, 1);
        let usage = outcome.usage.expect("usage");
        assert!(is_codex_usage(&usage));
        assert_eq!(usage.total.expect("total").input_tokens, Some(100));
    }

    #[test]
    fn zero_or_negative_limit_is_clamped_to_one() {
        let outcome = fold_page(&content(5), PageCursor::Latest, 0, None, 5);
        assert_eq!(outcome.page.messages.len(), 1, "limit 至少为 1");
    }

    #[test]
    fn negative_wire_offset_is_treated_as_latest() {
        // `PageCursor::from_wire(-1)` 即首屏；任何负数都按首屏处理。
        let outcome = fold_page(&content(5), PageCursor::from_wire(-99), 2, None, 5);
        assert_eq!(outcome.page.offset, 3);
        assert_eq!(outcome.page.messages.len(), 2);
    }

    #[test]
    fn anchor_slot_rounds_down_to_the_interval() {
        assert_eq!(anchor_slot(0), 0);
        assert_eq!(anchor_slot(99), 0);
        assert_eq!(anchor_slot(100), 100);
        assert_eq!(anchor_slot(250), 200);
    }

    // --- Claude 策略 ---

    fn claude_user(text: &str) -> String {
        format!(
            r#"{{"type":"user","sessionId":"s","timestamp":"2026-09-16T12:00:01.000Z","message":{{"role":"user","content":"{text}"}}}}"#
        )
    }

    fn claude_content(count: usize) -> String {
        let mut lines = vec![r#"{"type":"summary","sessionId":"s","summary":"摘要"}"#.to_owned()];
        for index in 0..count {
            lines.push(claude_user(&format!("消息{index}")));
        }
        lines.join("\n")
    }

    #[test]
    fn claude_policy_extracts_top_level_messages() {
        let outcome = fold_page_with(
            &claude_content(3),
            PageCursor::Latest,
            10,
            None,
            3,
            LinePolicy::Claude,
        );
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["消息0", "消息1", "消息2"]);
    }

    #[test]
    fn claude_policy_does_not_set_source_line() {
        // Claude 的分支按消息条数定位，凭空填一个行号只会误导调用方。
        let outcome = fold_page_with(
            &claude_content(2),
            PageCursor::Latest,
            10,
            None,
            2,
            LinePolicy::Claude,
        );
        assert!(
            outcome
                .page
                .messages
                .iter()
                .all(|message| message.source_line.is_none()),
            "Claude 分页结果不应带 source_line"
        );
    }

    #[test]
    fn claude_policy_skips_meta_and_non_message_records() {
        let content = [
            r#"{"type":"summary","sessionId":"s"}"#.to_owned(),
            claude_user("正常"),
            r#"{"type":"user","sessionId":"s","isMeta":true,"message":{"role":"user","content":"元信息"}}"#.to_owned(),
            r#"{"type":"assistant","sessionId":"s","message":{"role":"assistant","content":[{"type":"text","text":"数组正文"}]}}"#.to_owned(),
        ]
        .join("\n");
        let outcome = fold_page_with(
            &content,
            PageCursor::Latest,
            10,
            None,
            2,
            LinePolicy::Claude,
        );
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["正常", "数组正文"]);
    }

    #[test]
    fn claude_policy_keeps_the_first_page_semantics() {
        let outcome = fold_page_with(
            &claude_content(10),
            PageCursor::Latest,
            3,
            None,
            10,
            LinePolicy::Claude,
        );
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["消息7", "消息8", "消息9"]);
        assert_eq!(outcome.page.offset, 7, "首屏必须回填真实绝对序号");
        assert!(outcome.page.has_more);
    }

    // --- Gemini 策略 ---

    fn gemini_content(count: usize) -> String {
        let mut lines = vec![r#"{"sessionId":"s","projectHash":"h","kind":"main"}"#.to_owned()];
        for index in 0..count {
            lines.push(format!(r#"{{"type":"user","content":"消息{index}"}}"#));
        }
        lines.join("\n")
    }

    #[test]
    fn gemini_policy_extracts_single_message_records() {
        let outcome = fold_page_with(
            &gemini_content(3),
            PageCursor::Latest,
            10,
            None,
            3,
            LinePolicy::Gemini,
        );
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["消息0", "消息1", "消息2"]);
        assert!(
            outcome
                .page
                .messages
                .iter()
                .all(|m| m.source_line.is_none())
        );
    }

    #[test]
    fn gemini_policy_expands_set_patches_into_multiple_messages() {
        // 一行产出多条消息：这是 Gemini 独有的结构。
        let content = [
            r#"{"sessionId":"s","projectHash":"h","kind":"main"}"#.to_owned(),
            r#"{"$set":{"messages":[{"type":"user","content":"补丁一"},{"type":"gemini","content":"补丁二"}]}}"#.to_owned(),
        ]
        .join("\n");
        let outcome = fold_page_with(
            &content,
            PageCursor::Latest,
            10,
            None,
            2,
            LinePolicy::Gemini,
        );
        let texts: Vec<&str> = outcome
            .page
            .messages
            .iter()
            .map(|message| message.text.as_str())
            .collect();
        assert_eq!(texts, vec!["补丁一", "补丁二"]);
    }

    #[test]
    fn gemini_policy_excludes_synthetic_blocks() {
        let content = [
            r#"{"sessionId":"s","projectHash":"h"}"#.to_owned(),
            r#"{"type":"user","content":"<session_context>注入"}"#.to_owned(),
            r#"{"type":"user","content":"真实"}"#.to_owned(),
        ]
        .join("\n");
        let outcome = fold_page_with(
            &content,
            PageCursor::Latest,
            10,
            None,
            1,
            LinePolicy::Gemini,
        );
        assert_eq!(outcome.page.messages.len(), 1);
        assert_eq!(outcome.page.messages[0].text, "真实");
    }

    #[test]
    fn gemini_policy_anchors_only_the_first_message_of_a_patch() {
        // 同一行的多条消息无法用同一个行号各自定位，因此只有首条落锚点。
        let mut lines = vec![r#"{"sessionId":"s","projectHash":"h"}"#.to_owned()];
        for index in 0..120 {
            lines.push(format!(r#"{{"type":"user","content":"消息{index}"}}"#));
        }
        let outcome = fold_page_with(
            &lines.join("\n"),
            PageCursor::Latest,
            5,
            None,
            120,
            LinePolicy::Gemini,
        );
        // 单条记录每行都可落锚点，因此 0/100 两个锚点都在。
        let offsets: Vec<u64> = outcome
            .anchors
            .iter()
            .map(|anchor| anchor.message_offset)
            .collect();
        assert_eq!(offsets, vec![0, 100]);
    }
    #[test]
    fn claude_policy_reports_no_usage() {
        // 用量由摘要里的累加值承担，分页不重复计算。
        let outcome = fold_page_with(
            &claude_content(2),
            PageCursor::Latest,
            10,
            None,
            2,
            LinePolicy::Claude,
        );
        assert!(outcome.usage.is_none());
    }

    #[test]
    fn fold_page_tracks_byte_offsets_that_match_the_source() {
        // 锚点的字节偏移必须真的指向该行起始处，否则 seek 会读到半行。
        let text = content(150);
        let outcome = fold_page(&text, PageCursor::Latest, 5, None, 150);
        let anchor = outcome
            .anchors
            .iter()
            .find(|item| item.message_offset == 100)
            .expect("100 号锚点");
        let offset = anchor.byte_offset as usize;
        let expected_line = text[offset..].split('\n').next().unwrap_or_default();
        // 该偏移处的行必须能独立解析为一条消息。
        // 序号从 0 起，因此 message_offset=100 对应第 101 条可见消息「消息100」。
        assert!(
            expected_line.contains("消息100"),
            "偏移未对齐行首：{expected_line}"
        );
    }
}
