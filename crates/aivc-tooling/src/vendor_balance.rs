//! 供应商余额查询：JSON 路径取值、两种内置模板、倍率换算。
//!
//! # 为什么有「两种模板」
//!
//! 中转站的余额接口没有统一约定，但实际只有两大类形状：
//!
//! - **通用**：字段名叫 `remaining` / `balance` / `available` 之类，各家拼写不同，
//!   因此按**候选路径列表**逐个找，找到第一个能解析成数字的就用。
//! - **New API**：`quota` 字段表示**剩余额度**，单位是 1/500000 美元。
//!   它不能直接当总额度——把 `quota` 当 `total` 会让「已用」变成负数或 0。
//!
//! # 为什么数值要接受字符串
//!
//! 有些中转站把数字序列化成字符串（`"12.5"`）。只认 JSON number 会让这类站点
//! 全部显示「无法解析」，而它们其实是能用的。但**空串与不可解析的串必须拒绝**——
//! 把 `""` 当成 0 会显示「余额 0」，那是比「查不到」严重得多的误导。
//!
//! # 布尔值的字符串形式
//!
//! `"ok"` / `"active"` / `"yes"` 这类都要认成 `true`。这是实际接口里出现过的写法，
//! 只认 `true`/`false` 会让「账号有效」被判成无效。

use aivc_core::target::ProviderId;
use serde_json::Value;

/// 余额查询超时（15 秒）。
pub const BALANCE_TIMEOUT_MS: u64 = 15_000;
/// 批量查询的并发上限。
///
/// 限制并发的原因不是性能而是**礼貌**：一次刷新十几个供应商时，全并发会给中转站
/// 造成瞬时压力，容易被判成滥用。
pub const BALANCE_BATCH_CONCURRENCY: usize = 3;
/// New API 的额度单位（1 单位 = 1/500000 美元）。
pub const NEW_API_QUOTA_UNIT: f64 = 500_000.0;

/// 视为「有效」的字符串。
const TRUE_WORDS: [&str; 5] = ["true", "1", "yes", "ok", "active"];
/// 视为「无效」的字符串。
const FALSE_WORDS: [&str; 5] = ["false", "0", "no", "disabled", "inactive"];
/// 视为「已失效」的状态值。
const INVALID_STATUSES: [&str; 3] = ["expired", "quota_exhausted", "disabled"];

/// 是否是 JSON 对象（**不含数组**）。
///
/// 数组也是 `object`，但按路径取值时数组没有意义。不做这个区分会让
/// `["a"]` 被当成记录，随后取任意键都得到 `undefined`。
#[must_use]
pub fn is_record(value: &Value) -> bool {
    value.is_object()
}

/// 按点分路径取值。
///
/// 路径中任何一段落在非对象上就返回 [`None`]——不做报错，因为候选路径列表里
/// 本来就有大量「这条不存在」的情况。
#[must_use]
pub fn read_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for key in path.split('.') {
        let object = current.as_object()?;
        current = object.get(key)?;
    }
    Some(current)
}

/// 按路径取数字。
///
/// 接受 JSON number，也接受**能解析成有限数字的非空字符串**。
#[must_use]
pub fn read_json_number(root: &Value, path: &str) -> Option<f64> {
    let value = read_path(root, path)?;
    number_from(value)
}

/// 从任意值里取出数字。
fn number_from(value: &Value) -> Option<f64> {
    if let Some(number) = value.as_f64() {
        // JSON 里没有 NaN/Infinity，但 `as_f64` 对超大整数可能给出 inf。
        return number.is_finite().then_some(number);
    }
    let text = value.as_str()?.trim();
    if text.is_empty() {
        // 空串不是 0，是「没有值」。
        return None;
    }
    text.parse::<f64>().ok().filter(|number| number.is_finite())
}

/// 按路径取字符串（去空白后非空才算）。
#[must_use]
pub fn read_json_string(root: &Value, path: &str) -> Option<String> {
    let text = read_path(root, path)?.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// 按路径取布尔值。
///
/// 接受 `true`/`false`、`0`/`1`、以及实际接口里出现过的字符串写法。
#[must_use]
pub fn read_json_boolean(root: &Value, path: &str) -> Option<bool> {
    let value = read_path(root, path)?;
    boolean_from(value)
}

/// 从任意值里取出布尔。
fn boolean_from(value: &Value) -> Option<bool> {
    if let Some(flag) = value.as_bool() {
        return Some(flag);
    }
    if let Some(number) = value.as_i64() {
        // 只认 0 与 1：其他数字不是布尔语义，猜错比返回 None 更糟。
        return match number {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
    }
    let text = value.as_str()?.trim().to_ascii_lowercase();
    if TRUE_WORDS.contains(&text.as_str()) {
        return Some(true);
    }
    if FALSE_WORDS.contains(&text.as_str()) {
        return Some(false);
    }
    None
}

/// 按候选路径列表找第一个数字。
#[must_use]
pub fn first_number(root: &Value, paths: &[&str]) -> Option<f64> {
    paths
        .iter()
        .find_map(|path| read_path(root, path).and_then(number_from))
}

/// 按候选路径列表找第一个非空字符串。
#[must_use]
pub fn first_string(root: &Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| read_json_string(root, path))
}

/// 按候选路径列表找第一个布尔值。
#[must_use]
pub fn first_boolean(root: &Value, paths: &[&str]) -> Option<bool> {
    paths
        .iter()
        .find_map(|path| read_path(root, path).and_then(boolean_from))
}

/// 余额快照。
#[derive(Debug, Clone, PartialEq)]
pub struct BalanceSnapshot {
    /// 剩余。
    pub remaining: Option<f64>,
    /// 总额。
    pub total: Option<f64>,
    /// 已用。
    pub used: Option<f64>,
    /// 单位。
    pub unit: String,
    /// 套餐名。
    pub plan_name: Option<String>,
    /// 是否有效。
    pub is_valid: bool,
}

/// 通用模板的字段候选。
///
/// 路径顺序即优先级：越靠前越可能是该站点的真实字段。
mod generic {
    /// 是否成功。
    pub const SUCCESS: [&str; 2] = ["success", "data.success"];
    /// 是否有效。
    pub const ACTIVE: [&str; 8] = [
        "is_active",
        "active",
        "data.is_active",
        "data.active",
        "isValid",
        "is_valid",
        "data.isValid",
        "data.is_valid",
    ];
    /// 状态文案。
    pub const STATUS: [&str; 2] = ["status", "data.status"];
    /// 剩余（**必需**：没有它就无法确定这是余额响应）。
    pub const REMAINING: [&str; 9] = [
        "remaining",
        "balance",
        "available",
        "quota.remaining",
        "data.remaining",
        "data.balance",
        "data.available",
        "data.quota.remaining",
        "credits.balance",
    ];
    /// 总额。
    pub const TOTAL: [&str; 5] = [
        "total",
        "quota.total",
        "data.total",
        "data.quota.total",
        "credits.total",
    ];
    /// 已用。
    pub const USED: [&str; 5] = [
        "used",
        "quota.used",
        "data.used",
        "data.quota.used",
        "credits.used",
    ];
    /// 单位。
    pub const UNIT: [&str; 5] = [
        "unit",
        "currency",
        "quota.unit",
        "data.unit",
        "data.currency",
    ];
    /// 套餐名。
    pub const PLAN: [&str; 4] = ["plan", "plan_name", "data.plan", "data.plan_name"];
}

/// 解析通用模板的余额响应。
///
/// 返回 [`None`] 表示「这不是一个余额响应」——判定依据是**取不到剩余额度**。
/// 没有这个门槛，任何 JSON（比如一个模型列表）都会被当成余额 `0`。
#[must_use]
pub fn parse_generic_balance(body: &Value) -> Option<BalanceSnapshot> {
    let root = if is_record(body) {
        body
    } else {
        // 非对象按空对象处理：所有候选路径都取不到，最终返回 None。
        return None;
    };
    let success = first_boolean(root, &generic::SUCCESS);
    let active = first_boolean(root, &generic::ACTIVE);
    let status = first_string(root, &generic::STATUS).unwrap_or_default();
    let remaining = first_number(root, &generic::REMAINING)?;
    Some(BalanceSnapshot {
        remaining: Some(remaining),
        total: first_number(root, &generic::TOTAL),
        used: first_number(root, &generic::USED),
        unit: first_string(root, &generic::UNIT).unwrap_or_else(|| "USD".to_owned()),
        plan_name: first_string(root, &generic::PLAN),
        // 三重判定：`success !== false && active !== false && 状态不在失效集合里`。
        is_valid: success != Some(false)
            && active != Some(false)
            && !INVALID_STATUSES.contains(&status.to_ascii_lowercase().as_str()),
    })
}

/// 解析 New API 模板的余额响应。
///
/// # `quota` 是**剩余**而不是总额
///
/// 把 `quota` 当 `total` 会让「已用」变成 0 或负数，界面上看起来像「从未用过」。
/// 正确做法是：`remaining = quota / 单位`、`used = used_quota / 单位`、
/// `total = remaining + used`。
#[must_use]
pub fn parse_new_api_balance(body: &Value) -> Option<BalanceSnapshot> {
    if !is_record(body) {
        return None;
    }
    let success = first_boolean(body, &["success", "data.success"]);
    let quota = first_number(body, &["data.quota", "quota"])?;
    let used_quota = first_number(body, &["data.used_quota", "used_quota"]).unwrap_or(0.0);
    let remaining = quota / NEW_API_QUOTA_UNIT;
    let used = used_quota / NEW_API_QUOTA_UNIT;
    Some(BalanceSnapshot {
        total: Some(remaining + used),
        used: Some(used),
        remaining: Some(remaining),
        unit: "额度".to_owned(),
        plan_name: first_string(body, &["data.group", "data.plan", "group", "plan"]),
        is_valid: success != Some(false),
    })
}

/// 应用倍率。
///
/// 只在三个数值字段上乘：单位与套餐名不是数量。
/// 倍率为 0 或 1 时原样返回——0 是「未配置」，不是「全部归零」。
#[must_use]
pub fn apply_balance_multiplier(
    snapshot: BalanceSnapshot,
    multiplier: Option<f64>,
) -> BalanceSnapshot {
    let Some(multiplier) = multiplier else {
        return snapshot;
    };
    // 0 是「未配置」，1 是「不缩放」；两者都原样返回。
    // 用容差比较而不是 `== 1.0`：倍率来自设置文件，可能带浮点误差。
    if !multiplier.is_finite()
        || multiplier.abs() < f64::EPSILON
        || (multiplier - 1.0).abs() < f64::EPSILON
    {
        return snapshot;
    }
    BalanceSnapshot {
        remaining: snapshot.remaining.map(|value| value * multiplier),
        total: snapshot.total.map(|value| value * multiplier),
        used: snapshot.used.map(|value| value * multiplier),
        ..snapshot
    }
}

/// 某个平台的默认余额查询认证方式。
#[must_use]
pub const fn default_balance_auth_mode(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Gemini => "x-goog-api-key",
        ProviderId::Claude => "x-api-key",
        ProviderId::Codex | ProviderId::Qoder => "bearer",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_limits_match_the_desktop_reference() {
        assert_eq!(BALANCE_TIMEOUT_MS, 15_000);
        assert_eq!(BALANCE_BATCH_CONCURRENCY, 3);
        assert!((NEW_API_QUOTA_UNIT - 500_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn each_provider_has_its_default_auth_mode() {
        assert_eq!(
            default_balance_auth_mode(ProviderId::Gemini),
            "x-goog-api-key"
        );
        assert_eq!(default_balance_auth_mode(ProviderId::Claude), "x-api-key");
        assert_eq!(default_balance_auth_mode(ProviderId::Codex), "bearer");
        assert_eq!(default_balance_auth_mode(ProviderId::Qoder), "bearer");
    }

    #[test]
    fn reading_a_path_walks_nested_objects() {
        let body = json!({ "a": { "b": { "c": 1 } }, "n": 5 });
        assert_eq!(read_path(&body, "a.b.c"), Some(&json!(1)));
        assert_eq!(read_path(&body, "n"), Some(&json!(5)));
        // 不存在的路径返回 None，而不是报错——候选列表里大量路径本来就不存在。
        assert_eq!(read_path(&body, "a.x"), None);
        assert_eq!(read_path(&body, "a.b.c.d"), None);
    }

    #[test]
    fn reading_a_path_through_a_non_object_yields_nothing() {
        // 数组也是 object，但按路径取值时没有意义。
        let body = json!({ "a": [1, 2], "b": "文字" });
        assert_eq!(read_path(&body, "a.0"), None);
        assert_eq!(read_path(&body, "b.x"), None);
    }

    #[test]
    fn numbers_accept_numeric_strings() {
        // 有些中转站把数字序列化成字符串，只认 JSON number 会让它们全部查不到。
        let body = json!({ "a": "12.5", "b": " 8 ", "c": 3 });
        assert_eq!(read_json_number(&body, "a"), Some(12.5));
        assert_eq!(read_json_number(&body, "b"), Some(8.0));
        assert_eq!(read_json_number(&body, "c"), Some(3.0));
    }

    #[test]
    fn empty_or_unparseable_numbers_are_rejected() {
        // 把 `""` 当成 0 会显示「余额 0」——比「查不到」严重得多的误导。
        let body = json!({ "empty": "", "blank": "   ", "text": "很多", "null": null });
        assert_eq!(read_json_number(&body, "empty"), None);
        assert_eq!(read_json_number(&body, "blank"), None);
        assert_eq!(read_json_number(&body, "text"), None);
        assert_eq!(read_json_number(&body, "null"), None);
    }

    #[test]
    fn booleans_accept_the_word_forms_seen_in_the_wild() {
        let body = json!({
            "t": true, "f": false, "one": 1, "zero": 0,
            "ok": "ok", "active": "ACTIVE", "yes": "yes",
            "no": "no", "off": "inactive", "disabled": "disabled"
        });
        for path in ["t", "one", "ok", "active", "yes"] {
            assert_eq!(read_json_boolean(&body, path), Some(true), "{path}");
        }
        for path in ["f", "zero", "no", "off", "disabled"] {
            assert_eq!(read_json_boolean(&body, path), Some(false), "{path}");
        }
    }

    #[test]
    fn other_numbers_are_not_booleans() {
        // 只认 0 与 1：其他数字不是布尔语义，猜错比返回 None 更糟。
        let body = json!({ "two": 2, "minus": -1, "text": "maybe" });
        assert_eq!(read_json_boolean(&body, "two"), None);
        assert_eq!(read_json_boolean(&body, "minus"), None);
        assert_eq!(read_json_boolean(&body, "text"), None);
    }

    #[test]
    fn strings_are_trimmed_and_blanks_rejected() {
        let body = json!({ "a": "  x  ", "b": "   ", "c": 1 });
        assert_eq!(read_json_string(&body, "a").as_deref(), Some("x"));
        assert_eq!(read_json_string(&body, "b"), None);
        assert_eq!(read_json_string(&body, "c"), None);
    }

    #[test]
    fn the_generic_template_finds_the_common_field_spellings() {
        for key in ["remaining", "balance", "available", "credits"] {
            let body = if key == "credits" {
                json!({ "credits": { "balance": 7 } })
            } else {
                json!({ key: 7 })
            };
            let snapshot = parse_generic_balance(&body).expect("应当解析");
            assert_eq!(snapshot.remaining, Some(7.0), "字段 {key}");
        }
    }

    #[test]
    fn the_generic_template_reads_the_nested_data_envelope() {
        let body = json!({
            "data": { "remaining": 10, "total": 100, "used": 90, "currency": "CNY", "plan": "专业版" }
        });
        let snapshot = parse_generic_balance(&body).expect("解析");
        assert_eq!(snapshot.remaining, Some(10.0));
        assert_eq!(snapshot.total, Some(100.0));
        assert_eq!(snapshot.used, Some(90.0));
        assert_eq!(snapshot.unit, "CNY");
        assert_eq!(snapshot.plan_name.as_deref(), Some("专业版"));
    }

    #[test]
    fn the_generic_template_defaults_the_unit_to_usd() {
        let snapshot = parse_generic_balance(&json!({ "remaining": 1 })).expect("解析");
        assert_eq!(snapshot.unit, "USD");
        assert_eq!(snapshot.plan_name, None);
    }

    #[test]
    fn a_body_without_remaining_is_not_a_balance_response() {
        // 没有这个门槛，任何 JSON（比如一个模型列表）都会被当成余额 0。
        assert!(parse_generic_balance(&json!({ "total": 100 })).is_none());
        assert!(parse_generic_balance(&json!({ "data": [{ "id": "m" }] })).is_none());
        assert!(parse_generic_balance(&json!([])).is_none());
        assert!(parse_generic_balance(&json!("文字")).is_none());
    }

    #[test]
    fn validity_is_decided_by_three_signals() {
        // success !== false
        assert!(
            !parse_generic_balance(&json!({ "remaining": 1, "success": false }))
                .expect("解析")
                .is_valid
        );
        // active !== false
        assert!(
            !parse_generic_balance(&json!({ "remaining": 1, "is_active": false }))
                .expect("解析")
                .is_valid
        );
        // 状态在失效集合里
        for status in ["expired", "quota_exhausted", "disabled", "EXPIRED"] {
            let snapshot =
                parse_generic_balance(&json!({ "remaining": 1, "status": status })).expect("解析");
            assert!(!snapshot.is_valid, "状态 {status} 应当判为无效");
        }
        // 都没说就认为有效。
        assert!(
            parse_generic_balance(&json!({ "remaining": 1 }))
                .expect("解析")
                .is_valid
        );
        assert!(
            parse_generic_balance(&json!({ "remaining": 1, "status": "ok" }))
                .expect("解析")
                .is_valid
        );
    }

    #[test]
    fn the_new_api_template_treats_quota_as_remaining() {
        // 把 quota 当 total 会让「已用」变成 0 或负数，界面上看起来像「从未用过」。
        let body = json!({ "success": true, "data": { "quota": 500_000, "used_quota": 250_000 } });
        let snapshot = parse_new_api_balance(&body).expect("解析");
        assert_eq!(snapshot.remaining, Some(1.0));
        assert_eq!(snapshot.used, Some(0.5));
        assert_eq!(snapshot.total, Some(1.5));
        assert_eq!(snapshot.unit, "额度");
        assert!(snapshot.is_valid);
    }

    #[test]
    fn the_new_api_template_handles_a_missing_used_quota() {
        let body = json!({ "quota": 1_000_000 });
        let snapshot = parse_new_api_balance(&body).expect("解析");
        assert_eq!(snapshot.remaining, Some(2.0));
        assert_eq!(snapshot.used, Some(0.0));
        assert_eq!(snapshot.total, Some(2.0));
    }

    #[test]
    fn the_new_api_template_requires_quota() {
        assert!(parse_new_api_balance(&json!({ "remaining": 1 })).is_none());
        assert!(parse_new_api_balance(&json!("文字")).is_none());
    }

    #[test]
    fn the_new_api_template_reads_the_group_as_the_plan() {
        let body = json!({ "quota": 1, "data": { "group": "默认分组" } });
        assert_eq!(
            parse_new_api_balance(&body)
                .expect("解析")
                .plan_name
                .as_deref(),
            Some("默认分组")
        );
    }

    #[test]
    fn the_multiplier_scales_only_the_numbers() {
        let snapshot = BalanceSnapshot {
            remaining: Some(2.0),
            total: Some(10.0),
            used: Some(8.0),
            unit: "USD".to_owned(),
            plan_name: Some("套餐".to_owned()),
            is_valid: true,
        };
        let scaled = apply_balance_multiplier(snapshot, Some(3.0));
        assert_eq!(scaled.remaining, Some(6.0));
        assert_eq!(scaled.total, Some(30.0));
        assert_eq!(scaled.used, Some(24.0));
        // 单位与套餐名不是数量。
        assert_eq!(scaled.unit, "USD");
        assert_eq!(scaled.plan_name.as_deref(), Some("套餐"));
    }

    #[test]
    fn a_missing_multiplier_or_one_leaves_the_snapshot_alone() {
        let make = || BalanceSnapshot {
            remaining: Some(2.0),
            total: None,
            used: None,
            unit: "USD".to_owned(),
            plan_name: None,
            is_valid: true,
        };
        assert_eq!(apply_balance_multiplier(make(), None).remaining, Some(2.0));
        assert_eq!(
            apply_balance_multiplier(make(), Some(1.0)).remaining,
            Some(2.0)
        );
        // 0 是「未配置」，不是「全部归零」。
        assert_eq!(
            apply_balance_multiplier(make(), Some(0.0)).remaining,
            Some(2.0)
        );
        // 非有限值同样跳过。
        assert_eq!(
            apply_balance_multiplier(make(), Some(f64::INFINITY)).remaining,
            Some(2.0)
        );
    }

    #[test]
    fn the_multiplier_leaves_missing_fields_missing() {
        // 缺的字段不该因为乘 0 而变成「有值且为 0」——那是编造数据。
        let snapshot = BalanceSnapshot {
            remaining: Some(5.0),
            total: None,
            used: None,
            unit: "USD".to_owned(),
            plan_name: None,
            is_valid: true,
        };
        let scaled = apply_balance_multiplier(snapshot, Some(2.0));
        assert_eq!(scaled.remaining, Some(10.0));
        assert_eq!(scaled.total, None);
        assert_eq!(scaled.used, None);
    }
}
