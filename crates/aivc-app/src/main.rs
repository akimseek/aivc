//! 应用入口：装配 → 启动网关 → 打开窗口。
//!
//! # 启动顺序
//!
//! 1. **解析运行根**——数据库、日志、设置都从它派生。
//! 2. **装配全部组件**（[`App::start`]）——目录、数据库迁移、仓储、上下文、网关来源、
//!    命令路由器。
//! 3. **按设置启动网关**——失败了只记日志，不阻止窗口打开：网关是本应用的增强功能，
//!    没有它仍能浏览会话。
//! 4. **进入 Tauri 事件循环**。
//!
//! # 为什么网关启动失败不致命
//!
//! 端口被占用、防火墙拦截这类问题不该让整个应用打不开。用户至少应该能会话管理，
//! 并在设置里改端口后重启。
//!
//! # `windows_subsystem`
//!
//! 发布构建下不弹控制台窗口；调试构建保留它，否则 `tracing` 的输出没地方看。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use aivc_core::target::Platform;
use aivc_infra::config::resolve_runtime_storage_root;

fn main() {
    if let Err(error) = run() {
        // 启动失败时没有窗口可显示错误，只能写 stderr——发布构建下会落到日志文件。
        eprintln!("启动失败：{error:#}");
        std::process::exit(1);
    }
}

/// 真正的启动流程。
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::current_exe()?;
    let cwd = std::env::current_dir()?;
    let root = resolve_runtime_storage_root(
        // `isPackaged` 的等价物：发布构建视为已打包。
        !cfg!(debug_assertions),
        &executable,
        &cwd,
        Platform::current(),
    );
    tracing::info!(
        root = %root.root.display(),
        shape = ?root.shape,
        "运行根已解析"
    );

    // 日志落 `<运行根>/logs/app.log`：只写控制台时排查者读不到它。
    if let Some(path) = aivc_app::logging::init(&root.root) {
        tracing::info!(log = %path.display(), "日志已落文件");
    }

    // **生产环境不传 bash**。
    //
    // `bash` 参数是**测试钩子**：让 CI 在没有 WSL 的机器上也能跑 WSL 协议。
    // 生产传 `None` 才会用 `WslExeLauncher`（真正调用 `wsl.exe`）。
    //
    // 实测踩过：这里原本调 `discover_bash()` 去猜 Git Bash 路径，
    // 一旦找到（`D:\tools\git\Git\bin\bash.exe`）就会把 WSL 换成本机 bash——
    // 于是 `wsl.exe -l -q` 在 Git Bash 里执行、报「未找到 wsl.exe」，
    // **整个 WSL 功能静默失效**，而日志只说「未找到 wsl.exe」。
    let bash: Option<String> = None;
    let app = tauri::async_runtime::block_on(aivc_app::App::start(root.root, bash.as_deref()))?;
    // 目标预热跑在后台：它不属于装配，因此由入口显式启动。
    // 用 `tauri::async_runtime::spawn` 而不是 `tokio::spawn`——后者要求调用点已经在
    // 运行时上下文里，而这里不在（实测直接 panic：`there is no reactor running`）。
    {
        let context = app.router().context().clone();
        tauri::async_runtime::spawn(aivc_app::App::warm_targets(context));
    }

    // 网关启动失败不阻止窗口打开。
    let mut app = app;
    match tauri::async_runtime::block_on(app.start_gateway_if_enabled()) {
        Ok(Some(address)) => tracing::info!(%address, "本地 Gateway 已启动"),
        Ok(None) => tracing::info!("设置里未启用本地 Gateway"),
        Err(error) => tracing::warn!(%error, "本地 Gateway 启动失败，继续以无网关模式运行"),
    }

    aivc_app::tauri_app::run(app)?;
    Ok(())
}
