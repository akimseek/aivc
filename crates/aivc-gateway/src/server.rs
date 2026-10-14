//! 本地网关的 HTTP 代理服务器。
//!
//! # 请求流程
//!
//! ```text
//! 解析路径 → 找路由 → 鉴权 → 选供应商 → 转发（含故障转移）→ 回放响应
//! ```
//!
//! 每一步的判定逻辑都在各自的模块里（[`crate::routes`] / [`crate::failover`] /
//! [`crate::proxy`] / [`crate::response`]），本模块只负责把它们串起来并处理 I/O。
//!
//! # 响应为什么先探测再回放
//!
//! 上游经常用 `200 OK` 返回错误（尤其是 SSE 流的首个事件）。只看状态码会让这些错误
//! 被当成成功。因此转发前先读一小段响应体做判定，判定结束后把读到的字节**接回流的
//! 开头**再交给客户端——正常响应（含 SSE 首个 token）不能被吃掉。

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::{Body, HttpBody};
use axum::extract::State;
use axum::http::{HeaderMap, Request, Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::any;
use bytes::Bytes;
use futures::StreamExt;
use tokio::sync::{Mutex, RwLock};

use aivc_core::error::Result;
use aivc_core::gateway::{FailoverRule, GatewayAttempt, GatewayRequestRecord};
use aivc_core::id::VendorId;
use aivc_core::ports::BoxFuture;
use aivc_core::target::ProviderId;
use aivc_core::vendor::ApiVendor;

use crate::failover::{CircuitConfig, HealthTracker, choose_next_vendor, choose_vendor};
use crate::proxy::{
    FailureAction, FailureContext, NoNextAction, decide_after_failure, is_retryable_method,
    is_self_upstream, messages, plan_max_attempts, should_buffer_body,
};
use crate::record::RequestTracker;
use crate::response::{
    extract_gateway_response_error, has_structured_gateway_payload, matches_gateway_failover_error,
};
use crate::routes::{
    AuthRequest, RouteMode, RouteRegistry, VendorRoute, external_route_id,
    extract_external_provider, extract_provider, extract_route_id, has_external_access_token,
    has_route_token,
};
use crate::upstream::{build_upstream_headers, join_upstream_url};

/// 端口回退尝试次数。
///
/// 端口被占用时向后顺延：网关是本地辅助服务，换一个端口比拒绝启动有用得多。
pub const PORT_FALLBACK_ATTEMPTS: u16 = 20;

/// 优雅关闭的等待上限。
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

/// 网关运行时设置（快照）。
#[derive(Debug, Clone)]
pub struct GatewaySettings {
    /// 是否启用网关。
    pub enabled: bool,
    /// 期望监听端口。
    pub port: u16,
    /// 同一供应商连续失败多少次后切换。
    pub failure_threshold: usize,
    /// 熔断参数。
    pub circuit: CircuitConfig,
    /// 是否启用外部访问 API。
    pub external_enabled: bool,
    /// 外部访问令牌的 SHA-256（十六进制）。
    pub external_token_hash: Option<String>,
    /// 上游超时。
    pub upstream_timeout: Duration,
    /// 整次请求的总预算，所有上游尝试和响应流共用。
    pub request_timeout: Duration,
}

impl Default for GatewaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 8787,
            failure_threshold: 2,
            circuit: CircuitConfig::default(),
            external_enabled: false,
            external_token_hash: None,
            upstream_timeout: Duration::from_millis(crate::proxy::upstream_timeout_ms()),
            request_timeout: Duration::from_millis(crate::proxy::upstream_timeout_ms()),
        }
    }
}

/// 供应商与规则的来源。
///
/// 由上层实现（通常接仓储）。抽成 trait 是为了让网关的测试可以只提供内存数据，
/// 而不需要准备数据库。
pub trait GatewaySource: Send + Sync {
    /// 当前供应商快照。
    fn vendors(&self) -> BoxFuture<'_, Vec<ApiVendor>>;
    /// 故障识别规则。
    fn failover_rules(&self) -> BoxFuture<'_, Vec<FailoverRule>>;
    /// 运行时设置。
    fn settings(&self) -> GatewaySettings;

    /// 落地一条请求记录。
    ///
    /// 默认空实现：网关的转发与故障转移不依赖记录是否成功，因此未接入持久化的调用方
    /// （测试、只做转发的场景）不必实现它。真实应用接仓储后覆盖。
    ///
    /// 实现方**不应**在这里返回错误——记录失败不该影响已经完成的转发。
    fn record(&self, record: GatewayRequestRecord) -> BoxFuture<'_, ()> {
        let _ = record;
        Box::pin(async {})
    }

    /// 保存汇总及尝试明细；默认实现兼容仅关心汇总的来源。
    fn record_with_attempts(
        &self,
        record: GatewayRequestRecord,
        _attempts: Vec<GatewayAttempt>,
    ) -> BoxFuture<'_, ()> {
        self.record(record)
    }

    /// 重新读取设置快照。
    ///
    /// 默认空实现：设置已经在 [`GatewaySource::settings`] 里以同步快照形式提供，
    /// 内存来源不需要刷新。由仓储支撑的来源覆盖它去重读文件。
    fn refresh_settings(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// 网关共享状态。
/// 一次路由切换的通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchNotice {
    /// 路由编号（即终端绑定的标识）。
    pub route_id: String,
    /// 切换后的供应商。
    pub vendor_id: String,
    /// 切换原因。手动切换为 `None`，模式变更为 `mode`。
    pub reason: Option<String>,
    /// 切换后的模式。
    pub mode: RouteMode,
}

/// 切换通知钩子。
pub type SwitchHook = Arc<dyn Fn(&SwitchNotice) + Send + Sync>;

/// 健康状态的持久化钩子：读回与写回。
///
/// # 为什么是钩子而不是直接依赖仓储
///
/// 与 [`SwitchHook`] 同一个模式：网关不认识数据库，由组装根把两个动作注入进来。
/// 这样五处测试替身**一个都不用改**——没装钩子时就是「不持久化」，与从前一致。
///
/// # 为什么必须持久化
///
/// 现存的两个问题都源于它没接上：
///
/// 1. **熔断活不过一次重启**：转发路径读的是内存里的 `HealthTracker`，而它从不从库里读回，
///    于是一台已经被熔断的供应商，重启之后会被从头重试。
/// 2. **界面上的健康视图恒为空**：`gateway:get-vendor-health` 读的是数据库，
///    而那张表从来没有写入方——那不是「都健康」，是「什么都没有」。
///
/// 现网两边都做：每个请求前 `hydrateGatewayVendorHealth()`，失败/成功时写回。
/// 这里改成**启动时读回一次 + 变化时写回**：内存里的失败窗口是有状态的，
/// 每个请求都 hydrate 会把窗口清掉，反而让「窗口内 N 次失败」的判定失效。
#[derive(Clone)]
pub struct HealthHook {
    /// 读回全部已持久化的健康状态。
    pub load: LoadHealth,
    /// 写回一条健康状态。
    pub save: SaveHealth,
}

/// 读回健康状态的动作。
pub type LoadHealth =
    Arc<dyn Fn() -> BoxFuture<'static, Vec<aivc_core::gateway::GatewayVendorHealth>> + Send + Sync>;

/// 写回一条健康状态的动作。
pub type SaveHealth =
    Arc<dyn Fn(aivc_core::gateway::GatewayVendorHealth) -> BoxFuture<'static, ()> + Send + Sync>;

impl std::fmt::Debug for HealthHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HealthHook")
    }
}

/// 网关的共享状态：路由表、健康、数据来源与监听地址。
pub struct GatewayState {
    routes: RwLock<RouteRegistry>,
    /// 切换通知钩子；由组装根注入，网关不依赖事件机制（分层）。
    ///
    /// 用**标准库**锁而不是 tokio 的：钩子只在同步的 `notify_switch` 里读，
    /// 且临界区极短（读一个 `Arc` 再调用），不需要异步等待。
    switch_hook: std::sync::RwLock<Option<SwitchHook>>,
    health: Mutex<HealthTracker>,
    /// 健康钩子；未装时即「不持久化」。
    health_hook: std::sync::RwLock<Option<HealthHook>>,
    /// 每个供应商最近一次**成功**落库的时刻（毫秒），用于节流。
    ///
    /// 节流值的来源见 `aivc_core::constants::GATEWAY_HEALTH_SUCCESS_PERSIST_INTERVAL_MS`。
    health_persisted_at: Mutex<std::collections::HashMap<VendorId, i64>>,
    source: Arc<dyn GatewaySource>,
    client: reqwest::Client,
    address: RwLock<String>,
}

impl std::fmt::Debug for GatewayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayState")
            .field(
                "address",
                &self.address.try_read().map(|value| value.clone()),
            )
            .finish_non_exhaustive()
    }
}

impl GatewayState {
    /// 构造。
    pub fn new(source: Arc<dyn GatewaySource>) -> Result<Self> {
        let settings = source.settings();
        let client = reqwest::Client::builder()
            .timeout(settings.upstream_timeout)
            .connect_timeout(settings.upstream_timeout.min(Duration::from_secs(10)))
            .read_timeout(settings.upstream_timeout.min(Duration::from_secs(60)))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if is_loopback_host(attempt.url()) || attempt.previous().len() >= MAX_REDIRECTS {
                    // 见 `is_loopback_host` 与常量文档。
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|error| aivc_core::error::WslError::CommandFailed {
                message: format!("无法构造网关 HTTP 客户端：{error}"),
            })?;
        Ok(Self {
            routes: RwLock::new(RouteRegistry::new()),
            switch_hook: std::sync::RwLock::new(None),
            health: Mutex::new(HealthTracker::new()),
            health_hook: std::sync::RwLock::new(None),
            health_persisted_at: Mutex::new(std::collections::HashMap::new()),
            source,
            client,
            address: RwLock::new(String::new()),
        })
    }

    /// 装入健康持久化钩子。
    ///
    /// **必须在 `serve` 之前调用**：读回那一步由 [`Self::hydrate_health`] 在启动时做一次。
    pub fn set_health_hook(&self, hook: HealthHook) {
        if let Ok(mut slot) = self.health_hook.write() {
            *slot = Some(hook);
        }
    }

    /// 从钩子把健康状态读回内存。
    ///
    /// # 为什么在启动时读一次，而不是像现网那样每个请求都读
    ///
    /// 现网每次请求都 `hydrateGatewayVendorHealth()`，而内存里的失败记录是**有窗口**的
    /// （`HealthState.failures` 只保留窗口内的失败）。每个请求都 hydrate 会把窗口清成
    /// 「开着的熔断用一个占位时间戳」，于是「窗口内 N 次失败」的判定基本失效。
    ///
    /// 读一次、之后靠写回同步，既让熔断活过重启，又保住窗口语义。
    pub async fn hydrate_health(&self) {
        let Some(hook) = self.health_hook() else {
            return;
        };
        let stored = (hook.load)().await;
        if stored.is_empty() {
            return;
        }
        tracing::debug!(count = stored.len(), "已从存储读回供应商健康状态");
        self.health.lock().await.hydrate(stored, now_ms());
    }

    /// 记一次供应商失败：更新内存，**并落库**。
    pub async fn record_vendor_failure(
        &self,
        vendor: &ApiVendor,
        reason: &str,
        circuit: CircuitConfig,
    ) {
        let health = {
            let mut tracker = self.health.lock().await;
            tracker.record_failure(vendor, reason, circuit, now_ms());
            tracker.get(&vendor.id).map(|state| state.health.clone())
        };
        if let Some(health) = health {
            self.save_health(health).await;
        }
    }

    /// 记一次供应商成功：更新内存；**落库按固定间隔节流**。
    ///
    /// 成功是高频事件，每次落库会把健康表变成写放大。失败不节流：失败很少，
    /// 而且「刚熔断」这件事最需要跨重启记住。
    pub async fn record_vendor_success(&self, vendor: &ApiVendor) {
        let (health, recovered) = {
            let mut tracker = self.health.lock().await;
            let recovered = tracker.get(&vendor.id).is_some_and(|state| {
                state.health.status != aivc_core::gateway::GatewayVendorHealthStatus::Healthy
            });
            tracker.record_success(vendor, now_ms());
            (
                tracker.get(&vendor.id).map(|state| state.health.clone()),
                recovered,
            )
        };
        let Some(health) = health else {
            return;
        };
        let now = now_ms();
        {
            let mut persisted = self.health_persisted_at.lock().await;
            let too_soon = persisted.get(&vendor.id).is_some_and(|last| {
                now.saturating_sub(*last)
                    < aivc_core::constants::GATEWAY_HEALTH_SUCCESS_PERSIST_INTERVAL_MS
            });
            if too_soon && !recovered {
                return;
            }
            persisted.insert(vendor.id.clone(), now);
        }
        self.save_health(health).await;
    }

    /// 送一条健康状态给钩子；未装钩子时是空操作。
    async fn save_health(&self, health: aivc_core::gateway::GatewayVendorHealth) {
        let Some(hook) = self.health_hook() else {
            return;
        };
        (hook.save)(health).await;
    }

    /// 取一份健康钩子的两个动作（`Arc` 克隆，代价极小）。
    ///
    /// 用标准库锁：只在同步的一小段里克隆两个 `Arc`，不跨越 await。
    fn health_hook(&self) -> Option<HealthHook> {
        let slot = self.health_hook.read().ok()?;
        slot.as_ref().cloned()
    }

    /// 注册一条路由。
    pub async fn register_route(&self, route: VendorRoute) {
        self.routes.write().await.insert(route);
    }

    /// 路由切换的通知载荷。
    ///
    /// 字段与现网的 `gateway:vendor-switched` 事件对齐（`vendorId` / `reason` / `mode`）。
    ///
    /// **一处命名差异**：现网把它绑定的终端叫 `terminalId`，而这里用 `routeId`——
    /// 路由编号就是终端绑定的标识，两者在本实现里是同一个值，不额外维护一个别名字段。
    ///
    /// 改动路由当前的供应商并通知。
    ///
    /// 两条路径都用它，靠 `reason` 区分：
    ///
    /// | 场景 | `reason` |
    /// | --- | --- |
    /// | 用户手动切换（`vendor:route-switch`） | `None` |
    /// | 请求前发现原供应商不可用，候选池换了一家 | `"failure"`（原供应商已熔断）/ `"candidate-pool"` |
    ///
    /// **自动故障转移不走这里**：它改的是「本次请求用的上游」，对该终端后续请求的
    /// 当前供应商不产生持久影响——那一条是 [`Self::notify_failover`]。
    pub async fn switch_route_vendor(
        &self,
        route_id: &str,
        vendor_id: &str,
        reason: Option<&str>,
    ) -> Option<VendorRoute> {
        if vendor_id.trim().is_empty() {
            return None;
        }
        let updated = {
            let mut routes = self.routes.write().await;
            let route = routes.get_mut(route_id)?;
            vendor_id.clone_into(&mut route.vendor_id);
            route.clone()
        };
        self.notify_switch(&updated, reason);
        Some(updated)
    }

    /// 候选池变化后立即重新评估动态终端，包括池曾为空、现在重新有候选的情况。
    ///
    /// 请求开始时仍会再次校验候选池；这里的主动迁移用于让正在使用的终端
    /// 立即显示真实供应商，并避免用户必须先发出下一条请求才能看到切换。
    pub async fn reconcile_candidate_pool(&self) {
        let vendors = self.source.vendors().await;
        let routes = self.routes.read().await.dynamic_routes();
        let now = now_ms();
        for route in routes {
            let next_id = {
                let mut health = self.health.lock().await;
                choose_vendor(&vendors, route.provider_id, Some(&route.vendor_id), |id| {
                    health.is_circuit_open(id, now)
                })
                .map(|vendor| vendor.id.as_str().to_owned())
            };
            let Some(next_id) = next_id.filter(|id| *id != route.vendor_id) else {
                continue;
            };
            let updated = {
                let mut registry = self.routes.write().await;
                let Some(current) = registry.get_mut(&route.route_id) else {
                    continue;
                };
                // 不覆盖读取候选池期间发生的手动切换或锁定。
                if current.mode != RouteMode::Dynamic || current.vendor_id != route.vendor_id {
                    continue;
                }
                current.vendor_id = next_id;
                current.clone()
            };
            self.notify_switch(&updated, Some("candidate-pool"));
        }
    }

    /// 切换路由模式。
    pub async fn set_route_mode(&self, route_id: &str, mode: RouteMode) -> Option<VendorRoute> {
        let updated = {
            let mut routes = self.routes.write().await;
            let route = routes.get_mut(route_id)?;
            route.mode = mode;
            route.clone()
        };
        self.notify_switch(&updated, Some("mode"));
        Some(updated)
    }

    /// 装入切换通知钩子。
    pub fn set_switch_hook(&self, hook: SwitchHook) {
        if let Ok(mut slot) = self.switch_hook.write() {
            *slot = Some(hook);
        }
    }

    /// 通知一次切换。
    ///
    /// 钩子不存在或读锁失败时静默跳过：**通知失败不该影响切换本身**——
    /// 路由已经改好了，界面晚一拍知道不影响功能。
    fn notify_switch(&self, route: &VendorRoute, reason: Option<&str>) {
        if route.external {
            // 外部路由不绑定终端，界面没有对应的「当前供应商」可更新。
            return;
        }
        self.emit_notice(&SwitchNotice {
            route_id: route.route_id.clone(),
            vendor_id: route.vendor_id.clone(),
            reason: reason.map(str::to_owned),
            mode: route.mode,
        });
    }

    /// 通知一次**自动**故障转移。
    ///
    /// # 为什么不复用 [`Self::switch_route_vendor`]
    ///
    /// 自动转移**不修改路由的当前供应商**——它只影响本次请求。因此这里既不能、也不该
    /// 走那条会持久化改动的路径。
    ///
    /// # 但界面必须知道
    ///
    /// 少了这条通知，界面会一直显示一个**已经不是实际服务方**的供应商：请求正由第二家
    /// 服务，状态栏却写着第一家。用户看不到任何异常，只有账单对不上。
    ///
    /// 与现网一致，`reason` 用 `"failure"`（上游失败导致）——界面据此显示成
    /// 「已自动切换」而不是用户的手动操作。
    pub fn notify_failover(&self, route: &VendorRoute, vendor_id: &VendorId) {
        if route.external {
            return;
        }
        self.emit_notice(&SwitchNotice {
            route_id: route.route_id.clone(),
            vendor_id: vendor_id.as_str().to_owned(),
            reason: Some("failure".to_owned()),
            mode: route.mode,
        });
    }

    /// 送出一次切换通知。
    fn emit_notice(&self, notice: &SwitchNotice) {
        if notice.vendor_id.trim().is_empty() {
            return;
        }
        let Ok(slot) = self.switch_hook.read() else {
            return;
        };
        let Some(hook) = slot.as_ref() else {
            return;
        };
        hook(notice);
    }

    /// 移除一条路由。
    pub async fn remove_route(&self, route_id: &str) -> Option<VendorRoute> {
        self.routes.write().await.remove(route_id)
    }

    /// 当前监听地址（形如 `http://127.0.0.1:8787`）。
    pub async fn address(&self) -> String {
        self.address.read().await.clone()
    }

    /// 设置监听地址（端口回退后由启动流程写入）。
    pub async fn set_address(&self, address: String) {
        *self.address.write().await = address;
    }

    /// 健康状态快照。
    pub async fn health_snapshot(&self) -> Vec<aivc_core::gateway::GatewayVendorHealth> {
        self.health.lock().await.snapshot()
    }

    /// 重置**内存里**的供应商健康状态（`None` 表示全部）。
    ///
    /// # 为什么要与数据库那份一起做
    ///
    /// 转发路径读的是内存里的 `HealthTracker`，而 `gateway:get-vendor-health` 读的是
    /// 数据库。`gateway:reset-vendor-health` 过去只清数据库，于是：
    ///
    /// - 界面上那一栏本来就是空的（没有写入方）；
    /// - 而**真正生效的熔断状态在内存里纹丝不动**——按钮按下去行为完全不变。
    ///
    /// 这一步让「重置健康」对**活的**状态生效。
    pub async fn reset_health(&self, vendor_id: Option<&VendorId>) {
        self.health.lock().await.reset(vendor_id);
        let mut persisted = self.health_persisted_at.lock().await;
        match vendor_id {
            Some(id) => {
                persisted.remove(id);
            }
            None => persisted.clear(),
        }
    }

    /// 当前设置快照。
    ///
    /// 转发路径上多处要读设置，而它是同步的——见 [`GatewaySource::settings`] 的说明。
    #[must_use]
    pub fn settings(&self) -> GatewaySettings {
        self.source.settings()
    }

    /// 重新读取设置快照。
    pub async fn refresh_settings(&self) {
        self.source.refresh_settings().await;
    }
}

/// 处理一次代理请求。
///
/// # 为什么要包一层
///
/// 现网把记录写在 `finally` 里，因此**每条路径都会留下记录**——包括路由不存在、
/// 鉴权失败这些在早期就返回的情况。Rust 没有 `finally`，而 `handle_inner` 里有十几个
/// 提前返回点；逐个补记录必然漏掉某一个。
///
/// 因此把记录放在调用方：内层负责转发，外层无论内层怎么返回都记录一次。这也让
/// 「记录失败不影响已完成的转发」成为结构上的保证，而不是靠约定。
async fn handle(State(state): State<Arc<GatewayState>>, request: Request<Body>) -> Response<Body> {
    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let started_at = now_ms();
    let tracker = RequestTracker::start(
        "",
        ProviderId::Codex,
        &VendorId::from(""),
        &method,
        &path,
        started_at,
    );

    // 所有供应商尝试和响应转发共享 10 分钟预算，不能随候选数量倍增。
    let mut deadline = tokio::time::Instant::now() + state.settings().request_timeout;
    let mut completion = RequestCompletion::new(Arc::clone(&state), tracker);
    let mut response = if let Ok(response) = tokio::time::timeout_at(
        deadline,
        handle_inner(&state, &mut completion.tracker, request),
    )
    .await
    {
        response
    } else {
        let tracker = &mut completion.tracker;
        tracker.mark_timeout();
        tracker.set_error(Some("UPSTREAM_TIMEOUT"), Some(messages::UPSTREAM_TIMEOUT));
        deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        json_error(StatusCode::GATEWAY_TIMEOUT, messages::UPSTREAM_TIMEOUT)
    };
    completion.upstream = response.extensions_mut().remove::<CompletedUpstream>();
    let (parts, body) = response.into_parts();
    if method == "HEAD" || body.is_end_stream() {
        completion.finish();
        return Response::from_parts(parts, body);
    }
    let stream = futures::stream::unfold(
        (body.into_data_stream(), completion),
        move |(mut body, mut completion)| async move {
            if completion.finished {
                return None;
            }
            match tokio::time::timeout_at(deadline, body.next()).await {
                Ok(Some(Ok(bytes))) => {
                    completion.observe(&bytes);
                    Some((Ok::<_, std::io::Error>(bytes), (body, completion)))
                }
                Ok(Some(Err(error))) => {
                    completion.fail("UPSTREAM_STREAM_ERROR", &error.to_string(), false);
                    completion.finish();
                    Some((
                        Err(std::io::Error::other(error.to_string())),
                        (body, completion),
                    ))
                }
                Ok(None) => {
                    completion.finish();
                    None
                }
                Err(_) => {
                    completion.fail("UPSTREAM_TIMEOUT", messages::UPSTREAM_TIMEOUT, true);
                    completion.finish();
                    Some((
                        Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            messages::UPSTREAM_TIMEOUT,
                        )),
                        (body, completion),
                    ))
                }
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(stream))
}

#[derive(Clone)]
struct CompletedUpstream {
    vendor: ApiVendor,
    circuit: CircuitConfig,
}

/// 日志在响应结束时落地；被取消的处理任务或响应体也通过 Drop 留下记录。
struct RequestCompletion {
    state: Arc<GatewayState>,
    tracker: RequestTracker,
    finished: bool,
    upstream: Option<CompletedUpstream>,
    prefix: Vec<u8>,
    pending: Vec<u8>,
    usage: aivc_core::gateway::GatewayUsage,
    explicit_total: bool,
    stream_error: Option<String>,
}

impl RequestCompletion {
    fn new(state: Arc<GatewayState>, tracker: RequestTracker) -> Self {
        Self {
            state,
            tracker,
            finished: false,
            upstream: None,
            prefix: Vec::new(),
            pending: Vec::new(),
            usage: aivc_core::gateway::GatewayUsage::default(),
            explicit_total: false,
            stream_error: None,
        }
    }

    fn fail(&mut self, code: &str, message: &str, timeout: bool) {
        if !self.finished {
            let tracker = &mut self.tracker;
            if timeout {
                tracker.mark_timeout();
            }
            tracker.set_error(Some(code), Some(message));
            self.stream_error = Some(aivc_core::gateway::sanitize_error_message(message));
        }
    }

    fn observe_payload(&mut self, payload: &[u8]) {
        let text = String::from_utf8_lossy(payload);
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(
            text.trim().strip_prefix("data:").unwrap_or(&text).trim(),
        ) {
            self.observe_usage(&json);
        }
        if self.upstream.is_some()
            && let Some(message) = extract_gateway_response_error(&text)
        {
            self.fail("UPSTREAM_RESPONSE_ERROR", &message, false);
        }
    }

    fn observe_usage(&mut self, json: &serde_json::Value) {
        if let Some(usage) = aivc_core::gateway_usage::parse_gateway_usage(json) {
            self.explicit_total |= [
                "/usage/total_tokens",
                "/usage/totalTokenCount",
                "/response/usage/total_tokens",
                "/message/usage/total_tokens",
                "/usageMetadata/totalTokenCount",
                "/total_tokens",
                "/totalTokenCount",
            ]
            .iter()
            .any(|pointer| {
                json.pointer(pointer)
                    .is_some_and(serde_json::Value::is_number)
            });
            self.usage.merge(&usage);
        }
    }

    fn observe(&mut self, bytes: &[u8]) {
        self.tracker.add_bytes_out(bytes.len() as u64);
        let limit = aivc_core::constants::GATEWAY_REQUEST_LIMITS.response_capture_bytes;
        self.prefix
            .extend_from_slice(&bytes[..bytes.len().min(limit.saturating_sub(self.prefix.len()))]);
        // 按完整行解析 SSE，处理跨网络分块的 JSON，用量位于长流尾部时也能捕获。
        for segment in bytes.split_inclusive(|byte| *byte == b'\n') {
            self.pending.extend_from_slice(
                &segment[..segment.len().min(limit.saturating_sub(self.pending.len()))],
            );
            if segment.last() == Some(&b'\n') {
                let payload = std::mem::take(&mut self.pending);
                self.observe_payload(&payload);
            }
        }
    }

    fn finish(&mut self) {
        if self.finished {
            return;
        }
        let pending = std::mem::take(&mut self.pending);
        self.observe_payload(&pending);
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&self.prefix) {
            self.observe_usage(&json);
        }
        if !self.explicit_total
            && (self.usage.input_tokens.is_some() || self.usage.output_tokens.is_some())
        {
            self.usage.total_tokens = Some(
                self.usage
                    .input_tokens
                    .unwrap_or(0)
                    .saturating_add(self.usage.output_tokens.unwrap_or(0)),
            );
        }
        if let Some(pricing) = self
            .upstream
            .as_ref()
            .and_then(|upstream| upstream.vendor.pricing)
        {
            let input = self
                .usage
                .input_tokens
                .zip(pricing.input_per_million_usd)
                .map(|(tokens, rate)| tokens as f64 * rate / 1_000_000.0);
            let output = self
                .usage
                .output_tokens
                .zip(pricing.output_per_million_usd)
                .map(|(tokens, rate)| tokens as f64 * rate / 1_000_000.0);
            if input.is_some() || output.is_some() {
                self.usage.cost_usd = Some(input.unwrap_or(0.0) + output.unwrap_or(0.0));
            }
        }
        self.finished = true;
        let tracker = &mut self.tracker;
        tracker.complete_attempt(now_ms());
        if self.usage != aivc_core::gateway::GatewayUsage::default() {
            tracker.set_usage(Some(self.usage.clone()));
        }
        let record = tracker.finish(now_ms());
        let attempts = tracker.attempts();
        let state = Arc::clone(&self.state);
        let upstream = self.upstream.take().or_else(|| {
            tracker.active_vendor().map(|vendor| CompletedUpstream {
                vendor,
                circuit: state.settings().circuit,
            })
        });
        let stream_error = self.stream_error.take();
        let final_error = record.error_message.clone();
        tokio::spawn(async move {
            if let Some(upstream) = upstream {
                if let Some(message) = stream_error.or_else(|| {
                    (record.outcome == aivc_core::gateway::GatewayOutcome::Timeout)
                        .then_some(final_error)
                        .flatten()
                }) {
                    state
                        .record_vendor_failure(&upstream.vendor, &message, upstream.circuit)
                        .await;
                } else if record.outcome == aivc_core::gateway::GatewayOutcome::Ok {
                    state.record_vendor_success(&upstream.vendor).await;
                }
            }
            state.source.record_with_attempts(record, attempts).await;
        });
    }
}

impl Drop for RequestCompletion {
    fn drop(&mut self) {
        if !self.finished {
            let tracker = &mut self.tracker;
            tracker.set_error(
                Some("CLIENT_ABORTED"),
                Some("client disconnected before response completion"),
            );
            // set_error 保持错误结果；客户端主动断开由独立结果表示。
            tracker.mark_client_aborted();
            self.finish();
        }
    }
}

/// 转发一次请求。
#[allow(clippy::too_many_lines)]
async fn handle_inner(
    state: &Arc<GatewayState>,
    tracker: &mut RequestTracker,
    request: Request<Body>,
) -> Response<Body> {
    let settings = state.source.settings();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    let method = request.method().clone();
    let headers = request.headers().clone();
    // 请求体必须在借用之前取出：&Request 无法移出 body。
    let declared_length = header_str(&headers, header::CONTENT_LENGTH.as_str())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let raw_body = request.into_body();
    if !settings.enabled {
        tracker.set_error(Some("GATEWAY_DISABLED"), Some("gateway is disabled"));
        return json_error(StatusCode::SERVICE_UNAVAILABLE, "gateway is disabled");
    }

    // 路由解析 + 鉴权。
    //
    // 两类路径两个分支（2026-10-02 统一外部接入）：
    // - `/gateway/<平台>/<编号>/…`：终端路由，按编号查注册表；
    // - `/<平台>/…`：外部接入，每平台一条**惰性注册**的合成路由
    //   （`external-<平台>`，动态候选池、共享令牌鉴权）。
    let external_provider = extract_external_provider(&path);
    if let Some(provider) = external_provider {
        // 命中一次之后只读：先读锁查，缺了再写锁补。
        if state.routes.read().await.external_route(provider).is_none() {
            state.routes.write().await.ensure_external_route(provider);
        }
    }
    let route = {
        let routes = state.routes.read().await;
        let route_id = match external_provider {
            Some(provider) => external_route_id(provider),
            None => extract_route_id(&path),
        };
        // 网关请求结果与供应商健康分别记录；拒绝接入不更新任何供应商健康。
        let Some(route) = routes.get(&route_id).cloned() else {
            tracker.set_error(Some("ROUTE_NOT_FOUND"), Some(messages::ROUTE_NOT_FOUND));
            return json_error(StatusCode::NOT_FOUND, messages::ROUTE_NOT_FOUND);
        };
        tracker.set_route(&route.route_id, route.provider_id);
        // 平台必须与路径一致：终端路由看第 2 段，外部路由看第 1 段。
        let provider_matches = match external_provider {
            Some(provider) => route.provider_id == provider,
            None => extract_provider(&path) == Some(route.provider_id),
        };
        if !provider_matches {
            tracker.set_error(Some("ROUTE_NOT_FOUND"), Some(messages::ROUTE_NOT_FOUND));
            return json_error(StatusCode::NOT_FOUND, messages::ROUTE_NOT_FOUND);
        }
        let auth = AuthRequest {
            authorization: header_str(&headers, header::AUTHORIZATION.as_str()),
            x_api_key: header_str(&headers, "x-api-key"),
            x_goog_api_key: header_str(&headers, "x-goog-api-key"),
            query_keys: AuthRequest::from_query(query.as_deref()),
        };
        if route.external {
            if !settings.external_enabled {
                tracker.set_error(
                    Some("EXTERNAL_API_DISABLED"),
                    Some(messages::EXTERNAL_API_DISABLED),
                );
                return json_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    messages::EXTERNAL_API_DISABLED,
                );
            }
            let Some(hash) = settings.external_token_hash.as_deref() else {
                tracker.set_error(Some("UNAUTHORIZED"), Some(messages::UNAUTHORIZED));
                return json_error(StatusCode::UNAUTHORIZED, messages::UNAUTHORIZED);
            };
            if !has_external_access_token(&auth, hash) {
                tracker.set_error(Some("UNAUTHORIZED"), Some(messages::UNAUTHORIZED));
                return json_error(StatusCode::UNAUTHORIZED, messages::UNAUTHORIZED);
            }
        } else if !has_route_token(&auth, &route.local_token) {
            tracker.set_error(Some("UNAUTHORIZED"), Some(messages::UNAUTHORIZED));
            return json_error(StatusCode::UNAUTHORIZED, messages::UNAUTHORIZED);
        }
        route
    };

    tracker.set_route(&route.route_id, route.provider_id);

    let vendors = state.source.vendors().await;
    let rules = state.source.failover_rules().await;

    // 选供应商：锁定路由只用其当前供应商，动态路由走候选池。
    let selected = match route.mode {
        RouteMode::Locked => vendors
            .iter()
            .find(|vendor| {
                vendor.id.as_str() == route.vendor_id && vendor.provider_id == route.provider_id
            })
            .filter(|vendor| vendor.is_route_eligible())
            .cloned(),
        RouteMode::Dynamic => {
            let mut health = state.health.lock().await;
            let mut is_open = |id: &VendorId| {
                health.is_circuit_open(
                    id,
                    aivc_core::time::Timestamp::now()
                        .to_unix_millis()
                        .unwrap_or(0),
                )
            };
            choose_vendor(
                &vendors,
                route.provider_id,
                Some(&route.vendor_id),
                &mut is_open,
            )
            .cloned()
        }
    };
    let Some(mut attempt_vendor) = selected else {
        return vendor_unavailable_response(state, tracker, &route, &vendors).await;
    };
    tracker.set_vendor(&attempt_vendor.id);

    // **请求前的候选池调整。**
    //
    // 原供应商被禁用、删除或熔断时，动态路由会在请求前换一家。这与「请求失败后转移」
    // 是两件不同的事：它不是异常，**但同样要落回路由并通知界面**——否则状态栏会一直
    // 显示一个已经不存在的供应商，而请求其实由别人在服务。
    //
    // 这里**必须显式写回**：`handle_inner` 手里的是路由的**克隆**（`routes.get().cloned()`），
    // 改它不会影响注册表。现网是直接改内存里的 route 对象，因此移植时这一步漏了。
    //
    // 现网把这种情形分成两种原因：原供应商已被熔断是 `failure`，其余（被禁用/删除/配置
    // 不全）是 `candidate-pool`；界面只对前者显示异常提示。
    if attempt_vendor.id.as_str() != route.vendor_id {
        let previous_open = {
            let mut health = state.health.lock().await;
            health.is_circuit_open(&VendorId::from(route.vendor_id.clone()), now_ms())
        };
        let reason = if previous_open {
            "failure"
        } else {
            "candidate-pool"
        };
        state
            .switch_route_vendor(&route.route_id, attempt_vendor.id.as_str(), Some(reason))
            .await;
    }

    // 接收上限与重试上限分开：2–32 MiB 正文完整转发一次，不重试。
    let body_bytes = match read_body(raw_body, declared_length).await {
        Ok(bytes) => bytes,
        Err(response) => {
            tracker.set_error(
                Some("INVALID_REQUEST_BODY"),
                Some("request body is unreadable or exceeds 32 MiB"),
            );
            return *response;
        }
    };
    tracker.add_bytes_in(body_bytes.as_ref().map_or(0, |bytes| bytes.len() as u64));
    // 包括 chunked 请求在内都按实际字节读取；非 JSON 正文没有模型字段。
    // 取成 `String` 而不是借用——解析出的 `Value` 是局部变量。
    let model = body_bytes
        .as_ref()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok())
        .and_then(|value| {
            value
                .get("model")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        });
    tracker.set_model(model.as_deref());
    let body_buffered = body_bytes
        .as_ref()
        .is_some_and(|bytes| should_buffer_body(bytes.len() as u64));
    let suffix = strip_route_prefix(&path, &route);
    let candidate_count = vendors
        .iter()
        .filter(|vendor| vendor.provider_id == route.provider_id && vendor.is_route_eligible())
        .count();
    let max_attempts = match route.mode {
        RouteMode::Locked => 1,
        // 唯一候选按锁定供应商处理，失败原样返回；下一次请求仍向它转发。
        RouteMode::Dynamic if candidate_count == 1 => 1,
        RouteMode::Dynamic => plan_max_attempts(
            body_buffered,
            method.as_str(),
            settings.failure_threshold,
            candidate_count,
        ),
    };

    let mut attempted: Vec<VendorId> = Vec::new();
    let mut failures_on_vendor = 0usize;
    // key 是网关的访问令牌，两种路由都不能把它透传给供应商。
    let upstream_search = crate::routes::external_upstream_search(query.as_deref());
    let mut custom_failure_response = None;

    for attempt in 0..max_attempts {
        let attempt_started_at = now_ms();
        if attempt > 0 {
            tracker.record_retry();
        }
        tracker.begin_attempt();
        let upstream_url =
            match join_upstream_url(&attempt_vendor.api_base_url, &suffix, &upstream_search) {
                Ok(url) => url,
                Err(error) => {
                    tracker.set_error(Some("INVALID_UPSTREAM_URL"), Some(&error.to_string()));
                    return json_error(StatusCode::BAD_GATEWAY, &error.to_string());
                }
            };
        // 自我代理防护：否则请求会在网关与「上游」之间无限循环。
        if is_self_upstream(&upstream_url, &state.address().await) {
            state
                .record_vendor_failure(
                    &attempt_vendor,
                    messages::LOCAL_GATEWAY_UPSTREAM,
                    settings.circuit,
                )
                .await;
            tracker.set_error(
                Some("LOCAL_GATEWAY_UPSTREAM"),
                Some(messages::LOCAL_GATEWAY_UPSTREAM),
            );
            return json_error(StatusCode::BAD_GATEWAY, messages::LOCAL_GATEWAY_UPSTREAM);
        }

        let request_builder = state
            .client
            .request(method.clone(), &upstream_url)
            .timeout(settings.upstream_timeout)
            .headers(headers_from(&headers, &attempt_vendor));
        let request_builder = match &body_bytes {
            Some(bytes) => request_builder.body(bytes.clone()),
            None => request_builder,
        };

        tracker.start_upstream_attempt(&attempt_vendor, attempt_started_at);
        let outcome = request_builder.send().await;
        let response = match outcome {
            Ok(response) => response,
            Err(error) => {
                let error_message = upstream_error_message(&error);
                tracker.record_attempt(
                    &attempt_vendor,
                    attempt_started_at,
                    now_ms(),
                    None,
                    Some(&error_message),
                );
                // 连接失败：没有响应可回放。
                // 必须先算「还有没有候选」再取健康锁：`has_next` 内部也要取同一把锁，
                // 而 `tokio::sync::Mutex` 不可重入，在持锁时调用会直接死锁。
                let next_available =
                    has_next(state, &vendors, &route, &attempt_vendor, &attempted).await;
                state
                    .record_vendor_failure(&attempt_vendor, &error_message, settings.circuit)
                    .await;
                let action = decide_after_failure(
                    &FailureContext {
                        attempt,
                        max_attempts,
                        failures_on_vendor: failures_on_vendor + 1,
                        failure_threshold: settings.failure_threshold,
                        has_next_vendor: next_available,
                        // **超时也要可重试**：见 `a_hanging_vendor_is_abandoned_and_the_next_one_serves`。
                        // 老平台在超时后直接终止（`isRetryableFetchError` 里 `timeoutSignal.aborted`
                        // 返回 false），于是一台卡住的上游会把整次请求耗光——故障转移形同不存在。
                        retryable: true,
                    },
                    NoNextAction::PropagateError,
                );
                failures_on_vendor += 1;
                match action {
                    FailureAction::RetrySameVendor => continue,
                    FailureAction::FailoverToNext => {
                        if let Some(next) = take_next(
                            state,
                            &route,
                            &attempt_vendor,
                            &mut attempted,
                            &mut failures_on_vendor,
                        )
                        .await
                        {
                            tracker.switch_vendor(&next.id);
                            // 界面必须知道这次请求换了供应商，否则它显示的还是原来那一家。
                            state.notify_failover(&route, &next.id);
                            attempt_vendor = next;
                            continue;
                        }
                        return upstream_request_error(tracker, &error);
                    }
                    FailureAction::NoNext(_) | FailureAction::Stop => {
                        return upstream_request_error(tracker, &error);
                    }
                }
            }
        };

        let status = response.status();
        let status_code = status.as_u16();
        tracker.set_upstream_status(status_code);
        let probe = probe_response(response).await;
        let error_message = probe
            .error_message
            .clone()
            .or_else(|| probe.read_error.as_ref().map(ToString::to_string));
        let retryable_response = probe.read_error.is_some()
            || matches_gateway_failover_error(
                error_message.as_deref(),
                route.provider_id,
                &attempt_vendor.id,
                status_code,
                &rules,
            );
        let response_failed = !status.is_success() || error_message.is_some() || retryable_response;
        if response_failed {
            tracker.set_error(
                Some("UPSTREAM_RESPONSE_ERROR"),
                Some(
                    error_message
                        .as_deref()
                        .unwrap_or(&format!("HTTP {status_code}")),
                ),
            );
        }
        tracker.record_attempt(
            &attempt_vendor,
            attempt_started_at,
            now_ms(),
            Some(status_code),
            response_failed
                .then(|| {
                    error_message
                        .clone()
                        .unwrap_or_else(|| format!("HTTP {status_code}"))
                })
                .as_deref(),
        );
        if retryable_response && custom_failure_response.is_none() {
            custom_failure_response = rules
                .iter()
                .filter(|rule| {
                    rule.enabled
                        && rule.applies_to(route.provider_id, Some(&attempt_vendor.id))
                        && rule.matches(Some(status_code), error_message.as_deref())
                })
                .find_map(|rule| {
                    Some((
                        rule.custom_response_status?,
                        rule.custom_response_body.clone()?,
                    ))
                });
        }

        if retryable_response {
            // 同连接失败路径：先算候选再取锁。
            let next_available =
                has_next(state, &vendors, &route, &attempt_vendor, &attempted).await;
            state
                .record_vendor_failure(
                    &attempt_vendor,
                    error_message
                        .as_deref()
                        .unwrap_or(&format!("HTTP {status_code}")),
                    settings.circuit,
                )
                .await;
            let action = decide_after_failure(
                &FailureContext {
                    attempt,
                    max_attempts,
                    failures_on_vendor: failures_on_vendor + 1,
                    failure_threshold: settings.failure_threshold,
                    has_next_vendor: next_available,
                    retryable: true,
                },
                NoNextAction::ReplayLastResponse,
            );
            failures_on_vendor += 1;
            match action {
                FailureAction::RetrySameVendor => continue,
                FailureAction::FailoverToNext => {
                    if let Some(next) = take_next(
                        state,
                        &route,
                        &attempt_vendor,
                        &mut attempted,
                        &mut failures_on_vendor,
                    )
                    .await
                    {
                        // 与连接失败那条路径**必须完全一致**。
                        //
                        // 这两处曾经不一致：连接失败时会 `switch_vendor` + `record_retry`，
                        // 而「上游返回了可重试状态码」时两者都没有。后果是记录里的供应商
                        // 仍是**第一家**——也就是那个从没服务成功的供应商，`switched`
                        // 也永远是 false。界面上的故障转移视图因此什么都不会显示，
                        // 用量报表则把这次请求算在了一家没服务过它的供应商头上。
                        //
                        // 由 `a_failover_is_visible_in_the_request_record` 钉住。
                        tracker.switch_vendor(&next.id);
                        // 界面必须知道这次请求换了供应商，否则它显示的还是原来那一家。
                        state.notify_failover(&route, &next.id);
                        attempt_vendor = next;
                        continue;
                    }
                    return replay_failure(probe, custom_failure_response.as_ref());
                }
                // 回放最后一次上游响应：让客户端看到真实错误而不是 502。
                // 没有其他候选（或已达上限）时回放最后一次上游响应，
                // 让客户端看到真实错误而不是 502。
                FailureAction::NoNext(_) | FailureAction::Stop => {
                    return replay_failure(probe, custom_failure_response.as_ref());
                }
            }
        }

        if response_failed {
            // 未命中规则的普通 4xx 可能是调用参数问题，不当成供应商健康故障。
            if status.is_server_error()
                || matches!(status_code, 401 | 403 | 408 | 429)
                || status.is_success()
            {
                state
                    .record_vendor_failure(
                        &attempt_vendor,
                        error_message
                            .as_deref()
                            .unwrap_or(&format!("HTTP {status_code}")),
                        settings.circuit,
                    )
                    .await;
            }
            return replay_failure(probe, custom_failure_response.as_ref());
        }
        tracker.mark_ok();
        let mut response = replay(probe);
        // 正常上游到完整响应结束才算成功，避免 SSE 中途出错却清除熔断。
        response.extensions_mut().insert(CompletedUpstream {
            vendor: attempt_vendor,
            circuit: settings.circuit,
        });
        return response;
    }

    tracker.set_error(
        Some("VENDOR_UNAVAILABLE"),
        Some(messages::NO_AVAILABLE_VENDOR),
    );
    json_error(StatusCode::BAD_GATEWAY, messages::NO_AVAILABLE_VENDOR)
}

/// 未选中上游时保持供应商为空，但明确区分候选池配置与暂时熔断。
async fn vendor_unavailable_response(
    state: &GatewayState,
    tracker: &mut RequestTracker,
    route: &VendorRoute,
    vendors: &[ApiVendor],
) -> Response<Body> {
    let candidates: Vec<_> = vendors
        .iter()
        .filter(|vendor| crate::failover::is_configured_candidate(vendor, route.provider_id))
        .collect();
    let retry_after = if route.mode == RouteMode::Dynamic && candidates.len() > 1 {
        let health = state.health.lock().await;
        candidates
            .iter()
            .filter_map(|vendor| {
                health
                    .get(&vendor.id)?
                    .health
                    .circuit_until
                    .as_ref()?
                    .to_unix_millis()
            })
            .min()
            .map(|until| (until.saturating_sub(now_ms()).max(1) + 999) / 1000)
    } else {
        None
    };
    let (code, message) = if let Some(seconds) = retry_after {
        (
            "VENDOR_CIRCUIT_OPEN",
            format!(
                "{} 的 {} 家候选供应商均处于熔断冷却期，本次请求未转发。约 {seconds} 秒后可重试；也可启用其他供应商。",
                route.provider_id.as_str(),
                candidates.len()
            ),
        )
    } else if route.mode == RouteMode::Locked {
        (
            "VENDOR_UNAVAILABLE",
            "锁定的供应商已停用、被删除或配置不完整，本次请求未转发。请手动切换供应商或解除锁定。"
                .to_owned(),
        )
    } else {
        (
            "VENDOR_UNAVAILABLE",
            format!(
                "{} 没有参与候选池且 API Key、端点配置完整的供应商，本次请求未转发。请检查该平台的供应商配置。",
                route.provider_id.as_str()
            ),
        )
    };
    tracker.set_error(Some(code), Some(&message));
    let mut response = json_error(StatusCode::SERVICE_UNAVAILABLE, &message);
    if let Some(seconds) = retry_after
        && let Ok(value) = seconds.to_string().parse()
    {
        response.headers_mut().insert(header::RETRY_AFTER, value);
    }
    response
}

/// reqwest 的顶层 Display 隐藏了 DNS、TLS、连接拒绝等真正的失败原因。
fn upstream_error_message(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        message.push_str(": ");
        message.push_str(&error.to_string());
        cause = error.source();
    }
    message
}

/// 网络超时和连接失败有不同的 HTTP 语义；所有停止重试的路径统一处理。
fn upstream_request_error(tracker: &mut RequestTracker, error: &reqwest::Error) -> Response<Body> {
    if error.is_timeout() {
        tracker.mark_timeout();
        tracker.set_error(Some("UPSTREAM_TIMEOUT"), Some(messages::UPSTREAM_TIMEOUT));
        json_error(StatusCode::GATEWAY_TIMEOUT, messages::UPSTREAM_TIMEOUT)
    } else {
        let message = upstream_error_message(error);
        tracker.set_error(Some("UPSTREAM_ERROR"), Some(&message));
        json_error(StatusCode::BAD_GATEWAY, &message)
    }
}

/// 当前时间（毫秒）。
fn now_ms() -> i64 {
    aivc_core::time::Timestamp::now()
        .to_unix_millis()
        .unwrap_or(0)
}

/// 是否还有未尝试且未熔断的其他候选。
async fn has_next(
    state: &GatewayState,
    vendors: &[ApiVendor],
    route: &VendorRoute,
    current: &ApiVendor,
    attempted: &[VendorId],
) -> bool {
    if route.mode == RouteMode::Locked {
        return false;
    }
    let mut ids = attempted.to_vec();
    ids.push(current.id.clone());
    let mut health = state.health.lock().await;
    let mut is_open = |id: &VendorId| health.is_circuit_open(id, now_ms());
    choose_next_vendor(
        vendors,
        route.provider_id,
        current.id.as_str(),
        &ids,
        &mut is_open,
    )
    .is_some()
}

/// 选下一个候选并更新转发状态。
async fn take_next(
    state: &GatewayState,
    route: &VendorRoute,
    current: &ApiVendor,
    attempted: &mut Vec<VendorId>,
    failures_on_vendor: &mut usize,
) -> Option<ApiVendor> {
    attempted.push(current.id.clone());
    *failures_on_vendor = 0;

    let vendors = state.source.vendors().await;
    let mut health = state.health.lock().await;
    let mut is_open = |id: &VendorId| health.is_circuit_open(id, now_ms());
    choose_next_vendor(
        &vendors,
        route.provider_id,
        current.id.as_str(),
        attempted,
        &mut is_open,
    )
    .cloned()
}

/// 读取请求体。
///
/// 按实际字节读取，支持未声明长度的分块请求。32 MiB 是接收上限；
/// 是否允许重试另按 2 MiB 判断，不能把重试缓存上限当成接收上限。
async fn read_body(
    body: Body,
    declared_length: u64,
) -> std::result::Result<Option<Bytes>, Box<Response<Body>>> {
    if declared_length > crate::proxy::max_body_bytes() {
        return Err(Box::new(json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            messages::UPSTREAM_REQUEST_FAILED,
        )));
    }
    match axum::body::to_bytes(body, crate::proxy::max_body_bytes() as usize).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(_) => Err(Box::new(json_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            messages::UPSTREAM_REQUEST_FAILED,
        ))),
    }
}

/// 探测结果：缓存的前缀 + 剩余流 + 错误信息。
///
/// 前缀必须**接回**剩余流的开头：正常响应（尤其是 SSE 首个 token）不能被探测吃掉。
struct ProbedResponse {
    prefix: Vec<Bytes>,
    rest: std::pin::Pin<Box<dyn futures::Stream<Item = reqwest::Result<Bytes>> + Send>>,
    status: StatusCode,
    headers: HeaderMap,
    error_message: Option<String>,
    read_error: Option<reqwest::Error>,
    finished: bool,
}

/// 探测响应体，识别「状态码正常但内容是错误」。
///
/// 见模块文档：读到错误信息、事件边界或上限即停止；读到的字节全部保留用于回放。
async fn probe_response(response: reqwest::Response) -> ProbedResponse {
    let status = response.status();
    let headers = response.headers().clone();
    let mut stream = Box::pin(response.bytes_stream());
    let mut prefix: Vec<Bytes> = Vec::new();
    let mut total = 0usize;
    let mut error_message = None;
    let mut read_error = None;
    let mut finished = false;

    while total < crate::response::probe_max_bytes() {
        match stream.next().await {
            Some(Ok(chunk)) => {
                if chunk.is_empty() {
                    continue;
                }
                total += chunk.len();
                prefix.push(chunk);
                let text = prefix
                    .iter()
                    .map(|item| String::from_utf8_lossy(item))
                    .collect::<String>();
                error_message = extract_gateway_response_error(&text);
                // 正常 SSE 的第一个事件已完整；再读就是无意义地阻塞首 token。
                if error_message.is_some()
                    || text.contains("\n\n")
                    || text.contains("\r\n\r\n")
                    || total >= crate::response::probe_max_bytes()
                {
                    break;
                }
            }
            // 流读取失败：交给回放阶段把错误传递给客户端。
            Some(Err(error)) => {
                read_error = Some(error);
                break;
            }
            None => {
                finished = true;
                break;
            }
        }
    }

    ProbedResponse {
        prefix,
        rest: stream,
        status,
        headers,
        error_message,
        read_error,
        finished,
    }
}

/// 回放上游响应：缓存的前缀 + 剩余流。
fn replay(probed: ProbedResponse) -> Response<Body> {
    let mut builder = Response::builder().status(probed.status);
    for (name, value) in &probed.headers {
        // 逐跳头不能回传：它们描述的是「网关到上游」这一段连接。
        if crate::upstream::should_drop_header(name.as_str()) {
            continue;
        }
        // 长度头必须剔除：探测已经改变了流的切分方式，照抄会让客户端读到多截的 body。
        if name.as_str().eq_ignore_ascii_case("content-length") {
            continue;
        }
        builder = builder.header(name, value);
    }
    if probed.finished && probed.read_error.is_none() {
        let bytes: Vec<u8> = probed.prefix.into_iter().flatten().collect();
        return builder
            .body(Body::from(bytes))
            .unwrap_or_else(|_| json_error(StatusCode::BAD_GATEWAY, messages::GENERIC_FAILURE));
    }
    let prefix = futures::stream::iter(probed.prefix.into_iter().map(Ok));
    let stream = prefix
        .chain(futures::stream::iter(probed.read_error.map(Err)))
        .chain(probed.rest)
        .map(|item| item.map_err(|error| std::io::Error::other(error.to_string())));
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| json_error(StatusCode::BAD_GATEWAY, messages::GENERIC_FAILURE))
}

/// 候选耗尽才应用友好报错；上游的原始状态仍保留在诊断记录中。
fn replay_failure(probed: ProbedResponse, custom: Option<&(u16, String)>) -> Response<Body> {
    if let Some((status, body)) = custom
        && let Ok(status) = StatusCode::from_u16(*status)
    {
        return Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
            .body(Body::from(body.clone()))
            .unwrap_or_else(|_| json_error(StatusCode::BAD_GATEWAY, messages::GENERIC_FAILURE));
    }
    replay(probed)
}

/// 按供应商构造转发用的请求头。
///
/// # 透传的头用 `append`，凭据用 `insert`——这两处**不能统一**
///
/// 现网就是这么分的（`headers.append(…)` 逐条追加，`headers.set(…)` 替换凭据），
/// 而移植时一度两者都写成 `HeaderMap::insert`：客户端**重复的同名头**因此被合并成
/// 一条、只剩最后一个。同名重复是合法且常见的（`anthropic-beta` 就可能分多条发），
/// 代理把它们合并不是等价变换。
fn headers_from(headers: &HeaderMap, vendor: &ApiVendor) -> HeaderMap {
    let pairs: Vec<(&str, &str)> = headers
        .iter()
        .filter_map(|(name, value)| Some((name.as_str(), value.to_str().ok()?)))
        .collect();
    let built = build_upstream_headers(pairs, vendor);
    let mut result = HeaderMap::new();
    for (name, value) in built.forwarded {
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_bytes()),
            axum::http::HeaderValue::from_str(&value),
        ) {
            result.append(name, value);
        }
    }
    let (name, value) = built.credential;
    if let (Ok(name), Ok(value)) = (
        axum::http::HeaderName::from_bytes(name.as_bytes()),
        axum::http::HeaderValue::from_str(&value),
    ) {
        // 替换而不是追加：客户端的凭据已被剔除，这里写入的是供应商的。
        result.insert(name, value);
    }
    result
}

/// 去掉路由前缀，得到上游路径后缀。
fn strip_route_prefix(path: &str, route: &VendorRoute) -> String {
    let prefix = route.prefix();
    let suffix = path.strip_prefix(&prefix).unwrap_or(path);
    if suffix.is_empty() {
        "/".to_owned()
    } else {
        suffix.to_owned()
    }
}

/// 取请求头的字符串值。
fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

/// 网关生成的错误使用 API 客户端可识别的 error.message，避免显示 Unknown error。
fn json_error(status: StatusCode, message: &str) -> Response<Body> {
    let body = serde_json::json!({
        "error": {
            "message": message,
            "type": "gateway_error"
        }
    })
    .to_string();
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(body))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// 跟随重定向的上限。
///
/// 与两个参照一致：现网用的 undici 默认最多 20 跳，`reqwest` 默认最多 10 跳。
/// 取 10 是为了「够用」而不是「尽可能多」——正常供应商不会连环重定向。
const MAX_REDIRECTS: usize = 10;

/// 重定向目标是否指向本机。
///
/// # 为什么**跟随**重定向
///
/// 现网从不设置 `redirect`，也就是 Node `fetch` 的默认行为：**跟随**。
/// 而移植时这里写成了 `Policy::none()`，理由是「跟随会绕过『供应商地址不能指向本地
/// 网关』的判定」。那个担心是真的，但**不跟随也有代价，而且更大**：
///
/// 供应商返回 302 时，网关会把 302 原样交给 CLI，而 CLI 通常会自己跟过去——
/// **带着它手里的本地路由令牌**发给重定向指向的那个主机。也就是说，一个本地专用令牌
/// 被送去了第三方。现网让网关自己跟随、并且用**供应商自己的密钥**请求新地址，
/// 凭据流向是正确的。
///
/// # 为什么不能无脑跟随
///
/// 重定向目标若是**本机**，请求就会转回网关自己，在网关与「上游」之间打转，
/// 直到耗尽连接。自我代理防护只检查**配置里的**地址，看不到重定向目标，
/// 因此必须在这里补一刀。
///
/// 于是取两者之长：**跟随，但不跟随回本机**。
fn is_loopback_host(url: &reqwest::Url) -> bool {
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => {
            // IPv6 主机会带方括号（`[::1]`），直接 parse 会失败——这一点是单测抓出来的。
            let bare = host.trim_start_matches('[').trim_end_matches(']');
            bare.parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback() || address.is_unspecified())
        }
        None => false,
    }
}

/// 已启动的网关句柄。
pub struct GatewayHandle {
    /// 实际监听地址。
    pub address: String,
    /// 关闭信号。
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    join: tokio::task::JoinHandle<()>,
}

impl GatewayHandle {
    /// 关闭网关并等待任务结束。
    ///
    /// **必须带上限**：`axum` 的优雅关闭会等待所有连接结束，而客户端（包括本应用自己
    /// 的终端）通常保持 keep-alive 连接不关。不设上限的话，应用退出会卡在这里。
    pub async fn shutdown(mut self) {
        if let Some(sender) = self.shutdown.take() {
            let _ = sender.send(());
        }
        if tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut self.join)
            .await
            .is_err()
        {
            // 超时后强制中止：连接残留不应阻塞进程退出。
            self.join.abort();
        }
    }
}

/// 启动网关。
///
/// 端口被占用时向后顺延（见 [`PORT_FALLBACK_ATTEMPTS`]），因此返回的地址可能与
/// 期望端口不同——调用方必须使用返回值而不是自己拼地址。
pub async fn serve(state: Arc<GatewayState>) -> Result<GatewayHandle> {
    let settings = state.source.settings();
    let app = Router::new()
        .fallback(any(handle))
        .with_state(Arc::clone(&state));

    let mut last_error = None;
    for offset in 0..PORT_FALLBACK_ATTEMPTS {
        let port = settings.port.saturating_add(offset);
        let address = format!("127.0.0.1:{port}");
        match tokio::net::TcpListener::bind(&address).await {
            Ok(listener) => {
                let bound = listener
                    .local_addr()
                    .map_or(address.clone(), |value| value.to_string());
                let url = format!("http://{bound}");
                state.set_address(url.clone()).await;
                let (sender, receiver) = tokio::sync::oneshot::channel();
                let join = tokio::spawn(async move {
                    let _ = axum::serve(listener, app)
                        .with_graceful_shutdown(async {
                            let _ = receiver.await;
                        })
                        .await;
                });
                return Ok(GatewayHandle {
                    address: url,
                    shutdown: Some(sender),
                    join,
                });
            }
            Err(error) => {
                tracing::debug!(port, %error, "网关端口被占用，尝试下一个");
                last_error = Some(error);
            }
        }
    }
    Err(aivc_core::error::WslError::CommandFailed {
        message: format!(
            "无法为本地 Gateway 找到可用端口：{}",
            last_error.map_or_else(|| "未知原因".to_owned(), |error| error.to_string())
        ),
    }
    .into())
}

/// 构造路由器（供测试直接挂载，不必真的绑定端口）。
pub fn router(state: Arc<GatewayState>) -> Router {
    Router::new().fallback(any(handle)).with_state(state)
}

/// 便于上层判断某平台是否有可用候选。
#[must_use]
pub fn provider_has_candidates(vendors: &[ApiVendor], provider_id: ProviderId) -> bool {
    vendors
        .iter()
        .any(|vendor| vendor.provider_id == provider_id && vendor.is_route_eligible())
}

/// 便于测试与上层构造的最小规则集。
#[must_use]
pub fn is_retryable(method: &str) -> bool {
    is_retryable_method(method)
}

/// 便于测试：结构化载荷判定。
#[must_use]
pub fn body_is_structured(body: &str) -> bool {
    has_structured_gateway_payload(body)
}

/// 便于测试：错误信息提取。
#[must_use]
pub fn body_error_message(body: &str) -> Option<String> {
    extract_gateway_response_error(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::gateway::GatewayOutcome;
    use aivc_core::time::Timestamp;
    use sha2::{Digest, Sha256};
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// 会捕获记录的内存来源。
    struct RecordingSource {
        vendors: Vec<ApiVendor>,
        settings: GatewaySettings,
        records: std::sync::Mutex<Vec<GatewayRequestRecord>>,
        attempts: std::sync::Mutex<Vec<Vec<GatewayAttempt>>>,
        rules: Vec<FailoverRule>,
    }

    impl RecordingSource {
        fn new(vendors: Vec<ApiVendor>, settings: GatewaySettings) -> Self {
            Self {
                vendors,
                settings,
                records: std::sync::Mutex::new(Vec::new()),
                attempts: std::sync::Mutex::new(Vec::new()),
                rules: Vec::new(),
            }
        }

        fn taken(&self) -> Vec<GatewayRequestRecord> {
            std::mem::take(&mut self.records.lock().expect("lock"))
        }
    }

    impl GatewaySource for RecordingSource {
        fn vendors(&self) -> BoxFuture<'_, Vec<ApiVendor>> {
            let vendors = self.vendors.clone();
            Box::pin(async move { vendors })
        }

        fn failover_rules(&self) -> BoxFuture<'_, Vec<FailoverRule>> {
            Box::pin(async move { self.rules.clone() })
        }

        fn settings(&self) -> GatewaySettings {
            self.settings.clone()
        }

        fn record(&self, record: GatewayRequestRecord) -> BoxFuture<'_, ()> {
            self.records.lock().expect("lock").push(record);
            Box::pin(async {})
        }

        fn record_with_attempts(
            &self,
            record: GatewayRequestRecord,
            attempts: Vec<GatewayAttempt>,
        ) -> BoxFuture<'_, ()> {
            self.attempts.lock().expect("attempts").push(attempts);
            self.record(record)
        }
    }

    async fn start_recording(
        vendors: Vec<ApiVendor>,
        token: &str,
    ) -> (GatewayHandle, Arc<RecordingSource>) {
        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(vendors, settings));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state
            .register_route(VendorRoute {
                route_id: "r1".to_owned(),
                provider_id: ProviderId::Codex,
                vendor_id: "v1".to_owned(),
                mode: RouteMode::Dynamic,
                local_token: token.to_owned(),
                base_url: String::new(),
                external: false,
            })
            .await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");
        (handle, source)
    }

    async fn wait_for_record(source: &RecordingSource) -> GatewayRequestRecord {
        for _ in 0..100 {
            if let Some(record) = source.taken().into_iter().next() {
                return record;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("request record was not persisted");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn every_http_request_on_the_same_route_has_its_own_log() {
        let upstream = healthy_upstream(r#"{"usage":{"input_tokens":12,"output_tokens":8}}"#).await;
        let (handle, source) = start_recording(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        let client = reqwest::Client::new();
        let mut records = Vec::new();
        for _ in 0..3 {
            let response = client
                .post(format!("{}/gateway/codex/r1/v1/responses", handle.address))
                .bearer_auth("tok")
                .json(&serde_json::json!({"model": "test-model", "input": "hello"}))
                .send()
                .await
                .expect("send");
            assert_eq!(response.status(), 200);
            response.bytes().await.expect("body");
            records.push(wait_for_record(&source).await);
        }
        let ids: std::collections::BTreeSet<_> =
            records.iter().map(|record| &record.request_id).collect();
        assert_eq!(ids.len(), 3, "同一路由的多次请求不可合并或覆盖");
        assert!(records.iter().all(|record| record.route_id == "r1"));
        {
            let attempts = source.attempts.lock().expect("attempts");
            assert_eq!(attempts.len(), 3);
            assert!(attempts.iter().all(|items| items.len() == 1));
        }
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chunked_requests_keep_their_body_and_usage_is_recorded() {
        let upstream = healthy_upstream(r#"{"usage":{"input_tokens":12,"output_tokens":8}}"#).await;
        let mut supplier = vendor("v1", &upstream.uri(), 1);
        supplier.pricing = Some(aivc_core::vendor::VendorPricing {
            input_per_million_usd: Some(0.5),
            output_per_million_usd: Some(2.0),
        });
        let (handle, source) = start_recording(vec![supplier], "tok").await;
        let json = r#"{"model":"test-model","input":"hello"}"#;
        let chunks = futures::stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from_static(
            json.as_bytes(),
        ))]);
        let response = reqwest::Client::new()
            .post(format!("{}/gateway/codex/r1/v1/responses", handle.address))
            .bearer_auth("tok")
            .body(reqwest::Body::wrap_stream(chunks))
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 200);
        let output = response.bytes().await.expect("body");
        let received = upstream.received_requests().await.expect("received");
        assert_eq!(received[0].body, json.as_bytes());
        assert!(!received[0].headers.contains_key("transfer-encoding"));
        let record = wait_for_record(&source).await;
        assert_eq!(record.bytes_in, json.len() as u64);
        assert_eq!(record.bytes_out, output.len() as u64);
        assert_eq!(record.model.as_deref(), Some("test-model"));
        let usage = record.usage.expect("usage");
        assert_eq!(usage.total_tokens, Some(20));
        assert!((usage.cost_usd.expect("cost") - 0.000_022).abs() < 1e-10);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_body_above_the_retry_limit_is_forwarded_once() {
        let upstream = failing_upstream().await;
        let (handle, source) = start_recording(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        let input = vec![b'x'; crate::proxy::retry_buffer_bytes() as usize + 100];
        let response = reqwest::Client::new()
            .post(format!("{}/gateway/codex/r1/v1/responses", handle.address))
            .bearer_auth("tok")
            .body(input.clone())
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 429);
        response.bytes().await.expect("body");
        let requests = upstream.received_requests().await.expect("received");
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].body, input);
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.upstream_status, Some(429));
        assert_eq!(record.retry_count, 0);
        assert!(record.error_message.is_some());
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn all_35_supplier_attempts_produce_one_failed_request_with_details() {
        let upstream = failing_upstream().await;
        let vendors = (0..35)
            .map(|i| vendor(&format!("v{i}"), &upstream.uri(), i))
            .collect();
        let (handle, source) = start_with_thresholds(vendors, 1, 5, "tok").await;
        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 429);
        response.bytes().await.expect("body");
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.upstream_status, Some(429));
        assert_eq!(record.retry_count, 34);
        assert!(record.switched);
        assert!(record.error_message.is_some());
        assert_eq!(received(&upstream).await, 35);
        let details = source.attempts.lock().expect("attempts").clone();
        assert_eq!(details.len(), 1);
        assert_eq!(details[0].len(), 35);
        assert_eq!(
            details[0]
                .iter()
                .map(|a| a.vendor_id.clone())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            35
        );
        drop(details);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unmatched_http_error_is_not_a_success() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400).set_body_string(r#"{"error":"invalid input"}"#),
            )
            .mount(&upstream)
            .await;
        let mut source = RecordingSource::new(
            vec![vendor("v1", &upstream.uri(), 1)],
            GatewaySettings {
                enabled: true,
                port: 0,
                ..Default::default()
            },
        );
        source.rules.push(FailoverRule {
            id: "balance".into(),
            scope: aivc_core::gateway::FailoverRuleScope::Global,
            provider_id: None,
            vendor_id: None,
            pattern: "insufficient balance".into(),
            status_codes: Some(vec![418]),
            custom_response_body: None,
            custom_response_status: None,
            enabled: true,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        });
        let source = Arc::new(source);
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        let mut route = sample_route();
        route.local_token = "tok".into();
        state.register_route(route).await;
        let handle = serve(state.clone()).await.expect("serve");
        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 400);
        response.bytes().await.expect("body");
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.upstream_status, Some(400));
        assert_eq!(record.error_message.as_deref(), Some("invalid input"));
        let health = state.health_snapshot().await;
        assert!(health.is_empty(), "普通参数错误不影响供应商健康");
        handle.shutdown().await;
    }

    async fn streaming_upstream(
        chunks: Vec<(u64, &'static str, bool)>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listen");
        let address = format!("http://{}", listener.local_addr().expect("address"));
        let app = Router::new().fallback(any(move || {
            let chunks = chunks.clone();
            async move {
                let stream =
                    futures::stream::iter(chunks).then(|(delay, text, broken)| async move {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        if broken {
                            Err(std::io::Error::other("upstream interrupted"))
                        } else {
                            Ok(Bytes::from_static(text.as_bytes()))
                        }
                    });
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from_stream(stream))
                    .expect("response")
            }
        }));
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        (address, task)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn streaming_usage_waits_for_the_end_and_merges_split_events() {
        let (upstream, task) = streaming_upstream(vec![
            (0, "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n", false),
            (80, "data: {\"usage\":{\"output_", false),
            (0, "tokens\":8}}\n\n", false),
        ]).await;
        let (handle, source) = start_recording(vec![vendor("v1", &upstream, 1)], "tok").await;
        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 200);
        assert!(source.taken().is_empty(), "stream is still running");
        let text = response.text().await.expect("body");
        assert!(text.contains("output_tokens"));
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Ok);
        assert!(record.duration_ms >= 70);
        assert_eq!(record.bytes_out, text.len() as u64);
        let usage = record.usage.expect("usage");
        assert_eq!(usage.input_tokens, Some(12));
        assert_eq!(usage.output_tokens, Some(8));
        assert_eq!(usage.total_tokens, Some(20));
        handle.shutdown().await;
        task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_late_sse_error_is_logged_and_does_not_record_health_success() {
        let (upstream, task) = streaming_upstream(vec![
            (0, "data: {\"ok\":true}\n\n", false),
            (
                80,
                "event: error\ndata: {\"error\":{\"message\":\"credit insufficient balance\"}}\n\n",
                false,
            ),
        ])
        .await;
        let source = Arc::new(RecordingSource::new(
            vec![vendor("v1", &upstream, 1)],
            GatewaySettings {
                enabled: true,
                port: 0,
                ..Default::default()
            },
        ));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state.register_route(sample_route()).await;
        let handle = serve(state.clone()).await.expect("serve");
        post(&handle.address, "t").await.text().await.expect("body");
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.upstream_status, Some(200));
        assert_eq!(
            record.error_message.as_deref(),
            Some("credit insufficient balance")
        );
        assert_eq!(
            source.attempts.lock().expect("attempts")[0][0].error_message,
            record.error_message
        );
        let health = state.health_snapshot().await;
        assert_eq!(health[0].failure_count, 1);
        assert_eq!(health[0].success_count, 0);
        handle.shutdown().await;
        task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_upstream_stream_read_error_reaches_the_client_and_log() {
        let (upstream, task) =
            streaming_upstream(vec![(0, "data: {\"ok\":true}\n\n", false), (80, "", true)]).await;
        let (handle, source) = start_recording(vec![vendor("v1", &upstream, 1)], "tok").await;
        assert!(post(&handle.address, "tok").await.text().await.is_err());
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.error_code.as_deref(), Some("UPSTREAM_STREAM_ERROR"));
        handle.shutdown().await;
        task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_a_streaming_client_still_records_the_request() {
        let (upstream, task) = streaming_upstream(vec![
            (0, "data: {\"ok\":true}\n\n", false),
            (10_000, "data: [DONE]\n\n", false),
        ])
        .await;
        let (handle, source) = start_recording(vec![vendor("v1", &upstream, 1)], "tok").await;
        let response = post(&handle.address, "tok").await;
        let mut stream = response.bytes_stream();
        stream.next().await.expect("chunk").expect("data");
        drop(stream);
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::ClientAborted);
        assert_eq!(record.error_code.as_deref(), Some("CLIENT_ABORTED"));
        handle.shutdown().await;
        task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn total_request_budget_stops_an_active_upstream_and_records_timeout() {
        let (upstream, task) = streaming_upstream(vec![(10_000, "data: [DONE]\n\n", false)]).await;
        let source = Arc::new(RecordingSource::new(
            vec![vendor("v1", &upstream, 1)],
            GatewaySettings {
                enabled: true,
                port: 0,
                upstream_timeout: Duration::from_secs(5),
                request_timeout: Duration::from_millis(100),
                ..Default::default()
            },
        ));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state.register_route(sample_route()).await;
        let handle = serve(state.clone()).await.expect("serve");
        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 504);
        assert!(
            response
                .text()
                .await
                .expect("timeout body")
                .contains(messages::UPSTREAM_TIMEOUT)
        );
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Timeout);
        assert!(record.duration_ms < 1000);
        assert_eq!(source.attempts.lock().expect("attempts")[0].len(), 1);
        assert_eq!(state.health_snapshot().await[0].failure_count, 1);
        handle.shutdown().await;
        task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_upstream_response_is_logged_as_success() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(204))
            .mount(&upstream)
            .await;
        let (handle, source) = start_recording(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        assert_eq!(post(&handle.address, "tok").await.status(), 204);
        assert_eq!(wait_for_record(&source).await.outcome, GatewayOutcome::Ok);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_local_route_strips_the_query_token_before_forwarding() {
        let upstream = healthy_upstream("ok").await;
        let (handle, _source) =
            start_recording(vec![vendor("v1", &upstream.uri(), 1)], "local-secret").await;
        let response = reqwest::Client::new()
            .post(format!(
                "{}/gateway/codex/r1/v1/responses?key=local-secret&stream=true",
                handle.address
            ))
            .body("{}")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 200);
        response.bytes().await.expect("body");
        let requests = upstream.received_requests().await.expect("received");
        assert_eq!(requests[0].url.query(), Some("stream=true"));
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn recovery_bypasses_the_health_success_write_throttle() {
        let source = Arc::new(RecordingSource::new(Vec::new(), GatewaySettings::default()));
        let state = GatewayState::new(source).expect("state");
        let saved = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = saved.clone();
        state.set_health_hook(HealthHook {
            load: Arc::new(|| Box::pin(async { Vec::new() })),
            save: Arc::new(move |health| {
                sink.lock().expect("saved").push(health);
                Box::pin(async {})
            }),
        });
        let vendor = vendor("v1", "http://example.test", 1);
        state.record_vendor_success(&vendor).await;
        state
            .record_vendor_failure(&vendor, "failure", CircuitConfig::default())
            .await;
        state.record_vendor_success(&vendor).await;
        let health = saved.lock().expect("saved");
        assert_eq!(health.len(), 3);
        assert_eq!(
            health[2].status,
            aivc_core::gateway::GatewayVendorHealthStatus::Healthy
        );
        assert_eq!(health[2].success_count, 2);
        assert_eq!(health[2].failure_count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_custom_failure_response_is_applied_only_after_retries_are_exhausted() {
        let upstream = failing_upstream().await;
        let mut source = RecordingSource::new(
            vec![
                vendor("v1", &upstream.uri(), 1),
                vendor("v2", &upstream.uri(), 2),
            ],
            GatewaySettings {
                enabled: true,
                port: 0,
                failure_threshold: 2,
                ..Default::default()
            },
        );
        source.rules.push(FailoverRule {
            id: "custom".into(),
            scope: aivc_core::gateway::FailoverRuleScope::Global,
            provider_id: None,
            vendor_id: None,
            pattern: String::new(),
            status_codes: Some(vec![429]),
            custom_response_status: Some(402),
            custom_response_body: Some(r#"{"error":"please try later"}"#.into()),
            enabled: true,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        });
        let source = Arc::new(source);
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state.register_route(sample_route()).await;
        let handle = serve(state).await.expect("serve");
        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 402);
        assert_eq!(
            response.text().await.expect("body"),
            r#"{"error":"please try later"}"#
        );
        assert_eq!(received(&upstream).await, 4);
        let record = wait_for_record(&source).await;
        assert_eq!(record.outcome, GatewayOutcome::Error);
        assert_eq!(record.upstream_status, Some(429));
        assert_eq!(record.retry_count, 3);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_model_listing_get_can_fail_over_with_an_empty_request_body() {
        let first = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&first)
            .await;
        let second = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string("models"))
            .mount(&second)
            .await;
        let (handle, source) = start_with_thresholds(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            1,
            5,
            "tok",
        )
        .await;
        let response = reqwest::Client::new()
            .get(format!("{}/gateway/codex/r1/v1/models", handle.address))
            .bearer_auth("tok")
            .send()
            .await
            .expect("send");
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.expect("body"), "models");
        let record = wait_for_record(&source).await;
        assert_eq!(record.retry_count, 1);
        assert!(record.switched);
        assert_eq!(record.vendor_id.as_str(), "v2");
        handle.shutdown().await;
    }

    /// 外部接入形态的服务器：启用共享令牌（库内只存 SHA-256）。
    async fn start_external(
        vendors: Vec<ApiVendor>,
        secret: &str,
    ) -> (GatewayHandle, Arc<RecordingSource>) {
        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            external_enabled: true,
            external_token_hash: Some(hex::encode(Sha256::digest(secret.as_bytes()))),
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(vendors, settings));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        let handle = serve(Arc::clone(&state)).await.expect("serve");
        (handle, source)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn external_requests_are_rejected_until_enabled() {
        let upstream = MockServer::start().await;
        // 默认设置：`external_enabled = false`。
        let source = Arc::new(RecordingSource::new(
            vec![vendor("v1", &upstream.uri(), 1)],
            GatewaySettings {
                enabled: true,
                port: 0,
                ..GatewaySettings::default()
            },
        ));
        let state = Arc::new(GatewayState::new(source).expect("state"));
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/codex/v1/responses", handle.address))
            .bearer_auth("ext-secret")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 503);
        let body = response.text().await.expect("body");
        assert!(body.contains(messages::EXTERNAL_API_DISABLED), "{body}");
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn external_requests_need_the_shared_token() {
        let upstream = MockServer::start().await;
        let (handle, _source) =
            start_external(vec![vendor("v1", &upstream.uri(), 1)], "ext-secret").await;
        let client = reqwest::Client::new();

        // 不带令牌。
        let response = client
            .post(format!("{}/codex/v1/responses", handle.address))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 401);

        // 令牌不对。
        let response = client
            .post(format!("{}/codex/v1/responses", handle.address))
            .bearer_auth("wrong")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 401);

        // 换一种携带方式（x-api-key）就认：上游没有挂 mock 会回 404，
        // 关键是**不是 401**——鉴权这一关已经过了。
        let response = client
            .post(format!("{}/codex/v1/responses", handle.address))
            .header("x-api-key", "ext-secret")
            .send()
            .await
            .expect("request");
        assert_ne!(response.status(), 401);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn external_requests_reach_the_pool_and_keep_the_protocol_path() {
        let upstream = MockServer::start().await;
        // mock 只在收到 `/v1/responses` 时命中：前缀剥离错误会让这里拿不到 200。
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&upstream)
            .await;

        let (handle, source) =
            start_external(vec![vendor("v1", &upstream.uri(), 1)], "ext-secret").await;
        let client = reqwest::Client::new();
        let response = client
            // 认证走查询参数，顺便验证 `?key=` 被摘掉、其余参数保留。
            .post(format!(
                "{}/codex/v1/responses?stream=true&key=ext-secret",
                handle.address
            ))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200);

        let mut records = Vec::new();
        for _ in 0..50 {
            records = source.taken();
            if !records.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(records.len(), 1, "一次请求应当产生一条记录");
        let record = &records[0];
        // 合成路由的编号是 `external-<平台>`（日志里一眼可辨）。
        assert_eq!(record.route_id, "external-codex");
        assert_eq!(record.provider_id, ProviderId::Codex);
        // 访问令牌绝不能进日志；记录的是**路径**（不含查询串，键值自然也不在）。
        assert!(!record.path.contains("ext-secret"), "{}", record.path);
        assert!(
            record.path.contains("/codex/v1/responses"),
            "{}",
            record.path
        );
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unknown_top_level_paths_are_not_external_routes() {
        let upstream = MockServer::start().await;
        let (handle, _source) =
            start_external(vec![vendor("v1", &upstream.uri(), 1)], "ext-secret").await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/openai/v1/responses", handle.address))
            .bearer_auth("ext-secret")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 404);
        handle.shutdown().await;
    }

    /// 记录切换通知。
    #[derive(Default)]
    struct SwitchRecorder {
        notices: std::sync::Mutex<Vec<SwitchNotice>>,
    }

    impl SwitchRecorder {
        fn notices(&self) -> Vec<SwitchNotice> {
            self.notices.lock().expect("lock").clone()
        }
    }

    /// 建一个带内存来源的状态，并装上切换记录器。
    fn state_with_switch_recorder() -> (Arc<GatewayState>, Arc<SwitchRecorder>) {
        let source = Arc::new(RecordingSource::new(Vec::new(), GatewaySettings::default()));
        let state = Arc::new(GatewayState::new(source).expect("state"));
        let recorder = Arc::new(SwitchRecorder::default());
        let recorder_clone = Arc::clone(&recorder);
        state.set_switch_hook(Arc::new(move |notice| {
            recorder_clone
                .notices
                .lock()
                .expect("lock")
                .push(notice.clone());
        }));
        (state, recorder)
    }

    /// 一条用于测试的路由。
    fn sample_route() -> VendorRoute {
        VendorRoute {
            route_id: "r1".to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: "v1".to_owned(),
            mode: RouteMode::Dynamic,
            local_token: "t".to_owned(),
            base_url: "http://localhost".to_owned(),
            external: false,
        }
    }

    #[tokio::test]
    async fn switching_a_route_notifies_the_hook() {
        // 手动切换必须通知界面：工作台的「当前供应商」靠它更新。
        let (state, recorder) = state_with_switch_recorder();
        state.register_route(sample_route()).await;

        let updated = state
            .switch_route_vendor("r1", "v2", None)
            .await
            .expect("切换");
        assert_eq!(updated.vendor_id, "v2");
        let notices = recorder.notices();
        assert_eq!(notices.len(), 1, "应当通知一次");
        assert_eq!(notices[0].route_id, "r1");
        assert_eq!(notices[0].vendor_id, "v2");
        assert_eq!(notices[0].mode, RouteMode::Dynamic);
        assert_eq!(notices[0].reason, None, "手动切换没有原因");
    }

    #[tokio::test]
    async fn empty_vendor_switches_are_rejected_and_never_notified() {
        let (state, recorder) = state_with_switch_recorder();
        let route = sample_route();
        state.register_route(route.clone()).await;
        for id in ["", "  "] {
            assert!(state.switch_route_vendor("r1", id, None).await.is_none());
            state.notify_failover(&route, &VendorId::from(id));
        }
        assert!(recorder.notices().is_empty());
        assert_eq!(
            state
                .routes
                .read()
                .await
                .get("r1")
                .expect("route")
                .vendor_id,
            "v1"
        );
    }

    #[tokio::test]
    async fn changing_the_mode_notifies_with_a_reason() {
        let (state, recorder) = state_with_switch_recorder();
        state.register_route(sample_route()).await;

        state
            .set_route_mode("r1", RouteMode::Locked)
            .await
            .expect("切换模式");
        let notices = recorder.notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].mode, RouteMode::Locked);
        assert_eq!(notices[0].reason.as_deref(), Some("mode"));
    }

    #[tokio::test]
    async fn a_missing_route_does_not_notify() {
        // 找不到路由时没有发生任何切换，因此不该发事件——
        // 否则界面会显示一次并不存在的切换。
        let (state, recorder) = state_with_switch_recorder();
        assert!(
            state
                .switch_route_vendor("nope", "v2", None)
                .await
                .is_none()
        );
        assert!(
            state
                .set_route_mode("nope", RouteMode::Locked)
                .await
                .is_none()
        );
        assert!(recorder.notices().is_empty(), "没有切换就不该通知");
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn a_successful_request_is_recorded() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&upstream)
            .await;

        let (handle, source) = start_recording(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        let client = reqwest::Client::new();
        client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("request");

        // 记录是异步落地的，给它一点时间。
        let mut records = Vec::new();
        for _ in 0..50 {
            records = source.taken();
            if !records.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(records.len(), 1, "一次请求应当产生一条记录");
        let record = &records[0];
        assert_eq!(record.outcome, GatewayOutcome::Ok);
        assert_eq!(record.vendor_id.as_str(), "v1");
        assert_eq!(record.route_id, "r1");
        assert_eq!(record.provider_id, ProviderId::Codex);
        assert_eq!(record.upstream_status, Some(200));
        // 模型名从缓冲的请求体里提取。
        assert_eq!(record.model.as_deref(), Some("gpt-5"));
        assert!(!record.switched);
        assert_eq!(record.retry_count, 0);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_rejected_request_is_still_recorded() {
        // 关键不变式：路由不存在这类**在解析阶段就返回**的请求也必须留下记录，
        // 否则最需要排查的情况恰恰没有日志。
        let (handle, source) = start_recording(Vec::new(), "tok").await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "{}/gateway/codex/missing/v1/messages",
                handle.address
            ))
            .bearer_auth("tok")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 404);

        let mut records = Vec::new();
        for _ in 0..50 {
            records = source.taken();
            if !records.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(records.len(), 1, "被拒绝的请求也要留下记录");
        // 拒绝接入是网关请求失败，供应商未参与，健康状态不受影响。
        assert_eq!(records[0].outcome, GatewayOutcome::Error);
        assert_eq!(records[0].error_code.as_deref(), Some("ROUTE_NOT_FOUND"));
        let record = &records[0];
        assert_eq!(record.outcome, GatewayOutcome::Error);
        // 没有供应商参与，因此供应商为空、没有上游状态码。
        assert!(record.vendor_id.as_str().is_empty());
        assert!(record.upstream_status.is_none());
        assert!(
            record.path.contains("/gateway/codex/missing"),
            "{}",
            record.path
        );
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_sole_candidate_keeps_receiving_requests_after_its_circuit_opens() {
        let upstream = failing_upstream().await;
        let mut disabled = vendor("v2", "http://127.0.0.1:1", 2);
        disabled.enabled = false;
        let mut other = vendor("claude-only", "http://127.0.0.1:1", 3);
        other.provider_id = ProviderId::Claude;
        let source = Arc::new(RecordingSource::new(
            vec![vendor("v1", &upstream.uri(), 1), disabled, other],
            GatewaySettings {
                enabled: true,
                port: 0,
                failure_threshold: 3,
                circuit: CircuitConfig {
                    failure_threshold: 1,
                    duration: Duration::from_secs(60),
                },
                ..GatewaySettings::default()
            },
        ));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state.register_route(sample_route()).await;
        let handle = serve(state.clone()).await.expect("serve");
        for expected_requests in 1..=2 {
            let response = post(&state.address().await, "t").await;
            assert_eq!(response.status(), 429);
            response.bytes().await.expect("body");
            let record = wait_for_record(&source).await;
            assert_eq!(record.vendor_id.as_str(), "v1");
            assert_eq!(record.upstream_status, Some(429));
            assert_eq!(record.retry_count, 0, "唯一候选按锁定供应商处理");
            assert!(!record.switched);
            assert_eq!(received(&upstream).await, expected_requests);
            assert_eq!(
                state.health_snapshot().await[0].status,
                aivc_core::gateway::GatewayVendorHealthStatus::Open
            );
        }
        // 保留 dynamic 模式，未来启用另一家供应商时仍可故障转移。
        assert_eq!(
            state.routes.read().await.get("r1").expect("route").mode,
            RouteMode::Dynamic
        );
        upstream.reset().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("recovered"))
            .mount(&upstream)
            .await;
        let response = post(&state.address().await, "t").await;
        assert_eq!(response.status(), 200);
        response.bytes().await.expect("body");
        assert_eq!(wait_for_record(&source).await.outcome, GatewayOutcome::Ok);
        assert_eq!(
            state.health_snapshot().await[0].status,
            aivc_core::gateway::GatewayVendorHealthStatus::Healthy
        );
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn all_open_candidates_report_cooldown_without_switching_to_an_empty_vendor() {
        let upstream = healthy_upstream("recovered").await;
        let active = vendor("v1", &upstream.uri(), 1);
        let second = vendor("v2", &upstream.uri(), 2);
        let mut disabled = vendor("v3", &upstream.uri(), 3);
        disabled.enabled = false;
        let source = Arc::new(RecordingSource::new(
            vec![active.clone(), second.clone(), disabled],
            GatewaySettings {
                enabled: true,
                port: 0,
                ..GatewaySettings::default()
            },
        ));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state.register_route(sample_route()).await;
        let notices = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = notices.clone();
        state.set_switch_hook(Arc::new(move |notice| {
            sink.lock().expect("notices").push(notice.clone());
        }));
        for candidate in [&active, &second] {
            state.health.lock().await.record_failure(
                candidate,
                "connection refused",
                CircuitConfig {
                    failure_threshold: 1,
                    duration: Duration::from_secs(60),
                },
                now_ms(),
            );
        }
        let handle = serve(state.clone()).await.expect("serve");
        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 503);
        let retry_after: i64 = response.headers()[header::RETRY_AFTER]
            .to_str()
            .expect("header")
            .parse()
            .expect("seconds");
        assert!((1..=60).contains(&retry_after));
        let body = response.text().await.expect("body");
        assert!(body.contains("2 家候选供应商均处于熔断冷却期"), "{body}");
        let record = wait_for_record(&source).await;
        assert_eq!(record.error_code.as_deref(), Some("VENDOR_CIRCUIT_OPEN"));
        assert!(record.vendor_id.as_str().is_empty());
        assert!(record.upstream_status.is_none());
        assert!(!record.switched);
        assert!(source.attempts.lock().expect("attempts")[0].is_empty());
        assert!(notices.lock().expect("notices").is_empty());
        assert_eq!(
            state
                .routes
                .read()
                .await
                .get("r1")
                .expect("route")
                .vendor_id,
            "v1"
        );
        assert!(
            upstream
                .received_requests()
                .await
                .expect("requests")
                .is_empty()
        );

        // 不等待一分钟：注入已到期的熔断，验证下一条请求能恢复探测。
        state.health.lock().await.record_failure(
            &active,
            "old failure",
            CircuitConfig {
                failure_threshold: 1,
                duration: Duration::from_secs(1),
            },
            now_ms() - 2_000,
        );
        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 200);
        response.bytes().await.expect("body");
        let record = wait_for_record(&source).await;
        assert_eq!(record.vendor_id.as_str(), "v1");
        assert_eq!(record.outcome, GatewayOutcome::Ok);
        assert!(record.error_code.is_none());
        assert_eq!(
            state.health_snapshot().await[0].status,
            aivc_core::gateway::GatewayVendorHealthStatus::Healthy
        );
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn another_platform_cannot_fill_an_empty_candidate_pool() {
        let mut other = vendor("claude-only", "http://127.0.0.1:1", 1);
        other.provider_id = ProviderId::Claude;
        let (handle, source) = start_recording(vec![other], "tok").await;
        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 503);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
        let body = response.text().await.expect("body");
        assert!(body.contains("codex 没有参与候选池"), "{body}");
        let record = wait_for_record(&source).await;
        assert_eq!(record.error_code.as_deref(), Some("VENDOR_UNAVAILABLE"));
        assert!(record.vendor_id.as_str().is_empty());
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_recorded_path_never_contains_the_access_key() {
        // `key` 是访问令牌，落进日志等同于泄露。
        let (handle, source) = start_recording(Vec::new(), "tok").await;
        let client = reqwest::Client::new();
        client
            .get(format!(
                "{}/gateway/codex/r1/v1/models?key=super-secret",
                handle.address
            ))
            .bearer_auth("tok")
            .send()
            .await
            .expect("request");

        let mut records = Vec::new();
        for _ in 0..50 {
            records = source.taken();
            if !records.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(records.len(), 1);
        assert!(
            !records[0].path.contains("super-secret"),
            "路径里泄露了访问令牌：{}",
            records[0].path
        );
        handle.shutdown().await;
    }
    /// 内存供应商来源。
    struct MemorySource {
        vendors: Vec<ApiVendor>,
        settings: GatewaySettings,
    }

    impl GatewaySource for MemorySource {
        fn vendors(&self) -> BoxFuture<'_, Vec<ApiVendor>> {
            let vendors = self.vendors.clone();
            Box::pin(async move { vendors })
        }

        fn failover_rules(&self) -> BoxFuture<'_, Vec<FailoverRule>> {
            Box::pin(async move { Vec::new() })
        }

        fn settings(&self) -> GatewaySettings {
            self.settings.clone()
        }
    }

    fn vendor(id: &str, base_url: &str, sort: i64) -> ApiVendor {
        ApiVendor {
            id: VendorId::from(id),
            provider_id: ProviderId::Codex,
            name: id.to_owned(),
            api_key: format!("key-{id}"),
            api_base_url: base_url.to_owned(),
            sort,
            pricing: None,
            model_query: None,
            balance_query: None,
            configs: Vec::new(),
            enabled: true,
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

    async fn start_gateway(
        vendors: Vec<ApiVendor>,
        token: &str,
    ) -> (GatewayHandle, Arc<GatewayState>) {
        let settings = GatewaySettings {
            enabled: true,
            // 端口 0 = 让操作系统分配，测试之间不会互相冲突。
            port: 0,
            ..GatewaySettings::default()
        };
        let source = Arc::new(MemorySource { vendors, settings });
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state
            .register_route(VendorRoute {
                route_id: "r1".to_owned(),
                provider_id: ProviderId::Codex,
                vendor_id: "v1".to_owned(),
                mode: RouteMode::Dynamic,
                local_token: token.to_owned(),
                base_url: String::new(),
                external: false,
            })
            .await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");
        (handle, state)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_request_is_forwarded_with_the_vendor_key() {
        let upstream = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
            .mount(&upstream)
            .await;

        let (handle, _state) = start_gateway(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.expect("body"), r#"{"ok":true}"#);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_wrong_token_is_rejected() {
        let upstream = MockServer::start().await;
        let (handle, _state) = start_gateway(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("wrong")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 401);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unknown_route_is_not_found() {
        let upstream = MockServer::start().await;
        let (handle, _state) = start_gateway(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "{}/gateway/codex/missing/v1/messages",
                handle.address
            ))
            .bearer_auth("tok")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 404);
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_local_token_is_never_forwarded_to_the_upstream() {
        let upstream = MockServer::start().await;
        // 上游必须收到供应商自己的 Key，而不是本地路由令牌。
        Mock::given(method("POST"))
            .and(wiremock::matchers::header("authorization", "Bearer key-v1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
            .mount(&upstream)
            .await;

        let (handle, _state) =
            start_gateway(vec![vendor("v1", &upstream.uri(), 1)], "local-token").await;
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("local-token")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200, "上游必须看到供应商 Key");
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_retryable_status_fails_over_to_the_next_vendor() {
        // 第一家在 429 后应当被跳过，请求最终由第二家成功。
        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":"rate limited"}"#))
            .mount(&first)
            .await;
        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("from-second"))
            .mount(&second)
            .await;

        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            failure_threshold: 1,
            ..GatewaySettings::default()
        };
        let source = Arc::new(MemorySource {
            vendors: vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            settings,
        });
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state
            .register_route(VendorRoute {
                route_id: "r1".to_owned(),
                provider_id: ProviderId::Codex,
                vendor_id: "v1".to_owned(),
                mode: RouteMode::Dynamic,
                local_token: "tok".to_owned(),
                base_url: String::new(),
                external: false,
            })
            .await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.expect("body"), "from-second");
        handle.shutdown().await;
    }

    /// **端到端：客户端重复的同名头必须原样到达上游。**
    ///
    /// 这是「透传用 `append`、凭据用 `insert`」那条区分的**唯一真正检验点**：
    /// `build_upstream_headers` 的单测只能看到成对列表，看不到写进 `HeaderMap` 的那一步。
    /// 移植时两者一度都写成 `insert`，重复的同名头因此被合并成一条、只剩最后一个。
    #[tokio::test(flavor = "multi_thread")]
    async fn repeated_client_headers_reach_the_upstream_unchanged() {
        let upstream = healthy_upstream("ok").await;
        let (handle, _source) = start_gateway(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;

        // 用显式 append 组装，确保**线路上确实有两条**同名头（`RequestBuilder::header`
        // 的追加/替换语义不值得依赖）。
        let mut extra = HeaderMap::new();
        extra.append("anthropic-beta", "first-flag".parse().expect("头"));
        extra.append("anthropic-beta", "second-flag".parse().expect("头"));

        let response = reqwest::Client::new()
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .headers(extra)
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("请求");
        assert_eq!(response.status(), 200);

        let received = upstream.received_requests().await.expect("收到的请求");
        let betas: Vec<&str> = received[0]
            .headers
            .get_all("anthropic-beta")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect();
        assert_eq!(
            betas,
            vec!["first-flag", "second-flag"],
            "重复的同名头必须全部转发，不能只剩最后一个"
        );
        handle.shutdown().await;
    }

    /// **端到端：客户端的 `accept-encoding` 不能到达上游。**
    ///
    /// 网关要从响应体里读错误文本（失败转移规则）与用量，而它的客户端不解压；
    /// 上游一旦压缩，这两件事都会静默失效。这条端到端断言钉住「请求侧确实没带它」——
    /// 单测只看得到成对列表，看不到真正发出去的那一份。
    #[tokio::test(flavor = "multi_thread")]
    async fn the_upstream_never_sees_accept_encoding() {
        let upstream = healthy_upstream("ok").await;
        let (handle, _source) = start_gateway(vec![vendor("v1", &upstream.uri(), 1)], "tok").await;

        let response = reqwest::Client::new()
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .header("accept-encoding", "gzip, deflate, br")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("请求");
        assert_eq!(response.status(), 200);

        let received = upstream.received_requests().await.expect("收到的请求");
        assert!(
            !received[0].headers.contains_key("accept-encoding"),
            "上游一旦压缩，错误文本与用量都读不出来"
        );
        handle.shutdown().await;
    }

    /// **重定向判定里「目标是不是本机」那一半的纯函数测试。**
    ///
    /// # 为什么没有「跟随非本机重定向」的端到端测试
    ///
    /// `wiremock` 只监听 loopback（`127.0.0.1`），因此它给出的**每一个**重定向目标都落在
    /// 「本机、不跟随」那一侧。要端到端测「跟随」需要一个非 loopback 的测试服务，而
    /// `MockServerBuilder::listener()` 要求自备监听器，绑定到本机网卡在 CI 里并不总是允许。
    ///
    /// 因此策略的前一半只由**纯函数单测**保证，后一半（不跟随本机）有端到端测试。
    /// **这是一处已知的验证缺口，不是「已经验过了」。**
    #[test]
    fn loopback_redirect_targets_are_recognized() {
        let url = |value: &str| reqwest::Url::parse(value).expect("URL");
        assert!(is_loopback_host(&url("http://127.0.0.1:8080/v1")));
        assert!(is_loopback_host(&url("http://localhost:8080/v1")));
        assert!(is_loopback_host(&url("http://[::1]:8080/v1")));
        // `0.0.0.0` 作为**目标**同样是本机，不能跟。
        assert!(is_loopback_host(&url("http://0.0.0.0:8080/v1")));
        // 真实供应商地址：可以跟随。
        assert!(!is_loopback_host(&url("https://api.example.com/v1")));
        assert!(!is_loopback_host(&url("http://192.168.1.10:8080/v1")));
    }

    /// **端到端：重定向回本机时必须停下。**
    ///
    /// 否则请求会在网关与「上游」之间打转直到耗尽连接。自我代理防护只检查**配置里的**
    /// 供应商地址，看不到重定向目标，因此这一刀必须由重定向策略自己补。
    ///
    /// 探测用的客户端**自己不跟随**：否则它会把网关交出来的 307 再跟一遍，
    /// 观察到的就不是网关的行为了。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_redirect_back_to_the_gateway_is_not_followed() {
        let redirector = MockServer::start().await;
        let (handle, _source) =
            start_gateway(vec![vendor("v1", &redirector.uri(), 1)], "tok").await;
        // 指向**网关自己**。
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(307).insert_header(
                "location",
                format!("{}/gateway/codex/r1/v1/messages", handle.address),
            ))
            .mount(&redirector)
            .await;

        let response = post_raw(&handle.address, "tok").await;
        assert_eq!(
            response.status(),
            307,
            "跟上本机就会变成环，必须把重定向原样交出去"
        );
        // 关键断言：没有绕回来。绕回去的话这里会变成好几条。
        assert_eq!(received(&redirector).await, 1, "重定向回本机时不得跟过去");
        handle.shutdown().await;
    }

    /// **一台卡住的上游必须被放弃，请求转给下一家。**
    ///
    /// # 与老平台的一处**有意差别**
    ///
    /// 老平台在重试循环**之前**建一个 `AbortSignal.timeout(upstreamTimeoutMs)` 并给每次
    /// 尝试复用，也就是整轮共享一个总预算；而且 `isRetryableFetchError` 在超时信号触发后
    /// 返回 `false`——**超时即终止**，不重试也不故障转移：
    ///
    /// ```js
    /// if (controller.signal.aborted || timeoutSignal.aborted) return false;
    /// ```
    ///
    /// 于是一台卡住的上游会把整次请求耗光（默认 10 分钟）然后放弃，
    /// **网关的故障转移形同不存在**——而那正是它存在的理由。
    ///
    /// 每次尝试仍可因连接或读取超时切换供应商，所有尝试另共享 10 分钟总预算。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_hanging_vendor_is_abandoned_and_the_next_one_serves() {
        // 卡住的第一家：延迟远大于超时。
        let slow = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string("too-late")
                    .set_delay(Duration::from_secs(10)),
            )
            .mount(&slow)
            .await;
        let fast = healthy_upstream("from-second").await;

        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            failure_threshold: 1,
            // 测试里把超时压到 300ms；真实默认值是 10 分钟。
            upstream_timeout: Duration::from_millis(300),
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(
            vec![vendor("v1", &slow.uri(), 1), vendor("v2", &fast.uri(), 2)],
            settings,
        ));
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state.register_route(sample_route()).await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 200, "卡住的那家必须被放弃");
        assert_eq!(response.text().await.expect("正文"), "from-second");
        assert_eq!(received(&slow).await, 1, "卡住的那家只该被尝试一次");
        assert_eq!(received(&fast).await, 1);
        handle.shutdown().await;
    }

    /// 起一个网关：两家候选 + 指定的切换阈值与熔断阈值。
    ///
    /// `failure_threshold` 是「同一供应商失败几次后切换」，`circuit_failures` 是
    /// 「失败几次后熔断」。两者是**不同**的机制，测试必须能分别拧动它们——
    /// 之前只有一个 `start_gateway` 用默认值，因此「熔断之后下一个请求还打不打坏供应商」
    /// 这件事在端到端层面从未被验证过。
    async fn start_with_thresholds(
        vendors: Vec<ApiVendor>,
        failure_threshold: usize,
        circuit_failures: usize,
        token: &str,
    ) -> (GatewayHandle, Arc<RecordingSource>) {
        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            failure_threshold,
            circuit: CircuitConfig {
                failure_threshold: circuit_failures,
                duration: Duration::from_secs(60),
            },
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(vendors, settings));
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state
            .register_route(VendorRoute {
                route_id: "r1".to_owned(),
                provider_id: ProviderId::Codex,
                vendor_id: "v1".to_owned(),
                mode: RouteMode::Dynamic,
                local_token: token.to_owned(),
                base_url: String::new(),
                external: false,
            })
            .await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");
        (handle, source)
    }

    /// 向上游发一次带请求体的 POST。
    async fn post(address: &str, token: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{address}/gateway/codex/r1/v1/messages"))
            .bearer_auth(token)
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("请求")
    }

    /// 同上，但**客户端自己不跟随重定向**。
    ///
    /// 网关的重定向行为只能用这种客户端观察：默认客户端会把网关交出来的重定向
    /// 再跟一遍，于是断言看到的是**客户端**的行为——测试会因此在修复前后都通过。
    async fn post_raw(address: &str, token: &str) -> reqwest::Response {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("客户端")
            .post(format!("{address}/gateway/codex/r1/v1/messages"))
            .bearer_auth(token)
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("请求")
    }

    /// 上游收到的请求数。
    async fn received(server: &MockServer) -> usize {
        server.received_requests().await.expect("收到的请求").len()
    }

    /// 一个总是失败的上游（429）。
    async fn failing_upstream() -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":"rate limited"}"#))
            .mount(&server)
            .await;
        server
    }

    /// 一个总是成功的上游。
    async fn healthy_upstream(body: &str) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.to_owned()))
            .mount(&server)
            .await;
        server
    }

    /// **同一供应商要重试到阈值才切换，而不是一失败就切。**
    ///
    /// 这是现网的语义，也是用户能感觉到的差别：429 常常是瞬时的，立刻换一家会白白把
    /// 请求挪到更差（或更贵）的候选上。`failure_threshold: 2` 时第一家应当收到**两次**，
    /// 第二家收到**一次**。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failing_vendor_is_retried_up_to_the_threshold_before_switching() {
        let first = failing_upstream().await;
        let second = healthy_upstream("from-second").await;
        let (handle, source) = start_with_thresholds(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            2,
            // 熔断阈值调高，避免它在切换之前就把第一家摘掉——那样测到的就不是「重试到阈值」。
            10,
            "tok",
        )
        .await;

        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.expect("正文"), "from-second");
        assert_eq!(received(&first).await, 2, "阈值是 2，第一家应当被重试一次");
        assert_eq!(received(&second).await, 1, "切换之后只该问第二家一次");
        let record = wait_for_record(&source).await;
        assert_eq!(record.retry_count, 2, "同一家重试和切换后的重试都应计数");
        assert_eq!(source.attempts.lock().expect("attempts")[0].len(), 3);
        handle.shutdown().await;
    }

    /// **熔断之后，后续请求不再打到坏掉的供应商。**
    ///
    /// 这是「自动故障转移」真正的价值：不是把这一次请求救回来，而是让接下来一段时间里的
    /// 请求**直接跳过**已知坏掉的那家。`failover.rs` 单测了 `is_circuit_open`，
    /// 但「第一个请求把熔断打开、第二个请求因此绕开」这条链在端到端上是没有覆盖的。
    ///
    /// 断言方式：第一个请求之后第一家的**请求计数不再增长**。
    #[tokio::test(flavor = "multi_thread")]
    async fn an_open_circuit_keeps_later_requests_away_from_the_broken_vendor() {
        let first = failing_upstream().await;
        let second = healthy_upstream("from-second").await;
        let (handle, _source) = start_with_thresholds(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            1,
            // 失败一次就熔断。
            1,
            "tok",
        )
        .await;

        let one = post(&handle.address, "tok").await;
        assert_eq!(one.status(), 200);
        assert_eq!(one.text().await.expect("正文"), "from-second");
        let after_first = received(&first).await;

        let two = post(&handle.address, "tok").await;
        assert_eq!(two.status(), 200);
        assert_eq!(two.text().await.expect("正文"), "from-second");
        assert_eq!(
            received(&first).await,
            after_first,
            "熔断已打开，第二个请求不该再打到这一家"
        );
        handle.shutdown().await;
    }

    /// **全部候选都失败时，把最后一次上游响应原样回放，而不是换成 502。**
    ///
    /// 客户端（尤其是 CLI）靠上游的状态码与错误体决定要不要重试、要不要提示用户续费。
    /// 网关自己编一个 502 会把这些信息全部抹掉。
    #[tokio::test(flavor = "multi_thread")]
    async fn when_every_candidate_fails_the_real_upstream_error_is_replayed() {
        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":"a-busy"}"#))
            .mount(&first)
            .await;
        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":"b-busy"}"#))
            .mount(&second)
            .await;

        let (handle, _source) = start_with_thresholds(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            2,
            10,
            "tok",
        )
        .await;

        let response = post(&handle.address, "tok").await;
        assert_eq!(
            response.status(),
            429,
            "必须回放上游真实状态码，而不是网关自己造的 502"
        );
        let body = response.text().await.expect("正文");
        assert!(
            body.contains("b-busy"),
            "应当回放最后一次上游的正文：{body}"
        );
        handle.shutdown().await;
    }

    /// **连不上上游（没有 HTTP 响应）也要转移。**
    ///
    /// 这条走的是 `send()` 返回 `Err` 的分支，与「上游返回了错误状态码」是**两条不同的
    /// 代码路径**。地址指向一个必然没有服务在听的端口，比 mock 一个连接错误更接近真实。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_connection_failure_fails_over_to_a_healthy_vendor() {
        let second = healthy_upstream("from-second").await;
        let (handle, source) = start_with_thresholds(
            vec![
                // 1 号端口不会有服务在听。
                vendor("v1", "http://127.0.0.1:1", 1),
                vendor("v2", &second.uri(), 2),
            ],
            1,
            10,
            "tok",
        )
        .await;

        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.text().await.expect("正文"), "from-second");
        wait_for_record(&source).await;
        let attempts = source.attempts.lock().expect("attempts").clone();
        let message = attempts[0][0]
            .error_message
            .as_deref()
            .expect("connection error");
        assert!(
            message.contains("connect") && message.contains("os error"),
            "{message}"
        );
        handle.shutdown().await;
    }

    /// **记录里必须留下「这次请求换过供应商」。**
    ///
    /// 界面上的故障转移视图与用量报表都读这条记录。只把请求救回来、却不记录换过，
    /// 用户就没法解释「为什么这次计费算在另一家头上」。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_failover_is_visible_in_the_request_record() {
        let first = failing_upstream().await;
        let second = healthy_upstream("from-second").await;
        let (handle, source) = start_with_thresholds(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            1,
            10,
            "tok",
        )
        .await;

        let response = post(&handle.address, "tok").await;
        assert_eq!(response.status(), 200);
        // 记录是转发完成之后写的，给它一点时间；不轮询数据库，直接看内存来源。
        let record = (0..50)
            .find_map(|_| {
                let taken = source.taken();
                if taken.is_empty() {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                } else {
                    taken.into_iter().next_back()
                }
            })
            .expect("应当留下一条记录");
        assert_eq!(
            record.vendor_id.as_str(),
            "v2",
            "记录里的供应商应当是最终成功的那一家"
        );
        assert!(record.switched, "换过供应商必须被记录下来");
        handle.shutdown().await;
    }

    /// **自动故障转移必须通知界面。**
    ///
    /// 少了这条通知，界面会一直显示一个**已经不是实际服务方**的供应商：请求正由第二家
    /// 服务，状态栏却写着第一家。用户看不到任何异常，只有账单对不上。
    ///
    /// 与手动切换的区别是 `reason`：这里是 `"failure"`，界面据此显示成「已自动切换」
    /// 而不是用户的操作。而且它**不该**修改路由的当前供应商（那只影响本次请求）。
    #[tokio::test(flavor = "multi_thread")]
    async fn an_automatic_failover_notifies_the_hook() {
        let first = failing_upstream().await;
        let second = healthy_upstream("from-second").await;
        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            failure_threshold: 1,
            circuit: CircuitConfig {
                failure_threshold: 10,
                duration: Duration::from_secs(60),
            },
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            settings,
        ));
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state.register_route(sample_route()).await;
        let recorder = Arc::new(SwitchRecorder::default());
        let recorder_clone = Arc::clone(&recorder);
        state.set_switch_hook(Arc::new(move |notice| {
            recorder_clone
                .notices
                .lock()
                .expect("lock")
                .push(notice.clone());
        }));
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 200);

        let notices = recorder.notices();
        assert_eq!(notices.len(), 1, "应当恰好通知一次：{notices:?}");
        assert_eq!(notices[0].route_id, "r1");
        assert_eq!(notices[0].vendor_id, "v2", "通知里必须是**接手**的那一家");
        assert_eq!(notices[0].reason.as_deref(), Some("failure"));

        // 自动转移不改路由的当前供应商：它只影响本次请求。
        let route = state.routes.read().await.get("r1").cloned();
        assert_eq!(
            route.map(|item| item.vendor_id),
            Some("v1".to_owned()),
            "自动转移不该持久化地改掉路由绑定的供应商"
        );
        handle.shutdown().await;
    }

    /// 自动转移**不修改**路由的当前供应商，这一点与手动切换不同。
    ///
    /// 单独写一条是因为它很容易被「顺手统一一下」而改错：一旦自动转移也去改路由，
    /// 用户手动锁定的选择就会被一次网络抖动悄悄覆盖掉。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_manual_switch_does_persist_the_route_vendor() {
        let (state, _recorder) = state_with_switch_recorder();
        state.register_route(sample_route()).await;

        state.switch_route_vendor("r1", "v9", None).await;

        let route = state.routes.read().await.get("r1").cloned();
        assert_eq!(route.map(|item| item.vendor_id), Some("v9".to_owned()));
    }

    /// **请求前的候选池调整必须落回路由并通知界面。**
    ///
    /// 原供应商被禁用/删除时，动态路由会在请求前换一家。这与「请求失败后转移」是两件事：
    /// 它不是异常，但**同样要写回**——否则状态栏会一直显示一个已经不存在的供应商，
    /// 而请求其实由别人在服务。
    ///
    /// 这里尤其要盯住「写回」：`handle_inner` 手里的路由是**克隆**，改它不影响注册表。
    /// 现网直接改内存里的 route 对象，因此移植时这一步很容易整个漏掉——
    /// 漏掉的表现是「功能正常、界面永远不对」，比报错难查得多。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pool_switch_is_persisted_on_the_route_and_notified() {
        let second = healthy_upstream("from-second").await;
        // 第一家**被禁用**：它仍是路由绑定的那一家，但已经不参与候选池。
        let mut disabled = vendor("v1", "http://127.0.0.1:1", 1);
        disabled.enabled = false;

        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(
            vec![disabled, vendor("v2", &second.uri(), 2)],
            settings,
        ));
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state.register_route(sample_route()).await;
        let recorder = Arc::new(SwitchRecorder::default());
        let recorder_clone = Arc::clone(&recorder);
        state.set_switch_hook(Arc::new(move |notice| {
            recorder_clone
                .notices
                .lock()
                .expect("lock")
                .push(notice.clone());
        }));
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        let response = post(&handle.address, "t").await;
        assert_eq!(response.status(), 200, "应当改由第二家服务");
        assert_eq!(response.text().await.expect("正文"), "from-second");

        // 1. 通知：原因必须是「候选池调整」，不是请求失败。
        let notices = recorder.notices();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert_eq!(notices[0].vendor_id, "v2");
        assert_eq!(
            notices[0].reason.as_deref(),
            Some("candidate-pool"),
            "原供应商是被禁用（不是熔断），因此不是 failure"
        );

        // 2. **写回**：下一个请求不该再从头选一遍。
        let route = state.routes.read().await.get("r1").cloned();
        assert_eq!(
            route.map(|item| item.vendor_id),
            Some("v2".to_owned()),
            "候选池的调整必须落回路由，否则界面会一直显示已停用的那一家"
        );
        handle.shutdown().await;
    }

    /// 原供应商**被熔断**时，同一个切换的原因应当是 `failure`。
    ///
    /// 界面据此决定要不要显示异常提示：熔断意味着一连串失败，值得告诉用户；
    /// 「被禁用」只是配置变了，不值得打扰。
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pool_switch_after_a_circuit_opens_is_reported_as_a_failure() {
        let first = failing_upstream().await;
        let second = healthy_upstream("from-second").await;
        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            failure_threshold: 1,
            // 失败一次就熔断。
            circuit: CircuitConfig {
                failure_threshold: 1,
                duration: Duration::from_secs(60),
            },
            ..GatewaySettings::default()
        };
        let source = Arc::new(RecordingSource::new(
            vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            settings,
        ));
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state.register_route(sample_route()).await;
        let recorder = Arc::new(SwitchRecorder::default());
        let recorder_clone = Arc::clone(&recorder);
        state.set_switch_hook(Arc::new(move |notice| {
            recorder_clone
                .notices
                .lock()
                .expect("lock")
                .push(notice.clone());
        }));
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        // 第一次：v1 失败、熔断打开、转移到 v2（这一条走的是「失败后转移」，原因 failure）。
        assert_eq!(post(&handle.address, "t").await.status(), 200);
        // 第二次：请求前就发现 v1 已熔断，候选池直接选 v2——这一次走的是候选池调整，
        // 但原因是 failure（因为原供应商确实被熔断了）。
        assert_eq!(post(&handle.address, "t").await.status(), 200);

        let reasons: Vec<Option<String>> = recorder
            .notices()
            .into_iter()
            .map(|notice| notice.reason)
            .collect();
        assert!(
            reasons
                .iter()
                .any(|reason| reason.as_deref() == Some("failure")),
            "熔断导致的原因必须是 failure：{reasons:?}"
        );
        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_route_never_fails_over() {
        let first = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string(r#"{"error":"rate limited"}"#))
            .mount(&first)
            .await;
        let second = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("from-second"))
            .mount(&second)
            .await;

        let settings = GatewaySettings {
            enabled: true,
            port: 0,
            failure_threshold: 1,
            ..GatewaySettings::default()
        };
        let source = Arc::new(MemorySource {
            vendors: vec![
                vendor("v1", &first.uri(), 1),
                vendor("v2", &second.uri(), 2),
            ],
            settings,
        });
        let state = Arc::new(GatewayState::new(source).expect("state"));
        state
            .register_route(VendorRoute {
                route_id: "r1".to_owned(),
                provider_id: ProviderId::Codex,
                vendor_id: "v1".to_owned(),
                // 锁定路由永远只用当前供应商，不得被健康熔断绕过。
                mode: RouteMode::Locked,
                local_token: "tok".to_owned(),
                base_url: String::new(),
                external: false,
            })
            .await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("request");
        // 回放第一家的真实响应，而不是切到第二家。
        assert_eq!(response.status(), 429);
        handle.shutdown().await;
    }

    /// 供应商列表可变的来源：用于「启动后才知道自己地址」的场景。
    struct MutableSource {
        vendors: std::sync::Mutex<Vec<ApiVendor>>,
        settings: GatewaySettings,
    }

    impl GatewaySource for MutableSource {
        fn vendors(&self) -> BoxFuture<'_, Vec<ApiVendor>> {
            let vendors = self.vendors.lock().expect("lock").clone();
            Box::pin(async move { vendors })
        }

        fn failover_rules(&self) -> BoxFuture<'_, Vec<FailoverRule>> {
            Box::pin(async move { Vec::new() })
        }

        fn settings(&self) -> GatewaySettings {
            self.settings.clone()
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_vendor_pointing_at_the_gateway_itself_is_rejected() {
        // 自我代理会形成无限循环，必须在转发前拒绝。
        // 供应商地址要等网关启动后才知道，因此这里用可变来源。
        let source = Arc::new(MutableSource {
            vendors: std::sync::Mutex::new(Vec::new()),
            settings: GatewaySettings {
                enabled: true,
                port: 0,
                ..GatewaySettings::default()
            },
        });
        let state = Arc::new(GatewayState::new(source.clone()).expect("state"));
        state
            .register_route(VendorRoute {
                route_id: "r1".to_owned(),
                provider_id: ProviderId::Codex,
                vendor_id: "v1".to_owned(),
                mode: RouteMode::Dynamic,
                local_token: "tok".to_owned(),
                base_url: String::new(),
                external: false,
            })
            .await;
        let handle = serve(Arc::clone(&state)).await.expect("serve");

        // 现在把供应商指向网关自己的地址。
        *source.vendors.lock().expect("lock") = vec![vendor("v1", &handle.address, 1)];

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{}/gateway/codex/r1/v1/messages", handle.address))
            .bearer_auth("tok")
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), 502);
        let body = response.text().await.expect("body");
        assert!(body.contains("不能指向本地 Gateway"), "{body}");
        handle.shutdown().await;
    }
}
