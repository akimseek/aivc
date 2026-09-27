//! 时间戳。
//!
//! # 为什么不直接用 `time` / `chrono`
//!
//! 现网全部时间戳来自 JavaScript 的 `Date.prototype.toISOString()`，其输出恒为
//! **UTC + 固定 3 位毫秒**（`2026-09-16T12:34:56.789Z`）。RFC 3339 的通用实现会
//! 裁剪尾随零（`…56.789Z` 与 `…56Z` 混用），无法保证逐字一致；而时间戳字符串会
//! 直接落库、进 JSON 契约并被渲染层展示，属于功能一致的一部分。
//!
//! 因此这里保留字符串表示，自行完成「Unix 毫秒 ↔ 公历」转换（Howard Hinnant 的
//! `civil_from_days` 算法，无分支陷阱、可穷举验证），只依赖标准库。
//! `tests/timestamp_parity.rs` 会用 Node 的 `toISOString()` 交叉验证。

use std::cmp::Ordering;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// ISO-8601 UTC 时间戳字符串，格式固定为 `YYYY-MM-DDTHH:mm:ss.sssZ`。
///
/// 序列化后即为普通字符串，与现网 JSON 契约一致。
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// 构造一个未校验的时间戳。调用方需保证格式正确。
    #[must_use]
    pub fn from_raw(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// 当前时刻。
    #[must_use]
    pub fn now() -> Self {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        Self::from_unix_millis(millis)
    }

    /// 由 Unix 毫秒构造。
    #[must_use]
    pub fn from_unix_millis(millis: i64) -> Self {
        Self(format_unix_millis(millis))
    }

    /// 空时间戳（表示「未提供」），对应现网的可选字段缺省。
    #[must_use]
    pub fn empty() -> Self {
        Self(String::new())
    }

    /// 以字符串切片借用。
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 是否为空。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 解析为 Unix 毫秒；格式不合法时返回 [`None`]。
    ///
    /// 与 `Date.parse` 的差异：`Date.parse` 接受大量宽松格式，这里只接受本工程
    /// 自己产出的严格格式。会话文件里的时间戳由各 CLI 写入，可能是其他合法
    /// ISO-8601 变体，因此 [`Self::to_unix_millis`] 对它们返回 `None` 而非报错，
    /// 调用方按「无法比较」处理。
    #[must_use]
    pub fn to_unix_millis(&self) -> Option<i64> {
        parse_iso8601_millis(&self.0)
    }

    /// 解析为 Unix 秒（用于 `history.jsonl` 的 `ts` 字段）。
    #[must_use]
    pub fn to_unix_seconds(&self) -> Option<i64> {
        self.to_unix_millis().map(|millis| millis.div_euclid(1000))
    }

    /// 取两个时间戳中较晚的一个（按解析后的毫秒比较）。
    ///
    /// 与现网 `latestIsoTimestamp` 一致：无法解析的一侧视为不参与比较，
    /// 因此「一侧无法解析」与「两侧都无法解析」都保留当前值。
    #[must_use]
    pub fn latest(self, other: Self) -> Self {
        match (self.to_unix_millis(), other.to_unix_millis()) {
            (Some(a), Some(b)) if a >= b => self,
            // other 可解析且更晚 → 取 other。
            (_, Some(_)) => other,
            // other 无法解析（含两侧都无法解析）→ 保留 self。
            (_, None) => self,
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Timestamp {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Timestamp {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

/// 按解析后的毫秒排序。
///
/// 无法解析的时间戳与现网 `Date.parse` 返回 `NaN` 时的处理一致：参与比较时记作 0。
impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> Ordering {
        self.to_unix_millis()
            .unwrap_or(0)
            .cmp(&other.to_unix_millis().unwrap_or(0))
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// 把 Unix 毫秒格式化为 `YYYY-MM-DDTHH:mm:ss.sssZ`。
fn format_unix_millis(millis: i64) -> String {
    let days = millis.div_euclid(86_400_000);
    let ms_of_day = millis.rem_euclid(86_400_000);

    let (year, month, day) = civil_from_days(days);
    let hour = ms_of_day / 3_600_000;
    let minute = (ms_of_day / 60_000) % 60;
    let second = (ms_of_day / 1_000) % 60;
    let milli = ms_of_day % 1_000;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{milli:03}Z")
}

/// Howard Hinnant `civil_from_days`：把「1970-01-01 起的天数」转换为公历年月日。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // 把纪元平移到 0000-03-01，使闰日落在年末，从而消掉分支。
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    // [0, 146096]
    let day_of_era = u32::try_from(shifted - era * 146_097).unwrap_or(0);
    // [0, 399]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = i64::from(year_of_era) + era * 400;
    // [0, 365]
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // [0, 11]，3 月为 0
    let mp = (5 * day_of_year + 2) / 153;
    // [1, 31]
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    // [1, 12]
    let month = if mp < 10 { mp + 3 } else { mp - 9 };

    (year + i64::from(month <= 2), month, day)
}

/// Howard Hinnant `days_from_civil`：[`civil_from_days`] 的逆运算。
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = u32::try_from(year - era * 400).unwrap_or(0);
    let month_shift = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_shift + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    era * 146_097 + i64::from(day_of_era) - 719_468
}

/// 严格解析 `YYYY-MM-DDTHH:mm:ss[.sss]Z`，返回 Unix 毫秒。
fn parse_iso8601_millis(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    // 最短 `YYYY-MM-DDTHH:mm:ssZ` = 20 字节；带毫秒 `…ss.sssZ` = 24 字节。
    if bytes.len() != 20 && bytes.len() != 24 {
        return None;
    }
    if bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    if bytes[bytes.len() - 1] != b'Z' {
        return None;
    }
    if bytes.len() == 24 && bytes[19] != b'.' {
        return None;
    }

    let year: i64 = value.get(0..4)?.parse().ok()?;
    let month: u32 = value.get(5..7)?.parse().ok()?;
    let day: u32 = value.get(8..10)?.parse().ok()?;
    let hour: i64 = value.get(11..13)?.parse().ok()?;
    let minute: i64 = value.get(14..16)?.parse().ok()?;
    let second: i64 = value.get(17..19)?.parse().ok()?;
    let milli: i64 = if bytes.len() == 24 {
        value.get(20..23)?.parse().ok()?
    } else {
        0
    };

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    Some(days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1_000 + milli)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_epoch_like_javascript_to_iso_string() {
        assert_eq!(
            Timestamp::from_unix_millis(0).as_str(),
            "1970-01-01T00:00:00.000Z"
        );
    }

    #[test]
    fn always_emits_three_fractional_digits() {
        // 这是刻意不复用 RFC 3339 实现的原因：整秒也必须补足 .000。
        assert_eq!(
            Timestamp::from_unix_millis(1_000).as_str(),
            "1970-01-01T00:00:01.000Z"
        );
        assert_eq!(
            Timestamp::from_unix_millis(1_500).as_str(),
            "1970-01-01T00:00:01.500Z"
        );
        assert_eq!(
            Timestamp::from_unix_millis(1_007).as_str(),
            "1970-01-01T00:00:01.007Z"
        );
    }

    #[test]
    fn handles_negative_epoch_and_leap_day() {
        assert_eq!(
            Timestamp::from_unix_millis(-1).as_str(),
            "1969-12-31T23:59:59.999Z"
        );
        // 2024-02-29 是闰日，用于验证 civil_from_days 的闰年分支。
        let leap_day = 1_709_164_800_000;
        assert_eq!(
            Timestamp::from_unix_millis(leap_day).as_str(),
            "2024-02-29T00:00:00.000Z"
        );
    }

    #[test]
    fn round_trips_through_parse() {
        for millis in [
            0,
            1_007,
            -1,
            -86_400_000,
            1_709_164_800_000,
            1_787_000_000_123,
        ] {
            let stamp = Timestamp::from_unix_millis(millis);
            assert_eq!(
                stamp.to_unix_millis(),
                Some(millis),
                "round trip failed for {millis} ({stamp})"
            );
        }
    }

    #[test]
    fn rejects_malformed_values_without_panicking() {
        for bad in [
            "",
            "not-a-date",
            "1970-01-01T00:00:00",      // 缺 Z
            "1970-01-01 00:00:00.000Z", // 缺 T
            "1970-13-01T00:00:00.000Z", // 月份越界
            "1970-01-01T25:00:00.000Z", // 小时越界
            "1970-01-01T00:00:00.000+08:00",
        ] {
            assert_eq!(
                Timestamp::from_raw(bad).to_unix_millis(),
                None,
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn accepts_second_precision_written_by_cli_tools() {
        // 各 CLI 写入会话文件的时间戳可能不带毫秒，需要能解析。
        // 期望值经 Node `Date.parse("2026-09-16T12:34:56Z") / 1000` 核对。
        let stamp = Timestamp::from_raw("2026-09-16T12:34:56Z");
        assert_eq!(stamp.to_unix_seconds(), Some(1_789_562_096));
    }

    #[test]
    fn unparsable_timestamps_sort_as_zero_like_date_parse_nan() {
        let invalid = Timestamp::from_raw("garbage");
        let epoch = Timestamp::from_unix_millis(0);
        assert_eq!(invalid.cmp(&epoch), Ordering::Equal);
    }

    #[test]
    fn latest_prefers_the_parsable_and_later_value() {
        let early = Timestamp::from_unix_millis(1_000);
        let late = Timestamp::from_unix_millis(2_000);
        assert_eq!(early.clone().latest(late.clone()), late);
        assert_eq!(late.clone().latest(early.clone()), late);

        let invalid = Timestamp::from_raw("garbage");
        assert_eq!(invalid.clone().latest(early.clone()), early);
        assert_eq!(early.clone().latest(invalid.clone()), early);
    }

    #[test]
    fn empty_timestamp_reports_itself_as_empty() {
        assert!(Timestamp::empty().is_empty());
        assert!(!Timestamp::now().is_empty());
    }
}
