//! 上游响应探测：识别「HTTP 200 但内容其实是错误」的情况。
//!
//! # 为什么必须探测响应体
//!
//! 上游经常用 `200 OK` 返回错误：限流、模型不可用、额度耗尽都可能包在正常状态码里
//! （尤其是 SSE 流的首个事件）。只看状态码会让这些错误被当成成功，既不触发故障转移，
//! 也让用户看到一段无法解析的「回答」。
//!
//! # 探测必须可回放
//!
//! 探测要读走响应开头的字节，但正常响应（尤其是 SSE 首个 token）不能被吃掉。因此
//! 探测只**缓存**读到的字节，判定结束后把它们接回流的开头再转发。
//!
//! # 何时停止探测
//!
//! - 已识别出错误信息；
//! - 已读到事件边界（`\n\n`）——正常 SSE 的第一个事件已完整，再读就是无意义地阻塞
//!   首 token；
//! - 达到 [`RESPONSE_PROBE_MAX_BYTES`] 上限。
//!
//! [`RESPONSE_PROBE_MAX_BYTES`]: aivc_core::constants::RESPONSE_PROBE_MAX_BYTES

use aivc_core::constants::{GATEWAY_FAILOVER_DEFAULT_PATTERNS, RESPONSE_PROBE_MAX_BYTES};
use aivc_core::gateway::FailoverRule;
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use serde_json::Value;

/// 错误信息在日志与响应中的最大长度。
pub const ERROR_MESSAGE_MAX_CHARS: usize = 500;

/// 探测上限。
#[must_use]
pub const fn probe_max_bytes() -> usize {
    RESPONSE_PROBE_MAX_BYTES
}

/// 从响应体提取错误信息。
///
/// 依次尝试：整段文本、每个 SSE `data:` 行（跳过 `[DONE]`）。任一候选能解析出错误
/// 信息就返回。
///
/// 最后一条兜底：若**没有任何候选是合法 JSON**，且整段文本命中内置失败模式，就把整段
/// 文本当作错误信息。这条兜底是为纯文本错误设计的（部分网关直接返回一行英文错误）。
#[must_use]
pub fn extract_gateway_response_error(body: &str) -> Option<String> {
    let candidates = json_candidates(body);
    let mut has_parsed_candidate = false;
    for candidate in &candidates {
        let parsed = serde_json::from_str::<Value>(candidate);
        if parsed.is_ok() {
            has_parsed_candidate = true;
        }
        if let Some(message) = read_gateway_error_message(parsed.ok().as_ref()) {
            return Some(normalize_message(&message));
        }
    }
    let plain_text = collapse_whitespace(body);
    if !has_parsed_candidate && is_builtin_gateway_failover_error(&plain_text) {
        return Some(normalize_message(&plain_text));
    }
    None
}

/// 响应体是否包含**结构化**载荷（至少一个候选是合法 JSON）。
///
/// 用于区分「纯文本错误」与「结构化响应里的字段」：后者交给
/// [`extract_gateway_response_error`] 按字段读取，不应把整段文本当错误。
#[must_use]
pub fn has_structured_gateway_payload(body: &str) -> bool {
    json_candidates(body)
        .iter()
        .any(|candidate| serde_json::from_str::<Value>(candidate).is_ok())
}

/// 从解析后的 JSON 读取错误信息。
///
/// 支持的字段（按优先级）：`message`、`error`（字符串或嵌套对象）、`detail`。
/// 顶层字符串也直接当作错误信息——部分上游只返回一个带引号的字符串。
#[must_use]
pub fn read_gateway_error_message(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Object(object) => {
            if let Some(message) = object.get("message").and_then(Value::as_str) {
                return Some(message.to_owned());
            }
            if let Some(error) = object.get("error") {
                if let Some(text) = error.as_str() {
                    return Some(text.to_owned());
                }
                // 嵌套对象：递归读取。
                if error.is_object() {
                    return read_gateway_error_message(Some(error));
                }
            }
            object
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_owned)
        }
        _ => None,
    }
}

/// 文本是否命中内置失败模式。
#[must_use]
pub fn is_builtin_gateway_failover_error(message: &str) -> bool {
    let normalized = message.to_lowercase();
    GATEWAY_FAILOVER_DEFAULT_PATTERNS
        .iter()
        .any(|pattern| normalized.contains(pattern))
}

/// 判定是否应当触发故障转移。
///
/// 顺序：命中用户规则时触发，否则退回内置判定——
/// 状态码在白名单里，或文本命中内置模式。
#[must_use]
pub fn matches_gateway_failover_error(
    message: Option<&str>,
    provider_id: ProviderId,
    vendor_id: &VendorId,
    status_code: u16,
    rules: &[FailoverRule],
) -> bool {
    if rules.iter().any(|rule| {
        rule.enabled
            && rule.applies_to(provider_id, Some(vendor_id))
            && rule.matches(Some(status_code), message)
    }) {
        return true;
    }
    if aivc_core::constants::GATEWAY_FAILOVER_DEFAULT_STATUS_CODES.contains(&status_code) {
        return true;
    }
    message.is_some_and(is_builtin_gateway_failover_error)
}

/// 构造 JSON 候选列表：整段文本 + 每个 SSE `data:` 行。
fn json_candidates(body: &str) -> Vec<String> {
    let mut candidates = vec![body.trim().to_owned()];
    for line in body.split('\n') {
        let line = line.trim().trim_end_matches('\r').trim();
        let data = line.strip_prefix("data:").map_or(line, str::trim);
        if !data.is_empty() && data != "[DONE]" {
            candidates.push(data.to_owned());
        }
    }
    candidates
}

/// 折叠空白并截断。
///
/// 按**字符**截断而不是字节：按字节截断会把多字节字符切成半个，产生无效 UTF-8。
fn normalize_message(message: &str) -> String {
    collapse_whitespace(message)
        .chars()
        .take(ERROR_MESSAGE_MAX_CHARS)
        .collect()
}

/// 把连续的空白字符折叠成单个空格并去掉首尾空白。
fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::gateway::FailoverRuleScope;
    use aivc_core::time::Timestamp;

    /// 构造一条全局规则。
    ///
    /// 一个 `pattern` 字段承载全部模式（现网就是单字段，多行粘贴后归一化）。
    fn rule(patterns: &[&str], status_codes: &[u16]) -> FailoverRule {
        FailoverRule {
            id: "r1".to_owned(),
            scope: FailoverRuleScope::Global,
            provider_id: None,
            vendor_id: None,
            pattern: patterns.join(" "),
            status_codes: (!status_codes.is_empty()).then(|| status_codes.to_vec()),
            custom_response_status: None,
            custom_response_body: None,
            enabled: true,
            created_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
            updated_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
        }
    }

    #[test]
    fn a_json_error_field_is_extracted() {
        let body = r#"{"error":{"message":"rate limit exceeded","type":"rate_limit"}}"#;
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("rate limit exceeded")
        );
    }

    #[test]
    fn the_supported_field_order_is_respected() {
        // message 优先于 error。
        let body = r#"{"message":"from-message","error":"from-error","detail":"from-detail"}"#;
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("from-message")
        );
        // 没有 message 时用 error。
        let body = r#"{"error":"from-error","detail":"from-detail"}"#;
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("from-error")
        );
        // 只有 detail。
        let body = r#"{"detail":"from-detail"}"#;
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("from-detail")
        );
    }

    #[test]
    fn a_top_level_json_string_is_an_error_message() {
        assert_eq!(
            extract_gateway_response_error(r#""plain error string""#).as_deref(),
            Some("plain error string")
        );
    }

    #[test]
    fn sse_data_lines_are_scanned() {
        // 流式响应：错误藏在某个 data: 行里。
        let body = "event: error\ndata: {\"error\":{\"message\":\"overloaded\"}}\n\n";
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("overloaded")
        );
        // [DONE] 必须被跳过，否则会被当成解析候选。
        let body = "data: {\"error\":\"boom\"}\ndata: [DONE]\n";
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("boom")
        );
    }

    #[test]
    fn a_successful_structured_payload_yields_no_error() {
        let body = r#"{"id":"msg_1","content":[{"type":"text","text":"hello"}]}"#;
        assert!(extract_gateway_response_error(body).is_none());
        assert!(has_structured_gateway_payload(body));
    }

    #[test]
    fn a_plain_text_error_is_recognized_only_when_no_json_was_found() {
        // 纯文本错误命中内置模式 → 当作错误信息。
        let body = "Error: 429 Too Many Requests";
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("Error: 429 Too Many Requests")
        );
        assert!(!has_structured_gateway_payload(body));

        // 纯文本但不命中模式 → 不是错误。
        assert!(extract_gateway_response_error("hello there").is_none());
    }

    #[test]
    fn structured_payloads_never_fall_back_to_plain_text() {
        // 有合法 JSON 候选时不做纯文本兜底：否则正常响应里出现 "rate limit" 字样
        // 就会被误判成错误。
        let body = r#"{"content":"the docs mention rate limit handling"}"#;
        assert!(extract_gateway_response_error(body).is_none());
    }

    #[test]
    fn whitespace_is_collapsed_and_the_message_truncated() {
        let body = "{\"error\":\"line one\\n\\tline two\"}";
        assert_eq!(
            extract_gateway_response_error(body).as_deref(),
            Some("line one line two")
        );

        let long = "字".repeat(600);
        let body = format!("{{\"error\":\"{long}\"}}");
        let message = extract_gateway_response_error(&body).expect("message");
        assert_eq!(message.chars().count(), ERROR_MESSAGE_MAX_CHARS);
    }

    #[test]
    fn builtin_patterns_are_matched_case_insensitively() {
        assert!(is_builtin_gateway_failover_error("429 TOO MANY REQUESTS"));
        assert!(is_builtin_gateway_failover_error("rate limit exceeded"));
        assert!(!is_builtin_gateway_failover_error("all good"));
    }

    #[test]
    fn builtin_status_codes_trigger_failover_without_rules() {
        let vendor = VendorId::from("v1");
        for status in [429u16, 500, 502, 503, 504] {
            assert!(
                matches_gateway_failover_error(None, ProviderId::Codex, &vendor, status, &[]),
                "{status} 应触发故障转移"
            );
        }
        // 正常状态码不触发。
        assert!(!matches_gateway_failover_error(
            None,
            ProviderId::Codex,
            &vendor,
            200,
            &[]
        ));
        assert!(!matches_gateway_failover_error(
            None,
            ProviderId::Codex,
            &vendor,
            400,
            &[]
        ));
    }

    #[test]
    fn a_builtin_message_pattern_triggers_failover_even_on_200() {
        let vendor = VendorId::from("v1");
        assert!(matches_gateway_failover_error(
            Some("429 Too Many Requests"),
            ProviderId::Codex,
            &vendor,
            200,
            &[]
        ));
    }

    #[test]
    fn user_rules_extend_the_builtin_decision() {
        let vendor = VendorId::from("v1");
        // 增加 418 规则不应关闭对 500、429 等默认故障的处理。
        let rules = [rule(&[], &[418])];
        assert!(matches_gateway_failover_error(
            None,
            ProviderId::Codex,
            &vendor,
            500,
            &rules
        ));
        assert!(matches_gateway_failover_error(
            None,
            ProviderId::Codex,
            &vendor,
            418,
            &rules
        ));
    }

    #[test]
    fn disabled_rules_do_not_trigger_failover() {
        let mut disabled = rule(&["custom failure"], &[418]);
        disabled.enabled = false;
        assert!(!matches_gateway_failover_error(
            Some("custom failure"),
            ProviderId::Codex,
            &VendorId::from("v1"),
            418,
            &[disabled]
        ));
    }

    #[test]
    fn a_rule_scoped_to_another_provider_does_not_apply() {
        let vendor = VendorId::from("v1");
        // 作用域必须一起改成 Provider，否则这条规则仍然是全局的。
        let mut scoped = rule(&[], &[418]);
        scoped.scope = FailoverRuleScope::Provider;
        scoped.provider_id = Some(ProviderId::Claude);
        let rules = [scoped];
        // 规则不适用于 Codex → 退回内置判定。
        assert!(matches_gateway_failover_error(
            None,
            ProviderId::Codex,
            &vendor,
            500,
            &rules
        ));
        // 对 Claude 生效：418 触发，内置的 500 仍然触发。
        assert!(matches_gateway_failover_error(
            None,
            ProviderId::Claude,
            &vendor,
            418,
            &rules
        ));
        assert!(matches_gateway_failover_error(
            None,
            ProviderId::Claude,
            &vendor,
            500,
            &rules
        ));
    }

    #[test]
    fn a_rule_can_match_on_message_patterns() {
        let vendor = VendorId::from("v1");
        let rules = [rule(&["insufficient quota"], &[])];
        assert!(matches_gateway_failover_error(
            Some("Insufficient Quota for this model"),
            ProviderId::Codex,
            &vendor,
            200,
            &rules
        ));
        assert!(!matches_gateway_failover_error(
            Some("fine"),
            ProviderId::Codex,
            &vendor,
            200,
            &rules
        ));
    }

    #[test]
    fn the_probe_limit_matches_the_shared_constant() {
        assert_eq!(probe_max_bytes(), RESPONSE_PROBE_MAX_BYTES);
        assert_eq!(probe_max_bytes(), 64 * 1024);
    }
}
