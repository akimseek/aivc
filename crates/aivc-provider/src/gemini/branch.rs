//! Gemini 分支与复制的文本构造。
//!
//! # 元数据判据比其他平台更严
//!
//! [`is_metadata_record`] 要求 `sessionId` **和** `projectHash` **都**是字符串。
//! 只看 `sessionId` 会把某些消息记录误判成元数据——Gemini 的部分记录也带 `sessionId`。
//!
//! # 分支截断的三条规则
//!
//! 1. **元数据只保留第一条**，并改写为新编号；后续元数据记录全部丢弃（它们描述的是
//!    源会话的后续状态）。
//! 2. **消息计数与分页一致**：包含系统消息，排除合成块，连续重复消息只计一次。
//! 3. **截到选中消息即停止**：包括 `$set.messages` 数组内的截断，后续记录全部丢弃。
//!
//! # 结尾必须补一条 `$set`
//!
//! 两种操作最后都追加 `{"$set":{"lastUpdated":now}}`。Gemini 按 `lastUpdated` 排序，
//! 不更新它会让新文件在刷新后回到源会话的旧位置——用户会以为操作失败了。

use serde_json::{Value, json};

use crate::paging::{LinePolicy, PageFolder};
use aivc_core::error::{Result, SessionError};
use aivc_core::session::PageCursor;
use aivc_core::time::Timestamp;

use super::parser::{SYNTHETIC_PREFIX, extract_text, gemini_role};
use aivc_core::session::Role;

/// 判断一条记录是否是会话元数据。
///
/// 两个字段缺一不可：只看 `sessionId` 会把部分消息记录误判成元数据。
#[must_use]
pub fn is_metadata_record(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object.get("sessionId").is_some_and(Value::is_string)
        && object.get("projectHash").is_some_and(Value::is_string)
}

/// 判断一条记录是否是**可见对话消息**（用户或助手，且有非合成正文）。
///
/// 仅用于识别对话正文；分支计数还包含系统消息，统一交给分页折叠器。
#[must_use]
pub fn is_visible_record(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    let role = gemini_role(
        object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    if role != Some(Role::User) && role != Some(Role::Assistant) {
        return false;
    }
    let text = extract_text(object.get("displayContent"));
    let text = if text.is_empty() {
        extract_text(object.get("content"))
    } else {
        text
    };
    let trimmed = text.trim();
    !trimmed.is_empty() && !trimmed.starts_with(SYNTHETIC_PREFIX)
}

/// 改写元数据记录：换编号、刷新时间、保留 `kind`。
fn rewrite_metadata(record: &Value, new_id: &str, now: &Timestamp) -> Value {
    let mut object = record.clone();
    if let Some(map) = object.as_object_mut() {
        map.insert("sessionId".to_owned(), json!(new_id));
        map.insert("startTime".to_owned(), json!(now.as_str()));
        map.insert("lastUpdated".to_owned(), json!(now.as_str()));
        let kind = map
            .get("kind")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("main")
            .to_owned();
        map.insert("kind".to_owned(), json!(kind));
    }
    object
}

/// 构造分支文本：保留到第 `keep_count` 条可见消息为止。
pub fn build_branch_text(
    source_text: &str,
    branch_id: &str,
    now: &Timestamp,
    keep_count: u64,
) -> Result<String> {
    if keep_count == 0 {
        return Err(SessionError::BranchNeedsContext.into());
    }
    let mut lines: Vec<String> = Vec::new();
    let mut rewritten_meta = false;
    let mut counter = PageFolder::new(PageCursor::Latest, 1, None, LinePolicy::Gemini);

    for raw_line in source_text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        if is_metadata_record(&item) {
            // 只保留第一条元数据；后续的都在描述源会话的后续状态。
            if !rewritten_meta {
                let rewritten = rewrite_metadata(&item, branch_id, now);
                lines.push(serialize(&rewritten, line));
                rewritten_meta = true;
            }
            continue;
        }

        if let Some(patch) = item.get("$set").and_then(Value::as_object)
            && let Some(messages) = patch.get("messages").and_then(Value::as_array)
        {
            let mut kept: Vec<Value> = Vec::new();
            for message in messages {
                counter.push_line(&serialize(message, ""), 0, 0);
                kept.push(message.clone());
                if counter.visible_count() >= keep_count {
                    break;
                }
            }
            let mut next = item.clone();
            if let Some(map) = next.as_object_mut() {
                let mut next_patch = patch.clone();
                next_patch.insert("messages".to_owned(), Value::Array(kept));
                map.insert("$set".to_owned(), Value::Object(next_patch));
            }
            lines.push(serialize(&next, line));
            if counter.visible_count() >= keep_count {
                break;
            }
            continue;
        }

        counter.push_line(line, 0, 0);
        lines.push(line.to_owned());
        if counter.visible_count() >= keep_count {
            break;
        }
    }

    if !rewritten_meta {
        return Err(SessionError::GeminiMissingMetadata.into());
    }
    if counter.visible_count() < keep_count {
        return Err(SessionError::GeminiInsufficientContext.into());
    }
    lines.push(serialize(
        &json!({ "$set": { "lastUpdated": now.as_str() } }),
        "",
    ));
    Ok(format!("{}\n", lines.join("\n")))
}

/// 构造复制文本：整份复制，只改写元数据编号。
pub fn build_duplicate_text(
    source_text: &str,
    duplicate_id: &str,
    now: &Timestamp,
) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rewritten_meta = false;

    for raw_line in source_text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            lines.push(line.to_owned());
            continue;
        }
        let Ok(item) = serde_json::from_str::<Value>(line) else {
            lines.push(line.to_owned());
            continue;
        };
        if !is_metadata_record(&item) {
            lines.push(line.to_owned());
            continue;
        }
        let rewritten = rewrite_metadata(&item, duplicate_id, now);
        lines.push(serialize(&rewritten, line));
        rewritten_meta = true;
    }

    if !rewritten_meta {
        return Err(SessionError::GeminiDuplicateMissingMetadata.into());
    }
    // 显式刷新 lastUpdated：Gemini 按它排序，不更新会让新文件回到源会话的旧位置。
    lines.push(serialize(
        &json!({ "$set": { "lastUpdated": now.as_str() } }),
        "",
    ));
    Ok(format!("{}\n", lines.join("\n")))
}

/// 序列化；失败时退回原始行（不应发生，但比 panic 好）。
fn serialize(value: &Value, fallback: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| fallback.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> Timestamp {
        Timestamp::from_raw("2026-09-16T12:34:56.789Z")
    }

    fn meta(session_id: &str) -> String {
        format!(
            r#"{{"sessionId":"{session_id}","projectHash":"hash-1","startTime":"2026-09-16T12:00:00.000Z","lastUpdated":"2026-09-16T12:00:00.000Z","kind":"main"}}"#
        )
    }

    fn user(text: &str) -> String {
        format!(r#"{{"type":"user","content":"{text}"}}"#)
    }

    fn assistant(text: &str) -> String {
        format!(r#"{{"type":"gemini","content":"{text}"}}"#)
    }

    #[test]
    fn metadata_detection_requires_both_fields() {
        let both: Value = serde_json::from_str(r#"{"sessionId":"s","projectHash":"h"}"#).unwrap();
        assert!(is_metadata_record(&both));

        // 只有 sessionId：消息记录也可能带它，不能当成元数据。
        let only_id: Value = serde_json::from_str(r#"{"sessionId":"s"}"#).unwrap();
        assert!(!is_metadata_record(&only_id));

        let only_hash: Value = serde_json::from_str(r#"{"projectHash":"h"}"#).unwrap();
        assert!(!is_metadata_record(&only_hash));

        // 类型不对也不算。
        let wrong_type: Value =
            serde_json::from_str(r#"{"sessionId":42,"projectHash":"h"}"#).unwrap();
        assert!(!is_metadata_record(&wrong_type));
    }

    #[test]
    fn visibility_excludes_system_roles_and_synthetic_blocks() {
        assert!(is_visible_record(
            &serde_json::from_str(&user("x")).unwrap()
        ));
        assert!(is_visible_record(
            &serde_json::from_str(&assistant("x")).unwrap()
        ));
        // 系统类不算可见消息。
        assert!(!is_visible_record(
            &serde_json::from_str(r#"{"type":"info","content":"x"}"#).unwrap()
        ));
        // 合成块不算。
        assert!(!is_visible_record(
            &serde_json::from_str(r#"{"type":"user","content":"<session_context>注入"}"#).unwrap()
        ));
        // 空正文不算。
        assert!(!is_visible_record(
            &serde_json::from_str(r#"{"type":"user","content":"  "}"#).unwrap()
        ));
    }

    #[test]
    fn branching_rewrites_the_first_metadata_record_only() {
        let source = [
            meta("old-id"),
            meta("old-id"),
            user("第一条"),
            assistant("回答"),
        ]
        .join("\n");
        let text = build_branch_text(&source, "new-id", &stamp(), 1).expect("branch");
        let records: Vec<Value> = text
            .trim_end()
            .split('\n')
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();

        // 第一条元数据被改写，第二条被丢弃。
        assert_eq!(records[0]["sessionId"], "new-id");
        assert_eq!(records[0]["startTime"], "2026-09-16T12:34:56.789Z");
        assert_eq!(records[0]["kind"], "main");
        assert_eq!(records[0]["projectHash"], "hash-1", "其他字段保留");
        let meta_count = records
            .iter()
            .filter(|record| is_metadata_record(record))
            .count();
        assert_eq!(meta_count, 1, "只应保留一条元数据");
    }

    #[test]
    fn branching_truncates_at_the_requested_message_count() {
        let source = [
            meta("old-id"),
            user("第一条"),
            assistant("回答一"),
            user("第二条"),
            assistant("回答二"),
        ]
        .join("\n");
        let text = build_branch_text(&source, "b", &stamp(), 2).expect("branch");
        assert!(text.contains("第一条"));
        assert!(text.contains("回答一"));
        assert!(!text.contains("第二条"), "第 2 条之后必须截掉");
    }

    #[test]
    fn branching_counts_system_messages_like_the_messages_page() {
        let source = [
            meta("old-id"),
            r#"{"type":"info","content":"系统提示"}"#.to_owned(),
            user("第一条"),
            assistant("回答一"),
        ]
        .join("\n");
        let text = build_branch_text(&source, "b", &stamp(), 2).expect("branch");
        assert!(text.contains("系统提示"));
        assert!(text.contains("第一条"));
        assert!(!text.contains("回答一"), "额度用尽后停止读取");
    }

    #[test]
    fn branching_stops_reading_once_the_quota_is_reached() {
        // 额度用尽后不再继续读：即使后面只有不可见记录也不保留。
        let source = [
            meta("old-id"),
            user("第一条"),
            r#"{"type":"info","content":"额度之后的系统提示"}"#.to_owned(),
        ]
        .join("\n");
        let text = build_branch_text(&source, "b", &stamp(), 1).expect("branch");
        assert!(text.contains("第一条"));
        assert!(!text.contains("额度之后的系统提示"));
    }

    #[test]
    fn branching_filters_messages_inside_set_patches() {
        let source = [
            meta("old-id"),
            r#"{"$set":{"messages":[{"type":"user","content":"补丁一"},{"type":"info","content":"补丁系统"},{"type":"user","content":"补丁二"}]}}"#.to_owned(),
        ]
        .join("\n");
        let text = build_branch_text(&source, "b", &stamp(), 1).expect("branch");
        assert!(text.contains("补丁一"));
        assert!(!text.contains("补丁系统"), "分支点之后的系统消息也必须截掉");
        assert!(!text.contains("补丁二"), "超出额度的可见消息必须剔除");
    }

    #[test]
    fn branching_appends_a_last_updated_patch() {
        let source = [meta("old-id"), user("x")].join("\n");
        let text = build_branch_text(&source, "b", &stamp(), 1).expect("branch");
        let last = text.trim_end().split('\n').next_back().unwrap();
        let record: Value = serde_json::from_str(last).unwrap();
        // Gemini 按 lastUpdated 排序；不刷新会让新文件回到源会话的旧位置。
        assert_eq!(record["$set"]["lastUpdated"], "2026-09-16T12:34:56.789Z");
    }

    #[test]
    fn branching_position_matches_deduplicated_patches_and_system_messages() {
        let source = [meta("old"),
            r#"{"$set":{"messages":[{"type":"user","content":"第一条"},{"type":"user","content":"第一条"},{"type":"info","content":"系统提示"},{"type":"gemini","content":"选中的回答"},{"type":"user","content":"后续"}]}}"#.into()].join("\n");
        let text = build_branch_text(&source, "b", &stamp(), 3).unwrap();
        assert!(text.contains("选中的回答"));
        assert!(!text.contains("后续"));
        let mut page = PageFolder::new(PageCursor::Latest, 10, None, LinePolicy::Gemini);
        for line in text.lines() {
            page.push_line(line, 0, 0);
        }
        assert_eq!(page.visible_count(), 3);
    }

    #[test]
    fn branching_without_metadata_fails() {
        let source = user("没有元数据");
        let error = build_branch_text(&source, "b", &stamp(), 1).expect_err("必须失败");
        assert_eq!(error.to_string(), "Gemini 源会话缺少 session 元数据。");
    }

    #[test]
    fn branching_with_insufficient_context_fails() {
        let source = [meta("old-id"), user("只有一条")].join("\n");
        let error = build_branch_text(&source, "b", &stamp(), 5).expect_err("必须失败");
        assert_eq!(error.to_string(), "Gemini 源会话上下文不足，无法创建分支。");
    }

    #[test]
    fn duplicating_rewrites_metadata_and_preserves_everything_else() {
        let source = [
            meta("old-id"),
            user("第一条"),
            "not json".to_owned(),
            assistant("回答"),
        ]
        .join("\n");
        let text = build_duplicate_text(&source, "dup-id", &stamp()).expect("duplicate");

        assert!(text.contains("dup-id"));
        assert!(!text.contains("old-id"));
        assert!(text.contains("第一条"));
        assert!(text.contains("not json"), "解析不了的行也要保留");
        assert_eq!(text.trim_end().split('\n').count(), 5, "4 行 + 结尾的 $set");
    }

    #[test]
    fn duplicating_without_metadata_fails() {
        let source = user("没有元数据");
        let error = build_duplicate_text(&source, "dup-id", &stamp()).expect_err("必须失败");
        assert_eq!(error.to_string(), "复制 Gemini 会话失败：缺少会话元数据。");
    }

    #[test]
    fn both_operations_end_with_a_newline() {
        let source = [meta("old-id"), user("x")].join("\n");
        assert!(
            build_branch_text(&source, "b", &stamp(), 1)
                .expect("branch")
                .ends_with('\n')
        );
        assert!(
            build_duplicate_text(&source, "d", &stamp())
                .expect("duplicate")
                .ends_with('\n')
        );
    }
}
