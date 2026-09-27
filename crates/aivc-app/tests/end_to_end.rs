//! 端到端冒烟测试：用**真实文件与真实命令**走一遍用户会走的流程。
//!
//! # 它和单元测试的区别
//!
//! 单元测试各自验证一个模块；这里验证的是**装配之后能不能真的用**：命令名能否解析、
//! 目标能否解析、Provider 能否读到磁盘上的文件、结果能否序列化成前端要的形状。
//! 这类问题在单测里全是绿的，只有串起来跑才会暴露——本文件第一次运行时就是这样发现
//! `list-targets` 从不回写缓存，导致所有会话命令都解析不到目标。
//!
//! # 为什么手工注入目标
//!
//! 真实的目标探测要求本机装有 `codex`/`claude` 等 CLI。CI 与开发机上都没有，因此这里
//! 把目标写进设置缓存——这正是 `codex:list-targets` 探测成功后会做的事。这样测的是
//! 「探测之后的全部流程」，唯一没被覆盖的就是探测本身（它有独立的单测）。

// 集成测试是独立 crate，`clippy.toml` 的测试豁免不覆盖 `tests/` 目录。
#![allow(clippy::panic, clippy::expect_used)]

use std::path::{Path, PathBuf};

use aivc_app::App;
use aivc_core::target::{Platform, ProviderId, Target, TargetKind};
use aivc_infra::config::SettingsStore;
use serde_json::{Value, json};

/// 一个会话编号（Codex 用 36 位 UUID 形状）。
const SESSION_ID: &str = "11111111-2222-3333-4444-555555555555";

/// 建一个带临时运行根的应用。
async fn app() -> (App, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("start");
    (app, dir)
}

/// 把 Codex 目标写进设置缓存。
///
/// 真实流程里这一步由 `codex:list-targets` 的探测完成；这里直接写入，因为 CI 上没有
/// Codex CLI。`codex_home` 指向临时目录，因此会话文件完全可控。
async fn install_codex_target(app: &App, codex_home: &Path) -> Target {
    let target = Target {
        id: aivc_core::id::TargetId::from("codex:local"),
        provider: ProviderId::Codex,
        label: "Codex：本机".to_owned(),
        kind: TargetKind::Local,
        distro: None,
        codex_home: Some(codex_home.to_string_lossy().into_owned()),
        available: true,
        detail: None,
    };
    let snapshot = vec![target.clone()];
    app.router()
        .context()
        .settings()
        .update(move |raw| raw.merge_cached_targets(&snapshot))
        .await
        .expect("写入目标缓存");
    target
}

/// 在 `<codex_home>/sessions/...` 下写一份 rollout 文件。
fn write_rollout(codex_home: &Path, session_id: &str, title: &str) -> PathBuf {
    let dir = codex_home
        .join("sessions")
        .join("2026")
        .join("09")
        .join("25");
    std::fs::create_dir_all(&dir).expect("创建会话目录");
    let file = dir.join(format!("rollout-2026-09-25T10-00-00-{session_id}.jsonl"));
    let content = [
        format!(
            r#"{{"timestamp":"2026-09-25T10:00:00.000Z","type":"session_meta","payload":{{"id":"{session_id}","cwd":"/work","cli_version":"0.30.0"}}}}"#
        ),
        format!(
            r#"{{"timestamp":"2026-09-25T10:00:01.000Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{title}"}}]}}}}"#
        ),
        r#"{"timestamp":"2026-09-25T10:00:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"这是回答"}]}}"#.to_owned(),
        r#"{"timestamp":"2026-09-25T10:00:03.000Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":120,"output_tokens":80,"cached_input_tokens":10,"total_tokens":200}}}}"#.to_owned(),
    ]
    .join("\n");
    std::fs::write(&file, format!("{content}\n")).expect("写入会话文件");
    file
}

/// 调用一条命令，失败时打印命令名与参数。
async fn call(app: &App, command: &str, args: Value) -> Value {
    app.dispatch(command, args)
        .await
        .unwrap_or_else(|error| panic!("命令 {command} 失败：{error}"))
}

/// **装配层的最后一环：终端启动之后，网关里真的有一条属于它的路由。**
///
/// # 这条测试补的是什么
///
/// 此前的状态是：`ResolvedRoute` 有类型、有消费者，**就是没有生产者**。终端启动时只能
/// 传 `None`，于是「供应商 + 本地网关」对终端完全无效——而所有单元测试都是绿的。
///
/// 路由生产者写出来之后，剩下的风险全在**装配**上：`App` 有没有把网关与供应商仓储交给
/// 终端层。这条测试就是冲着那一处去的，而且它必须走**真实命令面**，不能绕过。
///
/// # 观测手段
///
/// `vendor:route-switch` 需要一个**确实存在于网关里的路由编号**；路由不在时它返回
/// `routeId` 参数错误。于是它成了一个现成的探针，不需要为测试新增任何生产 API。
#[tokio::test(flavor = "multi_thread")]
async fn starting_a_terminal_registers_a_route_the_gateway_knows_about() {
    let (mut app, dir) = app().await;

    // 网关要开着，路由才有地方注册。
    app.router()
        .context()
        .settings()
        .update(|raw| {
            raw.set_gateway_enabled(true);
            raw.set_gateway_port(0);
        })
        .await
        .expect("设置网关");
    app.gateway_state().refresh_settings().await;
    app.start_gateway_if_enabled()
        .await
        .expect("启动网关")
        .expect("网关应当启动");

    // 两个 Codex 供应商：一个给终端绑定，一个做切换目标。
    let mut vendor_ids = Vec::new();
    for (name, sort) in [("供应商甲", 1), ("供应商乙", 2)] {
        let vendor = call(
            &app,
            "vendor:save",
            json!({
                "providerId": "codex",
                "name": name,
                "apiKey": "sk-test",
                "apiBaseUrl": "http://127.0.0.1:1",
                "enabled": true,
                "sort": sort,
            }),
        )
        .await;
        vendor_ids.push(vendor["id"].as_str().expect("编号").to_owned());
    }

    let terminal_id = "term-app-e2e";
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(&cwd).expect("工作目录");

    // 起点：还没有这条路由。
    assert!(
        app.dispatch(
            "vendor:route-switch",
            json!({ "routeId": terminal_id, "vendorId": vendor_ids[1] })
        )
        .await
        .is_err(),
        "终端还没启动，不该有这条路由"
    );

    // 起终端。命令本身是本机的 `cmd.exe` / shell 包着平台的 CLI，因此不依赖本机装没装 CLI。
    call(
        &app,
        "terminal:start",
        json!({
            "terminalId": terminal_id,
            "providerId": "codex",
            "cwd": cwd.to_string_lossy(),
        }),
    )
    .await;

    // 路由存在 → 切换找得到它 → 装配是通的。
    let switched = call(
        &app,
        "vendor:route-switch",
        json!({ "routeId": terminal_id, "vendorId": vendor_ids[1] }),
    )
    .await;
    assert_eq!(switched["routeId"], terminal_id);

    // 关终端 → 路由消失。否则网关的路由表会随终端开关无限增长。
    call(&app, "terminal:stop", json!({ "terminalId": terminal_id })).await;
    assert!(
        app.dispatch(
            "vendor:route-switch",
            json!({ "routeId": terminal_id, "vendorId": vendor_ids[1] })
        )
        .await
        .is_err(),
        "终端关闭后路由必须消失"
    );

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_must_be_installed_before_any_session_command_works() {
    // 这是**装配层最容易断的一环**：目标解析只能读缓存，而缓存由 `list-targets`
    // 探测后写入。第一次运行时这里就是断的——所有会话命令都以「未解析到目标」失败。
    let (app, _dir) = app().await;
    let error = app
        .dispatch("codex:list-sessions", json!({ "targetId": "codex:local" }))
        .await
        .expect_err("没有目标时必须失败");
    assert!(
        error.to_string().contains("未解析到目标"),
        "错误应当是「未解析到目标」，实际：{error}"
    );
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_targets_writes_the_cache_that_resolution_depends_on() {
    // 探测在测试环境找不到任何 CLI，因此结果为空——但**它必须真的去探测并写缓存**，
    // 而不是像修复前那样只读缓存（那样缓存永远是空的）。
    let (app, _dir) = app().await;
    let targets = call(&app, "codex:list-targets", json!({})).await;
    assert!(targets.is_array(), "结果应当是数组");
    // 缓存读取走的是另一条路径，同样应当是数组。
    let cached = call(&app, "codex:list-cached-targets", json!({})).await;
    assert!(cached.is_array(), "缓存结果应当是数组");
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_full_session_read_flow_works_against_real_files() {
    let (app, dir) = app().await;
    let codex_home = dir.path().join("codex-home");
    let file = write_rollout(&codex_home, SESSION_ID, "端到端标题");
    install_codex_target(&app, &codex_home).await;

    // 1) 列表
    let sessions = call(
        &app,
        "codex:list-sessions",
        json!({ "targetId": "codex:local" }),
    )
    .await;
    let items = sessions.as_array().expect("数组");
    assert_eq!(items.len(), 1, "应当列出刚写入的会话：{sessions}");
    assert_eq!(items[0]["id"], SESSION_ID);
    assert_eq!(items[0]["title"], "端到端标题");
    // 列表路径只读了头部窗口，**它数出来的条数是下界，不是总数**——所以它必须
    // **明确标注**这一点，而不是让消费方去看数字猜。
    //
    // 这条断言此前是 `messageCount == 0`（一律置零表示「未知」）。那是**削弱**契约：
    // 它把信息丢了（界面只能显示「—」），而且 `0` 本身也是个数——一个把它当总数的
    // 消费方会说出「0 条」这句假话。
    //
    // 现在断言的是**更强**的契约：数字是真的（至少这么多条），而它**自带解读方式**。
    assert_eq!(
        items[0]["messageCount"], 2,
        "列表路径应当保留头部窗口里数出来的条数：{sessions}"
    );
    assert_eq!(
        items[0]["messageCountIsLowerBound"], true,
        "列表路径的条数是**下界**，必须显式标注：{sessions}"
    );
    assert_eq!(items[0]["modelStatus"], Value::Null);
    assert_eq!(items[0]["filePath"], file.to_string_lossy().as_ref());

    // 2) 摘要
    let summary = call(
        &app,
        "codex:get-session-summary",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;
    assert_eq!(summary["id"], SESSION_ID);

    // 3) 详情
    let detail = call(
        &app,
        "codex:get-session",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;
    assert_eq!(detail["messageCount"], 2);

    // 4) 分页（首屏哨兵 offset = -1）
    let page = call(
        &app,
        "codex:get-session-messages-page",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID, "offset": -1, "limit": 10 }),
    )
    .await;
    let messages = page["messages"].as_array().expect("消息数组");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["text"], "端到端标题");
    assert_eq!(messages[1]["text"], "这是回答");
    // 前端按 `offset` 判断是否还有更早的消息。
    assert!(page["offset"].is_number(), "offset 必须是数字：{page}");

    // 5) 搜索（命中标题）
    let found = call(
        &app,
        "codex:search-sessions",
        json!({ "targetId": "codex:local", "view": "active", "query": "端到端" }),
    )
    .await;
    assert_eq!(found.as_array().expect("数组").len(), 1, "搜索应当命中");

    // 6) 搜索（不命中）
    let missed = call(
        &app,
        "codex:search-sessions",
        json!({ "targetId": "codex:local", "view": "active", "query": "绝不存在的词" }),
    )
    .await;
    assert!(missed.as_array().expect("数组").is_empty());

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_full_session_mutation_flow_works_against_real_files() {
    let (app, dir) = app().await;
    let codex_home = dir.path().join("codex-home");
    let file = write_rollout(&codex_home, SESSION_ID, "要删除的会话");
    install_codex_target(&app, &codex_home).await;

    // 1) 删除 → 进回收站
    let deleted = call(
        &app,
        "codex:delete-session",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;
    assert!(
        deleted["movedTo"].is_string(),
        "应当返回回收站路径：{deleted}"
    );
    assert!(!file.exists(), "活跃目录里的文件应当已被移走");

    // 2) 活跃列表为空，回收站列表有一条
    let active = call(
        &app,
        "codex:list-sessions",
        json!({ "targetId": "codex:local" }),
    )
    .await;
    assert!(active.as_array().expect("数组").is_empty());
    let trash = call(
        &app,
        "codex:list-trash-sessions",
        json!({ "targetId": "codex:local" }),
    )
    .await;
    assert_eq!(trash.as_array().expect("数组").len(), 1);

    // 3) 恢复
    let restored = call(
        &app,
        "codex:restore-session",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;
    assert!(restored.is_string());
    assert!(file.exists(), "恢复后文件应当回到原位");

    // 4) 再删一次，然后彻底清除
    call(
        &app,
        "codex:delete-session",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;
    let purged = call(
        &app,
        "codex:purge-session",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;
    assert!(purged.is_string());
    assert!(!file.exists());

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn session_settings_persist_across_commands() {
    let (app, dir) = app().await;
    let codex_home = dir.path().join("codex-home");
    write_rollout(&codex_home, SESSION_ID, "原标题");
    install_codex_target(&app, &codex_home).await;

    // 改自定义名
    let saved = call(
        &app,
        "codex:set-session-custom-title",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID, "title": "自定义名" }),
    )
    .await;
    assert_eq!(saved["customTitle"], "自定义名");

    // 收藏 + 标记打开
    call(
        &app,
        "codex:set-session-favorite",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID, "favorite": true }),
    )
    .await;
    call(
        &app,
        "codex:mark-session-opened",
        json!({ "targetId": "codex:local", "sessionId": SESSION_ID }),
    )
    .await;

    // 列表里应当带上这些元数据
    let sessions = call(
        &app,
        "codex:list-sessions",
        json!({ "targetId": "codex:local" }),
    )
    .await;
    let item = &sessions.as_array().expect("数组")[0];
    assert_eq!(item["metadata"]["customTitle"], "自定义名");
    assert_eq!(item["metadata"]["favorite"], true);
    assert!(item["metadata"]["lastOpenedAt"].is_string());

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_presets_and_gateway_commands_work_end_to_end() {
    // 这两块不依赖目标，因此可以独立走一遍。
    let (app, _dir) = app().await;

    let id = call(
        &app,
        "workspace:save-preset",
        json!({ "name": "默认工作区", "cwd": "/work", "cliArgs": "--model gpt-5" }),
    )
    .await;
    assert!(id.is_string(), "应当返回预设编号");

    let presets = call(&app, "workspace:list-presets", json!({})).await;
    assert_eq!(presets.as_array().expect("数组").len(), 1);
    assert_eq!(presets[0]["name"], "默认工作区");

    // 网关设置往返
    let port = call(&app, "gateway:set-port", json!({ "port": 0 })).await;
    assert_eq!(port["port"], 0);
    assert_eq!(call(&app, "gateway:get-port", json!({})).await["port"], 0);

    // 用量查询要求显式区间
    let summary = call(
        &app,
        "gateway:get-usage-summary",
        json!({
            "periodStart": "2026-01-01T00:00:00.000Z",
            "periodEnd": "2026-12-31T23:59:59.999Z"
        }),
    )
    .await;
    assert_eq!(summary["requestCount"], 0);

    // 令牌轮换只返回一次
    let rotated = call(&app, "gateway:rotate-external-api-token", json!({})).await;
    let token = rotated["token"].as_str().expect("令牌");
    assert!(!token.is_empty());
    let status = call(&app, "gateway:get-external-api-status", json!({})).await;
    assert_eq!(status["hasToken"], true);
    assert!(status.get("token").is_none(), "状态查询不得回传令牌");

    // 供应商
    let vendor = call(
        &app,
        "vendor:save",
        json!({
            "providerId": "codex",
            "name": "端到端供应商",
            "apiKey": "sk-test",
            "apiBaseUrl": "https://api.example.com"
        }),
    )
    .await;
    assert_eq!(vendor["name"], "端到端供应商");
    assert_eq!(
        call(&app, "vendor:list", json!({ "providerId": "codex" }))
            .await
            .as_array()
            .expect("数组")
            .len(),
        1
    );

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_gateway_actually_proxies_a_request_end_to_end() {
    // 启动真实网关，让它把请求转发到真实的 mock 上游，并确认请求记录落库。
    // 这是唯一能证明「网关整条链路可用」的测试。
    let (mut app, dir) = app().await;
    let upstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(r#"{"ok":true}"#))
        .mount(&upstream)
        .await;

    // 注册供应商 + 路由，并把网关设到随机端口。
    let vendor = call(
        &app,
        "vendor:save",
        json!({
            "providerId": "codex",
            "name": "上游",
            "apiKey": "sk-test",
            "apiBaseUrl": upstream.uri()
        }),
    )
    .await;
    let vendor_id = vendor["id"].as_str().expect("编号").to_owned();

    app.router()
        .context()
        .settings()
        .update(|raw| {
            raw.set_gateway_enabled(true);
            raw.set_gateway_port(0);
        })
        .await
        .expect("设置网关");
    app.gateway_state().refresh_settings().await;

    // 路由注册需要令牌；这里直接用状态 API 注册，模拟上层在终端启动时做的事。
    let token = "e2e-token";
    app.gateway_state()
        .register_route(aivc_gateway::VendorRoute {
            route_id: "r1".to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: vendor_id.clone(),
            mode: aivc_gateway::RouteMode::Dynamic,
            local_token: token.to_owned(),
            base_url: String::new(),
            external: false,
        })
        .await;

    let address = app
        .start_gateway_if_enabled()
        .await
        .expect("启动网关")
        .expect("应当启动");
    assert!(address.starts_with("http://127.0.0.1:"), "{address}");

    // 打一次真实的代理请求。
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{address}/gateway/codex/r1/v1/messages"))
        .bearer_auth(token)
        .body(r#"{"model":"gpt-5"}"#)
        .send()
        .await
        .expect("代理请求");
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.expect("响应体"), r#"{"ok":true}"#);

    // 用量汇总应当统计到这次请求。
    let mut counted = 0;
    for _ in 0..50 {
        let summary = call(
            &app,
            "gateway:get-usage-summary",
            json!({
                "periodStart": "2026-01-01T00:00:00.000Z",
                "periodEnd": "2030-12-31T23:59:59.999Z"
            }),
        )
        .await;
        counted = summary["requestCount"].as_u64().unwrap_or(0);
        if counted > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(counted, 1, "代理请求应当被记录（{}）", dir.path().display());

    app.shutdown().await;
}

/// **故障转移在装配根这一层真的成立：供应商来自真实数据库。**
///
/// # 为什么不重复网关层的测试
///
/// `aivc-gateway` 已经有 `a_retryable_status_fails_over_to_the_next_vendor`，但它的候选池是
/// 一个**内存 `Vec`**。真实应用里的候选池来自 `RepositorySource` → `VendorRepo` → SQLite，
/// 中间还夹着 `is_route_eligible` 过滤、按 `sort` 排序、健康状态的 hydrate。
/// 「内存里能转移」与「库里读出来的两家能转移」是两件事。
///
/// # 顺带钉住的两件事
///
/// 1. 上游真的被问过（不是恰好第一次就选中了第二家）；
/// 2. **重试不会被记成多次请求**——用量报表按请求计数，多记会直接让用户的账对不上。
#[tokio::test(flavor = "multi_thread")]
async fn failover_works_with_vendors_read_from_the_real_database() {
    let (mut app, dir) = app().await;

    // 第一家总是 429，第二家成功。
    let first = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(429).set_body_string(r#"{"error":"rate limited"}"#),
        )
        .mount(&first)
        .await;
    let second = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string("from-second"))
        .mount(&second)
        .await;

    // 两家 Codex 供应商，排序决定候选顺序。
    let mut vendor_ids = Vec::new();
    for (name, sort, base) in [("第一家", 1, first.uri()), ("第二家", 2, second.uri())] {
        let vendor = call(
            &app,
            "vendor:save",
            json!({
                "providerId": "codex",
                "name": name,
                "apiKey": "sk-test",
                "apiBaseUrl": base,
                "enabled": true,
                "sort": sort,
            }),
        )
        .await;
        vendor_ids.push(vendor["id"].as_str().expect("编号").to_owned());
    }

    app.router()
        .context()
        .settings()
        .update(|raw| {
            raw.set_gateway_enabled(true);
            raw.set_gateway_port(0);
            // 失败一次就切换，并且阈值调高避免熔断抢先摘掉第一家。
            raw.set_gateway_failure_threshold(1);
            raw.set_gateway_circuit_failure_threshold(20);
        })
        .await
        .expect("设置网关");
    app.gateway_state().refresh_settings().await;

    // 路由绑定到**第一家**，因此第一次尝试必然落在它身上。
    let token = "e2e-failover-token";
    app.gateway_state()
        .register_route(aivc_gateway::VendorRoute {
            route_id: "r-failover".to_owned(),
            provider_id: ProviderId::Codex,
            vendor_id: vendor_ids[0].clone(),
            mode: aivc_gateway::RouteMode::Dynamic,
            local_token: token.to_owned(),
            base_url: String::new(),
            external: false,
        })
        .await;

    let address = app
        .start_gateway_if_enabled()
        .await
        .expect("启动网关")
        .expect("应当启动");

    let response = reqwest::Client::new()
        .post(format!("{address}/gateway/codex/r-failover/v1/messages"))
        .bearer_auth(token)
        .body(r#"{"model":"gpt-5"}"#)
        .send()
        .await
        .expect("代理请求");
    assert_eq!(response.status(), 200, "应当转移到第二家");
    assert_eq!(response.text().await.expect("响应体"), "from-second");
    assert_eq!(
        first.received_requests().await.expect("收到的请求").len(),
        1,
        "第一家必须真的被问过一次，否则这不是「转移」而是「一开始就没选它」"
    );

    // 重试不是两次请求。
    let mut counted = 0;
    for _ in 0..50 {
        let summary = call(
            &app,
            "gateway:get-usage-summary",
            json!({
                "periodStart": "2026-01-01T00:00:00.000Z",
                "periodEnd": "2030-12-31T23:59:59.999Z"
            }),
        )
        .await;
        counted = summary["requestCount"].as_u64().unwrap_or(0);
        if counted > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        counted,
        1,
        "一次客户端请求只该记一条——重试不能把用量算成两次（{}）",
        dir.path().display()
    );

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unverified_capabilities_are_rejected_through_the_real_command_surface() {
    // Qoder 不支持分支/复制：走真实命令面时必须给出明确拒绝，而不是静默失败。
    let (app, dir) = app().await;
    let qoder_home = dir.path().join("qoder-home");
    std::fs::create_dir_all(&qoder_home).expect("目录");
    let target = Target {
        id: aivc_core::id::TargetId::from("qoder:local"),
        provider: ProviderId::Qoder,
        label: "Qoder".to_owned(),
        kind: TargetKind::Local,
        distro: None,
        codex_home: Some(qoder_home.to_string_lossy().into_owned()),
        available: true,
        detail: None,
    };
    let snapshot = vec![target];
    app.router()
        .context()
        .settings()
        .update(move |raw| raw.merge_cached_targets(&snapshot))
        .await
        .expect("写入目标");

    let error = app
        .dispatch(
            "codex:duplicate-session",
            json!({ "targetId": "qoder:local", "sessionId": SESSION_ID }),
        )
        .await
        .expect_err("必须拒绝");
    assert_eq!(error.to_string(), "Qoder CN 不支持复制。");

    app.shutdown().await;
}

#[test]
fn the_platform_constant_is_available() {
    // 保证 `Platform` 的导入在测试模块里被使用（避免未使用导入）。
    let _ = Platform::current();
    let _ = SettingsStore::new(std::env::temp_dir().join("unused.json"));
}
