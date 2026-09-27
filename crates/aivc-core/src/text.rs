//! 纯文本处理：合成消息过滤、标题裁剪、搜索匹配。
//!
//! 这里集中现网 `session-parser.ts` 与 `provider-common.ts` 中的纯函数。放在
//! `aivc-core` 是因为它们**不含任何 I/O**，可以被穷举测试，而它们的行为直接决定
//! 用户看到的消息列表与搜索命中结果。
//!
//! # 与 JavaScript 的两处刻意差异
//!
//! 1. **长度按 Unicode 标量而非 UTF-16 码元计算。** JS 的 `slice` 按 UTF-16 码元切分，
//!    可能把代理对切成孤立代理（产生非法字符串）。Rust 的 `String` 无法表示孤立代理，
//!    因此这里按 `char` 边界截断，永不产生非法 UTF-8。对 BMP 文本（绝大多数标题）两者
//!    结果一致，仅在 emoji 等星平面字符上不同。
//! 2. **空白归一额外覆盖 U+FEFF。** JS 的 `\s` 包含零宽不换行空格，`char::is_whitespace`
//!    不包含，因此显式补齐。

use crate::session::Session;

/// 现网 `clampText` 的默认标题长度上限。
pub const TITLE_MAX_LENGTH: usize = 88;

/// Codex 多代理启动时写入的合成用户消息前缀。
///
/// 这些块不是用户输入，展示出来会变成 `unknown` 消息，必须过滤。
const SYNTHETIC_USER_BLOCK_PREFIXES: &[&str] = &[
    "<environment_context>",
    "<skill>",
    "<user_instructions>",
    "<developer_instructions>",
    "You are `/root`, the primary agent in a team of agents collaborating to fulfill the user's goals.",
];

/// 判断一个字符是否属于需要归一的空白（对齐 JS `\s` 的常用子集）。
pub(crate) fn is_js_whitespace(value: char) -> bool {
    value.is_whitespace() || value == '\u{feff}'
}

/// 判断是否为 Codex 合成用户消息块。
///
/// 现网规则：命中固定前缀，或同时包含 `<multi_agent_mode>` 与多代理激活说明。
#[must_use]
pub fn is_synthetic_user_block(text: &str) -> bool {
    let normalized = text.trim();
    if SYNTHETIC_USER_BLOCK_PREFIXES
        .iter()
        .any(|prefix| normalized.starts_with(prefix))
    {
        return true;
    }
    normalized.contains("<multi_agent_mode>")
        && normalized.contains("At the start of your turn, you are the active agent.")
}

/// 该消息是否应当保留在可见列表中（非空且非合成块）。
#[must_use]
pub fn should_keep_message(text: &str) -> bool {
    !text.trim().is_empty() && !is_synthetic_user_block(text)
}

/// 标题候选：非空且不是合成块。
#[must_use]
pub fn is_title_candidate(text: &str) -> bool {
    should_keep_message(text)
}

/// 把连续空白归一为单个空格并去掉首尾空白。
#[must_use]
pub fn normalize_whitespace(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut pending_space = false;
    for character in value.chars() {
        if is_js_whitespace(character) {
            // 仅在已经写出内容后才考虑插入空格，天然去掉前导空白。
            pending_space = !result.is_empty();
            continue;
        }
        if pending_space {
            result.push(' ');
            pending_space = false;
        }
        result.push(character);
    }
    result
}

/// 归一空白后按字符数截断，超出时以 `...` 结尾。
///
/// 与现网 `clampText` 一致：`length` 是**结果总长上限**，截断时保留 `length - 1`
/// 个字符再加 3 个点，因此结果长度为 `length + 2`。
#[must_use]
pub fn clamp_text(value: &str, length: usize) -> String {
    let normalized = normalize_whitespace(value);
    let char_count = normalized.chars().count();
    if char_count <= length {
        return normalized;
    }
    // length 为 0 或 1 时无法保留字符，直接返回省略号，避免下溢。
    let keep = length.saturating_sub(1);
    let mut truncated: String = normalized.chars().take(keep).collect();
    truncated.push_str("...");
    truncated
}

/// 按默认长度裁剪标题。
///
/// # 为什么不只是截断
///
/// 标题取自消息**正文**，而正文常以结构开头。直接截断会把标记带进列表：
///
/// ```text
/// # 工作状态摘要 ## 当前目标 - [ ] 完成历史会话加载性能优化收口…
/// ```
///
/// 这在列表里读起来像乱码，而标题只需要那几个词。因此先取**首个非空行**、
/// 再去掉行内的 markdown 标记，最后才截断。
#[must_use]
pub fn clamp_title(value: &str) -> String {
    let first_line = value
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    clamp_text(&strip_title_markers(first_line), TITLE_MAX_LENGTH)
}

/// 去掉标题里不该出现的 markdown 标记。
///
/// 逐词处理，只丢**纯标记词**：
///
/// - `#`、`##`、`>`、`***` 这类纯符号；
/// - 复选框拆开后的 `[`、`]`；
/// - 单独成词的无序列表符 `-` / `+`；
/// - 有序列表序号 `1.`。
///
/// **不碰正文词**：`-5`、`#tag`、`T[0]` 都原样保留——它们可能是内容。
#[must_use]
pub fn strip_title_markers(line: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for token in line.split_whitespace() {
        // 纯标记词：`#`、`##`、`---`、`***`、`-`、`+`，以及复选框拆开后的 `[`、`]`。
        // 一个字都没有的「词」不承载任何信息。
        if token.chars().all(|c| "#>*_`-+=~[]()".contains(c)) {
            continue;
        }
        // 有序列表序号：`1.`、`12.`
        if token.ends_with('.') && token[..token.len() - 1].chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        kept.push(token);
    }
    kept.join(" ")
}

/// 归一搜索词：去空白、转小写、按空白切分。
///
/// 现网要求**全部词都命中**（AND 语义），因此返回词列表而非单一子串。
#[must_use]
pub fn normalize_search_terms(query: &str) -> Vec<String> {
    query
        .trim()
        .to_lowercase()
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .map(str::to_owned)
        .collect()
}

/// 会话摘要级搜索匹配。
///
/// 检索范围与现网 `matchesSessionSummary` 一致：编号、标题、来源标题、工作目录、
/// 模型与预览消息全文；**不含**完整正文（正文检索由 Provider 单独扫描文件）。
#[must_use]
pub fn matches_session_summary(session: &Session, normalized_query: &str) -> bool {
    if normalized_query.is_empty() {
        return true;
    }
    session_search_haystack(session)
        .to_lowercase()
        .contains(normalized_query)
}

/// 会话摘要的多词匹配：**全部**词都必须命中。
///
/// 这是现网 Codex 搜索的语义（`normalizeSearchTerms` 分词后逐个 `includes`）。
#[must_use]
pub fn matches_session_terms(session: &Session, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    matches_all_terms(&session_search_haystack(session), terms)
}

/// 会话摘要拼成的可搜索文本。
///
/// 覆盖编号、标题、来源标题、工作目录、模型、CLI 版本与全部预览正文——与现网
/// `matchesSearch` 检查的字段一致。
#[must_use]
pub fn session_search_haystack(session: &Session) -> String {
    let mut haystack = String::new();
    for part in std::iter::once(&session.id.to_string())
        .chain(std::iter::once(&session.title))
        .chain(session.source_title.iter())
        .chain(session.cwd.iter())
        .chain(session.model.iter())
        .chain(std::iter::once(&session.cli_version).filter_map(|value| value.as_ref()))
        .chain(session.preview.iter().map(|message| &message.text))
    {
        haystack.push_str(part);
        haystack.push('\n');
    }
    haystack
}

/// 多词匹配：全部词都必须命中（现网 Codex 搜索语义）。
#[must_use]
pub fn matches_all_terms(haystack: &str, terms: &[String]) -> bool {
    let lowered = haystack.to_lowercase();
    terms.iter().all(|term| lowered.contains(term.as_str()))
}

/// 会话文件名时间戳：`2026-09-16T12-34-56`。
///
/// 对应现网 `date.toISOString().slice(0, 19).replace(/:/g, "-")`，用于构造
/// `rollout-<stamp>-<uuid>.jsonl`。
#[must_use]
pub fn format_session_stamp(timestamp: &crate::time::Timestamp) -> String {
    let raw = timestamp.as_str();
    // 形如 `YYYY-MM-DDTHH:mm:ss.sssZ`，取到秒并把冒号换成短横。
    let head: String = raw.chars().take(19).collect();
    head.replace(':', "-")
}

/// 错误信息长度截断：保留原始空白，仅做长度限制。
///
/// 与 [`clamp_text`] 不同：**不归一空白**，因为 CLI 返回的诊断文本里换行与缩进有意义。
#[must_use]
pub fn clamp_raw(value: &str, max_length: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_length {
        return value.to_owned();
    }
    let keep = max_length.saturating_sub(1);
    let mut truncated: String = value.chars().take(keep).collect();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::SessionId;
    use crate::session::{Message, Role};

    #[test]
    fn filters_codex_synthetic_initialization_blocks() {
        for block in [
            "<environment_context>\n<shell>bash</shell>",
            "<skill>\nname: x\n</skill>",
            "<user_instructions>do things</user_instructions>",
            "<developer_instructions>rules</developer_instructions>",
            "You are `/root`, the primary agent in a team of agents collaborating to fulfill the user's goals.",
        ] {
            assert!(is_synthetic_user_block(block), "should filter: {block}");
            assert!(!should_keep_message(block));
        }
    }

    #[test]
    fn filters_multi_agent_activation_block_only_when_both_markers_present() {
        let both = "<multi_agent_mode>\nAt the start of your turn, you are the active agent.\n</multi_agent_mode>";
        assert!(is_synthetic_user_block(both));
        // 只有其中一个标记时不得误判为合成块。
        assert!(!is_synthetic_user_block("<multi_agent_mode> alone"));
        assert!(!is_synthetic_user_block(
            "At the start of your turn, you are the active agent."
        ));
    }

    #[test]
    fn keeps_real_user_input_and_trims_blank_messages() {
        assert!(should_keep_message("请帮我重构这段代码"));
        assert!(!should_keep_message("   \n\t "));
        // 前缀必须从行首开始才算合成块，正文中间出现不算。
        assert!(should_keep_message("先看这个：<skill> 标签的用法"));
    }

    #[test]
    fn normalizes_whitespace_including_full_width_and_bom() {
        assert_eq!(normalize_whitespace("  a \n\t b  "), "a b");
        assert_eq!(normalize_whitespace("a\u{3000}b"), "a b");
        assert_eq!(normalize_whitespace("\u{feff}a"), "a");
        assert_eq!(normalize_whitespace(""), "");
        assert_eq!(normalize_whitespace("   "), "");
    }

    #[test]
    fn clamps_long_text_with_ellipsis_suffix() {
        let long = "a".repeat(200);
        let clamped = clamp_text(&long, 88);
        assert_eq!(clamped.chars().count(), 87 + 3);
        assert!(clamped.ends_with("..."));
        assert!(clamped.starts_with(&"a".repeat(87)));
    }

    #[test]
    fn does_not_clamp_text_at_or_below_the_limit() {
        let exact = "a".repeat(88);
        assert_eq!(clamp_text(&exact, 88), exact);
        assert_eq!(clamp_text("short", 88), "short");
    }

    #[test]
    fn clamp_never_underflows_for_degenerate_limits() {
        assert_eq!(clamp_text("abcdef", 0), "...");
        assert_eq!(clamp_text("abcdef", 1), "...");
        assert_eq!(clamp_text("ab", 2), "ab");
    }

    #[test]
    fn clamp_handles_multibyte_text_without_panicking() {
        let chinese = "中文标题".repeat(40);
        let clamped = clamp_title(&chinese);
        assert!(clamped.ends_with("..."));
        // 结果必须是合法 UTF-8 且能正常计数。
        assert!(clamped.chars().count() > 3);
    }

    #[test]
    fn normalizes_search_terms_with_and_semantics() {
        assert_eq!(normalize_search_terms("  Foo  BAR "), vec!["foo", "bar"]);
        assert!(normalize_search_terms("   ").is_empty());
    }

    #[test]
    fn matches_summary_across_id_title_cwd_model_and_preview() {
        let session = Session {
            id: SessionId::from("0191f2a3-aaaa"),
            title: "重构 Gateway".into(),
            source_title: Some("原始标题".into()),
            cwd: Some("/work/aivc".into()),
            created_at: None,
            updated_at: None,
            model: Some("gpt-5.1-codex".into()),
            model_status: None,
            cli_version: Some("0.154.0".into()),
            file_path: "/x.jsonl".into(),
            file_mtime_ms: None,
            file_change_ms: None,
            file_size: None,
            message_count: 1,
            message_count_is_lower_bound: false,
            preview: vec![Message {
                role: Role::User,
                text: "帮我看看失败转移".into(),
                timestamp: None,
                source_line: None,
            }],
            preview_offset: None,
            usage: None,
            metadata: None,
        };

        // 命中各字段，且大小写不敏感。
        assert!(matches_session_summary(&session, "gateway"));
        assert!(matches_session_summary(&session, "0191f2a3"));
        assert!(matches_session_summary(&session, "aivc"));
        assert!(matches_session_summary(&session, "gpt-5.1"));
        assert!(matches_session_summary(&session, "失败转移"));
        assert!(!matches_session_summary(&session, "不存在的词"));
        // 空查询视为全匹配。
        assert!(matches_session_summary(&session, ""));
    }

    #[test]
    fn matches_all_terms_requires_every_term() {
        let terms = normalize_search_terms("gateway 失败");
        assert!(matches_all_terms("Gateway 失败转移逻辑", &terms));
        assert!(!matches_all_terms("Gateway 正常", &terms));
        // 无词时全匹配。
        assert!(matches_all_terms("anything", &[]));
    }

    #[test]
    fn formats_session_stamp_like_javascript_slice_and_replace() {
        let stamp = crate::time::Timestamp::from_raw("2026-09-16T12:34:56.789Z");
        assert_eq!(format_session_stamp(&stamp), "2026-09-16T12-34-56");
    }

    #[test]
    fn clamp_raw_preserves_internal_whitespace_for_diagnostics() {
        // 诊断文本里的换行有意义，不能被归一。
        let text = "line1\n  line2";
        assert_eq!(clamp_raw(text, 100), text);
        let long = "x".repeat(200);
        assert_eq!(clamp_raw(&long, 160).chars().count(), 159 + 3);
    }
}
