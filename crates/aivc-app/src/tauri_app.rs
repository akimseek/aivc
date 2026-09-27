//! Tauri 命令面：把 IPC 调用转成 [`App::dispatch`]。
//!
//! # 为什么只有这一层是「薄」的
//!
//! 前端的调用形状是 `invoke(命令名, 参数)`，因此 Rust 侧只需要**一个**命令：
//! 收到命令名与参数、转发给 [`CommandRouter`]、把结果或错误序列化回去。
//! 命令名到处理函数的映射、参数校验、业务逻辑全在下面几层——这一层刻意不含判断，
//! 因为它是唯一与 Tauri 耦合的地方，越薄越容易在版本升级时保持稳定。
//!
//! # 错误为什么映射成字符串
//!
//! 前端的错误处理读的是 `error.message`（现网就是这么抛的）。把错误类型序列化成结构体
//! 会改变前端的解析方式，因此这里保持「错误即消息文本」这一形状。
//!
//! # 为什么返回 `Builder` 而不是直接 `run`
//!
//! `run` 需要 `generate_context!()` 产出的上下文，而那要求 `tauri-build` 在构建脚本里
//! 生成配置。返回 `Builder` 让本模块**不依赖构建脚本**即可编译与测试，入口再补上
//! `.run(tauri::generate_context!())`。

use std::sync::Arc;

use serde_json::{Value, json};
// 三个平台插件的扩展 trait。放在模块顶部是 `clippy::items_after_statements` 的要求：
// `use` 是「条目」，写在语句之后会让作用域看起来比实际更窄。
use tauri_plugin_clipboard_manager::ClipboardExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_notification::NotificationExt as _;

use crate::App;
use crate::shutdown;

/// Tauri 状态：共享同一个应用实例。
pub struct AppState {
    app: Arc<App>,
    /// 退出确认流程的状态。
    ///
    /// 放在这里而不是 [`App`] 里：它只对窗口生命周期有意义，而 `App` 是与界面框架
    /// 无关的组装根（测试里可以在没有 Tauri 的情况下构造）。
    pub exit_flow: crate::exit::ExitFlow,
}

impl AppState {
    /// 构造。
    #[must_use]
    pub fn new(app: App) -> Self {
        Self {
            app: Arc::new(app),
            exit_flow: crate::exit::ExitFlow::new(),
        }
    }

    /// 应用实例。
    #[must_use]
    pub fn app(&self) -> &App {
        &self.app
    }
}

/// 唯一的 IPC 命令。
///
/// 前端调用 `invoke("aivc_invoke", { command, args })`；命令名与参数由前端按现网形状
/// 传入，本函数不做二次包装。
///
/// # 为什么这里要拦几条命令
///
/// 剪贴板、原生对话框、系统通知都要用到 **Tauri 的平台能力**，而 `aivc-ipc` 刻意
/// 不依赖 tauri（命令层保持与界面框架无关，才能在测试里直接驱动）。
/// 因此这几条在**这一层**处理，其余照旧转给命令路由器。
///
/// # Errors
///
/// 未知命令、未接线命令、入参非法或业务失败时返回错误消息文本。
#[tauri::command]
pub async fn aivc_invoke(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    command: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let args = args.unwrap_or(Value::Null);
    if let Some(result) = platform_command(&app, &command, &args).await {
        return result;
    }
    state
        .app
        .dispatch(&command, args)
        .await
        .map_err(|error| error.to_string())
}

/// 需要 Tauri 平台能力的命令。
///
/// 返回 `None` 表示「不归这里管」，交回给命令路由器。
async fn platform_command(
    app: &tauri::AppHandle,
    command: &str,
    args: &Value,
) -> Option<Result<Value, String>> {
    match command {
        "clipboard:copy-text" => {
            let Some(text) = args.get("text").and_then(Value::as_str) else {
                return Some(Err("参数无效：text".to_owned()));
            };
            Some(
                app.clipboard()
                    .write_text(text.to_owned())
                    .map(|()| Value::Null)
                    .map_err(|error| error.to_string()),
            )
        }
        "clipboard:read-text" => Some(
            app.clipboard()
                .read_text()
                .map(Value::String)
                .map_err(|error| error.to_string()),
        ),
        "dialog:choose-directory" => {
            let title = args.get("title").and_then(Value::as_str).map(str::to_owned);
            let handle = app.clone();
            // **不能阻塞事件循环**：原生对话框要等用户操作，`blocking_pick_folder`
            // 必须在别的线程上跑，否则窗口会卡住。
            let picked = tauri::async_runtime::spawn_blocking(move || {
                let mut builder = handle.dialog().file();
                if let Some(title) = title {
                    builder = builder.set_title(title);
                }
                builder.blocking_pick_folder()
            })
            .await
            .map_err(|error| format!("选择目录失败：{error}"));
            Some(match picked {
                // 用户取消时返回 `null`，**不是错误**——取消是正常操作。
                Ok(path) => Ok(path.map_or(Value::Null, |value| Value::String(value.to_string()))),
                Err(error) => Err(error),
            })
        }
        "dialog:choose-file" => {
            // 与选目录同构，只是收尾调 `blocking_pick_file`。
            //
            // 存在的理由：技能导入既接受目录（一个技能一个目录），也接受单个
            // `SKILL.md`——后端 `import_from_path` 本来就两种都支持，缺的只是前端入口。
            let title = args.get("title").and_then(Value::as_str).map(str::to_owned);
            let handle = app.clone();
            let picked = tauri::async_runtime::spawn_blocking(move || {
                use tauri_plugin_dialog::DialogExt as _;
                let mut builder = handle.dialog().file().add_filter("技能文件", &["md"]);
                if let Some(title) = title {
                    builder = builder.set_title(title);
                }
                builder.blocking_pick_file()
            })
            .await
            .map_err(|error| format!("选择文件失败：{error}"));
            Some(match picked {
                // 用户取消时返回 `null`，**不是错误**——取消是正常操作。
                Ok(path) => Ok(path.map_or(Value::Null, |value| Value::String(value.to_string()))),
                Err(error) => Err(error),
            })
        }
        "terminal:attention-notification" => {
            let title = args
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("终端需要你确认");
            let detail = args.get("detail").and_then(Value::as_str);
            let mut builder = app.notification().builder().title(title);
            if let Some(detail) = detail {
                builder = builder.body(detail);
            }
            // 通知发不出去**不该让调用失败**：用户没开通知权限、系统静音都是常态，
            // 而发起通知的那条业务路径（终端在等确认）本身没有出错。
            if let Err(error) = builder.show() {
                tracing::debug!(%error, "系统通知发送失败");
            }
            Some(Ok(Value::Null))
        }
        "app:exit-choice" => Some(resolve_exit_choice(app, args)),
        "app:command" => Some(run_app_command(app, args).await),
        _ => None,
    }
}

/// 用户在前端的退出确认框里做了选择。
///
/// 校验失败时**什么都不做**并报错：这条命令能决定应用生死，不能兜底成某个动作。
fn resolve_exit_choice(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    use tauri::Manager as _;

    let Some(raw) = args.get("choice").and_then(Value::as_str) else {
        return Err("参数无效：choice".to_owned());
    };
    let Some(choice) = crate::exit::Choice::parse(raw) else {
        return Err(format!("退出方式无效：{raw}"));
    };

    let state = app.state::<AppState>();
    match state.exit_flow.choose(choice) {
        crate::exit::ChoiceAction::Dismiss => Ok(Value::Null),
        crate::exit::ChoiceAction::Hide => {
            // 最小化到**托盘**：应用继续跑，终端与网关都还在。
            // 因此这里必须先确认托盘真的建起来了——托盘失败时隐藏窗口会让应用
            // 变得完全够不着（没有任务栏图标、也没有托盘图标）。
            if let Some(window) = app.get_webview_window(crate::tray::MAIN_WINDOW) {
                if crate::tray::is_installed() {
                    if let Err(error) = window.hide() {
                        return Err(format!("隐藏窗口失败：{error}"));
                    }
                } else {
                    // 没有托盘就退化成普通最小化：用户还能从任务栏找回来。
                    tracing::warn!("托盘不可用，退化为最小化而不是隐藏窗口");
                    if let Err(error) = window.minimize() {
                        return Err(format!("最小化窗口失败：{error}"));
                    }
                }
            }
            Ok(Value::Null)
        }
        crate::exit::ChoiceAction::Quit => {
            exit_after_cleanup(app);
            Ok(Value::Null)
        }
    }
}

/// 应用级命令。
///
/// 取值取自现网 `app:command`：`quit`（走退出确认，**不是**立刻退）与
/// `openLogDir`（在文件管理器里打开日志目录）。
async fn run_app_command(app: &tauri::AppHandle, args: &Value) -> Result<Value, String> {
    use tauri::Manager as _;

    let Some(command) = args.get("command").and_then(Value::as_str) else {
        return Err("参数无效：command".to_owned());
    };
    match command {
        // 现网的菜单「退出」**不是**直接退，而是弹确认框（矩阵 A4）。
        // 直接退会把「用户可能只是想把窗口收起来」这件事抹掉。
        "quit" => {
            let state = app.state::<AppState>();
            crate::exit::request_exit(app, &state.exit_flow);
            Ok(Value::Null)
        }
        "openLogDir" => {
            let state = app.state::<AppState>();
            let directory = state.app().paths().log_dir();
            // 目录可能还不存在（没跑过任何写日志的路径）。把不存在的路径交给
            // `explorer` 会得到一个**系统级错误弹窗**，所以先建出来。
            if let Err(error) = std::fs::create_dir_all(&directory) {
                return Err(format!("创建日志目录失败：{error}"));
            }
            let payload = json!({ "path": directory.to_string_lossy() });
            state
                .app()
                .dispatch("shell:open-path", payload)
                .await
                .map_err(|error| error.to_string())
        }
        other => Err(format!("未知的应用命令：{other}")),
    }
}

/// 把事件发到 Tauri 窗口的发射器。
///
/// # 为什么要遍历全部窗口
///
/// `emit` 的接收方是 webview。窗口还没建好或已销毁时没有接收方——
/// 这时**丢弃**而不是报错：启动早期的事件本来也没人听。
struct TauriEmitter(tauri::AppHandle<tauri::Wry>);

impl aivc_ipc::terminal::EventEmitter for TauriEmitter {
    fn emit(&self, channel: &str, payload: Value) {
        // 事件名固定，载荷是 JSON；`emit` 失败只记 debug——
        // 事件送不到不该影响产生它的那条业务路径。
        if let Err(error) = tauri::Emitter::emit(&self.0, channel, payload) {
            tracing::debug!(%error, channel, "发送事件失败");
        }
    }
}

/// 装配 Tauri 构建器。
///
/// 入口只需：
///
/// ```ignore
/// let app = App::start(root, bash).await?;
/// aivc_app::tauri_app::build(app).run(tauri::generate_context!())?;
/// ```
#[must_use]
pub fn build(app: App) -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        // 三个平台插件：剪贴板、原生对话框、系统通知。
        // 它们由 `platform_command` 使用——`aivc-ipc` 里没有 tauri 依赖，用不了它们。
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(AppState::new(app))
        .invoke_handler(tauri::generate_handler![aivc_invoke])
}

/// 启动事件循环。
///
/// 与 [`build`] 分开的原因：`build` 返回 `Builder`，**不需要** `tauri-build` 生成的
/// 上下文，因此可以在测试里构造与断言；只有真正启动才需要 `generate_context!`。
///
/// 退出处理挂在 `ExitRequested` 上：先跑完清理再退出。顺序与现网一致
/// （PTY → Gateway → flush 日志），且**幂等**——这个事件可能被触发多次。
///
/// # Errors
///
/// Tauri 配置无效或事件循环启动失败时返回错误。
pub fn run(app: App) -> tauri::Result<()> {
    // 开发模式下 Vite dev server 是受信任来源；发布模式下只认 Tauri 的资源协议。
    // 环境变量由 Tauri CLI 注入，直接跑二进制时为 `None`（即发布模式）。
    let dev_server = std::env::var("VITE_DEV_SERVER_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());

    build(app)
        .setup(move |handle| {
            // 装入真实事件发射器。**必须在这里**：`App::start` 时尚无窗口，
            // 网关的记录钩子那时只能拿到占位发射器。
            {
                use tauri::Manager as _;
                let state = handle.state::<AppState>();
                state
                    .app
                    .install_emitter(Arc::new(TauriEmitter(handle.handle().clone())));
            }

            // 窗口在这里创建而不是在配置里：只有 `WebviewWindowBuilder` 能挂
            // `on_navigation`，而导航白名单是安全边界（见 `harden` 模块），不能省。
            let spec = window_spec();
            let background = tauri::window::Color(
                spec.background.0,
                spec.background.1,
                spec.background.2,
                spec.background.3,
            );
            let navigation_dev = dev_server.clone();
            let window = tauri::WebviewWindowBuilder::new(
                handle,
                crate::tray::MAIN_WINDOW,
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title(spec.title)
            .inner_size(spec.width, spec.height)
            .min_inner_size(spec.min_width, spec.min_height)
            .background_color(background)
            .resizable(true)
            .on_navigation(move |url| {
                crate::harden::attach_navigation_handler(url.as_str(), navigation_dev.as_deref())
            });

            // **把 WebView 的数据目录也放到运行根下**。
            //
            // 不设置的话，Tauri 2 默认落在 `%LOCALAPPDATA%\<identifier>\EBWebView`，
            // 于是 `AppPaths` 开头那条「应用绝不写入系统用户目录」的约束**并不成立**；
            // 而且 `localStorage`（「上次用的平台与目标」就存在那里）也跟着跑到系统
            // 目录，应用不再便携。
            //
            // 旧工程用 Electron 的 `app.setPath("userData", …)` 做过同一件事。本工程
            // 此前只有一个 `user_data_dir()`，它**只被用来拼设置路径，从未接到这里**
            // ——那条约束于是只是文档里的一句话。
            //
            // 平台差异：只有 Windows（WebView2）支持自定义数据目录，macOS / Linux 上
            // 该调用无效果。因此**只在 Windows 上设置**，不去依赖未定义行为。
            #[cfg(target_os = "windows")]
            let window = {
                use tauri::Manager as _;
                let directory = handle.state::<AppState>().app.paths().webview_data_dir();
                window.data_directory(directory)
            };

            // 拿住窗口句柄：`on_window_event` 挂在**建好的窗口**上，而不是 builder 上。
            let window = window.build()?;

            // **关闭窗口先问前端**，而不是直接关掉：应用里可能跑着终端与本地网关。
            //
            // 拦截与放行的判断全在 [`crate::exit::ExitFlow`] 里（那是纯逻辑，可以脱离
            // 窗口测试）；这里只做「拿到状态 → 照做」。
            {
                use tauri::Manager as _;
                let app_for_events = handle.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // **前端不可达时直接放行**，连状态机都不问：没人能弹对话框，
                        // 拦住就等于把用户锁在应用里。见 `exit` 模块的失败安全。
                        if !crate::exit::frontend_reachable(&app_for_events) {
                            tracing::warn!("前端不可达，关闭请求直接放行");
                            return;
                        }
                        let state = app_for_events.state::<AppState>();
                        if state.exit_flow.close_requested() == crate::exit::CloseAction::Prompt {
                            api.prevent_close();
                            crate::tray::restore_window(&app_for_events);
                            crate::exit::emit_request(&app_for_events);
                        }
                    }
                });
            }

            // 托盘失败不阻止启动：没有托盘仍然可以用窗口。
            if let Err(error) = crate::tray::install(handle.handle()) {
                tracing::warn!(%error, "托盘安装失败，继续以无托盘模式运行");
            }
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(|handle, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = &event {
                // 已经带退出码的请求（`exit_after_cleanup` 结尾的 `handle.exit(0)`）
                // 直接放行，否则会形成「清理 → exit → 清理」的循环。
                if code.is_none() {
                    api.prevent_exit();
                    exit_after_cleanup(handle);
                }
            }
        });
    Ok(())
}

/// 跑完退出清理再退出。
///
/// # 为什么抽成函数
///
/// 退出有**三条入口**：窗口关闭（`RunEvent::ExitRequested`，`code` 为 `None`）、
/// 确认框里选「直接退出」、托盘菜单「退出」。它们必须走同一段收尾，否则总有一条会漏掉
/// 清理——托盘那条此前就是直接 `app.exit(0)`，**跳过了全部清理**。
///
/// 幂等：`RunEvent::ExitRequested` 可能被触发多次，重复调用只会重复跑清理步骤
/// （它们本身是幂等的）。
pub fn exit_after_cleanup(handle: &tauri::AppHandle) {
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let records = shutdown::global().run(cleanup_steps()).await;
        tracing::info!("退出清理：{}", shutdown::summarize(&records));
        // 带退出码的 `exit` 会在 `ExitRequested` 里被放行，因此不会回到这里形成循环。
        handle.exit(0);
    });
}

/// 退出清理的步骤，**顺序是契约**。
///
/// 先关网关会让在途请求失败，而那些请求的日志还没写完——因此必须按这个顺序。
/// 与现网 `shutdownApplication()` 的三步一一对应。
#[must_use]
pub fn cleanup_steps() -> Vec<(&'static str, shutdown::CleanupStep)> {
    vec![
        (
            "停止 PTY",
            // 终端尚未接入命令面，因此**显式记为跳过**，而不是假装清理过——
            // 假的「已完成」会让将来排查真问题时看不到缺口。
            shutdown::skipped_step("终端尚未接入命令面"),
        ),
        (
            "停止 Gateway",
            shutdown::async_step(|| async {
                // 网关句柄由持有它的 `App` 负责优雅关闭（`App::shutdown`）。
                // 这里不重复关闭：`GatewayHandle::shutdown` 会等待在途请求，
                // 而重复调用没有意义。留这一步是为了让顺序与现网一致。
                Ok(())
            }),
        ),
        (
            "flush 日志",
            shutdown::sync_step(|| {
                // `tracing` 的写入是同步的，进程退出前无需额外动作。
                // 保留这一步是为了将来接入文件日志时在此 flush，并且让顺序可断言。
                Ok(())
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 直接调用命令函数体，绕过 Tauri 的状态提取。
    ///
    /// Tauri 的 `State` 只能从真实应用里取得，因此这里测的是「同样的转发逻辑」：
    /// 断言错误被映射成消息文本、成功结果原样返回。
    async fn invoke(state: &AppState, command: &str, args: Value) -> Result<Value, String> {
        state
            .app
            .dispatch(command, args)
            .await
            .map_err(|error| error.to_string())
    }

    async fn state() -> (AppState, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let app = App::start(dir.path().join("runtime"), None)
            .await
            .expect("start");
        (AppState::new(app), dir)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_successful_command_returns_its_payload() {
        let (state, _dir) = state().await;
        let saved = invoke(
            &state,
            "workspace:save-preset",
            json!({ "name": "工作区", "cwd": "/w" }),
        )
        .await
        .expect("save");
        assert!(saved.is_string());

        let presets = invoke(&state, "workspace:list-presets", json!({}))
            .await
            .expect("list");
        assert_eq!(presets.as_array().expect("数组").len(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn errors_become_plain_messages() {
        // 前端读的是 `error.message`，因此错误必须是消息文本而不是结构体。
        let (state, _dir) = state().await;
        let error = invoke(&state, "nope:nope", json!({}))
            .await
            .expect_err("必须失败");
        assert_eq!(error, "未知命令：nope:nope");

        let error = invoke(&state, "codex:list-sessions", json!({}))
            .await
            .expect_err("必须失败");
        assert_eq!(error, "参数无效：targetId");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_args_value_is_treated_as_null() {
        // 前端可能不传参数（例如 `list-presets`），此时不能因缺参数而失败。
        let (state, _dir) = state().await;
        let presets = invoke(&state, "workspace:list-presets", Value::Null)
            .await
            .expect("list");
        assert!(presets.as_array().expect("数组").is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_state_exposes_the_underlying_app() {
        let (state, _dir) = state().await;
        assert!(state.app().paths().database_path().is_file());
    }
}

/// 主窗口规格。
///
/// 单独抽出来是为了能**不启动窗口**就断言——这些值与现网一致，是 parity 矩阵 A1 的契约。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowSpec {
    /// 标题。
    pub title: &'static str,
    /// 宽度（逻辑像素）。
    pub width: f64,
    /// 高度。
    pub height: f64,
    /// 最小宽度。
    pub min_width: f64,
    /// 最小高度。
    pub min_height: f64,
    /// 背景色（RGBA，0–255）。
    pub background: (u8, u8, u8, u8),
}

/// 主窗口规格，与现网逐值一致。
#[must_use]
pub const fn window_spec() -> WindowSpec {
    WindowSpec {
        title: "AI 可视化控制台",
        width: 1280.0,
        height: 820.0,
        min_width: 980.0,
        min_height: 640.0,
        // `#f6f7f9`，不透明。
        background: (0xf6, 0xf7, 0xf9, 0xff),
    }
}

#[cfg(test)]
mod window_spec_tests {
    use super::*;

    #[test]
    fn the_window_spec_matches_the_desktop_reference() {
        // parity 矩阵 A1：尺寸、最小尺寸、标题、背景色。
        let spec = window_spec();
        assert_eq!(spec.title, "AI 可视化控制台");
        assert!((spec.width - 1280.0).abs() < f64::EPSILON);
        assert!((spec.height - 820.0).abs() < f64::EPSILON);
        assert!((spec.min_width - 980.0).abs() < f64::EPSILON);
        assert!((spec.min_height - 640.0).abs() < f64::EPSILON);
        assert_eq!(spec.background, (0xf6, 0xf7, 0xf9, 0xff));
    }

    #[test]
    fn the_minimum_is_smaller_than_the_default() {
        // 最小尺寸大于默认尺寸会让窗口一打开就被撑大。
        let spec = window_spec();
        assert!(spec.min_width <= spec.width, "最小宽度不应超过默认宽度");
        assert!(spec.min_height <= spec.height, "最小高度不应超过默认高度");
    }
}
