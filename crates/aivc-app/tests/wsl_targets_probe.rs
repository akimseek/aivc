//! WSL 目标探测的诊断测试。
//!
//! # 为什么单独写一个
//!
//! 「界面只显示本机目标」这个现象需要走完「选平台 → 探测 → 返回」才会出现，
//! 因此靠 UI 排查每次都要人点一下。这里直接驱动命令面，把探测结果与
//! **每一道门的结果**都打出来。
//!
//! 本机实测：`wsl.exe -l -q` 返回 `docker-desktop / Ubuntu-22.04 / Ubuntu-24.04`，
//! 且 `Ubuntu-24.04` 下 `/home/akim/.codex/sessions` 确实存在——所以正常情况下
//! 应当至少探测到一个 WSL 目标。
//!
//! **这个测试不断言具体数量**：断言「必须有 WSL 目标」会让它在没装 WSL 的机器上失败，
//! 而那不是缺陷。它的价值是把诊断信息打出来。

// 集成测试是独立 crate，`clippy.toml` 的测试豁免不覆盖 `tests/` 目录。
#![allow(clippy::panic, clippy::expect_used)]

use aivc_app::App;
use aivc_infra::wsl::INTERNAL_WSL_DISTROS;
use serde_json::json;
use std::path::PathBuf;

/// 这个文件里的用例**共用同一个外部资源（WSL）**，并行跑会互相争抢。
///
/// 争抢的后果是实测出来的：单独跑 32 秒的文件，在全量门禁里变成 **91 秒**，并且
/// `the_persistent_worker_keeps_the_distro_warm_across_an_idle_gap` **偶发失败**——
/// 它断言「空闲 25 秒后发行版仍在运行」，而七个 `App::start` 同时对着同一个发行版猛拉
/// `wsl.exe` 时，这个不变量会被外部因素打破，测到的就不再是被测代码的行为。
///
/// 这些是**诊断性集成用例**，不是单元测试：它们驱动真实的 WSL、真实的进程、真实的
/// 时间。对这类用例串行化是正确取舍——**宁可慢，也不要一个会随机变红的门禁**。
static WSL_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 取得串行令牌；用例第一行调用，函数返回时自动释放。
async fn wsl_serial() -> tokio::sync::MutexGuard<'static, ()> {
    WSL_SERIAL.lock().await
}

#[tokio::test(flavor = "multi_thread")]
async fn report_what_wsl_target_probing_yields() {
    let _serial = wsl_serial().await;
    // 装上日志订阅器：worker 的失败降级只写 tracing，不装就看不到。
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");

    let targets = app
        .dispatch("codex:list-targets", json!({ "providerId": "codex" }))
        .await
        .expect("探测目标");
    let items = targets.as_array().expect("数组");

    // 诊断输出：不带 `--nocapture` 时看不到，但失败时会被打印出来。
    println!("探测到 {} 个目标：", items.len());
    for item in items {
        println!(
            "  id={} label={} kind={}",
            item["id"], item["label"], item["kind"]
        );
    }
    let wsl: Vec<_> = items.iter().filter(|item| item["kind"] == "wsl").collect();
    println!("其中 WSL 目标 {} 个", wsl.len());

    // 至少本机目标必须在——这一条与 WSL 无关，任何机器上都成立。
    assert!(
        items.iter().any(|item| item["kind"] == "local"),
        "本机目标必须存在：{targets}"
    );

    // WSL 目标是否存在取决于本机环境，因此**不断言**，只报告。
    if wsl.is_empty() {
        println!(
            "提示：未探测到 WSL 目标。若本机装有 WSL 与 Codex，请检查上方的 debug 日志中\
             是哪一道门（is_available / probe_codex_home）拦下的。"
        );
    }

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn filtering_by_provider_does_not_mix_platforms() {
    let _serial = wsl_serial().await;
    // 桥层曾把 `providerId` 丢掉，导致命令返回**全部平台**的目标，
    // 界面上就变成「目标是各平台的」。这条测试从命令面确认过滤生效。
    // 装上日志订阅器：worker 的失败降级只写 tracing，不装就看不到。
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");

    for provider in ["codex", "gemini", "claude", "qoder"] {
        let targets = app
            .dispatch("codex:list-targets", json!({ "providerId": provider }))
            .await
            .expect("探测目标");
        for item in targets.as_array().expect("数组") {
            assert_eq!(
                item["provider"], provider,
                "按 {provider} 过滤后仍返回了别平台的目标：{item}"
            );
        }
    }
    app.shutdown().await;
}

/// `wsl.exe` 的输出是 **UTF-16LE**，且每行带一个 NUL——必须先按 u16 解码，
/// 直接当 UTF-8 读会得到一堆交错的空字符（连错误信息也是 UTF-16LE）。
fn decode_wsl_utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    String::from_utf16_lossy(&units)
}

/// 定位 `wsl.exe`。
fn find_wsl_exe() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for variable in ["SystemRoot", "windir"] {
        if let Ok(root) = std::env::var(variable) {
            candidates.push(PathBuf::from(root).join("System32").join("wsl.exe"));
        }
    }
    candidates.push(PathBuf::from("C:\\Windows\\System32\\wsl.exe"));
    candidates.into_iter().find(|candidate| candidate.is_file())
}

/// 读某个发行版的运行状态（`wsl -l -v` 的 STATE 列）。
///
/// 返回 `None` 表示查不到（没有 `wsl.exe`、调用失败，或输出里没有这个发行版）。
fn wsl_distro_state(distro: &str) -> Option<String> {
    let output = std::process::Command::new(find_wsl_exe()?)
        .args(["-l", "-v"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    for line in decode_wsl_utf16(&output.stdout)
        .replace('\0', "")
        .split(['\r', '\n'])
    {
        let mut columns = line.split_whitespace();
        // 默认发行版带 `* ` 前缀，它是独立的一个 token。
        let first = columns.next().unwrap_or_default();
        let (name, state) = if first == "*" {
            (columns.next(), columns.next())
        } else {
            (Some(first), columns.next())
        };
        if name == Some(distro) {
            return state.map(str::to_owned);
        }
    }
    None
}

/// 直接问 `wsl.exe -l -q` 要一份发行版清单，用与原平台相同的规则过滤。
///
/// 返回 `Err(reason)` 表示**本机无法提供基准**（没有 `wsl.exe`、WSL 未初始化，或调用
/// 被拒绝）。调用方必须把 `reason` **显式打印出来**再跳过。
///
/// # 为什么不用 `Option`
///
/// 这个测试第一次跑起来时「通过」了，耗时 0.06 秒——因为 `wsl.exe` 被沙箱拒绝
/// （`Wsl/EnumerateDistros/Service/E_ACCESSDENIED`），`Option` 把「无法取得基准」
/// 和「本机没有 WSL」合并成了同一个 `None`，测试于是**什么都没验证**。
///
/// **假绿比红灯危险得多**：红灯会有人去修，假绿只会让人以为已经测过了。
/// 因此这里把「为什么跳过」变成必须被读到的字符串。
fn expected_wsl_distros() -> Result<Vec<String>, String> {
    let executable = find_wsl_exe().ok_or_else(|| "本机没有 wsl.exe".to_owned())?;

    let output = std::process::Command::new(executable)
        .args(["-l", "-q"])
        .output()
        .map_err(|error| format!("无法启动 wsl.exe：{error}"))?;
    if !output.status.success() {
        return Err(format!(
            "wsl.exe -l -q 失败（{}）：{}",
            output.status,
            decode_wsl_utf16(&output.stderr).trim()
        ));
    }
    let distros: Vec<String> = decode_wsl_utf16(&output.stdout)
        .replace('\0', "")
        .split(['\r', '\n'])
        .map(|line| line.trim().trim_start_matches('*').trim().to_owned())
        .filter(|name| {
            if name.is_empty() {
                return false;
            }
            let lower = name.to_lowercase();
            !INTERNAL_WSL_DISTROS.contains(&lower.as_str())
        })
        .collect();
    if distros.is_empty() {
        return Err("wsl.exe 可用，但没有任何已安装的发行版".to_owned());
    }
    Ok(distros)
}

#[tokio::test(flavor = "multi_thread")]
async fn every_installed_wsl_distro_appears_in_the_target_list() {
    let _serial = wsl_serial().await;
    // **回归守门：「完整的列表」。**
    //
    // 目标探测曾经对每个发行版做两道 `continue`——解析不出 Codex 主目录就跳过、
    // `sessions` 目录不存在就跳过。实机上 Ubuntu-22.04 正是被第二道拦掉的：它没有
    // `~/.codex/sessions`，于是**从下拉框里消失了**，而界面没有任何解释。
    //
    // 这里拿 `wsl.exe -l -q` 的真实输出当基准：**每个非内部发行版都必须出现在目标
    // 列表里**，不论它内部的 Codex 是否已经初始化。
    let expected = match expected_wsl_distros() {
        Ok(distros) => distros,
        Err(reason) => {
            // 必须让人**看见**这一行：跳过不等于通过。
            println!("⚠ 跳过：{reason}——本用例没有验证任何东西。");
            return;
        }
    };

    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");
    for provider in ["codex", "claude", "gemini", "qoder"] {
        let targets = app
            .dispatch("codex:list-targets", json!({ "providerId": provider }))
            .await
            .expect("探测目标");
        let returned: Vec<String> = targets
            .as_array()
            .expect("数组")
            .iter()
            .filter(|item| item["kind"] == "wsl")
            .filter_map(|item| item["distro"].as_str().map(str::to_owned))
            .collect();
        println!("{provider}：wsl.exe 报告 {expected:?}；目标列表给出 {returned:?}");
        for distro in &expected {
            assert!(
                returned.contains(distro),
                "{provider} 隐藏了发行版 {distro}：{returned:?}"
            );
        }
        assert_eq!(
            returned.len(),
            expected.len(),
            "{provider} 的目标数量必须与发行版数量一致"
        );
        for target in targets.as_array().expect("数组") {
            println!(
                "{} available={} detail={}",
                target["id"], target["available"], target["detail"]
            );
        }
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn installed_claude_is_available_in_the_same_environment_as_the_terminal() {
    let _serial = wsl_serial().await;
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");
    let targets = app
        .dispatch("codex:list-targets", json!({ "providerId": "claude" }))
        .await
        .expect("探测 Claude");
    let items = targets.as_array().expect("数组");
    if aivc_provider::common::command_exists_local("claude") {
        let local = items
            .iter()
            .find(|target| target["kind"] == "local")
            .expect("本机目标");
        assert_eq!(local["available"], true, "已安装的本机 Claude 必须可用");
        println!("本机 Claude available=true");
    }
    if let Ok(distros) = expected_wsl_distros() {
        for distro in distros {
            let output = std::process::Command::new(find_wsl_exe().expect("wsl.exe"))
                .args(["-d", &distro, "--", "bash", "-ic", "command -v claude"])
                .output()
                .expect("shell 探测");
            let target = items
                .iter()
                .find(|target| target["distro"] == distro)
                .expect("WSL 目标");
            if output.status.success() {
                assert_eq!(
                    target["available"], true,
                    "终端能找到 Claude，目标也必须可用：{target}"
                );
                let environment = app
                    .dispatch(
                        "cli:check-environment",
                        json!({
                            "providerId": "claude", "targetId": target["id"]
                        }),
                    )
                    .await
                    .expect("环境检查");
                assert_eq!(
                    environment["hasCli"], true,
                    "环境检查必须进入所选发行版：{environment}"
                );
                println!(
                    "{distro} Claude available=true version={}",
                    environment["cliVersion"]
                );
            } else {
                println!("{distro}：终端未能找到 Claude；目标状态：{target}");
            }
        }
    } else {
        println!("WSL 基准不可用，本用例仅检查本机");
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn quick_access_returns_both_lists_without_probing() {
    let _serial = wsl_serial().await;
    // 「快捷访问」只读缓存：它必须**立即**返回，不能触发全量探测
    // （WSL 冷启动实测 3.3 秒）。
    //
    // 空库下两个列表都应为空数组，而不是缺字段——前端直接解构 `{ favorites, recent }`。
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");

    let started = std::time::Instant::now();
    let value = app
        .dispatch("codex:list-session-quick-access", json!({}))
        .await
        .expect("快捷访问");
    let elapsed = started.elapsed();

    // **内部排序键不得泄漏到响应里**。
    //
    // 实现里有 `_sort` / `_label` / `_title` 三个只用于比较的键；它们曾原样出现在
    // 返回的 JSON 中。前端 `SessionQuickAccessItem` 只声明四个字段，
    // 多余的键不会报错，但会让契约变模糊、载荷无谓变大。
    for list in ["favorites", "recent"] {
        for item in value[list].as_array().expect("数组") {
            for leaked in ["_sort", "_label", "_title"] {
                assert!(
                    item.get(leaked).is_none(),
                    "{list} 里泄漏了内部键 {leaked}：{item}"
                );
            }
        }
    }

    assert!(
        value["favorites"].is_array(),
        "favorites 必须是数组：{value}"
    );
    assert!(value["recent"].is_array(), "recent 必须是数组：{value}");
    assert!(
        elapsed.as_millis() < 1000,
        "只读缓存不该触发探测，实测耗时 {elapsed:?}"
    );

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
// **根因已定位**（见 docs/rust-rewrite/09）：不是协议层的问题，而是 **WSL2 会在空闲后
// 自动关闭发行版**。实测：发行版 Stopped 时首次调用 **5.3 秒**，紧接着调用 0.09 秒，
// 空闲 20 秒后再调用 **6.9 秒**。
//
// 原来的实现每个会话文件发一次 `read_head`、配 6 路并发，于是 **6 个 `wsl.exe` 同时
// 去抢同一次冷启动**——应用日志里因此出现过 `head` 之间 **29.99 秒**的空白。
// 改成一次批量读之后，14 个文件从 14 次调用（3 波并发）压成 1 次。
async fn listing_wsl_sessions_is_fast() {
    let _serial = wsl_serial().await;
    // **回归守门**：`列出 <distro 内路径>` 曾因脚本穿过三层命令行解析而卡住，
    // 撞上 30 秒超时后以退出码 -1 失败。
    //
    // 修法是按现网做法把脚本 base64 后再传（见 `aivc_infra::wsl::script_transfer`），
    // 并把 stdin 从「开着不写的管道」改成 null。
    //
    // 断言「远小于 30 秒」而不是一个精确值：WSL 冷启动本身就要数秒，
    // 而这里要守的是**不再卡死**。
    // 装日志订阅器：`执行 WSL 脚本` 的调用次数只写 tracing，不装就看不到——
    // 而「WSL 慢」几乎总是「调用次数多」，那正是要测的东西。
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");

    let targets = app
        .dispatch("codex:list-targets", json!({ "providerId": "codex" }))
        .await
        .expect("探测目标");
    // **只测可达的目标**。
    //
    // 这里原本取「第一个 kind == wsl 的目标」。目标列表改为列出全部发行版之后，
    // 第一个 WSL 目标恰好是一个**进不去**的发行版（`available: false`）——
    // 它的会话列表会走「空 codex_home」的 guard 直接返回，耗时 405 µs、0 条会话。
    // 测试于是「通过」了，却**完全没有走到批量读取那条路**。
    //
    // 又一次假绿。因此这里按 `available` 过滤，并且要求至少测到一个。
    let reachable: Vec<(String, String)> = targets
        .as_array()
        .expect("数组")
        .iter()
        .filter(|item| item["kind"] == "wsl" && item["available"] == true)
        .filter_map(|item| {
            Some((
                item["id"].as_str()?.to_owned(),
                item["detail"].as_str().unwrap_or_default().to_owned(),
            ))
        })
        .collect();
    if reachable.is_empty() {
        println!("⚠ 跳过：没有**可达**的 WSL 目标——本用例没有验证任何东西。");
        app.shutdown().await;
        return;
    }

    for (target_id, detail) in &reachable {
        println!("WSL 目标：{target_id}（{detail}）");
        let started = std::time::Instant::now();
        let sessions = app
            .dispatch("codex:list-sessions", json!({ "targetId": target_id }))
            .await
            .expect("列出会话");
        let elapsed = started.elapsed();
        println!(
            "  列出耗时 {elapsed:?}，共 {} 条",
            sessions.as_array().map_or(0, Vec::len)
        );
        assert!(
            elapsed.as_secs() < 20,
            "{target_id} 列出会话不该卡到超时，实测 {elapsed:?}"
        );
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_persistent_worker_keeps_the_distro_warm_across_an_idle_gap() {
    // **这条测试写的是 worker 存在的理由本身。**
    //
    // 实测（见 docs/rust-rewrite/10）：WSL2 会在**空闲约 20 秒后自动关闭发行版**，
    // 下次调用要冷启动 **5–7 秒**。一次性路径每次操作都是一个新进程，因此只要调用
    // 之间有间隔，就反复付这个代价。
    //
    // 常驻 worker 是一个**活着的 bash 进程**，WSL 不会在有进程运行时关闭发行版。
    // 所以断言是：**跨过一段足以让一次性路径冷启动的空闲之后，调用仍然很快**。
    //
    // 这条测试要跑 30 秒左右，比其余用例慢得多。它值这个时间：worker 一旦悄悄失效
    // （例如 stdin 契约被改回去），这里会立刻变成 5–7 秒，而不是等到用户在界面上
    // 才发现「打开会话要等好几秒」。
    const IDLE_GAP: std::time::Duration = std::time::Duration::from_secs(25);

    // 取得串行令牌要放在常量声明**之后**：`clippy::items_after_statements` 不接受
    // 语句之后再出现 `const`。
    let _serial = wsl_serial().await;

    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_test_writer()
        .try_init();
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");

    let targets = app
        .dispatch("codex:list-targets", json!({ "providerId": "codex" }))
        .await
        .expect("探测目标");
    let target = targets
        .as_array()
        .expect("数组")
        .iter()
        .find(|item| item["kind"] == "wsl" && item["available"] == true)
        .and_then(|item| item["id"].as_str().map(str::to_owned));
    let Some(target_id) = target else {
        println!("⚠ 跳过：没有可达的 WSL 目标——本用例没有验证任何东西。");
        app.shutdown().await;
        return;
    };

    // 第一次调用：建立 worker。
    let started = std::time::Instant::now();
    let sessions = app
        .dispatch("codex:list-sessions", json!({ "targetId": target_id }))
        .await
        .expect("首次列出会话");
    let first = started.elapsed();
    let count = sessions.as_array().map_or(0, Vec::len);
    println!("首次列出：{first:?}，{count} 条");

    // 空闲一段——一次性路径在这之后必须冷启动。
    println!("空闲 {IDLE_GAP:?} 后再列一次……");
    tokio::time::sleep(IDLE_GAP).await;

    let started = std::time::Instant::now();
    let sessions = app
        .dispatch("codex:list-sessions", json!({ "targetId": target_id }))
        .await
        .expect("空闲后列出会话");
    let second = started.elapsed();
    println!(
        "空闲后列出：{second:?}，{} 条",
        sessions.as_array().map_or(0, Vec::len)
    );

    assert_eq!(
        sessions.as_array().map_or(0, Vec::len),
        count,
        "空闲前后会话数必须一致"
    );

    // **主断言是「机制」，不是「耗时」。**
    //
    // 这里原本断言 `second < 3s`。它单独跑很稳，但在门禁里**偶发失败**：这个测试与
    // 其它用例并行运行时会互相争抢 `wsl.exe`，绝对耗时在竞争环境下不是一个可靠的判据。
    //
    // 换成一个不受负载影响的机制断言：**空闲之后发行版必须仍然处于 Running**。
    // 这正是常驻 worker 的作用——它让 WSL 不把发行版关掉。
    let distro = target_id.strip_prefix("codex:wsl:").unwrap_or(&target_id);
    let state = wsl_distro_state(distro);
    println!("空闲后 {distro} 的状态：{state:?}");
    if state.is_some() {
        assert_eq!(
            state.as_deref(),
            Some("Running"),
            "空闲 {IDLE_GAP:?} 之后发行版应当仍在运行——这正是常驻 worker 的作用。\
             若这里是 Stopped，说明 worker 已经失效，下一次调用要付 5–7 秒冷启动。"
        );
    } else {
        println!("⚠ 查不到发行版状态（wsl -l -v 不可用），跳过机制断言。");
    }

    // 耗时仍留一个**宽松**的上限：它的作用是抓住「整条路彻底退化」，
    // 而不是精确测量。冷启动是 5–7 秒，因此 10 秒足以区分。
    assert!(
        second < std::time::Duration::from_secs(10),
        "空闲后列出会话不该退化到冷启动量级；实测 {second:?}"
    );

    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_targets_persists_them_for_the_next_start() {
    let _serial = wsl_serial().await;
    // **回归守门**：`list-targets` 探测完曾**不写缓存**——
    // `merge_cached_targets` 只在测试里被调用过。
    //
    // 后果有两层：
    // 1. `list-cached-targets` 永远返回空，前端「先渲染缓存再增量刷新」的策略失效；
    // 2. 启动预热白做——它探测并打印了结果，却没留下任何东西。
    let dir = tempfile::tempdir().expect("tempdir");
    let runtime = dir.path().join("runtime");
    let app = App::start(runtime.clone(), None).await.expect("启动");

    let probed = app
        .dispatch("codex:list-targets", json!({ "providerId": "codex" }))
        .await
        .expect("探测目标");
    let probed_len = probed.as_array().expect("数组").len();
    assert!(probed_len > 0, "至少要探测到本机目标");

    let cached = app
        .dispatch(
            "codex:list-cached-targets",
            json!({ "providerId": "codex" }),
        )
        .await
        .expect("读缓存");
    assert_eq!(
        cached.as_array().expect("数组").len(),
        probed_len,
        "探测结果必须写进缓存，否则前端的缓存路径永远是空的"
    );
    app.shutdown().await;

    // 再启一个进程级实例（同一个运行根），缓存应当仍在。
    let again = App::start(runtime, None).await.expect("重启");
    let restored = again
        .dispatch(
            "codex:list-cached-targets",
            json!({ "providerId": "codex" }),
        )
        .await
        .expect("读缓存");
    assert_eq!(
        restored.as_array().expect("数组").len(),
        probed_len,
        "缓存要跨启动保留，否则每次开应用都要现场探测"
    );
    again.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn startup_warmup_only_probes_the_primary_platform() {
    let _serial = wsl_serial().await;
    // **回归守门**：启动预热曾经无条件探测**全部 4 个平台 × 2 个发行版**，实测
    // **6.3 秒**的 `wsl.exe` 调用（日志：`目标预热完成 count=8 elapsed_ms=6296`），
    // 而且和界面自己的首次探测抢同一个发行版——反而让首屏更慢。
    //
    // 现在只预热主平台（Codex）。这条测试看的是**副产物**：预热是启动后第一个写缓存的
    // 人，因此全新运行根下缓存里应当**只有 codex 目标**。
    let dir = tempfile::tempdir().expect("tempdir");
    let app = App::start(dir.path().join("runtime"), None)
        .await
        .expect("启动");

    // 预热跑在后台任务里：轮询等待，**不假设它已经完成**。
    let mut cached: Vec<serde_json::Value> = Vec::new();
    for _ in 0..40 {
        let value = app
            .dispatch("codex:list-cached-targets", json!({}))
            .await
            .expect("读缓存（不过滤平台）");
        cached = value.as_array().cloned().unwrap_or_default();
        if !cached.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    // 这一半**任何机器上都成立**：预热只允许写主平台的目标。
    for item in &cached {
        assert_eq!(
            item["provider"], "codex",
            "预热只该探测主平台，却写入了别平台的目标：{item}"
        );
    }
    if cached.is_empty() {
        // 本机一个 Codex 目标都没有（没装 Codex、也没有 WSL）时，上面那一半是
        // 空真。**说出来**，不要让它伪装成「验证过了」。
        println!("⚠ 本机没有探测到任何 Codex 目标——本用例只验证了「没有别平台」这一半。");
    } else {
        println!("预热写入了 {} 个目标，全部属于 codex", cached.len());
    }

    app.shutdown().await;
}
