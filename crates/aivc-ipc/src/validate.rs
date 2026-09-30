//! IPC 入参校验。
//!
//! # 为什么这一层不能省
//!
//! IPC 是进程边界，渲染层的任何值都可能到达这里（前端 bug、版本不匹配、手工调用）。
//! 缺了校验，一个 `undefined` 就可能一路穿到文件系统操作或 SQL 参数上。因此每个命令
//! 的每个参数都在这里过一遍，**先校验再执行**。
//!
//! # 文案是契约
//!
//! 全部错误文案与现网逐字一致（含半角冒号、句末句号）。前端已有针对这些文案的断言，
//! 改写文案等于破坏兼容性。因此这里不做任何「统一措辞」。
//!
//! # 哨兵值
//!
//! [`require_message_page_offset`] 保留 `-1` 作为「首屏」哨兵：它不是合法的非负整数，
//! 但语义明确且已经在渲染层广泛使用，改成 `null` 或独立字段会破坏既有调用。

use serde_json::Value;

use aivc_core::error::{IpcError, Result};
use aivc_core::id::{SessionId, VendorId};
use aivc_core::session::{ExportFormat, SessionMutationRef, SessionView};
use aivc_core::target::ProviderId;

/// 自定义会话名长度上限。
pub const CUSTOM_TITLE_MAX_CHARS: usize = 120;
/// 终端提示标题长度上限。
pub const NOTIFICATION_TITLE_MAX_CHARS: usize = 160;
/// 终端提示详情长度上限。
pub const NOTIFICATION_DETAIL_MAX_CHARS: usize = 240;

/// 参数非法。
fn invalid(name: &str) -> aivc_core::Error {
    IpcError::InvalidParameter {
        name: name.to_owned(),
    }
    .into()
}

/// 取值越界（携带完整文案）。
fn invalid_value(message: &str) -> aivc_core::Error {
    IpcError::InvalidValue {
        message: message.to_owned(),
    }
    .into()
}

/// 必填非空字符串。返回**原始值**（不去空白），与现网一致。
pub fn require_string(value: Option<&Value>, name: &str) -> Result<String> {
    let text = value.and_then(Value::as_str).ok_or_else(|| invalid(name))?;
    if text.trim().is_empty() {
        return Err(invalid(name));
    }
    Ok(text.to_owned())
}

/// 可选字符串：空白串视为未提供。
#[must_use]
pub fn optional_string(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// 终端编号长度上限。
pub const TERMINAL_ID_MAX_CHARS: usize = 128;

/// 终端编号。
///
/// # 为什么不只是「非空字符串」
///
/// 终端编号会被**直接用作网关的路由编号**，而路由编号出现在请求路径
/// `/gateway/<平台>/<路由编号>` 的中间段（见 `aivc_gateway::route_prefix`）。
/// 含 `/` 的编号会让路径段错位，网关于是按一个不存在的路由去鉴权——终端能起来、
/// 每次请求都失败，而界面上看不到任何线索。
///
/// 因此编号限制为**单个路径段**：以字母或数字开头，只含 `[A-Za-z0-9._-]`。
/// 这里刻意返回明确错误而不是把非法字符替换掉：替换会让前端记录的编号与真实路由编号
/// 不一致，那是更难查的问题。
pub fn require_terminal_id(value: Option<&Value>) -> Result<String> {
    let id = require_string(value, "terminalId")?;
    if id.chars().count() > TERMINAL_ID_MAX_CHARS {
        return Err(invalid_value("终端编号过长。"));
    }
    let mut chars = id.chars();
    let starts_ok = chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric());
    let rest_ok =
        chars.all(|value| value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.'));
    if !starts_ok || !rest_ok {
        return Err(invalid_value(
            "终端编号只能包含字母、数字、点、下划线与连字符，且必须以字母或数字开头。",
        ));
    }
    Ok(id)
}

/// 自定义会话名：允许空串（表示清除），但不得超长。
pub fn require_custom_title(value: Option<&Value>) -> Result<String> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("title"))?;
    let title = text.trim();
    if title.chars().count() > CUSTOM_TITLE_MAX_CHARS {
        return Err(invalid_value("会话名称不能超过 120 个字符。"));
    }
    Ok(title.to_owned())
}

/// 有限数值。
pub fn require_number(value: Option<&Value>, name: &str) -> Result<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|number| number.is_finite())
        .ok_or_else(|| invalid(name))
}

/// 布尔值。
pub fn require_boolean(value: Option<&Value>, name: &str) -> Result<bool> {
    value.and_then(Value::as_bool).ok_or_else(|| invalid(name))
}

/// 正整数。
pub fn require_positive_integer(value: Option<&Value>, name: &str) -> Result<i64> {
    let number = require_number(value, name)?;
    if number.fract() != 0.0 || number <= 0.0 {
        return Err(invalid(name));
    }
    Ok(number as i64)
}

/// 非负整数。
pub fn require_non_negative_integer(value: Option<&Value>, name: &str) -> Result<i64> {
    let number = require_number(value, name)?;
    if number.fract() != 0.0 || number < 0.0 {
        return Err(invalid(name));
    }
    Ok(number as i64)
}

/// 消息分页偏移。
///
/// `-1` 是「首屏」哨兵，其余必须是非负整数。返回现网的有符号形式，
/// 由调用方转成 [`aivc_core::session::PageCursor`]。
pub fn require_message_page_offset(value: Option<&Value>) -> Result<i64> {
    // 用整数比较而不是浮点相等：-1 是精确可表示的整数，浮点相等比较会触发 lint
    // 且语义上这里是「是不是那个哨兵」而不是「约等于多少」。
    if value.and_then(Value::as_i64) == Some(-1) {
        return Ok(-1);
    }
    require_non_negative_integer(value, "offset")
}

/// 会话视图。
pub fn require_view(value: Option<&Value>) -> Result<SessionView> {
    match value.and_then(Value::as_str) {
        Some("active") => Ok(SessionView::Active),
        Some("trash") => Ok(SessionView::Trash),
        _ => Err(invalid("view")),
    }
}

/// 平台标识。
pub fn require_provider_id(value: Option<&Value>) -> Result<ProviderId> {
    value
        .and_then(Value::as_str)
        .and_then(aivc_crate_parse_provider)
        .ok_or_else(|| invalid("providerId"))
}

/// 解析平台短名。
fn aivc_crate_parse_provider(value: &str) -> Option<ProviderId> {
    match value {
        "codex" => Some(ProviderId::Codex),
        "claude" => Some(ProviderId::Claude),
        "gemini" => Some(ProviderId::Gemini),
        "qoder" => Some(ProviderId::Qoder),
        _ => None,
    }
}

/// 导出格式。
pub fn require_export_format(value: Option<&Value>) -> Result<ExportFormat> {
    match value.and_then(Value::as_str) {
        Some("markdown") => Ok(ExportFormat::Markdown),
        Some("json") => Ok(ExportFormat::Json),
        Some("html") => Ok(ExportFormat::Html),
        _ => Err(invalid("format")),
    }
}

/// 网关端口：`0` 表示自动分配。
pub fn require_gateway_port(value: Option<&Value>) -> Result<u16> {
    // 现网接受字符串形式的数字（设置项来自输入框）。
    let number = match value {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    };
    let Some(number) = number.filter(|value| value.is_finite()) else {
        return Err(invalid_value(
            "网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。",
        ));
    };
    if number.fract() != 0.0 || number < 0.0 || number > 65535.0 {
        return Err(invalid_value(
            "网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。",
        ));
    }
    // 上方已确认落在 0..=65535，截断转换不会丢符号。
    #[allow(clippy::cast_sign_loss)]
    let port = number as u16;
    Ok(port)
}

/// 异常切换阈值：1–10。
pub fn require_gateway_failure_threshold(value: Option<&Value>) -> Result<usize> {
    require_bounded_integer(value, 1, 10, "异常切换阈值必须是 1 到 10 之间的整数。")
}

/// 熔断次数：1–20。
pub fn require_gateway_circuit_failure_threshold(value: Option<&Value>) -> Result<usize> {
    require_bounded_integer(value, 1, 20, "熔断次数必须是 1 到 20 之间的整数。")
}

/// 熔断持续时长（秒）：10–86400。
pub fn require_gateway_circuit_duration_seconds(value: Option<&Value>) -> Result<u64> {
    require_bounded_integer(
        value,
        10,
        86_400,
        "熔断持续时间必须是 10 到 86400 秒之间的整数。",
    )
    .map(|value| value as u64)
}

/// 区间内的整数。
///
/// 上方已确认落在 `[min, max]` 内且 `min >= 0`，因此截断转换不会丢符号。
#[allow(clippy::cast_sign_loss)]
fn require_bounded_integer(
    value: Option<&Value>,
    min: i64,
    max: i64,
    message: &str,
) -> Result<usize> {
    let Some(number) = value.and_then(Value::as_f64) else {
        return Err(invalid_value(message));
    };
    if number.fract() != 0.0 || number < min as f64 || number > max as f64 {
        return Err(invalid_value(message));
    }
    Ok(number as usize)
}

/// 终端数据：允许空串之外的一切字符串。
pub fn require_terminal_data(value: Option<&Value>) -> Result<String> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("terminal data"))?;
    if text.is_empty() {
        return Err(invalid("terminal data"));
    }
    Ok(text.to_owned())
}

/// 终端提示通知。
pub fn require_terminal_attention_notification(
    value: Option<&Value>,
) -> Result<(String, String, Option<String>)> {
    let Some(object) = value.and_then(Value::as_object) else {
        return Err(invalid("terminal attention notification"));
    };
    let tab_key = require_string(object.get("tabKey"), "tabKey")?;
    let title = require_string(object.get("title"), "title")?;
    let detail = match object.get("detail") {
        None | Some(Value::Null) => None,
        Some(value) => Some(require_string(Some(value), "detail")?),
    };
    Ok((
        tab_key,
        title.chars().take(NOTIFICATION_TITLE_MAX_CHARS).collect(),
        detail.map(|value| value.chars().take(NOTIFICATION_DETAIL_MAX_CHARS).collect()),
    ))
}

/// 会话文件引用。
pub fn require_session_file_ref(value: Option<&Value>) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Object(object)) => Ok(object
            .get("filePath")
            .and_then(Value::as_str)
            .map(str::to_owned)),
        _ => Err(invalid("sessionFileRef")),
    }
}

/// 会话变更引用列表。
///
/// 每一项都必须带有效编号；路径提示可选（渲染层列表里已有，但不是可信输入，
/// 使用前仍需围栏校验）。
pub fn require_session_mutation_refs(value: Option<&Value>) -> Result<Vec<SessionMutationRef>> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Err(invalid("sessions"));
    };
    let mut refs = Vec::with_capacity(items.len());
    for item in items {
        let Some(object) = item.as_object() else {
            return Err(invalid("sessions"));
        };
        let id = require_string(object.get("id"), "id")?;
        refs.push(SessionMutationRef {
            id: SessionId::from(id),
            file_path: object
                .get("filePath")
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned),
        });
    }
    Ok(refs)
}

/// 供应商编号。
pub fn require_vendor_id(value: Option<&Value>, name: &str) -> Result<VendorId> {
    require_string(value, name).map(VendorId::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn required_strings_reject_empty_and_whitespace() {
        assert_eq!(
            require_string(Some(&json!("abc")), "targetId").expect("ok"),
            "abc"
        );
        for value in [json!(""), json!("   "), json!(42), json!(null)] {
            let error = require_string(Some(&value), "targetId").expect_err("必须拒绝");
            assert_eq!(error.to_string(), "参数无效：targetId");
        }
        assert!(require_string(None, "targetId").is_err());
    }

    #[test]
    fn required_strings_return_the_original_value_untrimmed() {
        // 现网返回原值而不是 trim 后的值，路径里的前后空白由各自的使用方处理。
        assert_eq!(
            require_string(Some(&json!("  abc  ")), "targetId").expect("ok"),
            "  abc  "
        );
    }

    #[test]
    fn optional_strings_treat_whitespace_as_absent() {
        assert_eq!(optional_string(Some(&json!(" a "))), Some("a".to_owned()));
        assert_eq!(optional_string(Some(&json!("  "))), None);
        assert_eq!(optional_string(Some(&json!(1))), None);
        assert_eq!(optional_string(None), None);
    }

    #[test]
    fn custom_titles_are_trimmed_and_bounded() {
        assert_eq!(
            require_custom_title(Some(&json!("  标题  "))).expect("ok"),
            "标题"
        );
        // 空串表示清除自定义名，是合法的。
        assert_eq!(require_custom_title(Some(&json!(""))).expect("ok"), "");
        let long = "字".repeat(121);
        let error = require_custom_title(Some(&json!(long))).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "会话名称不能超过 120 个字符。");
        // 恰好 120 个字符可以通过。
        let exact = "字".repeat(120);
        assert!(require_custom_title(Some(&json!(exact))).is_ok());
    }

    #[test]
    fn numbers_must_be_finite() {
        assert!((require_number(Some(&json!(1.5)), "x").expect("ok") - 1.5).abs() < f64::EPSILON);
        assert!(require_number(Some(&json!("1.5")), "x").is_err());
        assert!(require_number(Some(&json!(null)), "x").is_err());
    }

    #[test]
    fn positive_and_non_negative_integers_reject_fractions_and_signs() {
        assert_eq!(
            require_positive_integer(Some(&json!(3)), "cols").expect("ok"),
            3
        );
        assert!(require_positive_integer(Some(&json!(0)), "cols").is_err());
        assert!(require_positive_integer(Some(&json!(-1)), "cols").is_err());
        assert!(require_positive_integer(Some(&json!(1.5)), "cols").is_err());

        assert_eq!(
            require_non_negative_integer(Some(&json!(0)), "offset").expect("ok"),
            0
        );
        assert!(require_non_negative_integer(Some(&json!(-1)), "offset").is_err());
    }

    #[test]
    fn the_page_offset_sentinel_survives() {
        // `-1` 是首屏哨兵：它不是合法的非负整数，但语义明确且渲染层广泛使用。
        assert_eq!(
            require_message_page_offset(Some(&json!(-1))).expect("ok"),
            -1
        );
        assert_eq!(require_message_page_offset(Some(&json!(0))).expect("ok"), 0);
        assert_eq!(
            require_message_page_offset(Some(&json!(42))).expect("ok"),
            42
        );
        // 其他负数不是哨兵。
        assert!(require_message_page_offset(Some(&json!(-2))).is_err());
    }

    #[test]
    fn views_and_providers_are_exhaustive() {
        assert_eq!(
            require_view(Some(&json!("active"))).expect("ok"),
            SessionView::Active
        );
        assert_eq!(
            require_view(Some(&json!("trash"))).expect("ok"),
            SessionView::Trash
        );
        assert!(require_view(Some(&json!("all"))).is_err());

        for (name, expected) in [
            ("codex", ProviderId::Codex),
            ("claude", ProviderId::Claude),
            ("gemini", ProviderId::Gemini),
            ("qoder", ProviderId::Qoder),
        ] {
            assert_eq!(
                require_provider_id(Some(&json!(name))).expect("ok"),
                expected
            );
        }
        assert!(require_provider_id(Some(&json!("openai"))).is_err());
    }

    #[test]
    fn export_formats_map_to_the_domain_enum() {
        assert_eq!(
            require_export_format(Some(&json!("markdown"))).expect("ok"),
            ExportFormat::Markdown
        );
        assert_eq!(
            require_export_format(Some(&json!("json"))).expect("ok"),
            ExportFormat::Json
        );
        assert_eq!(
            require_export_format(Some(&json!("html"))).expect("ok"),
            ExportFormat::Html
        );
        let error = require_export_format(Some(&json!("pdf"))).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：format");
    }

    #[test]
    fn gateway_ports_accept_strings_and_both_boundaries() {
        // 设置项来自输入框，因此字符串形式也要接受。
        assert_eq!(require_gateway_port(Some(&json!(8787))).expect("ok"), 8787);
        assert_eq!(
            require_gateway_port(Some(&json!("8787"))).expect("ok"),
            8787
        );
        assert_eq!(require_gateway_port(Some(&json!(0))).expect("ok"), 0);
        assert_eq!(
            require_gateway_port(Some(&json!(65535))).expect("ok"),
            65535
        );
        for bad in [
            json!(65536),
            json!(-1),
            json!(1.5),
            json!("abc"),
            json!(null),
        ] {
            let error = require_gateway_port(Some(&bad)).expect_err("必须拒绝");
            assert_eq!(
                error.to_string(),
                "网关端口必须是 0 到 65535 之间的整数。端口 0 表示自动分配。"
            );
        }
    }

    #[test]
    fn gateway_thresholds_enforce_their_ranges() {
        assert_eq!(
            require_gateway_failure_threshold(Some(&json!(1))).expect("ok"),
            1
        );
        assert_eq!(
            require_gateway_failure_threshold(Some(&json!(10))).expect("ok"),
            10
        );
        for bad in [json!(0), json!(11), json!(1.5)] {
            let error = require_gateway_failure_threshold(Some(&bad)).expect_err("必须拒绝");
            assert_eq!(error.to_string(), "异常切换阈值必须是 1 到 10 之间的整数。");
        }

        assert!(require_gateway_circuit_failure_threshold(Some(&json!(20))).is_ok());
        assert!(require_gateway_circuit_failure_threshold(Some(&json!(21))).is_err());

        assert_eq!(
            require_gateway_circuit_duration_seconds(Some(&json!(10))).expect("ok"),
            10
        );
        assert_eq!(
            require_gateway_circuit_duration_seconds(Some(&json!(86_400))).expect("ok"),
            86_400
        );
        let error =
            require_gateway_circuit_duration_seconds(Some(&json!(9))).expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "熔断持续时间必须是 10 到 86400 秒之间的整数。"
        );
    }

    #[test]
    fn terminal_data_rejects_only_empty() {
        assert_eq!(require_terminal_data(Some(&json!("a"))).expect("ok"), "a");
        // 换行与空白是合法输入（用户按回车）。
        assert!(require_terminal_data(Some(&json!("\n"))).is_ok());
        assert!(require_terminal_data(Some(&json!(""))).is_err());
        assert!(require_terminal_data(None).is_err());
    }

    #[test]
    fn terminal_notifications_require_the_two_key_fields() {
        let value = json!({ "tabKey": "k", "title": "t", "detail": "d" });
        let (key, title, detail) =
            require_terminal_attention_notification(Some(&value)).expect("ok");
        assert_eq!(
            (key.as_str(), title.as_str(), detail.as_deref()),
            ("k", "t", Some("d"))
        );

        // detail 可省。
        let value = json!({ "tabKey": "k", "title": "t" });
        let (_, _, detail) = require_terminal_attention_notification(Some(&value)).expect("ok");
        assert!(detail.is_none());

        // 缺 tabKey 或 title 一律拒绝。
        assert!(require_terminal_attention_notification(Some(&json!({ "title": "t" }))).is_err());
        assert!(require_terminal_attention_notification(Some(&json!({ "tabKey": "k" }))).is_err());
    }

    #[test]
    fn terminal_notifications_are_truncated() {
        let long_title = "字".repeat(200);
        let long_detail = "字".repeat(300);
        let value = json!({ "tabKey": "k", "title": long_title, "detail": long_detail });
        let (_, title, detail) = require_terminal_attention_notification(Some(&value)).expect("ok");
        assert_eq!(title.chars().count(), NOTIFICATION_TITLE_MAX_CHARS);
        assert_eq!(
            detail.map(|value| value.chars().count()),
            Some(NOTIFICATION_DETAIL_MAX_CHARS)
        );
    }

    #[test]
    fn session_mutation_refs_require_ids_and_accept_optional_paths() {
        let value = json!([
            { "id": "a", "filePath": "/x/a.jsonl" },
            { "id": "b" },
        ]);
        let refs = require_session_mutation_refs(Some(&value)).expect("ok");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].id.as_str(), "a");
        assert_eq!(refs[0].file_path.as_deref(), Some("/x/a.jsonl"));
        assert!(refs[1].file_path.is_none());

        // 空白路径视为未提供，而不是一个空路径。
        let value = json!([{ "id": "a", "filePath": "   " }]);
        let refs = require_session_mutation_refs(Some(&value)).expect("ok");
        assert!(refs[0].file_path.is_none());

        // 缺编号或结构不对一律拒绝。
        assert!(require_session_mutation_refs(Some(&json!([{ "filePath": "/x" }]))).is_err());
        assert!(require_session_mutation_refs(Some(&json!("not-an-array"))).is_err());
        assert!(require_session_mutation_refs(None).is_err());
        // 空数组是合法的（批量操作收到空列表）。
        assert!(
            require_session_mutation_refs(Some(&json!([])))
                .expect("ok")
                .is_empty()
        );
    }

    #[test]
    fn session_file_refs_accept_both_shapes() {
        // 渲染层可能传字符串路径、`{filePath}` 对象或不传。
        assert_eq!(
            require_session_file_ref(Some(&json!("/x/a.jsonl"))).expect("ok"),
            Some("/x/a.jsonl".to_owned())
        );
        assert_eq!(
            require_session_file_ref(Some(&json!({ "filePath": "/x/a.jsonl" }))).expect("ok"),
            Some("/x/a.jsonl".to_owned())
        );
        assert_eq!(require_session_file_ref(None).expect("ok"), None);
        assert_eq!(
            require_session_file_ref(Some(&json!(null))).expect("ok"),
            None
        );
        // 对象里没有 filePath 时视为未提供。
        assert_eq!(
            require_session_file_ref(Some(&json!({}))).expect("ok"),
            None
        );
        assert!(require_session_file_ref(Some(&json!(42))).is_err());
    }

    #[test]
    fn vendor_ids_reuse_the_string_rules() {
        assert_eq!(
            require_vendor_id(Some(&json!("v1")), "vendorId")
                .expect("ok")
                .as_str(),
            "v1"
        );
        assert!(require_vendor_id(Some(&json!("  ")), "vendorId").is_err());
    }
}
