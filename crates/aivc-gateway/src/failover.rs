//! 故障转移引擎：候选选择与熔断状态。
//!
//! # 候选选择的两种语义
//!
//! - [`choose_vendor`]：**首次**选择。排除已熔断的供应商，首选供应商优先，其余按
//!   `sort` 升序。
//! - [`choose_next_vendor`]：**失败后**换一家。这里**不能**排除已熔断的供应商来构建
//!   候选列表——当前供应商刚失败、可能已进入熔断，但仍需要它在完整排序中的位置，
//!   才能从它的下一个 `sort` 值开始环形选择。排除动作发生在**挑选**阶段。
//!
//! 这两者的差异很容易在重构中被「顺手统一」，导致故障切换从「换下一家」退化成
//! 「总是换到 sort 最小的那家」——即反复回到刚失败的那家。因此拆成两个函数并各有测试。
//!
//! # 熔断三态
//!
//! ```text
//! healthy ──失败达阈值──> open ──熔断期结束──> half-open ──成功──> healthy
//!    ^                     |                                  |
//!    └──────成功───────────┴────────────失败──────────────────┘
//! ```
//!
//! `open` 到期后**不**自动变回 `healthy`，而是变成 `half-open`：放一个请求过去验证，
//! 成功才恢复。直接恢复会让上游还在故障时被瞬间打满。

use std::collections::HashMap;
use std::time::Duration;

use aivc_core::constants::GATEWAY_FAILURE_WINDOW_MS;
use aivc_core::gateway::{GatewayVendorHealth, GatewayVendorHealthStatus};
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;
use aivc_core::vendor::ApiVendor;

/// 失败原因在持久化时的截断长度。
pub const FAILURE_REASON_MAX_CHARS: usize = 240;

/// 熔断参数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitConfig {
    /// 失败窗口内的失败次数达到该值即熔断。
    pub failure_threshold: usize,
    /// 熔断持续时长。
    pub duration: Duration,
}

impl Default for CircuitConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            duration: Duration::from_secs(60),
        }
    }
}

/// 供应商的内存健康状态。
#[derive(Debug, Clone)]
pub struct HealthState {
    /// 对外快照。
    pub health: GatewayVendorHealth,
    /// 失败时间戳（毫秒），只保留窗口内的。
    failures: Vec<i64>,
}

impl HealthState {
    /// 由快照构造。
    #[must_use]
    pub fn from_health(health: GatewayVendorHealth, now_ms: i64) -> Self {
        // 持久化时状态是 `open` 说明熔断正在进行；用一个「现在」占位，让窗口内的
        // 计数从恢复后重新开始。
        let failures = if health.status == GatewayVendorHealthStatus::Open {
            vec![now_ms]
        } else {
            Vec::new()
        };
        Self { health, failures }
    }
}

/// 健康状态追踪器。
///
/// 时间通过参数注入而不是读系统时钟：熔断行为完全由时间决定，可注入才能穷举测试
/// 「阈值边界」「熔断到期」这些分支。
#[derive(Debug, Default)]
pub struct HealthTracker {
    states: HashMap<VendorId, HealthState>,
}

impl HealthTracker {
    /// 空追踪器。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 由持久化快照恢复。
    pub fn hydrate(&mut self, health: Vec<GatewayVendorHealth>, now_ms: i64) {
        for item in health {
            self.states.insert(
                item.vendor_id.clone(),
                HealthState::from_health(item, now_ms),
            );
        }
    }

    /// 取某个供应商的状态（不存在时返回 [`None`]）。
    #[must_use]
    pub fn get(&self, vendor_id: &VendorId) -> Option<&HealthState> {
        self.states.get(vendor_id)
    }

    /// 全部状态快照，按平台与编号排序。
    ///
    /// `open` 到期后按 [`GatewayVendorHealthStatus::HalfOpen`] 呈现——`is_circuit_open`
    /// 会就地更新状态，因此这里不需要再判断一次。
    #[must_use]
    pub fn snapshot(&self) -> Vec<GatewayVendorHealth> {
        let mut items: Vec<GatewayVendorHealth> = self
            .states
            .values()
            .map(|state| state.health.clone())
            .collect();
        items.sort_by(|left, right| {
            left.provider_id
                .as_str()
                .cmp(right.provider_id.as_str())
                .then_with(|| left.vendor_id.as_str().cmp(right.vendor_id.as_str()))
        });
        items
    }

    /// 判断某供应商是否处于熔断中。
    ///
    /// 熔断期已过时**就地**转为 `half-open` 并清除到期时间，返回 `false`（放行一个请求
    /// 去验证）。这个副作用是必要的：调用方拿到 `false` 后就会真的发请求，状态必须
    /// 已经反映「正在验证」。
    pub fn is_circuit_open(&mut self, vendor_id: &VendorId, now_ms: i64) -> bool {
        let Some(state) = self.states.get_mut(vendor_id) else {
            return false;
        };
        let Some(circuit_until) = state.health.circuit_until.clone() else {
            return false;
        };
        if circuit_until
            .to_unix_millis()
            .is_none_or(|until| until <= now_ms)
        {
            state.health.status = GatewayVendorHealthStatus::HalfOpen;
            state.health.circuit_until = None;
            return false;
        }
        state.health.status == GatewayVendorHealthStatus::Open
    }

    /// 记录一次成功。
    ///
    /// 成功会**完全**清空失败窗口并恢复为 `healthy`：一次成功说明上游已经恢复，
    /// 继续保留历史失败会让它在下一次偶发失败时立刻再次熔断。
    pub fn record_success(&mut self, vendor: &ApiVendor, now_ms: i64) {
        let state = self.state_for(vendor);
        state.health.success_count += 1;
        state.health.status = GatewayVendorHealthStatus::Healthy;
        state.health.circuit_until = None;
        state.failures.clear();
        state.health.last_success_at = Some(millis_to_timestamp(now_ms));
        state.health.failure_rate = failure_rate(state);
    }

    /// 记录一次失败。
    ///
    /// 只有**窗口内**的失败才计入熔断判定：五分钟前的一串失败不该让供应商一直熔断。
    pub fn record_failure(
        &mut self,
        vendor: &ApiVendor,
        reason: &str,
        config: CircuitConfig,
        now_ms: i64,
    ) {
        let state = self.state_for(vendor);
        state
            .failures
            .retain(|timestamp| now_ms - timestamp < GATEWAY_FAILURE_WINDOW_MS);
        state.failures.push(now_ms);
        state.health.failure_count += 1;
        state.health.last_failure_at = Some(millis_to_timestamp(now_ms));
        state.health.last_failure_reason = Some(truncate_reason(reason));
        state.health.failure_rate = failure_rate(state);

        let should_open = state.failures.len() >= config.failure_threshold;
        state.health.status = if should_open {
            GatewayVendorHealthStatus::Open
        } else {
            GatewayVendorHealthStatus::Degraded
        };
        state.health.circuit_until = should_open.then(|| {
            millis_to_timestamp(
                now_ms + i64::try_from(config.duration.as_millis()).unwrap_or(i64::MAX),
            )
        });
    }

    /// 清除健康状态（全部或某一个）。
    pub fn reset(&mut self, vendor_id: Option<&VendorId>) {
        match vendor_id {
            Some(id) => {
                self.states.remove(id);
            }
            None => self.states.clear(),
        }
    }

    /// 取得或创建状态。
    fn state_for(&mut self, vendor: &ApiVendor) -> &mut HealthState {
        self.states
            .entry(vendor.id.clone())
            .or_insert_with(|| HealthState {
                health: GatewayVendorHealth {
                    vendor_id: vendor.id.clone(),
                    provider_id: vendor.provider_id,
                    status: GatewayVendorHealthStatus::Healthy,
                    failure_count: 0,
                    success_count: 0,
                    failure_rate: 0.0,
                    last_failure_at: None,
                    last_success_at: None,
                    circuit_until: None,
                    last_failure_reason: None,
                },
                failures: Vec::new(),
            })
    }
}

/// 失败率：失败数 / 总数。
fn failure_rate(state: &HealthState) -> f64 {
    let total = state.health.failure_count + state.health.success_count;
    if total == 0 {
        return 0.0;
    }
    // 计数是整数，转成 f64 的精度损失远小于比率本身的用途（仅用于展示与排序）。
    #[allow(clippy::cast_precision_loss)]
    {
        state.health.failure_count as f64 / total as f64
    }
}

/// 截断失败原因。
fn truncate_reason(reason: &str) -> String {
    reason.chars().take(FAILURE_REASON_MAX_CHARS).collect()
}

/// 毫秒时间戳转 [`Timestamp`]。
fn millis_to_timestamp(now_ms: i64) -> Timestamp {
    Timestamp::from_unix_millis(now_ms)
}

/// 供应商是否已配置完整、可作为候选。
#[must_use]
pub fn is_configured_candidate(vendor: &ApiVendor, provider_id: ProviderId) -> bool {
    vendor.provider_id == provider_id
        && vendor.enabled
        && !vendor.api_key.trim().is_empty()
        && !vendor.api_base_url.trim().is_empty()
}

/// 候选排序：`sort` 升序，相同则按创建时间。
#[must_use]
pub fn compare_vendor_order(left: &ApiVendor, right: &ApiVendor) -> std::cmp::Ordering {
    left.sort
        .cmp(&right.sort)
        .then_with(|| left.created_at.as_str().cmp(right.created_at.as_str()))
}

/// 首次选择供应商。
///
/// 排除已熔断的；首选供应商优先，其余按 [`compare_vendor_order`]。
#[must_use]
pub fn choose_vendor<'a, F>(
    vendors: &'a [ApiVendor],
    provider_id: ProviderId,
    preferred_vendor_id: Option<&str>,
    mut is_open: F,
) -> Option<&'a ApiVendor>
where
    F: FnMut(&VendorId) -> bool,
{
    let mut candidates: Vec<&ApiVendor> = vendors
        .iter()
        .filter(|vendor| is_configured_candidate(vendor, provider_id))
        .filter(|vendor| !is_open(&vendor.id))
        .collect();
    candidates.sort_by(|left, right| {
        // 首选供应商排在最前，其余按 sort 升序。
        let preferred = |vendor: &ApiVendor| {
            i32::from(preferred_vendor_id.is_none_or(|id| vendor.id.as_str() != id))
        };
        preferred(left)
            .cmp(&preferred(right))
            .then_with(|| compare_vendor_order(left, right))
    });
    candidates.first().copied()
}

/// 失败后换一家供应商。
///
/// 从当前供应商在**完整排序**中的下一个位置开始环形查找，跳过已尝试过与已熔断的。
///
/// 见模块文档：这里刻意不预先排除熔断中的供应商。
#[must_use]
pub fn choose_next_vendor<'a, F>(
    vendors: &'a [ApiVendor],
    provider_id: ProviderId,
    current_vendor_id: &str,
    attempted_ids: &[VendorId],
    mut is_open: F,
) -> Option<&'a ApiVendor>
where
    F: FnMut(&VendorId) -> bool,
{
    let mut candidates: Vec<&ApiVendor> = vendors
        .iter()
        .filter(|vendor| is_configured_candidate(vendor, provider_id))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|left, right| compare_vendor_order(left, right));

    let start = candidates
        .iter()
        .position(|vendor| vendor.id.as_str() == current_vendor_id)
        .map_or(0, |index| index + 1);

    for offset in 0..candidates.len() {
        let candidate = candidates[(start + offset) % candidates.len()];
        if attempted_ids.iter().any(|id| id == &candidate.id) {
            continue;
        }
        if is_open(&candidate.id) {
            continue;
        }
        return Some(candidate);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vendor(id: &str, sort: i64, enabled: bool) -> ApiVendor {
        ApiVendor {
            id: VendorId::from(id),
            provider_id: ProviderId::Codex,
            name: id.to_owned(),
            api_key: "key".to_owned(),
            api_base_url: "https://api.example.com".to_owned(),
            sort,
            pricing: None,
            model_query: None,
            balance_query: None,
            configs: Vec::new(),
            enabled,
            created_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
            updated_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
            last_enabled_at: None,
            balance: None,
            balance_status: None,
            balance_error: None,
            balance_queried_at: None,
            gateway_health: None,
        }
    }

    fn ids(vendors: &[ApiVendor]) -> Vec<String> {
        vendors
            .iter()
            .map(|vendor| vendor.id.as_str().to_owned())
            .collect()
    }

    #[test]
    fn candidates_require_a_complete_configuration() {
        let mut broken = vendor("broken", 1, true);
        broken.api_key = "   ".to_owned();
        let mut other_provider = vendor("other", 2, true);
        other_provider.provider_id = ProviderId::Claude;
        let vendors = [
            vendor("ok", 3, true),
            broken,
            other_provider,
            vendor("off", 4, false),
        ];
        assert_eq!(ids(&vendors[..1]), vec!["ok"]);
        assert!(is_configured_candidate(&vendors[0], ProviderId::Codex));
        assert!(!is_configured_candidate(&vendors[1], ProviderId::Codex));
        assert!(!is_configured_candidate(&vendors[2], ProviderId::Codex));
        assert!(!is_configured_candidate(&vendors[3], ProviderId::Codex));
    }

    #[test]
    fn the_preferred_vendor_wins_the_first_choice() {
        let vendors = vec![
            vendor("a", 1, true),
            vendor("b", 2, true),
            vendor("c", 3, true),
        ];
        let chosen = choose_vendor(&vendors, ProviderId::Codex, Some("c"), |_| false);
        assert_eq!(chosen.map(|item| item.id.as_str()), Some("c"));
    }

    #[test]
    fn without_a_preference_the_lowest_sort_wins() {
        let vendors = vec![
            vendor("c", 3, true),
            vendor("a", 1, true),
            vendor("b", 2, true),
        ];
        let chosen = choose_vendor(&vendors, ProviderId::Codex, None, |_| false);
        assert_eq!(chosen.map(|item| item.id.as_str()), Some("a"));
    }

    #[test]
    fn the_first_choice_skips_open_circuits() {
        let vendors = vec![vendor("a", 1, true), vendor("b", 2, true)];
        let chosen = choose_vendor(&vendors, ProviderId::Codex, None, |id| id.as_str() == "a");
        assert_eq!(chosen.map(|item| item.id.as_str()), Some("b"));
        // 全部熔断时没有候选。
        assert!(choose_vendor(&vendors, ProviderId::Codex, None, |_| true).is_none());
    }

    #[test]
    fn failover_rings_forward_from_the_current_vendor() {
        // 关键：换一家要从**当前位置的下一个**开始，而不是回到 sort 最小的那家。
        let vendors = vec![
            vendor("a", 1, true),
            vendor("b", 2, true),
            vendor("c", 3, true),
        ];
        let next = choose_next_vendor(&vendors, ProviderId::Codex, "a", &[], |_| false);
        assert_eq!(next.map(|item| item.id.as_str()), Some("b"));

        let next = choose_next_vendor(&vendors, ProviderId::Codex, "b", &[], |_| false);
        assert_eq!(next.map(|item| item.id.as_str()), Some("c"));

        // 环形：c 的下一个是 a。
        let next = choose_next_vendor(&vendors, ProviderId::Codex, "c", &[], |_| false);
        assert_eq!(next.map(|item| item.id.as_str()), Some("a"));
    }

    #[test]
    fn failover_keeps_the_current_vendor_in_the_ordering_even_when_it_is_open() {
        // 当前供应商刚失败、可能已熔断；仍需要它在排序中的位置才能环形前进。
        let vendors = vec![vendor("a", 1, true), vendor("b", 2, true)];
        let next = choose_next_vendor(&vendors, ProviderId::Codex, "a", &[], |id| {
            id.as_str() == "a"
        });
        assert_eq!(
            next.map(|item| item.id.as_str()),
            Some("b"),
            "熔断的当前供应商不应让环形起点失效"
        );
    }

    #[test]
    fn failover_skips_attempted_vendors() {
        let vendors = vec![
            vendor("a", 1, true),
            vendor("b", 2, true),
            vendor("c", 3, true),
        ];
        let attempted = vec![VendorId::from("b")];
        let next = choose_next_vendor(&vendors, ProviderId::Codex, "a", &attempted, |_| false);
        assert_eq!(next.map(|item| item.id.as_str()), Some("c"));
        // 全部尝试过则没有候选。
        let attempted = vec![
            VendorId::from("a"),
            VendorId::from("b"),
            VendorId::from("c"),
        ];
        assert!(
            choose_next_vendor(&vendors, ProviderId::Codex, "a", &attempted, |_| false).is_none()
        );
    }

    #[test]
    fn failover_returns_none_when_every_other_vendor_is_open() {
        let vendors = vec![vendor("a", 1, true), vendor("b", 2, true)];
        let next = choose_next_vendor(&vendors, ProviderId::Codex, "a", &[], |_| true);
        assert!(next.is_none());
    }

    #[test]
    fn failover_from_an_unknown_vendor_starts_at_the_beginning() {
        let vendors = vec![vendor("a", 1, true), vendor("b", 2, true)];
        let next = choose_next_vendor(&vendors, ProviderId::Codex, "gone", &[], |_| false);
        assert_eq!(next.map(|item| item.id.as_str()), Some("a"));
    }

    #[test]
    fn ordering_falls_back_to_the_creation_time() {
        let mut first = vendor("a", 5, true);
        first.created_at = Timestamp::from_raw("2026-01-01T00:00:00.000Z");
        let mut second = vendor("b", 5, true);
        second.created_at = Timestamp::from_raw("2026-02-01T00:00:00.000Z");
        assert_eq!(
            compare_vendor_order(&first, &second),
            std::cmp::Ordering::Less
        );
    }

    // --- 健康状态与熔断 ---

    fn config(threshold: usize, seconds: u64) -> CircuitConfig {
        CircuitConfig {
            failure_threshold: threshold,
            duration: Duration::from_secs(seconds),
        }
    }

    #[test]
    fn a_single_failure_degrades_but_does_not_open() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        tracker.record_failure(&item, "boom", config(3, 60), 1_000);
        let state = tracker.get(&item.id).expect("state");
        assert_eq!(state.health.status, GatewayVendorHealthStatus::Degraded);
        assert!(state.health.circuit_until.is_none());
        assert_eq!(state.health.failure_count, 1);
        assert_eq!(state.health.last_failure_reason.as_deref(), Some("boom"));
    }

    #[test]
    fn reaching_the_threshold_opens_the_circuit() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        for offset in 0..3 {
            tracker.record_failure(&item, "boom", config(3, 60), 1_000 + offset);
        }
        let state = tracker.get(&item.id).expect("state");
        assert_eq!(state.health.status, GatewayVendorHealthStatus::Open);
        // 熔断到期时间 = 最后一次失败 + 时长。
        assert_eq!(
            state
                .health
                .circuit_until
                .as_ref()
                .and_then(Timestamp::to_unix_millis),
            Some(1_002 + 60_000)
        );
    }

    #[test]
    fn an_open_circuit_is_reported_as_open_until_it_expires() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        tracker.record_failure(&item, "boom", config(1, 60), 1_000);
        assert!(tracker.is_circuit_open(&item.id, 1_000));
        assert!(tracker.is_circuit_open(&item.id, 60_999));
        // 到期后放行一个请求去验证，状态转为 half-open。
        assert!(!tracker.is_circuit_open(&item.id, 61_000));
        let state = tracker.get(&item.id).expect("state");
        assert_eq!(state.health.status, GatewayVendorHealthStatus::HalfOpen);
        assert!(state.health.circuit_until.is_none());
    }

    #[test]
    fn success_fully_clears_the_failure_window() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        tracker.record_failure(&item, "boom", config(3, 60), 1_000);
        tracker.record_failure(&item, "boom", config(3, 60), 1_001);
        tracker.record_success(&item, 1_002);

        let state = tracker.get(&item.id).expect("state");
        assert_eq!(state.health.status, GatewayVendorHealthStatus::Healthy);
        assert!(state.failures.is_empty(), "成功必须清空失败窗口");
        // 再失败两次不应立刻熔断——历史失败已被清空。
        tracker.record_failure(&item, "boom", config(3, 60), 1_003);
        tracker.record_failure(&item, "boom", config(3, 60), 1_004);
        assert_eq!(
            tracker.get(&item.id).expect("state").health.status,
            GatewayVendorHealthStatus::Degraded
        );
    }

    #[test]
    fn failures_outside_the_window_do_not_count_toward_the_threshold() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        // 两次发生在窗口之外。
        tracker.record_failure(&item, "old", config(3, 60), 0);
        tracker.record_failure(&item, "old", config(3, 60), 1);
        // 一次发生在很久之后：前两次已过期。
        let later = GATEWAY_FAILURE_WINDOW_MS + 10;
        tracker.record_failure(&item, "new", config(3, 60), later);
        assert_eq!(
            tracker.get(&item.id).expect("state").health.status,
            GatewayVendorHealthStatus::Degraded,
            "五分钟前的失败不该计入熔断判定"
        );
    }

    #[test]
    fn failure_rates_are_computed_from_both_counters() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        tracker.record_success(&item, 1_000);
        tracker.record_success(&item, 1_001);
        tracker.record_failure(&item, "boom", config(9, 60), 1_002);
        let state = tracker.get(&item.id).expect("state");
        // 1 失败 / 3 总数。
        assert!((state.health.failure_rate - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn long_failure_reasons_are_truncated() {
        let mut tracker = HealthTracker::new();
        let item = vendor("a", 1, true);
        let long = "字".repeat(500);
        tracker.record_failure(&item, &long, config(9, 60), 1_000);
        let state = tracker.get(&item.id).expect("state");
        assert_eq!(
            state
                .health
                .last_failure_reason
                .as_deref()
                .map(|reason| reason.chars().count()),
            Some(FAILURE_REASON_MAX_CHARS)
        );
    }

    #[test]
    fn hydration_restores_open_circuits_and_normalizes_expired_ones() {
        let mut tracker = HealthTracker::new();
        let health = GatewayVendorHealth {
            vendor_id: VendorId::from("a"),
            provider_id: ProviderId::Codex,
            status: GatewayVendorHealthStatus::Open,
            failure_count: 9,
            success_count: 1,
            failure_rate: 0.9,
            last_failure_at: None,
            last_success_at: None,
            // 已过期。
            circuit_until: Some(Timestamp::from_unix_millis(500)),
            last_failure_reason: Some("boom".to_owned()),
        };
        tracker.hydrate(vec![health], 1_000);
        // 恢复后第一次判定就会把它转成 half-open。
        assert!(!tracker.is_circuit_open(&VendorId::from("a"), 1_000));
        assert_eq!(
            tracker
                .get(&VendorId::from("a"))
                .expect("state")
                .health
                .status,
            GatewayVendorHealthStatus::HalfOpen
        );
    }

    #[test]
    fn snapshot_is_sorted_and_reflects_the_current_state() {
        let mut tracker = HealthTracker::new();
        let a = vendor("b", 1, true);
        let b = vendor("a", 2, true);
        tracker.record_failure(&a, "boom", config(1, 60), 1_000);
        tracker.record_success(&b, 1_000);

        let snapshot = tracker.snapshot();
        let ids: Vec<&str> = snapshot
            .iter()
            .map(|item| item.vendor_id.as_str())
            .collect();
        assert_eq!(ids, vec!["a", "b"]);
        assert_eq!(snapshot[1].status, GatewayVendorHealthStatus::Open);
    }

    #[test]
    fn reset_clears_one_or_every_vendor() {
        let mut tracker = HealthTracker::new();
        let a = vendor("a", 1, true);
        let b = vendor("b", 2, true);
        tracker.record_failure(&a, "boom", config(1, 60), 1_000);
        tracker.record_failure(&b, "boom", config(1, 60), 1_000);
        assert_eq!(tracker.snapshot().len(), 2);

        tracker.reset(Some(&a.id));
        assert_eq!(tracker.snapshot().len(), 1);
        tracker.reset(None);
        assert!(tracker.snapshot().is_empty());
    }

    #[test]
    fn an_unknown_vendor_is_never_open() {
        let mut tracker = HealthTracker::new();
        assert!(!tracker.is_circuit_open(&VendorId::from("missing"), 1_000));
    }
}
