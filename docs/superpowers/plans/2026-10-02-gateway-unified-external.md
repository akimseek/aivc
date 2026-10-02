# 网关统一外部接入 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 外部接入路径改为 `/<平台>`（如 `http://127.0.0.1:8787/codex` 直达 codex 候选池），界面端点与命名同步收口——本地/外部概念合一。

**Architecture:** 单网关服务器不变。`/gateway/<平台>/<编号>/…` 继续走终端路由分支（一字不动）；新增「首段即平台」的外部分支：每平台惰性注册一条合成路由（`external-<平台>`，`external: true`、动态池、共享令牌鉴权），路径剥离与日志/脱敏复用现有 `prefix()` / `strip_route_prefix` / `sanitize_external_request_path`。UI 端点清单改为 `{base}/<平台>`，去掉路由编号占位符与凭据头列，导航/卡片/开关措辞统一。

**Tech Stack:** Rust（axum、tokio RwLock、wiremock 测试）、React + TypeScript + vitest（jsdom）。

**Spec:** `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`（§6 为 2026-10-02 用户裁定：导航名「网关」、端点不保留凭据头列、外部接入保持默认关闭 + 共享令牌）。

## Global Constraints

- 终端路由形制 `/gateway/<平台>/<编号>` **不变**（parity 矩阵 G1 契约）。
- 外部接入**保持默认关闭 + 共享令牌**（四种凭据候选、sha256 比对、`?key=` 从上游与日志剔除——全部复用，不改）。
- 仅监听 `127.0.0.1`，不改。
- 旧版 `/external/<平台>` 形制**不采用**（刻意偏离，记入 03/40/41 注记）。
- 端点清单**不保留凭据头列**（用户裁定）；端点行只留 平台名 + 地址 + 复制。
- 无 IPC 命令面改动（端点地址由 UI 从 `baseUrl` + 平台清单拼出）。
- Rust 命令一律经隔离工具链脚本（`scripts/cargo.ps1` / `scripts/verify.ps1`）；命令补 `--offline`。
- 既有注释风格：解释「为什么」；改动处注明 2026-10-02 与 spec 路径。

---

### Task 1: routes.rs —— 外部路径识别、裸前缀与惰性注册

**Files:**
- Modify: `crates/aivc-gateway/src/routes.rs`（新增 3 个函数/方法 + `prefix()` 分支；测试追加在 `mod tests` 内）
- Test: 同文件 `mod tests`

**Interfaces:**
- Consumes: 现有 `ProviderId`、`RouteMode`、`VendorRoute`、`RouteRegistry`、`segment()`、`parse_provider()`。
- Produces:
  - `pub fn external_route_id(provider_id: ProviderId) -> String`（`"external-<平台>"`）
  - `pub fn extract_external_provider(path: &str) -> Option<ProviderId>`（首段为平台且非 `gateway` 时返回；其余 `None`）
  - `VendorRoute::prefix()`：`external == true` 时返回 `/<平台>`
  - `RouteRegistry::ensure_external_route(&mut self, provider_id: ProviderId) -> String`（幂等；返回路由编号）

- [ ] **Step 1: 写失败测试**

在 `mod tests` 里 `external_routes_are_indexed_per_provider` 测试之后追加：

```rust
    #[test]
    fn the_first_segment_decides_the_external_provider() {
        assert_eq!(extract_external_provider("/codex"), Some(ProviderId::Codex));
        assert_eq!(
            extract_external_provider("/codex/v1/responses"),
            Some(ProviderId::Codex)
        );
        assert_eq!(
            extract_external_provider("/claude/v1/messages?stream=true"),
            Some(ProviderId::Claude)
        );
        // 终端路由不走外部分支。
        assert_eq!(extract_external_provider("/gateway/codex/r1/v1/messages"), None);
        // 未知平台与根路径都不是外部路由。
        assert_eq!(extract_external_provider("/openai/v1"), None);
        assert_eq!(extract_external_provider("/"), None);
    }

    #[test]
    fn external_routes_use_the_bare_provider_prefix() {
        let mut external = route("external-codex", "tok");
        external.external = true;
        // 外部前缀就是 `/<平台>`；终端路由仍是 `/gateway/<平台>/<编号>`。
        assert_eq!(external.prefix(), "/codex");
        assert_eq!(route("r1", "tok").prefix(), "/gateway/codex/r1");
    }

    #[test]
    fn ensure_external_route_registers_lazily_and_is_idempotent() {
        let mut registry = RouteRegistry::new();
        let id = registry.ensure_external_route(ProviderId::Codex);
        assert_eq!(id, "external-codex");
        let registered = registry.external_route(ProviderId::Codex).expect("已注册");
        assert!(registered.external);
        assert_eq!(registered.mode, RouteMode::Dynamic);
        assert!(registered.vendor_id.is_empty());
        // 再调一次不重复建。
        assert_eq!(
            registry.ensure_external_route(ProviderId::Codex),
            "external-codex"
        );
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn external_route_ids_are_named_after_the_provider() {
        assert_eq!(external_route_id(ProviderId::Codex), "external-codex");
        assert_eq!(external_route_id(ProviderId::Qoder), "external-qoder");
    }
```

- [ ] **Step 2: 跑测试确认失败**

Run: `.\scripts\cargo.ps1 test -p aivc-gateway routes:: --offline`
（隔离工具链环境见 README「环境准备」；如用全局 cargo 直接 `cargo test -p aivc-gateway routes:: --offline`。）
Expected: 编译失败——`extract_external_provider` / `external_route_id` / `ensure_external_route` 未定义。

- [ ] **Step 3: 最小实现**

3a. 在 `route_prefix()` 之后新增：

```rust
/// 外部路由的编号：`external-<平台>`（与旧平台一致，日志里一眼可辨）。
#[must_use]
pub fn external_route_id(provider_id: ProviderId) -> String {
    format!("external-{}", provider_id.as_str())
}

/// 外部接入路径的第一段平台。
///
/// 路径形状为 `/<平台>/…`（如 `/codex/v1/responses`）。`/gateway/…` 走终端路由
/// 分支、未知平台与其它路径都会落 404——那些不由这里判定，返回 [`None`]。
#[must_use]
pub fn extract_external_provider(path: &str) -> Option<ProviderId> {
    let first = segment(path, 0)?;
    if first == ROUTE_PREFIX.trim_start_matches('/') {
        return None;
    }
    parse_provider(first)
}
```

3b. `VendorRoute::prefix()` 改为：

```rust
    /// 该路由在网关上的前缀。
    ///
    /// 终端路由是 `/gateway/<平台>/<路由编号>`；外部路由是 `/<平台>`（2026-10-02
    /// 统一外部接入，见 `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`）。
    #[must_use]
    pub fn prefix(&self) -> String {
        if self.external {
            format!("/{}", self.provider_id.as_str())
        } else {
            route_prefix(self.provider_id, &self.route_id)
        }
    }
```

3c. `impl RouteRegistry` 内（`external_route()` 之后）新增：

```rust
    /// 确保某平台的外部路由存在（惰性注册，旧平台同款；幂等）。
    ///
    /// 外部路由没有终端绑定，永远是动态候选池；`vendor_id` / `local_token` /
    /// `base_url` 对外部路由都没有意义，留空。返回其路由编号。
    pub fn ensure_external_route(&mut self, provider_id: ProviderId) -> String {
        let route_id = external_route_id(provider_id);
        if self.external_route(provider_id).is_none() {
            self.insert(VendorRoute {
                route_id: route_id.clone(),
                provider_id,
                vendor_id: String::new(),
                mode: RouteMode::Dynamic,
                local_token: String::new(),
                base_url: String::new(),
                external: true,
            });
        }
        route_id
    }
```

- [ ] **Step 4: 跑测试确认通过**

Run: `.\scripts\cargo.ps1 test -p aivc-gateway routes:: --offline`
Expected: PASS（routes 模块全部用例，含既有用例无回归）。

- [ ] **Step 5: 提交**

```bash
git add crates/aivc-gateway/src/routes.rs
git commit -m "feat(gateway): 外部接入路径识别、裸平台前缀与惰性路由注册"
```

---

### Task 2: server.rs —— handle_inner 外部分支（端到端）

**Files:**
- Modify: `crates/aivc-gateway/src/server.rs:47-50`（use 列表）、`crates/aivc-gateway/src/server.rs:544-558`（解析分支）
- Test: 同文件 `mod tests`（追加 1 个 helper + 4 个端到端用例）

**Interfaces:**
- Consumes: Task 1 的 `extract_external_provider` / `external_route_id` / `RouteRegistry::ensure_external_route` / `prefix()`；现有 `serve`、`RecordingSource`、`GatewaySettings`、`messages::EXTERNAL_API_DISABLED`。
- Produces: 外部请求端到端可用；记录里 `route_id == "external-<平台>"`、`path` 不含 `?key=` 值。

- [ ] **Step 1: 写失败测试**

在测试模块 `use` 区（`use wiremock::...` 之后）加两个 import，并在 `start_recording` helper 之后追加 helper + 4 个用例：

```rust
    use sha2::{Digest, Sha256};
```

```rust
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
        // 访问令牌绝不能进日志；其余查询参数保留。
        assert!(!record.path.contains("ext-secret"), "{}", record.path);
        assert!(record.path.contains("stream=true"), "{}", record.path);
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
```

- [ ] **Step 2: 跑测试确认失败**

Run: `.\scripts\cargo.ps1 test -p aivc-gateway external_ --offline`
Expected: 4 个新用例 FAIL（当前 `/codex/...` 落 ROUTE_NOT_FOUND 404：启用用例拿不到 200/503）。

- [ ] **Step 3: 最小实现**

3a. `use crate::routes::{...}` 列表追加 `external_route_id, extract_external_provider`（保持字母序安装处现状排列即可）。

3b. 把解析块开头（`// 路由解析 + 鉴权。` 到平台校验的 `}`）替换为：

```rust
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
    let (route, external) = {
        let routes = state.routes.read().await;
        let route_id = match external_provider {
            Some(provider) => external_route_id(provider),
            None => extract_route_id(&path),
        };
        // 这几条路径**都没有转发**。保持 `outcome = ok` 是有意的（不是供应商的错，
        // 记成失败会污染按供应商统计的失败率与熔断判定），但必须留下**原因**——
        // 否则它在日志里与一次真正转发成功的记录长得一样。见 `mark_ok_with`。
        let Some(route) = routes.get(&route_id).cloned() else {
            tracker.mark_ok_with("ROUTE_NOT_FOUND");
            return json_error(StatusCode::NOT_FOUND, messages::ROUTE_NOT_FOUND);
        };
        // 平台必须与路径一致：终端路由看第 2 段，外部路由看第 1 段。
        let provider_matches = match external_provider {
            Some(provider) => route.provider_id == provider,
            None => extract_provider(&path) == Some(route.provider_id),
        };
        if !provider_matches {
            tracker.mark_ok_with("ROUTE_NOT_FOUND");
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
                tracker.mark_ok_with("EXTERNAL_API_DISABLED");
                return json_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    messages::EXTERNAL_API_DISABLED,
                );
            }
            let Some(hash) = settings.external_token_hash.as_deref() else {
                tracker.mark_ok_with("UNAUTHORIZED");
                return json_error(StatusCode::UNAUTHORIZED, messages::UNAUTHORIZED);
            };
            if !has_external_access_token(&auth, hash) {
                tracker.mark_ok_with("UNAUTHORIZED");
                return json_error(StatusCode::UNAUTHORIZED, messages::UNAUTHORIZED);
            }
        } else if !has_route_token(&auth, &route.local_token) {
            tracker.mark_ok_with("UNAUTHORIZED");
            return json_error(StatusCode::UNAUTHORIZED, messages::UNAUTHORIZED);
        }
        let external = route.external;
        (route, external)
    };
```

（即：鉴权与之前逐字相同，只改了 `route_id` 取值与 `provider_matches` 两处。）

3c. **不改** `strip_route_prefix`（走 `route.prefix()`，外部路由自动得 `/<平台>`）、不改 `sanitize_external_request_path` / `external_upstream_search` 调用点、不改 `mark_switch` 相关的 `route.external` 跳过逻辑。

- [ ] **Step 4: 跑测试确认通过（含全 crate 回归）**

Run: `.\scripts\cargo.ps1 test -p aivc-gateway --offline`
Expected: PASS——4 个新用例 + 全部既有用例（终端路由 404/401/转发行为无回归）。

- [ ] **Step 5: 提交**

```bash
git add crates/aivc-gateway/src/server.rs
git commit -m "feat(gateway): 外部接入 /<平台> 端到端可用（惰性合成路由 + 共享令牌）"
```

---

### Task 3: UI —— 端点形制收口（`{base}/{平台}`、去占位符、去凭据列）

**Files:**
- Modify: `ui/src/features/settings/panels/gateway-external-card.tsx`
- Modify: `ui/src/app/app.css:3120-3127`（删 `.gateway-endpoint-auth` 规则）
- Test: `ui/src/features/settings/panels/gateway-external-card.test.tsx`

**Interfaces:**
- Consumes: 现有 `providerIds()` / `providerLabel()` / `api.copyText`。
- Produces: `ExternalEndpoint = { provider; label; url }`（**删 `auth` 字段**）；`buildExternalEndpoints(baseUrl, providers)` 返回 `{host}/{平台}`；`ROUTE_ID_PLACEHOLDER` 与 `PROVIDER_AUTH` **删除**。

- [ ] **Step 1: 改测试到新契约（先红）**

1a. 文件头注释（第 8-24 行）整段替换为：

```tsx
/**
 * 外部网关卡：端点清单 = **网关根地址 + 平台路径**。
 *
 * # 形制（2026-10-02 统一外部接入）
 *
 * 外部接入路径是 `/<平台>`（如 `http://127.0.0.1:8787/codex`），直达该平台的
 * 候选池；协议后缀（`/v1/responses` 等）由接入方按自己的协议追加、网关原样透传。
 * 设计与裁定见 `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`。
 *
 * 旧两种形制都不能用：demo 的 `/external/<平台>`（旧 Electron 版）与更早界面上的
 * `/gateway/<平台>/<routeId>` 占位符（Rust 版外部路由从未注册，那个地址必然 404）。
 */
```

1b. `buildExternalEndpoints` 相关用例（67-131 行）整体替换为：

```tsx
describe("buildExternalEndpoints：`{base}/{平台}`，清单从 `providers` 派生", () => {
  it("**端点就是 `{网关根地址}/{平台}`**：没有 `/gateway`、没有 `/external`、没有占位符", () => {
    const endpoints = buildExternalEndpoints(HOST, FOUR_PROVIDERS);
    expect(endpoints.map((item) => item.url)).toEqual([
      `${HOST}/codex`,
      `${HOST}/gemini`,
      `${HOST}/claude`,
      `${HOST}/qoder`
    ]);
    // 显示名也取自 `providers`（与规则表同源），不从本地兜底表来。
    expect(endpoints.map((item) => item.label)).toEqual([
      "Codex",
      "Gemini",
      "Claude Code",
      "Qoder"
    ]);
  });

  it("**给三个平台就只出三条**：清单是 `providers` 的映射，不是本地那份写死的四个", () => {
    expect(buildExternalEndpoints(HOST, THREE_PROVIDERS).map((item) => item.url)).toEqual([
      `${HOST}/codex`,
      `${HOST}/claude`,
      `${HOST}/qoder`
    ]);
  });

  it("拿不到 `providers` 时退回本地兜底清单（穷举表；claude 与后端逐字一致，不再漂移）", () => {
    const endpoints = buildExternalEndpoints(HOST);
    expect(endpoints.map((item) => item.url)).toHaveLength(4);
    expect(endpoints.map((item) => item.label)).toEqual([
      "Codex",
      "Claude Code",
      "Gemini",
      "Qoder"
    ]);
  });

  it("host 的尾斜杠被去掉（`build_route_url` 就是 trim 之后拼接）", () => {
    expect(buildExternalEndpoints(`${HOST}///`)[0]!.url).toBe(`${HOST}/codex`);
  });

  it("没在监听（空地址）时返回空清单——编一个 `:0` 的地址只会误导", () => {
    expect(buildExternalEndpoints("")).toEqual([]);
    expect(buildExternalEndpoints("   ")).toEqual([]);
  });
});
```

（原「没有一个端点长得像 `/external/`」与「每个平台带上凭据方式」两条用例删除——前者被新断言覆盖，后者对应的 `auth` 字段已不存在。）

1c. 组件用例（133 行起）按下述替换关键断言：

- 134-150「端点清单渲染到界面上」：URL 断言改为

```tsx
    expect(text).toContain("http://127.0.0.1:8787/codex");
    expect(text).toContain("仅监听 127.0.0.1");
    // 凭据头列已移除（用户 2026-10-02 裁定）；地址里不能有占位符。
    expect(view.container.querySelectorAll(".gateway-endpoint-auth")).toHaveLength(0);
    expect(text).not.toContain("<routeId>");
```

- 152-168：`${HOST}/gateway/qoder/<routeId>` → `${HOST}/qoder`。
- 183-205 复制用例：期望值 → `"http://127.0.0.1:8787/claude"`。
- 207-226 aria-label 用例不变。
- 234-289 开关与轮换用例中 `findButton(view.container, "外部网关已开启")` → `"外部接入已开启"`（两处：247、276）。

- [ ] **Step 2: 跑测试确认失败**

Run: `cd ui && npx vitest run src/features/settings/panels/gateway-external-card.test.tsx`
Expected: FAIL（URL 仍是 `/gateway/<id>/<routeId>`；`.gateway-endpoint-auth` 仍存在；按钮仍叫「外部网关已开启」）。

- [ ] **Step 3: 实现**

3a. `gateway-external-card.tsx` 逐项：

- 头部注释（8-25 行）替换为 Step 1a 的新版本。
- 删 `PROVIDER_AUTH` 常量与注释（89-101 行）。
- 删 `ROUTE_ID_PLACEHOLDER` 与注释（103-110 行）。
- `ExternalEndpoint` 类型改为：

```ts
export type ExternalEndpoint = {
  provider: AiProviderId;
  label: string;
  /** 形如 `http://127.0.0.1:8787/codex`：网关根地址 + 平台路径。 */
  url: string;
};
```

- `buildExternalEndpoints` 内映射改为：

```ts
  return providerIds(providers).map((id) => ({
    provider: id,
    label: providerLabel(providers, id),
    url: `${host}/${id}`
  }));
```

（函数文档注释同步：删「外部路由编号由网关注册」段，改述新形制与 spec 出处。）

- 卡标题与描述：

```tsx
    <Card
      title="外部接入"
      description="供本机第三方应用与 IDE 扩展调用；地址即网关根地址 + 平台路径，复用供应商候选池与熔断机制。"
```

- 开关文案（243-250 行）：`label={status.enabled ? "外部接入已开启" : "外部接入已关闭"}`、`action={status.enabled ? "关闭外部接入" : "启用外部接入"}`。
- 失败 Notice（261 行）：「外部网关状态读取失败」→「外部接入状态读取失败」。
- 端点行（344-364 行）：删 `<span className="gateway-endpoint-auth">{item.auth}</span>`。
- 底部说明（367-374 行）整段替换为：

```tsx
        <p className="field-hint">
          把上面的地址作为外部客户端的<b> API 基地址</b>使用：协议路径（例如 Codex 客户端的
          <code>/v1/responses</code>、Claude 的 <code>/v1/messages</code>）由客户端按自己的
          协议追加，网关把平台之后的路径<b>原样透传</b>给供应商。凭据用上面那个共享令牌，
          四种携带方式都认：<code>Authorization: Bearer</code>、<code>api-key</code>、
          <code>x-goog-api-key</code> 或查询参数 <code>?key=</code>（查询参数那种在转发前会被
          摘掉，不会带给上游）。
        </p>
```

（注意把 `x-api-key` 写成完整的 `<code>x-api-key</code>`。）

3b. `ui/src/app/app.css` 删 3120-3127 行 `.gateway-endpoint-auth { ... }` 整条规则（`.gateway-endpoint-main` 是隐式行网格，删一行不需要改列定义）。

3c. **同页测试也要改**——`gateway-panel.test.tsx` 渲染整页、pin 了卡内文案：

- 第 88、98 行：`"外部网关状态读取失败"` → `"外部接入状态读取失败"`（两处，含 `not.toContain` 那条）
- 第 97 行：`"外部网关已开启"` → `"外部接入已开启"`

- [ ] **Step 4: 跑测试确认通过 + 类型检查**

Run: `cd ui && npx vitest run src/features/settings/panels/gateway-external-card.test.tsx && npx tsc --noEmit`
Expected: PASS 且无类型错误（若有其它文件引用 `ExternalEndpoint.auth` / `ROUTE_ID_PLACEHOLDER`，此处会报出来，一并清掉：`grep -rn "ROUTE_ID_PLACEHOLDER\|\.auth\b" ui/src --include="*.tsx" --include="*.ts"`）。

- [ ] **Step 5: 提交**

```bash
git add ui/src/features/settings/panels/gateway-external-card.tsx ui/src/features/settings/panels/gateway-external-card.test.tsx ui/src/app/app.css
git commit -m "feat(ui): 外部接入端点改为 /<平台> 短路径，去掉占位符与凭据头列"
```

---

### Task 4: UI —— 命名统一（导航「网关」、面板/开关/提示去「本地」对立）

**Files:**
- Modify: `ui/src/features/settings/settings-nav.ts:34`
- Modify: `ui/src/features/settings/panels/gateway-panel.tsx:129-130, 144, 170, 181-182, 186`
- Modify: `ui/src/features/settings/panels/gateway-port-card.tsx:228`
- Modify: `ui/src/features/settings/panels/gateway-switch-pill.tsx:28, 30`（注释示例）
- Modify: `ui/src/types.ts:206`、`ui/src/ipc/commands.ts:416`（注释）
- Test: `ui/src/features/settings/settings-nav.test.ts:30`、`ui/src/features/settings/settings-center.test.tsx:153`、`ui/src/features/settings/panels/gateway-panel.test.tsx:143`

**Interfaces:**
- Consumes: 无（纯文案与注释）。
- Produces: 可见文案与测试同源更新；`grep -rn "本地网关\|外部网关" ui/src` 的剩余命中只允许描述性叙述（见 Step 4）。

- [ ] **Step 1: 全量盘点（先看清再改）**

Run: `grep -rn "本地网关\|外部网关" ui/src --include="*.tsx" --include="*.ts"`
Expected 命中（2026-10-02 现状）：`types.ts:206`、`commands.ts:416`、`commands.ts:641`、`exit-confirm.tsx:12`、`settings-nav.ts:34`、`settings-center.test.tsx:153`、`settings-nav.test.ts:30`、`gateway-panel.tsx`（14/129/130/144/170/181/186）、`gateway-port-card.tsx:228`、`gateway-panel.test.tsx:143`、`gateway-switch-pill.tsx:28/30`、`gateway-external-card.tsx`（Task 3 已处理）。

- [ ] **Step 2: 逐处替换**

| 文件:行 | 旧 | 新 |
| --- | --- | --- |
| `settings-nav.ts:34` | `label: "本地网关 & 路由"` | `label: "网关"` |
| `settings-nav.test.ts:30` | `"本地网关 & 路由"` | `"网关"` |
| `settings-center.test.tsx:153` | `"本地网关 & 路由": "读取网关状态…"` | `"网关": "读取网关状态…"` |
| `gateway-panel.tsx:14`（文件头注释） | `本地网关 & 路由（demo 第 1006-1371 行）。` | `网关（demo 第 1006-1371 行）。` |
| `gateway-panel.tsx:129` | `"已启用本地网关。重启应用后开始监听。"` | `"已启用网关。重启应用后开始监听。"` |
| `gateway-panel.tsx:130` | `"已关闭本地网关。使用网关的终端路由将立即停止。"` | `"已关闭网关。使用网关的终端路由将立即停止。"` |
| `gateway-panel.tsx:144 / 170 / 181` | `title="本地网关 & 路由"`（三处） | `title="网关"` |
| `gateway-panel.tsx:145 / 171 / 182` | `description="控制本地反向代理端口、……"` | `description="控制反向代理端口、外部接入、熔断器阈值与故障识别引擎。"` |
| `gateway-panel.tsx:186` | `label={status.enabled ? "本地网关已启用" : "本地网关已关闭"}` | `label={status.enabled ? "网关已启用" : "网关已关闭"}` |
| `gateway-panel.test.tsx:143` | `"本地网关已启用"` | `"网关已启用"` |
| `gateway-port-card.tsx:228` | `"尚未开启本地网关。请先在上面点「启用网关」，然后重启应用。"` | `"尚未开启网关。请先在上面点「启用网关」，然后重启应用。"` |
| `gateway-switch-pill.tsx:28/30`（注释） | 示例「本地网关已启用」「关闭本地网关」 | 示例「网关已启用」「关闭网关」 |
| `gateway-switch-pill.tsx:7`（注释） | `外部网关卡头的外部网关开关` | `外部接入卡头的外部接入开关` |
| `gateway-panel.tsx:27`（注释表格） | `\| 外部网关 \| \`gateway-external-card.tsx\` \|` | `\| 外部接入 \| \`gateway-external-card.tsx\` \|` |
| `gateway-panel.tsx:109`（错误文案） | `"后端没有返回外部网关状态。"` | `"后端没有返回外部接入状态。"` |
| `types.ts:206`（注释） | `本地网关的端口与地址状态` | `网关的端口与地址状态` |
| `commands.ts:416`（注释） | `/** 打开/关闭本地网关。 */` | `/** 打开/关闭网关。 */` |

**不改**：`commands.ts:641` 与 `exit-confirm.tsx:12`（「应用里可能跑着终端与本地网关」是叙述性说明，不构成本地/外部对立）。

`gateway-panel.tsx:109` 的错误文案如果被测试 pin（`grep -rn "没有返回外部" ui/src`），同步更新。

- [ ] **Step 3: 检查测试里 pin 住的旧文案**

Run: `grep -rn "网关已启用\|网关已关闭\|尚未开启" ui/src --include="*.test.*"`
把命中的断言同步到新文案（预期：`gateway-panel.test.tsx:143`；若 `gateway-port-card` 相关测试 pin 了「尚未开启本地网关」，同改为「尚未开启网关」）。

- [ ] **Step 4: 复跑盘点 + 相关测试 + 类型检查**

Run:
```
grep -rn "本地网关\|外部网关" ui/src --include="*.tsx" --include="*.ts"
cd ui && npx vitest run src/features/settings && npx tsc --noEmit
```
Expected: 盘点只剩 `commands.ts:641`、`exit-confirm.tsx:12`（允许项）；测试全绿、无类型错误。

- [ ] **Step 5: 提交**

```bash
git add -u ui/src
git commit -m "feat(ui): 网关命名统一——导航「网关」、面板/开关去「本地」对立措辞"
```

---

### Task 5: 文档注记与落地记录（49 号）+ 终验

**Files:**
- Create: `docs/rust-rewrite/49-gateway-unified-external.md`
- Modify: `docs/rust-rewrite/03-parity-matrix-and-migration.md`（G41 段落注记）
- Modify: `docs/rust-rewrite/40-ui-redesign-batch2.md`、`docs/rust-rewrite/41-ui-redesign-batch3.md`（端点形制注记）

**Interfaces:**
- Consumes: Task 1-4 的成果与验证输出（写入记录前必须已经全部通过）。
- Produces: 记录文档 + 偏离注记；本批收尾。

- [ ] **Step 1: 全量门禁**

Run（依序，全部在 `D:\code\ai\aivc-rs`）:
```
cd ui && npx tsc --noEmit && npx vitest run && npm run build
cd .. && powershell -NoProfile -ExecutionPolicy Bypass -File scripts/verify.ps1 -Quick
.\scripts\cargo.ps1 test --workspace --offline
```
Expected: 全绿（前端 883+ 项、Rust 全 workspace、verify 八段）。

- [ ] **Step 2: 写 49 号记录**

创建 `docs/rust-rewrite/49-gateway-unified-external.md`：

```markdown
# 49 · 网关统一外部接入（外部路径 /<平台>）

> 2026-10-02。实机反馈第 4 项的落地记录。设计与用户裁定见
> `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`（§6）。

## 一、做了什么

- **外部接入落地**：`/<平台>`（如 `http://127.0.0.1:8787/codex`）直达该平台候选池。
  每平台一条**惰性注册**的合成路由（`external-<平台>`、`external: true`、动态池），
  首段即平台（`extract_external_provider`）；`/gateway/<平台>/<编号>` 终端分支一字未动。
- **前缀剥离复用**：`VendorRoute::prefix()` 对外部路由返回 `/<平台>`，
  `strip_route_prefix` / 日志 route 标识 / `?key=` 脱敏全部沿用。
- **鉴权零改动**：默认关闭 + 共享令牌（sha256）；未启用 503 `EXTERNAL_API_DISABLED`、
  令牌错 401；`?key=` 不进上游与日志。
- **UI 收口**：端点清单从占位符 `/gateway/<平台>/<routeId>` 改为 `{网关根地址}/<平台>`
  （编号占位符与凭据头列移除——后者为 2026-10-02 用户裁定）；导航「本地网关 & 路由」→
  「网关」，卡「外部网关与 API 接入」→「外部接入」，面板/开关去「本地」对立措辞。

## 二、刻意偏离

| 项 | 旧 Electron 版 | 本实现 | 理由 |
| --- | --- | --- | --- |
| 外部路径 | `/external/<平台>` | `/<平台>` | 更短；用户 2026-10-02 指定形制 |

（G41 令牌语义、终端路由契约不变；03 号 G41 段与 40/41 号已加注记。）

## 三、证据

- Rust：`routes.rs` 4 条新单测 + `server.rs` 4 条端到端（未启用 503 / 令牌 401 /
  成功 200 且上游收到去前缀路径与保留查询 / 未知一段 404）；全 workspace 测试通过。
- 前端：`gateway-external-card.test.tsx` 按新契约更新；tsc / vitest / build 全绿。
- 门禁：`verify.ps1 -Quick` 八段通过。
```

- [ ] **Step 3: 加注记（三处）**

3a. `03-parity-matrix-and-migration.md`：`grep -n "G41" docs/rust-rewrite/03-parity-matrix-and-migration.md` 找到 G41 所在表格/段落，在其**之后**插入一行引用块：

```markdown
> 2026-10-02 偏离：外部接入路径由旧版 `/external/<平台>` 改为 `/<平台>`（短路径，
> 语义不变）；设计与裁定见 `docs/superpowers/specs/2026-10-02-gateway-unified-external-design.md`，
> 落地记录见 `49-gateway-unified-external.md`。
```

3b. `40-ui-redesign-batch2.md`（第 23 行附近）与 `41-ui-redesign-batch3.md`（第 49/58 行附近）：在提到端点形制的段落末尾追加：

```markdown
> 2026-10-02：外部端点已改为 `/<平台>` 短路径（此前界面为占位符
> `/gateway/<平台>/<routeId>`，从未可用）；见 spec 与 49 号。
```

- [ ] **Step 4: 提交**

```bash
git add docs/rust-rewrite/49-gateway-unified-external.md docs/rust-rewrite/03-parity-matrix-and-migration.md docs/rust-rewrite/40-ui-redesign-batch2.md docs/rust-rewrite/41-ui-redesign-batch3.md
git commit -m "docs: 网关统一外部接入落地记录（49 号）与 parity/41/40 注记"
```

---

## 验收清单（全部完成后逐条核对）

- [ ] `curl http://127.0.0.1:<port>/codex`（外部接入开启 + 令牌正确）→ 走 codex 候选池（200/上游语义响应）。
- [ ] 外部接入关闭时同一请求 → 503 `EXTERNAL_API_DISABLED`；令牌错 → 401。
- [ ] 终端会话照常走 `/gateway/<平台>/<编号>`（既有测试全绿即证明）。
- [ ] 界面上端点显示 `http://127.0.0.1:<port>/codex` 等四条，无占位符、无凭据头列。
- [ ] 导航与卡片文案为「网关」「外部接入」。
- [ ] `grep -rn "本地网关\|外部网关" ui/src` 仅剩两处叙述性命中（允许项）。
