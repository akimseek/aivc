//! 会话元数据的归一化与合并。
//!
//! 这些是**纯函数**，但它们是 parity 关键：自定义标题、分支来源、收藏与最近打开
//! 共用同一条元数据记录，任何一处合并写错都会互相覆盖（现网 DEV-GUIDE 专门记录过
//! 「在同一元数据记录中合并更新，彼此不覆盖」）。
//!
//! # 为什么用「宽松读取 + 归一化」而不是直接反序列化
//!
//! 现网在 `JSON.parse` 之后逐字段判断类型：
//!
//! ```js
//! favorite: metadata.favorite === true || undefined,
//! parentMessageLine: typeof v === "number" && Number.isInteger(v) ? v : undefined,
//! ```
//!
//! 也就是说，某个字段类型不符只会让**该字段**被丢弃，其余元数据照常保留。
//! 若直接反序列化到强类型，一个字段损坏就会让整条记录消失——用户会莫名其妙丢掉
//! 自定义标题。因此这里保留 [`RawSessionMetadata`] 作为磁盘形态。
//!
//! # 关键语义（必须逐字保持）
//!
//! - `favorite`：只有显式 `true` 才算收藏；`false` 会被归一化为「未设置」，
//!   因此取消收藏在没有任何其他元数据时会**删除整条记录**。
//! - `branch`：缺少 `parentSessionId` 时整块丢弃；`createdBy` 缺省为 `branch`。
//! - `customTitle`：`trim` 后为空视为未设置。
//! - `lastOpenedAt`：非空字符串才保留。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::session::{BranchMetadata, BranchOrigin, Session, SessionMetadata};

/// 磁盘形态的会话元数据：字段宽松读取，逐字段容错。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawSessionMetadata {
    updated_at: Option<Value>,
    custom_title: Option<Value>,
    branch: Option<Value>,
    favorite: Option<Value>,
    last_opened_at: Option<Value>,
}

impl RawSessionMetadata {
    /// 归一化为强类型元数据。
    #[must_use]
    pub fn normalize(&self) -> SessionMetadata {
        SessionMetadata {
            updated_at: self
                .updated_at
                .as_ref()
                .and_then(read_non_empty_string)
                .map(Into::into),
            // 自定义标题需要 trim（现网 `normalizeCustomTitle` 返回 trim 后的值）。
            custom_title: self
                .custom_title
                .as_ref()
                .and_then(read_trimmed_non_empty_string),
            branch: self.branch.as_ref().and_then(read_branch),
            // 现网是 `=== true`：显式 false 与任何非布尔值都归一化为「未设置」。
            favorite: (self.favorite.as_ref().and_then(Value::as_bool) == Some(true))
                .then_some(true),
            last_opened_at: self
                .last_opened_at
                .as_ref()
                .and_then(read_non_empty_string)
                .map(Into::into),
        }
    }
}

/// 把强类型元数据重新归一化（合并操作后调用）。
#[must_use]
pub fn normalize(metadata: &SessionMetadata) -> SessionMetadata {
    SessionMetadata {
        updated_at: metadata
            .updated_at
            .as_ref()
            .filter(|value| !value.is_empty())
            .cloned(),
        custom_title: metadata
            .custom_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        branch: metadata.branch.as_ref().and_then(normalize_branch_typed),
        favorite: (metadata.favorite == Some(true)).then_some(true),
        last_opened_at: metadata
            .last_opened_at
            .as_ref()
            .filter(|value| !value.is_empty())
            .cloned(),
    }
}

/// 元数据是否为空（四个业务字段全空）。
///
/// 空元数据不落库；若某次更新让它变空（例如取消收藏且无其他元数据），
/// 调用方应删除整条记录而不是写入空对象。
#[must_use]
pub fn is_empty(metadata: &SessionMetadata) -> bool {
    metadata.custom_title.is_none()
        && metadata.branch.is_none()
        && metadata.favorite.is_none()
        && metadata.last_opened_at.is_none()
}

/// 把元数据附着到会话上。
///
/// 现网 `attachMetadata` 语义：
///
/// - 归一化后为空 → 原样返回会话（**不**写入 `metadata`，也**不**设置 `sourceTitle`）；
/// - 否则 `sourceTitle = 原 sourceTitle ?? 原 title`，`title = customTitle ?? sourceTitle`。
///
/// 这个细节很重要：它让「CLI 原始标题」在首次设置自定义标题时被固化下来，
/// 之后取消自定义标题仍能回退到原始标题而不是编号。
#[must_use]
pub fn attach(mut session: Session, metadata: &SessionMetadata) -> Session {
    let normalized = normalize(metadata);
    if is_empty(&normalized) {
        return session;
    }
    let source_title = session
        .source_title
        .clone()
        .unwrap_or_else(|| session.title.clone());
    session.title = normalized
        .custom_title
        .clone()
        .unwrap_or_else(|| source_title.clone());
    session.source_title = Some(source_title);
    session.metadata = Some(normalized);
    session
}

/// 合并自定义标题。返回归一化后的元数据与「是否应删除记录」。
#[must_use]
pub fn merge_custom_title(
    current: &SessionMetadata,
    custom_title: &str,
    now: crate::time::Timestamp,
) -> (SessionMetadata, bool) {
    let mut next = current.clone();
    next.custom_title = Some(custom_title.to_owned());
    next.updated_at = Some(now);
    finish(&next)
}

/// 合并收藏状态。
///
/// `favorite = false` 会被归一化为「未设置」，因此取消收藏可能让记录变空并被删除。
#[must_use]
pub fn merge_favorite(
    current: &SessionMetadata,
    favorite: bool,
    now: crate::time::Timestamp,
) -> (SessionMetadata, bool) {
    let mut next = current.clone();
    next.favorite = favorite.then_some(true);
    next.updated_at = Some(now);
    finish(&next)
}

/// 合并「最近打开」时间。现网此操作**总是**落库，不检查是否为空。
#[must_use]
pub fn merge_last_opened(
    current: &SessionMetadata,
    now: crate::time::Timestamp,
) -> SessionMetadata {
    let mut next = current.clone();
    next.last_opened_at = Some(now.clone());
    next.updated_at = Some(now);
    normalize(&next)
}

/// 合并分支来源。现网此操作**总是**落库，不检查是否为空。
#[must_use]
pub fn merge_branch(
    current: &SessionMetadata,
    branch: &BranchMetadata,
    now: crate::time::Timestamp,
) -> SessionMetadata {
    let mut next = current.clone();
    next.branch = Some(branch.clone());
    next.updated_at = Some(now);
    normalize(&next)
}

/// 归一化并判断是否为空。
fn finish(next: &SessionMetadata) -> (SessionMetadata, bool) {
    let normalized = normalize(next);
    let empty = is_empty(&normalized);
    (normalized, empty)
}

/// 读取「非空字符串」字段，**保留原始空白**。
///
/// 用于时间戳：现网 `normalizeTimestamp` 只用 `value.trim()` 做判空，
/// 返回的仍是原值，不得擅自 trim。
fn read_non_empty_string(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// 读取「非空字符串」字段并 **trim**。用于自定义标题。
fn read_trimmed_non_empty_string(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// 从宽松 JSON 读取分支元数据。
fn read_branch(value: &Value) -> Option<BranchMetadata> {
    let object = value.as_object()?;
    let parent_session_id = object
        .get("parentSessionId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;

    Some(BranchMetadata {
        parent_target_id: object
            .get("parentTargetId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(Into::into),
        parent_session_id: Some(parent_session_id.into()),
        // 现网要求 `Number.isInteger`；小数与字符串都会被丢弃。
        parent_message_index: object.get("parentMessageIndex").and_then(integral_u64),
        parent_message_line: object.get("parentMessageLine").and_then(integral_u64),
        // 缺省为 `branch`，只有显式 `manual` 才视为手工创建。
        created_by: Some(
            if object.get("createdBy").and_then(Value::as_str) == Some("manual") {
                BranchOrigin::Manual
            } else {
                BranchOrigin::Branch
            },
        ),
    })
}

/// 归一化强类型分支元数据。
fn normalize_branch_typed(branch: &BranchMetadata) -> Option<BranchMetadata> {
    let parent_session_id = branch
        .parent_session_id
        .as_ref()
        .filter(|value| !value.is_empty())?;
    Some(BranchMetadata {
        parent_target_id: branch
            .parent_target_id
            .clone()
            .filter(|value| !value.is_empty()),
        parent_session_id: Some(parent_session_id.clone()),
        parent_message_index: branch.parent_message_index,
        parent_message_line: branch.parent_message_line,
        created_by: Some(branch.created_by.unwrap_or(BranchOrigin::Branch)),
    })
}

/// 读取整数型 JSON 值（对齐 JS `Number.isInteger`）。
///
/// JS 里 `3.0` 与 `3` 是同一个值，因此整值浮点也必须接受；而
/// `serde_json::Number::as_u64` 对以浮点形式存储的 `3.0` 返回 `None`，
/// 所以这里必须先做范围与整值检查，再转换。
#[allow(clippy::cast_sign_loss)] // 上方已确认 number >= 0.0 且小数部分为 0
fn integral_u64(value: &Value) -> Option<u64> {
    let number = value.as_f64()?;
    if number.is_finite() && number.fract() == 0.0 && number >= 0.0 {
        Some(number as u64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::{SessionId, TargetId};
    use crate::session::Message;
    use crate::time::Timestamp;

    fn parse(json: &str) -> SessionMetadata {
        serde_json::from_str::<RawSessionMetadata>(json)
            .expect("raw parse")
            .normalize()
    }

    fn session(title: &str, source_title: Option<&str>) -> Session {
        Session {
            id: SessionId::from("s1"),
            title: title.to_owned(),
            source_title: source_title.map(str::to_owned),
            cwd: None,
            created_at: None,
            updated_at: None,
            model: None,
            model_status: None,
            cli_version: None,
            file_path: "/x.jsonl".to_owned(),
            file_mtime_ms: None,
            file_change_ms: None,
            file_size: None,
            message_count: 0,
            message_count_is_lower_bound: false,
            preview: Vec::<Message>::new(),
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    fn at(value: &str) -> Timestamp {
        Timestamp::from_raw(value)
    }

    #[test]
    fn favorite_requires_strict_true() {
        // 现网是 `=== true`：false 与非布尔值都归一化为「未设置」。
        assert_eq!(parse(r#"{"favorite":true}"#).favorite, Some(true));
        assert_eq!(parse(r#"{"favorite":false}"#).favorite, None);
        assert_eq!(parse(r#"{"favorite":"true"}"#).favorite, None);
        assert_eq!(parse(r#"{"favorite":1}"#).favorite, None);
        assert_eq!(parse("{}").favorite, None);
    }

    #[test]
    fn custom_title_is_trimmed_and_blank_becomes_unset() {
        assert_eq!(
            parse(r#"{"customTitle":"  我的标题  "}"#)
                .custom_title
                .as_deref(),
            Some("我的标题")
        );
        assert_eq!(parse(r#"{"customTitle":"   "}"#).custom_title, None);
        assert_eq!(parse(r#"{"customTitle":123}"#).custom_title, None);
    }

    #[test]
    fn timestamps_are_kept_verbatim_and_only_checked_for_blankness() {
        // 现网对时间戳只做判空、不做 trim；擅自 trim 会改变落库内容。
        assert_eq!(
            parse(r#"{"lastOpenedAt":" 2026-01-01T00:00:00.000Z "}"#)
                .last_opened_at
                .as_ref()
                .map(Timestamp::as_str),
            Some(" 2026-01-01T00:00:00.000Z ")
        );
        assert!(parse(r#"{"lastOpenedAt":"   "}"#).last_opened_at.is_none());
        assert!(parse(r#"{"lastOpenedAt":123}"#).last_opened_at.is_none());
    }

    #[test]
    fn branch_requires_parent_session_id() {
        assert!(
            parse(r#"{"branch":{"parentTargetId":"local"}}"#)
                .branch
                .is_none()
        );
        assert!(
            parse(r#"{"branch":{"parentSessionId":""}}"#)
                .branch
                .is_none()
        );
        assert!(parse(r#"{"branch":"nope"}"#).branch.is_none());

        let branch = parse(r#"{"branch":{"parentSessionId":"p1"}}"#)
            .branch
            .expect("branch");
        assert_eq!(
            branch.parent_session_id.as_ref().map(AsRef::as_ref),
            Some("p1")
        );
        // createdBy 缺省为 branch。
        assert_eq!(branch.created_by, Some(BranchOrigin::Branch));
    }

    #[test]
    fn branch_created_by_defaults_to_branch_unless_explicitly_manual() {
        let manual = parse(r#"{"branch":{"parentSessionId":"p1","createdBy":"manual"}}"#)
            .branch
            .expect("branch");
        assert_eq!(manual.created_by, Some(BranchOrigin::Manual));

        let other = parse(r#"{"branch":{"parentSessionId":"p1","createdBy":"whatever"}}"#)
            .branch
            .expect("branch");
        assert_eq!(other.created_by, Some(BranchOrigin::Branch));
    }

    #[test]
    fn branch_message_positions_require_integers() {
        let branch = parse(
            r#"{"branch":{"parentSessionId":"p1","parentMessageIndex":3,"parentMessageLine":42}}"#,
        )
        .branch
        .expect("branch");
        assert_eq!(branch.parent_message_index, Some(3));
        assert_eq!(branch.parent_message_line, Some(42));

        // 小数与字符串被丢弃，但分支本身保留。
        let loose = parse(
            r#"{"branch":{"parentSessionId":"p1","parentMessageIndex":1.5,"parentMessageLine":"9"}}"#,
        )
        .branch
        .expect("branch");
        assert_eq!(loose.parent_message_index, None);
        assert_eq!(loose.parent_message_line, None);

        // JS 里 3.0 与 3 是同一个值，整值浮点必须接受。
        let integral = parse(r#"{"branch":{"parentSessionId":"p1","parentMessageLine":3.0}}"#)
            .branch
            .expect("branch");
        assert_eq!(integral.parent_message_line, Some(3));
    }

    #[test]
    fn malformed_fields_do_not_discard_the_whole_record() {
        // 这是本模块存在的核心理由：一个字段类型不符不得让自定义标题消失。
        let metadata = parse(
            r#"{"customTitle":"保留我","favorite":"oops","branch":123,"lastOpenedAt":{"x":1}}"#,
        );
        assert_eq!(metadata.custom_title.as_deref(), Some("保留我"));
        assert_eq!(metadata.favorite, None);
        assert!(metadata.branch.is_none());
        assert!(metadata.last_opened_at.is_none());
    }

    #[test]
    fn emptiness_ignores_updated_at() {
        // updatedAt 只是审计字段，单独存在不构成「有元数据」。
        let metadata = parse(r#"{"updatedAt":"2026-01-01T00:00:00.000Z"}"#);
        assert!(is_empty(&metadata));

        let with_title = parse(r#"{"customTitle":"x","updatedAt":"2026-01-01T00:00:00.000Z"}"#);
        assert!(!is_empty(&with_title));
    }

    #[test]
    fn attach_freezes_the_cli_title_into_source_title() {
        let metadata = parse(r#"{"customTitle":"我的标题"}"#);
        let attached = attach(session("首条用户消息", None), &metadata);
        assert_eq!(attached.title, "我的标题");
        // 原始标题被固化到 sourceTitle，取消自定义标题后可回退。
        assert_eq!(attached.source_title.as_deref(), Some("首条用户消息"));
        assert!(attached.metadata.is_some());
    }

    #[test]
    fn attach_prefers_an_existing_source_title() {
        let metadata = parse(r#"{"customTitle":"我的标题"}"#);
        let attached = attach(session("当前标题", Some("CLI 原始标题")), &metadata);
        assert_eq!(attached.title, "我的标题");
        assert_eq!(attached.source_title.as_deref(), Some("CLI 原始标题"));
    }

    #[test]
    fn attach_leaves_the_session_untouched_when_metadata_is_empty() {
        // 没有元数据时不得凭空写入 sourceTitle 或 metadata 字段。
        let attached = attach(session("原标题", None), &SessionMetadata::default());
        assert_eq!(attached.title, "原标题");
        assert!(attached.source_title.is_none());
        assert!(attached.metadata.is_none());
    }

    #[test]
    fn unsetting_favorite_on_an_otherwise_empty_record_marks_it_for_deletion() {
        let (metadata, empty) = merge_favorite(
            &SessionMetadata::default(),
            false,
            at("2026-01-01T00:00:00.000Z"),
        );
        assert!(empty, "取消收藏且无其他元数据时应删除记录");
        assert!(is_empty(&metadata));
    }

    #[test]
    fn setting_favorite_keeps_existing_custom_title() {
        // 收藏与自定义标题必须互不覆盖。
        let current = parse(r#"{"customTitle":"标题"}"#);
        let (metadata, empty) = merge_favorite(&current, true, at("2026-01-02T00:00:00.000Z"));
        assert!(!empty);
        assert_eq!(metadata.favorite, Some(true));
        assert_eq!(metadata.custom_title.as_deref(), Some("标题"));
    }

    #[test]
    fn setting_custom_title_keeps_existing_favorite_and_branch() {
        let current =
            parse(r#"{"favorite":true,"branch":{"parentSessionId":"p1","parentMessageLine":7}}"#);
        let (metadata, empty) =
            merge_custom_title(&current, "新标题", at("2026-01-02T00:00:00.000Z"));
        assert!(!empty);
        assert_eq!(metadata.custom_title.as_deref(), Some("新标题"));
        assert_eq!(metadata.favorite, Some(true));
        assert_eq!(
            metadata
                .branch
                .as_ref()
                .and_then(|branch| branch.parent_message_line),
            Some(7)
        );
    }

    #[test]
    fn clearing_custom_title_keeps_other_fields() {
        let current = parse(r#"{"customTitle":"旧","favorite":true}"#);
        let (metadata, empty) = merge_custom_title(&current, "   ", at("2026-01-02T00:00:00.000Z"));
        assert!(!empty);
        assert_eq!(metadata.custom_title, None);
        assert_eq!(metadata.favorite, Some(true));
    }

    #[test]
    fn mark_opened_always_produces_a_storable_record() {
        // 现网 markSessionOpened 不做空检查，总是落库。
        let metadata =
            merge_last_opened(&SessionMetadata::default(), at("2026-01-02T00:00:00.000Z"));
        assert!(!is_empty(&metadata));
        assert_eq!(
            metadata.last_opened_at.as_ref().map(Timestamp::as_str),
            Some("2026-01-02T00:00:00.000Z")
        );
        assert_eq!(
            metadata.updated_at.as_ref().map(Timestamp::as_str),
            Some("2026-01-02T00:00:00.000Z")
        );
    }

    #[test]
    fn branch_merge_keeps_favorite_and_title() {
        let current = parse(r#"{"customTitle":"标题","favorite":true}"#);
        let branch = BranchMetadata {
            parent_target_id: Some(TargetId::from("local")),
            parent_session_id: Some(SessionId::from("parent")),
            parent_message_index: None,
            parent_message_line: Some(11),
            created_by: Some(BranchOrigin::Branch),
        };
        let metadata = merge_branch(&current, &branch, at("2026-01-02T00:00:00.000Z"));
        assert_eq!(metadata.custom_title.as_deref(), Some("标题"));
        assert_eq!(metadata.favorite, Some(true));
        assert_eq!(
            metadata
                .branch
                .as_ref()
                .and_then(|b| b.parent_session_id.as_ref())
                .map(AsRef::as_ref),
            Some("parent")
        );
    }

    #[test]
    fn branch_merge_drops_a_branch_without_parent_session_id() {
        // 缺少父会话编号的分支块必须被丢弃，而不是留下半截记录。
        let branch = BranchMetadata {
            parent_target_id: Some(TargetId::from("local")),
            parent_session_id: None,
            parent_message_index: None,
            parent_message_line: None,
            created_by: Some(BranchOrigin::Branch),
        };
        let metadata = merge_branch(
            &SessionMetadata::default(),
            &branch,
            at("2026-01-02T00:00:00.000Z"),
        );
        assert!(metadata.branch.is_none());
    }

    #[test]
    fn normalize_is_idempotent() {
        let metadata = parse(
            r#"{"customTitle":"  t  ","favorite":true,"branch":{"parentSessionId":"p","createdBy":"manual"},"lastOpenedAt":"2026-01-01T00:00:00.000Z"}"#,
        );
        assert_eq!(normalize(&metadata), metadata);
    }
}
