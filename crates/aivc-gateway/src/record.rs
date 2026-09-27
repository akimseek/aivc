//! 网关请求记录：把一次转发的全过程累积成一条可落库的记录。
//!
//! # 为什么用累加器而不是在末尾拼一个结构
//!
//! 一次请求的数据来自四个不同阶段：进入时（路径、方法）、转发中（字节数、重试、切换）、
//! 上游返回后（状态码、错误）、结束时（耗时、结果分类）。它们分散在控制流的各个分支里，
//! 有的分支（客户端断开）甚至中途就返回了。
//!
//! 集中成一个累加器之后，每个分支只负责**更新自己那一项**，「最终记录长什么样」只有
//! 一处定义。否则每加一个提前返回的分支，就多一次漏记字段的机会。
//!
//! # 两条必须保证的性质
//!
//! 1. **任何路径都要留下记录**。客户端中途断开、上游超时、没有可用供应商——这些恰恰是
//!    最需要排查的情况，漏记它们等于让日志在最有用的时候失灵。
//! 2. **路径与错误摘要必须清洗与截断后再落库**。它们直接来自请求，可能很长、可能含换行
//!    （伪造日志行）、可能含凭据（查询串里的 `key`）。

use aivc_core::gateway::REQUEST_PATH_MAX_LENGTH;
use aivc_core::gateway::{
    GatewayOutcome, GatewayRequestRecord, GatewayUsage, sanitize_error_message,
};
use aivc_core::id::{RequestId, VendorId};
use aivc_core::target::ProviderId;
use aivc_core::time::Timestamp;

/// 一次转发的累积状态。
#[derive(Debug, Clone)]
pub struct RequestTracker {
    request_id: RequestId,
    route_id: String,
    provider_id: ProviderId,
    vendor_id: VendorId,
    method: String,
    path: String,
    model: Option<String>,
    upstream_status: Option<u16>,
    outcome: GatewayOutcome,
    started_at_ms: i64,
    bytes_in: u64,
    bytes_out: u64,
    retry_count: u32,
    switched: bool,
    error_code: Option<String>,
    error_message: Option<String>,
    usage: Option<GatewayUsage>,
}

impl RequestTracker {
    /// 开始记录一次请求。
    ///
    /// `path` 会在这里就清洗与截断——它是**最不可信**的字段（直接来自请求行）。
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        route_id: &str,
        provider_id: ProviderId,
        vendor_id: &VendorId,
        method: &str,
        path: &str,
        now_ms: i64,
    ) -> Self {
        Self {
            request_id: RequestId::from(uuid::Uuid::new_v4().to_string()),
            route_id: route_id.to_owned(),
            provider_id,
            vendor_id: vendor_id.clone(),
            method: method.to_owned(),
            path: sanitize_path(path),
            model: None,
            upstream_status: None,
            // 兜底为 `error`：请求没跑完就退出时，它多半确实失败了。
            // 明确的成功或客户端断开由后续调用覆盖。
            outcome: GatewayOutcome::Error,
            started_at_ms: now_ms,
            bytes_in: 0,
            bytes_out: 0,
            retry_count: 0,
            switched: false,
            error_code: None,
            error_message: None,
            usage: None,
        }
    }

    /// 请求编号。
    #[must_use]
    pub const fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// 路由解析成功后补齐路由与平台。
    ///
    /// 记录在请求**开始时**就创建（那时还不知道路由），身份信息随后补齐。原因是
    /// 「任何路径都要留下记录」——包括路由不存在、鉴权失败这些在解析阶段就返回的情况。
    pub fn set_route(&mut self, route_id: &str, provider_id: ProviderId) {
        route_id.clone_into(&mut self.route_id);
        self.provider_id = provider_id;
    }

    /// 供应商选定后补齐供应商。
    pub fn set_vendor(&mut self, vendor_id: &VendorId) {
        self.vendor_id = vendor_id.clone();
    }

    /// 切换供应商后更新记录。
    ///
    /// 同时把 `switched` 置位：一次请求里只要发生过切换，用量统计就必须能看出这条记录
    /// 不是「一个供应商从头服务到尾」。
    pub fn switch_vendor(&mut self, vendor_id: &VendorId) {
        self.vendor_id = vendor_id.clone();
        self.switched = true;
    }

    /// 记一次重试。
    pub const fn record_retry(&mut self) {
        self.retry_count += 1;
    }

    /// 记录上行字节。
    pub const fn add_bytes_in(&mut self, bytes: u64) {
        self.bytes_in = self.bytes_in.saturating_add(bytes);
    }

    /// 记录下行字节。
    pub const fn add_bytes_out(&mut self, bytes: u64) {
        self.bytes_out = self.bytes_out.saturating_add(bytes);
    }

    /// 记录模型名。
    pub fn set_model(&mut self, model: Option<&str>) {
        self.model = model
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
    }

    /// 记录上游状态码。
    pub const fn set_upstream_status(&mut self, status: u16) {
        self.upstream_status = Some(status);
    }

    /// 记录用量。
    pub fn set_usage(&mut self, usage: Option<GatewayUsage>) {
        self.usage = usage;
    }

    /// 标记成功完成。
    pub fn mark_ok(&mut self) {
        self.outcome = GatewayOutcome::Ok;
        // 成功时不该残留之前的错误信息。
        self.error_code = None;
        self.error_message = None;
    }

    /// 标记为「成功完成，但带一个原因码」。
    ///
    /// # 为什么需要它
    ///
    /// 「路由不存在」「鉴权失败」这些路径**没有转发任何请求**，但把它们记成 `ok` 是
    /// **有意的**：那不是供应商的错，记成失败会污染按供应商统计的失败率与熔断判定。
    ///
    /// 代价是记录里**看不出差别**——`outcome: ok`、供应商为空、上游状态为空，
    /// 与一次真正转发成功的记录几乎一样。于是有人查「我的 CLI 为什么失败」时，
    /// 在 `gateway:query-logs` 里看到的是一串 `ok`，而它们其实是 404。
    ///
    /// 因此这里在**保持 `outcome = ok` 不变**的前提下补一个原因码。这样做是安全的：
    /// 失败列表与诊断只查 `outcome IN ('error','timeout')`（见仓储层的说明），
    /// 用量统计也只看 `outcome`，因此这些记录不会被混进失败里。
    pub fn mark_ok_with(&mut self, code: &str) {
        self.mark_ok();
        // 有意在 `mark_ok` **之后**设：它就是「成功但有一个原因」的表达。
        self.error_code = Some(code.to_owned());
    }

    /// 标记客户端中途断开。
    ///
    /// 单独一类而不是归入 `error`：它不是上游故障，不该影响供应商健康与熔断判定。
    pub const fn mark_client_aborted(&mut self) {
        self.outcome = GatewayOutcome::ClientAborted;
    }

    /// 标记上游超时。
    pub const fn mark_timeout(&mut self) {
        self.outcome = GatewayOutcome::Timeout;
    }

    /// 记录错误。
    ///
    /// 摘要会清洗换行（否则可以在日志里伪造新行）并截断。
    pub fn set_error(&mut self, code: Option<&str>, message: Option<&str>) {
        if let Some(code) = code.map(str::trim).filter(|value| !value.is_empty()) {
            self.error_code = Some(code.to_owned());
        }
        if let Some(message) = message.map(str::trim).filter(|value| !value.is_empty()) {
            self.error_message = Some(sanitize_error_message(message));
        }
    }

    /// 收尾并产出记录。
    ///
    /// `now_ms` 由调用方给出，因此耗时可以脱离真实时钟测试。
    #[must_use]
    pub fn finish(&self, now_ms: i64) -> GatewayRequestRecord {
        // 耗时不取负值：时钟回拨时不该出现一条「负耗时」的记录。
        let duration = now_ms.saturating_sub(self.started_at_ms).max(0);
        GatewayRequestRecord {
            request_id: self.request_id.clone(),
            route_id: self.route_id.clone(),
            provider_id: self.provider_id,
            vendor_id: self.vendor_id.clone(),
            method: self.method.clone(),
            path: self.path.clone(),
            model: self.model.clone(),
            upstream_status: self.upstream_status,
            outcome: self.outcome,
            #[allow(clippy::cast_sign_loss)]
            duration_ms: duration as u64,
            bytes_in: self.bytes_in,
            bytes_out: self.bytes_out,
            retry_count: self.retry_count,
            switched: self.switched,
            error_code: self.error_code.clone(),
            error_message: self.error_message.clone(),
            usage: self.usage.clone(),
            created_at: Timestamp::from_unix_millis(self.started_at_ms),
        }
    }
}

/// 清洗并截断请求路径。
///
/// 去掉查询串里的访问凭据——`key` 参数是本地/外部访问令牌，落进日志等同于泄露。
#[must_use]
pub fn sanitize_path(path: &str) -> String {
    let without_key = strip_key_param(path);
    let collapsed = sanitize_error_message(&without_key);
    collapsed.chars().take(REQUEST_PATH_MAX_LENGTH).collect()
}

/// 去掉查询串里的 `key` 参数。
fn strip_key_param(path: &str) -> String {
    let Some((base, query)) = path.split_once('?') else {
        return path.to_owned();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split_once('=').is_none_or(|(name, _)| name != "key"))
        .collect();
    if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // 只在测试里用：生产路径上的错误摘要截断由 sanitize_error_message 内部完成。
    use aivc_core::gateway::REQUEST_ERROR_MAX_LENGTH;

    fn vendor() -> VendorId {
        VendorId::from("v1")
    }

    fn new_tracker() -> RequestTracker {
        RequestTracker::start(
            "r1",
            ProviderId::Codex,
            &vendor(),
            "POST",
            "/gateway/codex/r1/v1/messages",
            1_000,
        )
    }

    #[test]
    fn a_fresh_request_defaults_to_error_until_marked_otherwise() {
        // 请求没跑完就退出时，它多半确实失败了。默认 `error` 比默认 `ok` 安全。
        let record = new_tracker().finish(1_000);
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.duration_ms, 0);
        assert!(!record.switched);
        assert_eq!(record.retry_count, 0);
    }

    #[test]
    fn marking_ok_clears_any_earlier_error() {
        let mut tracker = new_tracker();
        tracker.set_error(Some("HTTP_500"), Some("upstream exploded"));
        tracker.mark_ok();
        let record = tracker.finish(1_000);
        assert_eq!(record.outcome, GatewayOutcome::Ok);
        assert!(record.error_code.is_none(), "成功时不该残留错误代码");
        assert!(record.error_message.is_none());
    }

    #[test]
    fn a_client_abort_is_its_own_outcome() {
        // 客户端断开不是上游故障，不该影响供应商健康与熔断判定。
        let mut tracker = new_tracker();
        tracker.mark_client_aborted();
        assert_eq!(tracker.finish(1_000).outcome, GatewayOutcome::ClientAborted);
    }

    #[test]
    fn a_timeout_is_its_own_outcome() {
        let mut tracker = new_tracker();
        tracker.mark_timeout();
        assert_eq!(tracker.finish(1_000).outcome, GatewayOutcome::Timeout);
    }

    #[test]
    fn failover_updates_the_vendor_and_sets_the_switched_flag() {
        let mut tracker = new_tracker();
        tracker.switch_vendor(&VendorId::from("v2"));
        let record = tracker.finish(1_000);
        assert_eq!(record.vendor_id.as_str(), "v2");
        assert!(record.switched, "发生过切换必须被记下");
    }

    #[test]
    fn retries_are_counted() {
        let mut tracker = new_tracker();
        tracker.record_retry();
        tracker.record_retry();
        assert_eq!(tracker.finish(1_000).retry_count, 2);
    }

    #[test]
    fn byte_counters_accumulate_and_saturate() {
        let mut tracker = new_tracker();
        tracker.add_bytes_in(100);
        tracker.add_bytes_in(50);
        tracker.add_bytes_out(2_000);
        let record = tracker.finish(1_000);
        assert_eq!(record.bytes_in, 150);
        assert_eq!(record.bytes_out, 2_000);

        // 溢出时饱和而不是回绕——回绕会让一条记录显示成极小的字节数。
        let mut tracker = new_tracker();
        tracker.add_bytes_in(u64::MAX);
        tracker.add_bytes_in(1);
        assert_eq!(tracker.finish(1_000).bytes_in, u64::MAX);
    }

    #[test]
    fn duration_uses_the_supplied_clock() {
        let tracker = new_tracker();
        assert_eq!(tracker.finish(1_250).duration_ms, 250);
    }

    #[test]
    fn a_backwards_clock_never_produces_a_negative_duration() {
        // 时钟回拨时不该出现「负耗时」的记录。
        let tracker = new_tracker();
        assert_eq!(tracker.finish(500).duration_ms, 0);
    }

    #[test]
    fn the_created_at_timestamp_is_the_start_time() {
        let record = new_tracker().finish(9_999);
        assert_eq!(
            record.created_at.to_unix_millis(),
            Some(1_000),
            "记录时间应是请求开始时刻，不是结束时刻"
        );
    }

    #[test]
    fn the_path_is_truncated() {
        let long = format!("/gateway/codex/r1/{}", "x".repeat(2_000));
        let tracker =
            RequestTracker::start("r1", ProviderId::Codex, &vendor(), "POST", &long, 1_000);
        assert_eq!(
            tracker.finish(1_000).path.chars().count(),
            REQUEST_PATH_MAX_LENGTH
        );
    }

    #[test]
    fn the_access_key_is_stripped_from_the_logged_path() {
        // `key` 是访问令牌，落进日志等同于泄露。
        let tracker = RequestTracker::start(
            "r1",
            ProviderId::Codex,
            &vendor(),
            "POST",
            "/external/codex/r1/v1/messages?key=super-secret&stream=true",
            1_000,
        );
        let path = tracker.finish(1_000).path;
        assert!(!path.contains("super-secret"), "{path}");
        assert!(path.contains("stream=true"), "{path}");
    }

    #[test]
    fn a_path_with_only_the_key_loses_the_question_mark() {
        let tracker = RequestTracker::start(
            "r1",
            ProviderId::Codex,
            &vendor(),
            "GET",
            "/x?key=abc",
            1_000,
        );
        assert_eq!(tracker.finish(1_000).path, "/x");
    }

    #[test]
    fn error_messages_are_sanitized_against_log_forgery() {
        // 换行必须在落库前折叠：否则一条错误能在日志里伪造出全新的行。
        let mut tracker = new_tracker();
        tracker.set_error(Some("HTTP_500"), Some("line one\r\nline two\ttabbed"));
        let record = tracker.finish(1_000);
        let message = record.error_message.expect("message");
        assert!(!message.contains('\n'), "{message}");
        assert!(!message.contains('\r'), "{message}");
        assert!(!message.contains('\t'), "{message}");
        assert_eq!(message, "line one  line two tabbed");
    }

    #[test]
    fn error_messages_are_truncated() {
        // 截断走 `aivc_core::text::clamp_raw`：它保留 `max_length - 1` 个字符再追加 `...`，
        // 因此结果是 `max_length + 2` 个字符。这里断言**可观察的契约**而不是那个算术，
        // 以免将来调整省略号形式时还要改这里。
        let mut tracker = new_tracker();
        let long = "x".repeat(2_000);
        tracker.set_error(None, Some(&long));
        let message = tracker.finish(1_000).error_message.expect("message");
        assert!(message.ends_with("..."), "截断后应能看出被截断：{message}");
        assert!(
            message.chars().count() <= REQUEST_ERROR_MAX_LENGTH + 2,
            "长度 {} 超出上限",
            message.chars().count()
        );

        // 未超长的内容原样保留。
        let mut tracker = new_tracker();
        tracker.set_error(None, Some("short"));
        assert_eq!(
            tracker.finish(1_000).error_message.as_deref(),
            Some("short")
        );
    }

    #[test]
    fn blank_error_fields_are_ignored() {
        let mut tracker = new_tracker();
        tracker.set_error(Some("  "), Some(""));
        let record = tracker.finish(1_000);
        assert!(record.error_code.is_none());
        assert!(record.error_message.is_none());
    }

    #[test]
    fn a_blank_model_is_treated_as_absent() {
        let mut tracker = new_tracker();
        tracker.set_model(Some("   "));
        assert!(tracker.finish(1_000).model.is_none());
        tracker.set_model(Some(" gpt-5 "));
        assert_eq!(tracker.finish(1_000).model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn the_upstream_status_is_recorded() {
        let mut tracker = new_tracker();
        tracker.set_upstream_status(429);
        assert_eq!(tracker.finish(1_000).upstream_status, Some(429));
    }

    #[test]
    fn every_request_gets_a_distinct_id() {
        let first = new_tracker().finish(1_000).request_id;
        let second = new_tracker().finish(1_000).request_id;
        assert_ne!(first.as_str(), second.as_str());
    }

    #[test]
    fn a_recorded_request_keeps_its_identity_fields() {
        let record = new_tracker().finish(1_000);
        assert_eq!(record.route_id, "r1");
        assert_eq!(record.provider_id, ProviderId::Codex);
        assert_eq!(record.method, "POST");
    }
}
