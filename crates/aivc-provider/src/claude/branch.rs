//! Claude 分支与复制的文本构造。
//!
//! # 与 Codex 的关键差异
//!
//! | 维度 | Codex | Claude |
//! | --- | --- | --- |
//! | 分支锚点 | **原始 JSONL 行号** → 先映射成轮次编号 | **可见消息条数**，直接截到第 N 条 |
//! | 编号字段 | `payload.id` + `payload.session_id` | 顶层 `sessionId` |
//! | 分支失败条件 | 行号处不是可见消息 | 文件里可保留的消息**少于**请求条数 |
//!
//! Claude 不需要轮次映射：它的会话文件没有「轮次」概念，按可见消息计数截断即可。
//!
//! # 只改写存在的字段
//!
//! [`rewrite_session_id`] 仅在 `sessionId` 已是字符串时才改写。若强行补上这个字段，
//! 会让本来没有编号的记录凭空多出一个字段——那是伪造数据，而不是复制。
//! 因此「一条都没改写」被当作失败（[`SessionError::DuplicateMissingSessionId`]），
//! 而不是静默产出一个仍指向原会话的文件。

use serde_json::{Value, json};

use aivc_core::error::{Result, SessionError};

use super::parser::is_countable_message;

/// 改写一条记录的 `sessionId`。
///
/// 字段不存在时原样返回——见模块文档。
#[must_use]
pub fn rewrite_session_id(record: Value, new_id: &str) -> Value {
    let mut object = record;
    if let Some(map) = object.as_object_mut()
        && map.get("sessionId").is_some_and(Value::is_string)
    {
        map.insert("sessionId".to_owned(), json!(new_id));
    }
    object
}

/// 构造分支文本：保留到第 `keep_message_count` 条可见消息为止。
///
/// 截断点是**消息计数**而不是行号：最后一条保留消息之后的所有记录都被丢弃
/// （包括同一条消息的附属记录），这正是「从这条消息之后重新开始」的语义。
///
/// 可保留消息不足时返回 [`SessionError::BranchInsufficientMessages`]——静默产出一个
/// 不完整的分支比明确失败更糟：用户会以为分支成功，却发现上下文缺了一截。
pub fn build_branch_text(
    source_text: &str,
    branch_id: &str,
    keep_message_count: u64,
) -> Result<String> {
    if keep_message_count == 0 {
        return Err(SessionError::BranchNeedsContext.into());
    }
    let mut output: Vec<String> = Vec::new();
    let mut kept: u64 = 0;

    for raw_line in source_text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            // 解析不了的行直接跳过：分支只需要可理解的记录。
            continue;
        };
        if !record.is_object() {
            continue;
        }
        let countable = is_countable_message(&record);
        let rewritten = rewrite_session_id(record, branch_id);
        output.push(serde_json::to_string(&rewritten).unwrap_or_else(|_| trimmed.to_owned()));
        if countable {
            kept += 1;
            if kept >= keep_message_count {
                break;
            }
        }
    }

    if kept < keep_message_count {
        return Err(SessionError::BranchInsufficientMessages.into());
    }
    Ok(format!("{}\n", output.join("\n")))
}

/// 构造复制文本：整份复制，只把 `sessionId` 换成新编号。
pub fn build_duplicate_text(source_text: &str, duplicate_id: &str) -> Result<String> {
    let mut output: Vec<String> = Vec::new();
    let mut rewritten = false;

    for raw_line in source_text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // 空行原样保留：复制应当尽可能忠实。
            output.push(line.to_owned());
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            output.push(line.to_owned());
            continue;
        };
        if !record.is_object() {
            output.push(line.to_owned());
            continue;
        }
        let next = rewrite_session_id(record, duplicate_id);
        if next.get("sessionId").and_then(Value::as_str) == Some(duplicate_id) {
            rewritten = true;
        }
        output.push(serde_json::to_string(&next).unwrap_or_else(|_| trimmed.to_owned()));
    }

    if !rewritten {
        return Err(SessionError::DuplicateMissingSessionId.into());
    }
    Ok(format!("{}\n", output.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(timestamp: &str, text: &str) -> String {
        format!(
            r#"{{"type":"user","sessionId":"old-id","timestamp":"{timestamp}","message":{{"role":"user","content":"{text}"}}}}"#
        )
    }

    fn assistant(timestamp: &str, text: &str) -> String {
        format!(
            r#"{{"type":"assistant","sessionId":"old-id","timestamp":"{timestamp}","message":{{"role":"assistant","content":"{text}"}}}}"#
        )
    }

    #[test]
    fn rewriting_only_touches_an_existing_string_field() {
        let record: Value = serde_json::from_str(r#"{"type":"user","sessionId":"old"}"#).unwrap();
        assert_eq!(rewrite_session_id(record, "new")["sessionId"], "new");

        // 字段不存在时不得凭空补上。
        let record: Value = serde_json::from_str(r#"{"type":"user"}"#).unwrap();
        assert!(rewrite_session_id(record, "new").get("sessionId").is_none());

        // 字段类型不对时也不得改写。
        let record: Value = serde_json::from_str(r#"{"type":"user","sessionId":42}"#).unwrap();
        assert_eq!(rewrite_session_id(record, "new")["sessionId"], 42);
    }

    #[test]
    fn branching_keeps_up_to_the_requested_message_count() {
        let source = [
            user("2026-09-16T12:00:00.000Z", "第一条"),
            assistant("2026-09-16T12:00:01.000Z", "回答一"),
            user("2026-09-16T12:00:02.000Z", "第二条"),
            assistant("2026-09-16T12:00:03.000Z", "回答二"),
        ]
        .join("\n");

        let text = build_branch_text(&source, "branch-id", 2).expect("branch");
        let lines: Vec<&str> = text.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 2, "第 2 条消息之后的内容必须被截掉");
        assert!(lines[0].contains("第一条"));
        assert!(lines[1].contains("回答一"));
        assert!(!text.contains("第二条"));
        // 编号必须全部改写。
        for line in &lines {
            let record: Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["sessionId"], "branch-id");
        }
    }

    #[test]
    fn branching_fails_when_there_are_fewer_messages_than_requested() {
        let source = user("2026-09-16T12:00:00.000Z", "唯一一条");
        let error = build_branch_text(&source, "b", 5).expect_err("必须失败");
        assert_eq!(
            error.to_string(),
            "创建 Claude 分支失败：原始 jsonl 中可保留消息不足。"
        );
    }

    #[test]
    fn branching_zero_messages_is_rejected() {
        let source = user("2026-09-16T12:00:00.000Z", "x");
        let error = build_branch_text(&source, "b", 0).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "请选择至少一条前置上下文后再创建分支。");
    }

    #[test]
    fn branching_ignores_non_countable_records_when_counting() {
        // 元信息与空正文记录不算消息，因此不消耗保留额度。
        let source = [
            user("2026-09-16T12:00:00.000Z", "第一条"),
            r#"{"type":"user","sessionId":"old-id","isMeta":true,"message":{"role":"user","content":"元信息"}}"#.to_owned(),
            r#"{"type":"summary","sessionId":"old-id","summary":"摘要"}"#.to_owned(),
            assistant("2026-09-16T12:00:01.000Z", "回答一"),
        ]
        .join("\n");

        let text = build_branch_text(&source, "b", 2).expect("branch");
        assert!(text.contains("元信息"), "不可计数的记录应当保留");
        assert!(text.contains("回答一"));
    }

    #[test]
    fn branching_skips_unparseable_lines() {
        let source = [
            "not json".to_owned(),
            user("2026-09-16T12:00:00.000Z", "第一条"),
        ]
        .join("\n");
        let text = build_branch_text(&source, "b", 1).expect("branch");
        assert!(!text.contains("not json"));
        assert!(text.contains("第一条"));
    }

    #[test]
    fn branching_ends_with_a_newline() {
        let source = user("2026-09-16T12:00:00.000Z", "x");
        assert!(
            build_branch_text(&source, "b", 1)
                .expect("branch")
                .ends_with('\n')
        );
    }

    #[test]
    fn duplicating_rewrites_every_session_id() {
        let source = [
            user("2026-09-16T12:00:00.000Z", "第一条"),
            assistant("2026-09-16T12:00:01.000Z", "回答"),
        ]
        .join("\n");
        let text = build_duplicate_text(&source, "dup-id").expect("duplicate");
        for line in text.trim_end().split('\n') {
            let record: Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["sessionId"], "dup-id");
        }
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn duplicating_preserves_content_and_unparseable_lines() {
        let source = [
            user("2026-09-16T12:00:00.000Z", "内容"),
            "not json".to_owned(),
        ]
        .join("\n");
        let text = build_duplicate_text(&source, "dup-id").expect("duplicate");
        assert!(text.contains("内容"));
        assert!(text.contains("not json"), "解析不了的行也要保留");
    }

    #[test]
    fn duplicating_without_any_session_id_fails() {
        // 一条记录都没有 sessionId：静默产出会得到一个仍指向原会话的文件。
        let source = r#"{"type":"summary","summary":"没有编号"}"#;
        let error = build_duplicate_text(source, "dup-id").expect_err("必须失败");
        assert_eq!(
            error.to_string(),
            "复制 Claude Code 会话失败：缺少会话编号。"
        );
    }

    #[test]
    fn duplicating_does_not_count_as_rewritten_when_the_field_is_absent() {
        let source = [
            r#"{"type":"summary"}"#.to_owned(),
            r#"{"type":"user","message":{"role":"user","content":"x"}}"#.to_owned(),
        ]
        .join("\n");
        assert!(build_duplicate_text(&source, "dup-id").is_err());
    }
}
