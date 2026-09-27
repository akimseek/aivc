//! 终端 → 网关的路由绑定。
//!
//! # 这个模块此前**不存在**，而它是整条链路的断点
//!
//! 「供应商 + 本地网关」要生效，终端拿到的上游地址必须指向网关上的某个路由，凭据必须换成
//! 该路由的**本地令牌**。这两样东西合起来就是 [`ResolvedRoute`]。
//!
//! 而在此之前：
//!
//! | 环节 | 状态 |
//! | --- | --- |
//! | [`ResolvedRoute`] 类型 | 有 |
//! | 消费它的三个函数（`provider_environment` 等） | 有 |
//! | **产生它的函数** | **不存在** |
//!
//! 全仓唯一的构造点在 `aivc-terminal` 的单元测试里，生产代码一处都没有。于是终端启动时
//! 只能传 `None`：**网关永远不参与，界面上的供应商配置对终端没有任何影响**。
//!
//! # 路由是**按终端**的，不是按平台的
//!
//! 每个终端启动时新建一条路由，终端关闭时销毁。理由有三条：
//!
//! 1. **故障转移要按终端记录**：一台终端切到备用供应商，不该影响另一台；
//! 2. **令牌不能共用**：路由的本地令牌是这台终端的凭据，共用等于把一台终端的权限给了全部；
//! 3. **候选池是动态的**：供应商的启用状态随时可变，按终端新建就不会拿到过期的绑定。
//!
//! 路由编号**直接用终端编号**（`terminal:start` 的参数）。这不是偷懒：
//! [`aivc_gateway::SwitchNotice`] 用路由编号作为「哪台终端」的标识，两者不同名时
//! 界面收到故障切换通知也不知道该提醒哪一台。终端编号因此必须是**路径安全**的——
//! 它会出现在 `/gateway/<平台>/<路由编号>` 里，由
//! [`crate::validate::require_terminal_id`] 在入口处保证。
//!
//! # `None` 不是错误
//!
//! 「没有可用供应商」时返回 [`None`]，终端照常启动、直连官方 API、用用户自己的登录。
//! 这是现网语义，也是合理的默认：**没配供应商的人仍然要能用 CLI**。

use std::sync::Arc;

use aivc_core::error::{Error, IpcError, Result};
use aivc_core::id::VendorId;
use aivc_core::target::ProviderId;
use aivc_gateway::{GatewayState, RouteMode, VendorRoute, build_route_url};
use aivc_provider::ProviderContext;
use aivc_terminal::command::{ResolvedRoute, supports_gateway};
use serde_json::Value;

/// 本地路由令牌的字节数对应的十六进制长度。
///
/// 与外部访问令牌同一个构造方式（两个 v4 UUID 的简单形式），因此是 32 个十六进制字符
/// 的两倍——**不是**为了好看：本地令牌是网关唯一的本地鉴权凭据，长度不够就等于没有。
const LOCAL_TOKEN_HEX_LEN: usize = 64;

/// 一条已创建并登记到网关的路由。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteBinding {
    /// 路由编号（等于终端编号）。
    pub route_id: String,
    /// 交给 CLI 的上游地址与令牌。
    pub route: ResolvedRoute,
    /// 当前供应商编号。
    pub vendor_id: String,
    /// 路由模式。
    pub mode: RouteMode,
}

/// 为终端创建与销毁网关路由。
///
/// 抽成 trait 的理由与 [`crate::terminal::EventEmitter`] 相同：`terminal:start` 的行为
/// （拼哪条命令、注入哪些变量）必须能在**没有网关**的情况下被测试驱动。
pub trait RouteSource: Send + Sync {
    /// 为一路终端创建路由。
    ///
    /// 返回 [`None`] 表示「这次不走网关」（平台不支持、网关没启动、或没有可用供应商），
    /// 而不是失败。
    ///
    /// `distro` 是目标的 WSL 发行版；`Some` 时上游地址必须先探测成**发行版内可达**的宿主
    /// 地址（见 [`ResolvedRoute`] 的说明）。
    ///
    /// # Errors
    ///
    /// 显式指定的供应商属于**另一个平台**时返回错误——静默换一个供应商比报错更糟，
    /// 用户会以为自己的选择生效了。
    fn create<'a>(
        &'a self,
        provider: ProviderId,
        distro: Option<&'a str>,
        vendor_id: Option<&'a str>,
        terminal_id: &'a str,
    ) -> aivc_core::ports::BoxFuture<'a, Result<Option<RouteBinding>>>;

    /// 销毁一路终端的路由。
    ///
    /// 路由不存在时静默返回：终端可能压根没走网关，而「关闭终端」不该因此报错。
    fn destroy<'a>(&'a self, route_id: &'a str) -> aivc_core::ports::BoxFuture<'a, ()>;
}

/// 不做任何事的路由源。
///
/// 用于「网关未启用」与测试：终端照常启动，但不注入路由。
pub struct NullRouteSource;

impl RouteSource for NullRouteSource {
    fn create<'a>(
        &'a self,
        _provider: ProviderId,
        _distro: Option<&'a str>,
        _vendor_id: Option<&'a str>,
        _terminal_id: &'a str,
    ) -> aivc_core::ports::BoxFuture<'a, Result<Option<RouteBinding>>> {
        Box::pin(async { Ok(None) })
    }

    fn destroy<'a>(&'a self, _route_id: &'a str) -> aivc_core::ports::BoxFuture<'a, ()> {
        Box::pin(async {})
    }
}

/// 真实实现：把网关状态、供应商仓储与 WSL 探测拼在一起。
///
/// 三者缺一不可：网关给出**监听地址**，供应商仓储给出**用哪一条上游**，
/// WSL 探测给出**发行版里怎么找到宿主**。
pub struct GatewayRouteSource {
    gateway: Arc<GatewayState>,
    context: ProviderContext,
}

impl GatewayRouteSource {
    /// 构造。
    #[must_use]
    pub const fn new(gateway: Arc<GatewayState>, context: ProviderContext) -> Self {
        Self { gateway, context }
    }

    /// 挑出这条路由要绑定的供应商。
    ///
    /// 顺序与现网一致：**显式指定且可用**的优先，否则取该平台候选池里的第一个。
    /// 显式指定的供应商存在但属于另一个平台时**报错**。
    ///
    /// # Errors
    ///
    /// 平台不匹配。
    async fn pick_vendor(
        &self,
        provider: ProviderId,
        vendor_id: Option<&str>,
    ) -> Result<Option<aivc_core::vendor::ApiVendor>> {
        // 只取**配置完整**的供应商（启用 + Key 非空 + 地址非空）。现网的
        // `createVendorRoute` 只看 `enabled`，于是「启用了但没填 Key」的供应商会生成一条
        // 必定失败的路由——终端能起来、每次请求都报上游错误。这里收紧成 `is_route_eligible`。
        let eligible = self.context.vendor().list_route_eligible(provider).await?;
        let Some(requested) = vendor_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(eligible.into_iter().next());
        };
        if let Some(found) = eligible
            .iter()
            .find(|vendor| vendor.id.as_str() == requested)
        {
            return Ok(Some(found.clone()));
        }
        // 不在候选池里：可能是「属于别的平台」——那要报错，否则用户以为自己选的生效了。
        if let Some(other) = self
            .context
            .vendor()
            .get(&VendorId::from(requested))
            .await?
            && other.provider_id != provider
        {
            return Err(Error::Ipc(IpcError::CommandFailed {
                message: "供应商协议与终端类型不匹配。".to_owned(),
            }));
        }
        // 属于本平台但不可用（未启用 / 配置不全）：退回候选池的第一个，与现网一致。
        Ok(eligible.into_iter().next())
    }
}

impl RouteSource for GatewayRouteSource {
    fn create<'a>(
        &'a self,
        provider: ProviderId,
        distro: Option<&'a str>,
        vendor_id: Option<&'a str>,
        terminal_id: &'a str,
    ) -> aivc_core::ports::BoxFuture<'a, Result<Option<RouteBinding>>> {
        Box::pin(async move {
            if !supports_gateway(provider) {
                return Ok(None);
            }
            // 网关地址为空 = 没有在跑。这不是错误（用户可能关掉了网关），但**必须留痕**：
            // 否则「配了供应商却不生效」在日志里查不出任何线索。
            let address = self.gateway.address().await;
            if address.trim().is_empty() {
                tracing::info!(
                    provider = provider.as_str(),
                    "网关未运行，本次终端不注入路由"
                );
                return Ok(None);
            }
            let Some(vendor) = self.pick_vendor(provider, vendor_id).await? else {
                tracing::info!(
                    provider = provider.as_str(),
                    "该平台没有可用的供应商，本次终端不注入路由"
                );
                return Ok(None);
            };
            let Some(port) = port_of(&address) else {
                tracing::warn!(address, "网关地址里解析不出端口，本次终端不注入路由");
                return Ok(None);
            };

            // 宿主地址：本机就是网关地址本身；WSL 下 127.0.0.1 指向发行版自己，
            // 必须换成探测到的默认路由网关地址。
            let host = match distro.map(str::trim).filter(|value| !value.is_empty()) {
                Some(name) => {
                    let detected = self.context.wsl().detect_gateway_host(name, port).await;
                    format!("http://{detected}:{port}")
                }
                None => address.clone(),
            };

            let local_token = generate_local_token();
            let base_url = build_route_url(&host, provider, terminal_id);
            self.gateway
                .register_route(VendorRoute {
                    route_id: terminal_id.to_owned(),
                    provider_id: provider,
                    vendor_id: vendor.id.as_str().to_owned(),
                    mode: RouteMode::Dynamic,
                    local_token: local_token.clone(),
                    // 这一项是**供应商的上游地址**，不是路由地址。路由地址由 `base_url`
                    // 单独算出来交给 CLI——两者混用会让网关把请求转发给自己。
                    base_url: vendor.api_base_url.clone(),
                    external: false,
                })
                .await;
            tracing::info!(
                provider = provider.as_str(),
                route = terminal_id,
                vendor = vendor.id.as_str(),
                // **地址要记下来。** WSL 目标下这个 host 必须是**发行版内可达**的宿主地址
                // （`127.0.0.1` 在发行版里指向它自己）。出问题时只看「绑定了路由」查不出来——
                // 必须能看到它到底指向哪里。这里不含令牌，可以安全落日志。
                base_url,
                distro = distro.unwrap_or(""),
                "已为终端绑定网关路由"
            );
            Ok(Some(RouteBinding {
                route_id: terminal_id.to_owned(),
                route: ResolvedRoute {
                    base_url,
                    local_token,
                },
                vendor_id: vendor.id.as_str().to_owned(),
                mode: RouteMode::Dynamic,
            }))
        })
    }

    fn destroy<'a>(&'a self, route_id: &'a str) -> aivc_core::ports::BoxFuture<'a, ()> {
        Box::pin(async move {
            if self.gateway.remove_route(route_id).await.is_some() {
                tracing::debug!(route = route_id, "已销毁终端的网关路由");
            }
        })
    }
}

/// 从 `http://127.0.0.1:8787` 里取出端口。
///
/// **必须从地址里取**，不能读设置里的端口：设置的默认值是 `0`（自动分配），
/// 此时真实端口由操作系统决定，只有监听成功后回写的地址才知道。
#[must_use]
pub fn port_of(address: &str) -> Option<u16> {
    let rest = address.rsplit_once(':')?.1;
    rest.trim_end_matches('/').parse().ok()
}

/// 生成一个本地路由令牌。
#[must_use]
pub fn generate_local_token() -> String {
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    debug_assert_eq!(token.len(), LOCAL_TOKEN_HEX_LEN);
    token
}

/// 供命令层构造返回值用的形状。
#[must_use]
pub fn binding_value(binding: &RouteBinding) -> Value {
    serde_json::json!({
        "routeId": binding.route_id,
        "vendorId": binding.vendor_id,
        "mode": if binding.mode == RouteMode::Locked { "locked" } else { "dynamic" },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use aivc_core::gateway::GatewayVendorHealthStatus as GatewayHealthStatus;
    use aivc_core::target::Platform;
    use aivc_core::vendor::ApiVendorInput;
    use aivc_gateway::{GatewayState, RepositorySource};
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
    use aivc_provider::ProviderRepositories;

    #[test]
    fn a_port_is_read_from_the_address_that_was_actually_bound() {
        // 设置的默认端口是 0（自动分配），因此**只有地址是真的**。
        assert_eq!(port_of("http://127.0.0.1:8787"), Some(8787));
        assert_eq!(port_of("http://127.0.0.1:51000/"), Some(51000));
        assert_eq!(port_of("http://127.0.0.1"), None);
        assert_eq!(port_of(""), None);
    }

    #[test]
    fn a_local_token_is_long_enough_and_differs_every_time() {
        let first = generate_local_token();
        let second = generate_local_token();
        assert_eq!(first.len(), LOCAL_TOKEN_HEX_LEN);
        assert!(first.chars().all(|value| value.is_ascii_hexdigit()));
        // 令牌重复等于两台终端共用凭据，那是最不该出现的情况。
        assert_ne!(first, second);
    }

    #[tokio::test]
    async fn the_null_source_never_creates_a_route() {
        let source = NullRouteSource;
        let created = source
            .create(ProviderId::Codex, None, None, "term-1")
            .await
            .expect("不会失败");
        assert!(created.is_none());
        // 销毁不存在的路由不该 panic。
        source.destroy("term-1").await;
    }

    // -----------------------------------------------------------------------
    // 真实链路：真网关 + 真供应商仓储 + 真路由源
    // -----------------------------------------------------------------------

    /// 一套能跑起来的真实环境：临时运行根、真数据库、真网关。
    ///
    /// 返回的 `TempDir` 必须活着——它一被丢弃，数据库与设置就都没了。
    async fn environment() -> (ProviderContext, Arc<GatewayState>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let settings = SettingsStore::new(dir.path().join("runtime/settings.json"));
        settings
            .update(|raw| {
                raw.set_gateway_enabled(true);
                // 0 = 由操作系统分配端口：这样测试不会和真实应用的端口撞车。
                raw.set_gateway_port(0);
            })
            .await
            .expect("settings");
        let wsl = WslAdapter::new(
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
            WslConfig {
                use_worker: false,
                ..WslConfig::default()
            },
        );
        let context = ProviderContext::new(
            paths,
            settings.clone(),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );
        let source = Arc::new(
            RepositorySource::new(
                context.vendor().clone(),
                context.failover_rule().clone(),
                context.gateway_request_log().clone(),
                settings,
            )
            .await
            .expect("source"),
        );
        let state = Arc::new(GatewayState::new(source).expect("state"));
        (context, state, dir)
    }

    /// 存一条可用的 Claude 供应商。
    async fn save_claude_vendor(context: &ProviderContext) {
        let input: ApiVendorInput = serde_json::from_value(serde_json::json!({
            "providerId": "claude",
            "name": "test-vendor",
            "apiKey": "upstream-key",
            // 上游不存在也不要紧：本测试只关心**网关认不认这条路由**，
            // 转发失败发生在鉴权之后。
            "apiBaseUrl": "http://127.0.0.1:1",
            "enabled": true,
        }))
        .expect("供应商输入");
        context.vendor().save(&input).await.expect("保存供应商");
    }

    /// **熔断必须活过一次重启。**
    ///
    /// 转发路径读的是内存里的 `HealthTracker`，而它过去**从不**从数据库读回。于是：
    ///
    /// - 一台已经被熔断的供应商，重启之后会被从头重试（用户又白等一轮失败）；
    /// - `gateway:get-vendor-health` 读的那张表也没有写入方，界面上那一栏恒为空。
    ///
    /// 这里用**真实数据库**走完整条路：打一次失败把熔断打开并落库，
    /// 然后换一套全新的 `GatewayState` + `RepositorySource`（模拟重启）读回，断言熔断仍开着。
    #[tokio::test(flavor = "multi_thread")]
    async fn an_open_circuit_survives_a_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let settings = SettingsStore::new(dir.path().join("runtime/settings.json"));
        settings
            .update(|raw| {
                raw.set_gateway_enabled(true);
                raw.set_gateway_port(0);
                // 一次失败就熔断；只有一家候选，因此不会转移到别处。
                raw.set_gateway_circuit_failure_threshold(1);
                raw.set_gateway_failure_threshold(1);
            })
            .await
            .expect("settings");
        let wsl = WslAdapter::new(
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
            WslConfig {
                use_worker: false,
                ..WslConfig::default()
            },
        );
        let context = ProviderContext::new(
            paths,
            settings.clone(),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );
        // 一台**打不通**的供应商：1 号端口不会有服务在听。
        let input: ApiVendorInput = serde_json::from_value(serde_json::json!({
            "providerId": "codex",
            "name": "坏供应商",
            "apiKey": "k",
            "apiBaseUrl": "http://127.0.0.1:1",
            "enabled": true,
        }))
        .expect("供应商输入");
        let vendor = context.vendor().save(&input).await.expect("保存供应商");

        let state = {
            let source = Arc::new(
                RepositorySource::new(
                    context.vendor().clone(),
                    context.failover_rule().clone(),
                    context.gateway_request_log().clone(),
                    settings.clone(),
                )
                .await
                .expect("来源"),
            );
            let state = Arc::new(GatewayState::new(source).expect("状态"));
            wire_health(&state, &context);
            state
                .register_route(VendorRoute {
                    route_id: "r-restart".to_owned(),
                    provider_id: ProviderId::Codex,
                    vendor_id: vendor.id.as_str().to_owned(),
                    mode: RouteMode::Dynamic,
                    local_token: "tok".to_owned(),
                    base_url: "http://127.0.0.1:1".to_owned(),
                    external: false,
                })
                .await;
            state
        };
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");
        let _ = reqwest::Client::new()
            .post(format!(
                "{}/gateway/codex/r-restart/v1/messages",
                handle.address
            ))
            .bearer_auth("tok")
            .body(r#"{"model":"gpt-5"}"#)
            .send()
            .await
            .expect("请求");
        assert!(
            state
                .health_snapshot()
                .await
                .into_iter()
                .any(|item| item.status == GatewayHealthStatus::Open),
            "一次失败 + 阈值 1 应当把熔断打开"
        );
        handle.shutdown().await;

        // **重启**：全新的状态、全新的来源、同一个数据库。
        let restored = {
            let source = Arc::new(
                RepositorySource::new(
                    context.vendor().clone(),
                    context.failover_rule().clone(),
                    context.gateway_request_log().clone(),
                    settings,
                )
                .await
                .expect("来源"),
            );
            let fresh = Arc::new(GatewayState::new(source).expect("状态"));
            wire_health(&fresh, &context);
            fresh.hydrate_health().await;
            fresh
        };
        assert!(
            restored
                .health_snapshot()
                .await
                .into_iter()
                .any(|item| item.status == GatewayHealthStatus::Open),
            "熔断必须活过重启，否则用户会对着同一台坏供应商再白等一轮"
        );
    }

    /// **过期的熔断在读回时要被归一化，不能把供应商一直关在门外。**
    ///
    /// 熔断是有期限的。库里存着一行「开着、但截止时间已经过去」的记录时，
    /// 如果读回后仍然算「开着」，那台供应商就**永远不会再被尝试**——
    /// 用户看到的是一台怎么都不用的供应商，而且没有任何报错。
    ///
    /// 归一化发生在仓储的读取路径上（`normalize_status`：`open` + 已过期 → `HalfOpen`）。
    /// 这条测试从**真实数据库**走一遍：写一行已过期的熔断，读回，断言它**不再是开着的**。
    #[tokio::test(flavor = "multi_thread")]
    async fn an_expired_circuit_is_normalized_when_read_back() {
        let (context, state, _dir) = environment().await;

        // 直接往库里写一行：状态「开着」，但截止时间远在过去。
        let vendor_id = VendorId::from("expired-vendor");
        context
            .gateway_health()
            .upsert(&aivc_core::gateway::GatewayVendorHealth {
                vendor_id: vendor_id.clone(),
                provider_id: ProviderId::Codex,
                status: aivc_core::gateway::GatewayVendorHealthStatus::Open,
                failure_count: 3,
                success_count: 0,
                failure_rate: 1.0,
                last_failure_at: Some(aivc_core::time::Timestamp::from_raw(
                    "2020-01-01T00:00:00.000Z",
                )),
                last_success_at: None,
                circuit_until: Some(aivc_core::time::Timestamp::from_raw(
                    "2020-01-01T00:05:00.000Z",
                )),
                last_failure_reason: Some("很久以前的失败".to_owned()),
            })
            .await
            .expect("写入健康记录");

        wire_health(&state, &context);
        state.hydrate_health().await;

        let still_open = state.health_snapshot().await.into_iter().any(|item| {
            item.vendor_id == vendor_id
                && item.status == aivc_core::gateway::GatewayVendorHealthStatus::Open
        });
        assert!(
            !still_open,
            "已过期的熔断读回后必须归一化，否则那台供应商永远不会再被尝试"
        );
    }

    /// 给网关装上健康持久化钩子（与 `aivc-app` 的组装根同一套动作）。
    ///
    /// 抽出来是为了让「重启」那一步能重复装配——组装根里那段是内联的。
    fn wire_health(state: &Arc<GatewayState>, context: &ProviderContext) {
        let health = context.gateway_health().clone();
        let loader = health.clone();
        state.set_health_hook(aivc_gateway::HealthHook {
            load: Arc::new(move || {
                let repo = loader.clone();
                Box::pin(async move { repo.load_all().await.unwrap_or_default() })
            }),
            save: Arc::new(move |entry| {
                let repo = health.clone();
                Box::pin(async move {
                    let _ = repo.upsert(&entry).await;
                })
            }),
        });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_route_the_terminal_gets_is_one_the_gateway_actually_accepts() {
        // **这是「供应商 + 本地网关」这条链路上第一条真正的端到端证据。**
        //
        // 它一次回答三个问题：
        // 1. 终端拿到的地址真的指向网关上的那条路由；
        // 2. 终端拿到的本地令牌真的能通过网关鉴权（错的令牌不能）；
        // 3. 终端关掉之后路由真的没了。
        let (context, state, _dir) = environment().await;
        save_claude_vendor(&context).await;
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");
        let address = state.address().await;

        let source = GatewayRouteSource::new(Arc::clone(&state), context);
        let binding = source
            .create(ProviderId::Claude, None, None, "term-e2e")
            .await
            .expect("创建路由")
            .expect("有可用供应商时应当创建路由");

        assert_eq!(
            binding.route.base_url,
            format!("{address}/gateway/claude/term-e2e"),
            "终端拿到的地址必须是网关上的这条路由"
        );

        let client = reqwest::Client::new();
        let wrong = client
            .post(&binding.route.base_url)
            .header("authorization", "Bearer not-the-token")
            .body("{}")
            .send()
            .await
            .expect("请求");
        assert_eq!(
            wrong.status().as_u16(),
            401,
            "错的令牌必须被拒——否则本地令牌形同虚设"
        );

        let right = client
            .post(&binding.route.base_url)
            .header(
                "authorization",
                format!("Bearer {}", binding.route.local_token),
            )
            .body("{}")
            .send()
            .await
            .expect("请求");
        assert_ne!(right.status().as_u16(), 401, "终端拿到的令牌必须能通过鉴权");
        assert_ne!(
            right.status().as_u16(),
            404,
            "路由必须存在——否则终端会拿着一个网关不认识的地址去请求"
        );

        // 终端关闭 → 路由消失。
        source.destroy(&binding.route_id).await;
        let after = client
            .post(&binding.route.base_url)
            .header(
                "authorization",
                format!("Bearer {}", binding.route.local_token),
            )
            .body("{}")
            .send()
            .await
            .expect("请求");
        assert_eq!(after.status().as_u16(), 404, "终端关闭后路由必须消失");

        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn without_an_eligible_vendor_the_terminal_runs_direct() {
        // 没配供应商时**不是错误**：CLI 直连官方 API、用用户自己的登录。
        // 这是现网语义，也是「没配供应商的人仍然要能用 CLI」的保证。
        let (context, state, _dir) = environment().await;
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");

        let source = GatewayRouteSource::new(Arc::clone(&state), context);
        let binding = source
            .create(ProviderId::Claude, None, None, "term-none")
            .await
            .expect("不该失败");
        assert!(binding.is_none(), "没有可用供应商时不该创建路由");

        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_vendor_from_another_provider_is_refused() {
        // 静默换一个供应商比报错更糟：用户会以为自己的选择生效了。
        let (context, state, _dir) = environment().await;
        save_claude_vendor(&context).await;
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");

        // 造一条 Codex 供应商。
        let input: ApiVendorInput = serde_json::from_value(serde_json::json!({
            "providerId": "codex",
            "name": "codex-vendor",
            "apiKey": "k",
            "apiBaseUrl": "http://127.0.0.1:1",
        }))
        .expect("供应商输入");
        let codex_vendor = context.vendor().save(&input).await.expect("保存");

        let source = GatewayRouteSource::new(Arc::clone(&state), context);
        let error = source
            .create(
                ProviderId::Claude,
                None,
                Some(codex_vendor.id.as_str()),
                "term-mismatch",
            )
            .await
            .expect_err("平台不匹配必须报错");
        assert!(error.to_string().contains("不匹配"), "{error}");

        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_qoder_terminal_never_gets_a_route() {
        // Qoder 不支持网关：给它注入地址只会让它报一个难懂的错误。
        let (context, state, _dir) = environment().await;
        save_claude_vendor(&context).await;
        let handle = aivc_gateway::serve(Arc::clone(&state))
            .await
            .expect("起网关");

        let source = GatewayRouteSource::new(Arc::clone(&state), context);
        assert!(
            source
                .create(ProviderId::Qoder, None, None, "term-q")
                .await
                .expect("不该失败")
                .is_none()
        );

        handle.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_gateway_that_is_not_running_produces_no_route() {
        // 网关没起来时终端照常启动、直连官方 API——但**必须留痕**，否则
        // 「配了供应商却不生效」在日志里查不出线索。
        let (context, state, _dir) = environment().await;
        save_claude_vendor(&context).await;
        // 刻意不 `serve`。

        let source = GatewayRouteSource::new(state, context);
        assert!(
            source
                .create(ProviderId::Claude, None, None, "term-off")
                .await
                .expect("不该失败")
                .is_none(),
            "网关未运行时不该产出路由"
        );
    }
}
