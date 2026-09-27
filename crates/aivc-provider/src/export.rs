//! 会话导出：Markdown / JSON / HTML（各平台共用）。
//!
//! 导出只依赖 [Session]（平台无关的会话模型），因此放在 crate 根而不是某个平台
//! 目录下：三个平台的导出格式完全一致，差异只在于「怎么拿到 [Session]」。
//!
//! # 导出前的净化
//!
//! 导出内容会离开本应用，因此先过滤掉两类**内部标记消息**：
//!
//! - `<turn_aborted>`：用户中断留下的占位；
//! - `<permissions instructions>`：注入的权限说明。
//!
//! 它们在界面上有意义（提示发生了什么），但写进导出文件只是噪声。
//!
//! # 数字格式
//!
//! Token 数量按 `en-US` 千分位呈现（现网用 `toLocaleString("en-US")`），
//! 因此 12345 输出 `12,345`。这与界面语言无关——导出文件是给人读的，
//! 千分位比一串裸数字好读得多。

use aivc_core::session::{ExportFormat, Role, Session, TokenUsage};

/// 导出时需要隐藏的内部标记。
const HIDDEN_MARKERS: [&str; 2] = ["<turn_aborted>", "<permissions instructions>"];

/// 按导出规则净化会话。
///
/// 只保留可见消息，并把 `message_count` 改成过滤后的条数——否则导出文件会声称
/// 有 N 条消息却只列出 M 条。
#[must_use]
pub fn sanitize_for_export(session: &Session) -> Session {
    let preview: Vec<_> = session
        .preview
        .iter()
        .filter(|message| !is_hidden_message(&message.text))
        .cloned()
        .collect();
    Session {
        message_count: preview.len() as u64,
        // 导出文件里**列出的是全部内容**，所以这个计数是精确的——即使源会话来自列表路径
        // （那时它是下界），导出之后也不再是。必须显式清掉，不能靠 `..session` 继承。
        message_count_is_lower_bound: false,
        preview,
        ..session.clone()
    }
}

/// 是否是应当从导出中隐藏的内部标记消息。
#[must_use]
pub fn is_hidden_message(text: &str) -> bool {
    HIDDEN_MARKERS.iter().any(|marker| text.contains(marker))
}

/// 按格式渲染导出内容。
#[must_use]
pub fn format_session(session: &Session, format: ExportFormat) -> String {
    match format {
        ExportFormat::Json => format_json(session),
        ExportFormat::Html => format_html(session),
        ExportFormat::Markdown => format_markdown(session),
    }
}

/// 渲染为 JSON（2 空格缩进 + 结尾换行）。
#[must_use]
pub fn format_json(session: &Session) -> String {
    // 序列化失败在结构固定时不会发生；真发生也要给出可用的文本而不是 panic。
    serde_json::to_string_pretty(session)
        .map_or_else(|_| "{}\n".to_owned(), |text| format!("{text}\n"))
}

/// 渲染为 Markdown。
#[must_use]
pub fn format_markdown(session: &Session) -> String {
    let mut lines = vec![
        format!("# {}", escape_markdown(&session.title)),
        String::new(),
        format!("- 会话编号：`{}`", session.id.as_str()),
        format!(
            "- 创建时间：{}",
            timestamp_or_dash(session.created_at.as_ref())
        ),
        format!(
            "- 更新时间：{}",
            timestamp_or_dash(session.updated_at.as_ref())
        ),
        format!("- 工作目录：{}", session.cwd.as_deref().unwrap_or("-")),
        format!("- 模型：{}", format_model_status(session)),
        format!(
            "- CLI 版本：{}",
            session.cli_version.as_deref().unwrap_or("-")
        ),
        format!("- Token：{}", format_token_usage(session)),
        format!("- 上下文：{}", format_context_usage(session)),
    ];

    if let Some(branch) = session
        .metadata
        .as_ref()
        .and_then(|meta| meta.branch.as_ref())
        && let Some(parent) = branch.parent_session_id.as_ref()
    {
        lines.push(format!("- 来源会话：`{}`", parent.as_str()));
        if let Some(index) = branch.parent_message_index {
            lines.push(format!("- 来源消息位置：{index}"));
        }
        if let Some(line) = branch.parent_message_line {
            lines.push(format!("- 来源 JSONL 行：{line}"));
        }
    }

    lines.push(String::new());
    lines.push("## 对话".to_owned());
    lines.push(String::new());
    for message in &session.preview {
        lines.push(format!("### {}", role_label(message.role)));
        if let Some(timestamp) = message.timestamp.as_ref() {
            lines.push(format!("_时间：{}_", timestamp.as_str()));
            lines.push(String::new());
        }
        let text = message.text.trim();
        lines.push(if text.is_empty() {
            "(空)".to_owned()
        } else {
            text.to_owned()
        });
        lines.push(String::new());
    }

    format!("{}\n", lines.join("\n"))
}

/// 渲染为自带样式的单文件 HTML。
#[must_use]
pub fn format_html(session: &Session) -> String {
    let branch = session
        .metadata
        .as_ref()
        .and_then(|meta| meta.branch.as_ref());
    let metadata_rows = [
        ("会话编号", session.id.as_str().to_owned()),
        ("创建时间", timestamp_or_dash(session.created_at.as_ref())),
        ("更新时间", timestamp_or_dash(session.updated_at.as_ref())),
        (
            "工作目录",
            session.cwd.clone().unwrap_or_else(|| "-".to_owned()),
        ),
        ("模型", format_model_status(session)),
        (
            "CLI 版本",
            session
                .cli_version
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
        ),
        ("Token", format_token_usage(session)),
        ("上下文", format_context_usage(session)),
        (
            "来源会话",
            branch
                .and_then(|item| item.parent_session_id.as_ref())
                .map_or_else(|| "-".to_owned(), |value| value.as_str().to_owned()),
        ),
        (
            "来源消息位置",
            branch
                .and_then(|item| item.parent_message_index)
                .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        ),
        (
            "来源 JSONL 行",
            branch
                .and_then(|item| item.parent_message_line)
                .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        ),
    ];

    let rows: String = metadata_rows
        .iter()
        .map(|(key, value)| {
            format!(
                "      <tr><th>{}</th><td>{}</td></tr>",
                escape_html(key),
                escape_html(value)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let messages: String = session
        .preview
        .iter()
        .map(format_html_message)
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <title>{title}</title>
  <style>
    body {{ margin: 32px; color: #1f2937; font: 14px/1.6 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
    h1 {{ font-size: 24px; }}
    table {{ border-collapse: collapse; margin: 16px 0 28px; width: 100%; }}
    th, td {{ border: 1px solid #d1d5db; padding: 8px 10px; text-align: left; vertical-align: top; }}
    th {{ width: 140px; background: #f3f4f6; }}
    article {{ border: 1px solid #d1d5db; border-left: 4px solid #6b7280; border-radius: 8px; margin: 12px 0; padding: 12px 14px; }}
    article.user {{ border-left-color: #3268d5; }}
    article.assistant {{ border-left-color: #2b8a66; }}
    article header {{ color: #6b7280; font-size: 12px; margin-bottom: 8px; }}
    pre {{ white-space: pre-wrap; word-break: break-word; font: inherit; margin: 0; }}
  </style>
</head>
<body>
  <h1>{title}</h1>
  <table>
    <tbody>
{rows}
    </tbody>
  </table>
  <h2>对话</h2>
{messages}
</body>
</html>
"#,
        title = escape_html(&session.title),
    )
}

/// 渲染单条 HTML 消息。
#[must_use]
pub fn format_html_message(message: &aivc_core::session::Message) -> String {
    let timestamp = message
        .timestamp
        .as_ref()
        .map_or_else(String::new, |value| {
            format!(" · {}", escape_html(value.as_str()))
        });
    format!(
        "  <article class=\"{}\">\n    <header>{}{}</header>\n    <pre>{}</pre>\n  </article>",
        escape_html(role_class(message.role)),
        escape_html(&role_label(message.role)),
        timestamp,
        escape_html(&message.text)
    )
}

/// 角色对应的展示名。
#[must_use]
pub fn role_label(role: Role) -> String {
    match role {
        Role::User => "用户",
        Role::Assistant => "助手",
        Role::System => "系统",
        Role::Tool => "工具",
        Role::Unknown => "未知",
    }
    .to_owned()
}

/// 角色对应的 CSS 类名。
fn role_class(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::System => "system",
        Role::Tool => "tool",
        Role::Unknown => "unknown",
    }
}

/// 时间戳或占位符。
fn timestamp_or_dash(value: Option<&aivc_core::time::Timestamp>) -> String {
    value.map_or_else(|| "-".to_owned(), |item| item.as_str().to_owned())
}

/// Token 用量摘要。
#[must_use]
pub fn format_token_usage(session: &Session) -> String {
    let Some(total) = session
        .usage
        .as_ref()
        .and_then(|usage| usage.total.as_ref())
    else {
        return "-".to_owned();
    };
    if total.total_tokens.unwrap_or(0) == 0 {
        return "-".to_owned();
    }
    [
        format!("总计 {}", format_number(total.total_tokens)),
        format!("输入 {}", format_number(total.input_tokens)),
        format!("缓存输入 {}", format_number(total.cached_input_tokens)),
        format!("输出 {}", format_number(total.output_tokens)),
        format!("推理输出 {}", format_number(total.reasoning_output_tokens)),
    ]
    .join(" / ")
}

/// 上下文占用摘要。
#[must_use]
pub fn format_context_usage(session: &Session) -> String {
    let Some(usage) = session.usage.as_ref() else {
        return "-".to_owned();
    };
    let Some(left) = usage.context_left_percent else {
        return "-".to_owned();
    };
    format!(
        "{}% left ({} used / {})",
        format_percent(left),
        format_number(usage.context_used_tokens),
        format_number(usage.context_window)
    )
}

/// 模型状态摘要。
#[must_use]
pub fn format_model_status(session: &Session) -> String {
    let status = session.model_status.as_ref();
    let model = status
        .and_then(|item| item.model.clone())
        .or_else(|| session.model.clone());
    let Some(model) = model else {
        return "-".to_owned();
    };

    let mut details = Vec::new();
    if let Some(reasoning) = status.and_then(|item| item.reasoning.clone()) {
        details.push(format!("reasoning {reasoning}"));
    }
    if let Some(summaries) = status.and_then(|item| item.summaries.clone()) {
        details.push(format!("summaries {summaries}"));
    }
    let label = if details.is_empty() {
        model
    } else {
        format!("{model} ({})", details.join(", "))
    };
    match status.and_then(|item| item.model_provider.clone()) {
        Some(provider) => format!("{label} / {provider}"),
        None => label,
    }
}

/// 按 `en-US` 规则加千分位。
#[must_use]
pub fn format_number(value: Option<u64>) -> String {
    let Some(value) = value else {
        return "-".to_owned();
    };
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// 百分比：整数不带小数位，其余保留一位。
fn format_percent(value: f64) -> String {
    if (value.fract()).abs() < f64::EPSILON {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// HTML 转义。
#[must_use]
pub fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// Markdown 转义：给所有有语义的符号加反斜杠。
#[must_use]
pub fn escape_markdown(value: &str) -> String {
    const SPECIAL: &[char] = &[
        '\\', '`', '*', '_', '{', '}', '[', ']', '(', ')', '#', '+', '-', '.', '!', '|', '>',
    ];
    let mut out = String::with_capacity(value.len() * 2);
    for ch in value.chars() {
        if SPECIAL.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// 供调用方判断是否需要非空的 Token 用量。
#[must_use]
pub fn has_token_usage(usage: &Option<TokenUsage>) -> bool {
    usage
        .as_ref()
        .and_then(|item| item.total_tokens)
        .is_some_and(|value| value > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::SessionId;
    use aivc_core::session::{Message, RateLimits, SessionUsage, UsageSource};
    use aivc_core::time::Timestamp;

    fn message(role: Role, text: &str) -> Message {
        Message {
            role,
            text: text.to_owned(),
            timestamp: Some(Timestamp::from_raw("2026-09-16T12:00:01.000Z")),
            source_line: Some(2),
        }
    }

    fn session() -> Session {
        Session {
            id: SessionId::from("0191f2a3-0000-7000-8000-00000000000a"),
            title: "标题 *带* 符号".to_owned(),
            source_title: None,
            cwd: Some("/work".to_owned()),
            created_at: Some(Timestamp::from_raw("2026-09-16T12:00:00.000Z")),
            updated_at: Some(Timestamp::from_raw("2026-09-16T12:05:00.000Z")),
            model: Some("gpt-5".to_owned()),
            model_status: None,
            cli_version: Some("0.154.0".to_owned()),
            file_path: "/a/rollout-x.jsonl".to_owned(),
            file_mtime_ms: Some(1.0),
            file_change_ms: Some(1.0),
            file_size: Some(10),
            message_count: 2,
            message_count_is_lower_bound: false,
            preview: vec![
                message(Role::User, "你好"),
                message(Role::Assistant, "你好！"),
            ],
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    fn session_with_usage() -> Session {
        Session {
            usage: Some(SessionUsage {
                total: Some(TokenUsage {
                    input_tokens: Some(1_234),
                    cached_input_tokens: Some(50),
                    output_tokens: Some(600),
                    reasoning_output_tokens: Some(20),
                    total_tokens: Some(12_345),
                }),
                last: None,
                context_window: Some(100_000),
                context_used_tokens: Some(2_500),
                context_percent: Some(2.5),
                context_left_percent: Some(97.5),
                rate_limits: Some(RateLimits {
                    primary: None,
                    secondary: None,
                }),
                updated_at: None,
                source: UsageSource::CodexTokenCount,
            }),
            ..session()
        }
    }

    #[test]
    fn sanitizing_drops_internal_marker_messages_and_fixes_the_count() {
        let mut source = session();
        source.preview.push(message(Role::User, "<turn_aborted>"));
        source
            .preview
            .push(message(Role::System, "<permissions instructions>"));
        source.message_count = 4;

        let cleaned = sanitize_for_export(&source);
        assert_eq!(cleaned.preview.len(), 2);
        // 数量必须跟着改，否则导出文件会声称 4 条却只列 2 条。
        assert_eq!(cleaned.message_count, 2);
    }

    #[test]
    fn marker_detection_is_substring_based() {
        assert!(is_hidden_message("前缀 <turn_aborted> 后缀"));
        assert!(is_hidden_message("<permissions instructions>"));
        assert!(!is_hidden_message("正常内容"));
    }

    #[test]
    fn markdown_has_a_heading_and_metadata_list() {
        let text = format_markdown(&session());
        assert!(text.starts_with("# 标题"));
        assert!(text.contains("- 会话编号：`0191f2a3-0000-7000-8000-00000000000a`"));
        assert!(text.contains("- 工作目录：/work"));
        assert!(text.contains("- 模型：gpt-5"));
        assert!(text.contains("## 对话"));
        assert!(text.contains("### 用户"));
        assert!(text.contains("### 助手"));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn markdown_escapes_the_title() {
        let text = format_markdown(&session());
        // 标题里的 `*` 必须转义，否则会被渲染成斜体。
        assert!(text.starts_with("# 标题 \\*带\\* 符号"));
    }

    #[test]
    fn markdown_uses_dashes_for_missing_fields() {
        let mut source = session();
        source.cwd = None;
        source.cli_version = None;
        source.usage = None;
        let text = format_markdown(&source);
        assert!(text.contains("- 工作目录：-"));
        assert!(text.contains("- CLI 版本：-"));
        assert!(text.contains("- Token：-"));
        assert!(text.contains("- 上下文：-"));
    }

    #[test]
    fn markdown_shows_branch_provenance_when_present() {
        let mut source = session();
        source.metadata = Some(aivc_core::session::SessionMetadata {
            branch: Some(aivc_core::session::BranchMetadata {
                parent_session_id: Some(SessionId::from("parent-id")),
                parent_message_index: Some(3),
                parent_message_line: Some(7),
                ..Default::default()
            }),
            ..Default::default()
        });
        let text = format_markdown(&source);
        assert!(text.contains("- 来源会话：`parent-id`"));
        assert!(text.contains("- 来源消息位置：3"));
        assert!(text.contains("- 来源 JSONL 行：7"));
    }

    #[test]
    fn markdown_renders_empty_message_text_as_a_placeholder() {
        let mut source = session();
        source.preview = vec![message(Role::User, "   ")];
        let text = format_markdown(&source);
        assert!(text.contains("(空)"));
    }

    #[test]
    fn html_escapes_user_content() {
        let mut source = session();
        source.title = "<script>alert(1)</script>".to_owned();
        source.preview = vec![message(Role::User, "a & b < c > d \" e")];
        let text = format_html(&source);
        // 标题与正文都必须转义，否则导出的 HTML 会执行注入的脚本。
        assert!(!text.contains("<script>alert(1)</script>"));
        assert!(text.contains("&lt;script&gt;"));
        assert!(text.contains("a &amp; b &lt; c &gt; d &quot; e"));
    }

    #[test]
    fn html_has_metadata_rows_and_message_articles() {
        let text = format_html(&session());
        assert!(text.starts_with("<!doctype html>"));
        assert!(text.contains("<th>会话编号</th>"));
        assert!(text.contains("class=\"user\""));
        assert!(text.contains("class=\"assistant\""));
        assert!(text.contains("</html>"));
    }

    #[test]
    fn json_is_pretty_printed_and_newline_terminated() {
        let text = format_json(&session());
        assert!(text.ends_with('\n'));
        assert!(text.contains("\n  \"id\""), "必须是 2 空格缩进");
        let parsed: serde_json::Value = serde_json::from_str(&text).expect("合法 JSON");
        assert_eq!(parsed["title"], "标题 *带* 符号");
    }

    #[test]
    fn numbers_use_english_thousands_separators() {
        assert_eq!(format_number(None), "-");
        assert_eq!(format_number(Some(0)), "0");
        assert_eq!(format_number(Some(999)), "999");
        assert_eq!(format_number(Some(1_000)), "1,000");
        assert_eq!(format_number(Some(12_345)), "12,345");
        assert_eq!(format_number(Some(1_234_567)), "1,234,567");
    }

    #[test]
    fn token_usage_summarizes_every_dimension() {
        let text = format_token_usage(&session_with_usage());
        assert!(text.contains("总计 12,345"));
        assert!(text.contains("输入 1,234"));
        assert!(text.contains("缓存输入 50"));
        assert!(text.contains("输出 600"));
        assert!(text.contains("推理输出 20"));
    }

    #[test]
    fn token_usage_with_zero_total_is_a_dash() {
        let mut source = session_with_usage();
        if let Some(usage) = source.usage.as_mut()
            && let Some(total) = usage.total.as_mut()
        {
            total.total_tokens = Some(0);
        }
        assert_eq!(format_token_usage(&source), "-");
        assert!(!has_token_usage(
            &source
                .usage
                .as_ref()
                .map(|item| item.total.unwrap_or_default())
        ));
    }

    #[test]
    fn context_usage_keeps_one_decimal_when_needed() {
        let text = format_context_usage(&session_with_usage());
        assert_eq!(text, "97.5% left (2,500 used / 100,000)");
    }

    #[test]
    fn model_status_appends_details_and_provider() {
        let mut source = session();
        source.model_status = Some(aivc_core::session::ModelStatus {
            model: Some("gpt-5.1".to_owned()),
            reasoning: Some("high".to_owned()),
            summaries: Some("auto".to_owned()),
            model_provider: Some("openai".to_owned()),
        });
        assert_eq!(
            format_model_status(&source),
            "gpt-5.1 (reasoning high, summaries auto) / openai"
        );
    }

    #[test]
    fn model_status_without_a_model_is_a_dash() {
        let mut source = session();
        source.model = None;
        assert_eq!(format_model_status(&source), "-");
    }

    #[test]
    fn markdown_escape_covers_the_special_characters() {
        assert_eq!(escape_markdown("a*b_c"), "a\\*b\\_c");
        assert_eq!(escape_markdown("#标题"), "\\#标题");
        assert_eq!(escape_markdown("正常中文"), "正常中文");
    }
}
