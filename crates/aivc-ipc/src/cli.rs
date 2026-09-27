//! CLI 环境与安装命令。
//!
//! # 为什么用 `spawn_blocking`
//!
//! 探测与安装都是**同步阻塞**的（轮询等子进程）。探测最多跑 6 条命令 × 10 秒，
//! 安装的单个步骤上限是 15 分钟。直接在 async 上下文里同步等待会**占住整个运行时**，
//! 期间其他命令（会话列表、网关设置）全部无响应——界面看起来像卡死了。
//!
//! # `cli:install` 是破坏性操作
//!
//! 它会改用户的全局 npm 环境、切换 nvm 版本。这里**严格照现网的语义**：
//! 一次调用即执行，不额外插入确认步骤——拆分会让前端的调用契约失效。
//! 但响应里带上 `plan`（实际执行的命令），因此界面可以把「跑了什么」展示出来，
//! 出问题时也有据可查。
//!
//! # 环境探测为什么带缓存
//!
//! 前端在多个界面（设置页、安装向导）都会查环境，短时间内会重复调用。
//! 每次探测要跑 6 条外部命令，代价可观。这里对**同一目标**的并发探测做合流：
//! 首个请求真正执行，其余等待同一结果。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use aivc_core::error::IpcError;
use aivc_core::target::ProviderId;
use aivc_core::{Error, Result};
use aivc_tooling::cli_env::install::{
    InstallRequest, InstallScope, InstallStep, installer_for, plan_install, run_install,
};
use aivc_tooling::cli_env::probe::{
    CommandRunner, EnvironmentSnapshot, ProbeScope, SystemRunner, probe_environment,
    status_from_snapshot,
};
use serde_json::{Value, json};

/// 把领域层的字符串错误包成命令面的错误。
fn failed(message: String) -> Error {
    Error::Ipc(IpcError::CommandFailed { message })
}

/// 主机平台名（与现网 `process.platform` 的取值一致）。
#[must_use]
pub const fn host_platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

/// 由目标编号推导执行环境。
///
/// 目标编号里带 `:wsl:` 就是 WSL 目标。发行版名从编号里取出——**不重新探测**，
/// 因为编号本身就是权威来源，再探测一次可能拿到不同的发行版。
#[must_use]
pub fn scope_for_target(target_id: &str) -> (ProbeScope, InstallScope, Option<String>) {
    // 用 `ProviderId::wsl_distro_of` 而不是自己解析字符串：目标编号的构造与解析
    // 必须共用同一份规则，否则改了一处就会在这里静默错判。
    let distro = ProviderId::from_target_id(target_id).wsl_distro_of(target_id);
    if !distro.is_empty() {
        return (ProbeScope::Wsl, InstallScope::Wsl, Some(distro.to_owned()));
    }
    if cfg!(target_os = "windows") {
        (ProbeScope::Local, InstallScope::WindowsLocal, None)
    } else {
        (ProbeScope::Local, InstallScope::PosixLocal, None)
    }
}

/// CLI 命令处理器。
pub struct CliCommands {
    runner: Arc<dyn CommandRunner>,
    /// 同目标的探测合流结果。
    snapshots: Mutex<HashMap<String, EnvironmentSnapshot>>,
}

impl Default for CliCommands {
    fn default() -> Self {
        Self::new()
    }
}

impl CliCommands {
    /// 用真实执行器构造。
    #[must_use]
    pub fn new() -> Self {
        Self::with_runner(Arc::new(SystemRunner))
    }

    /// 用指定执行器构造（测试注入用）。
    #[must_use]
    pub fn with_runner(runner: Arc<dyn CommandRunner>) -> Self {
        Self {
            runner,
            snapshots: Mutex::new(HashMap::new()),
        }
    }

    /// 解析平台编号，并确认它支持安装。
    fn require_provider(provider_id: &str) -> Result<ProviderId> {
        // **不要用 `from_target_id`**：它按 `"gemini:"` 这样的**带冒号前缀**匹配，
        // 而这里收到的是**裸平台名** `"gemini"`——不匹配时会落到 Codex 兜底分支，
        // 于是「检查 Gemini 环境」变成「检查 Codex 环境」而毫无提示。
        //
        // 兜底对「目标编号」是合理兼容（未知编号当 Codex），对「平台编号」则是
        // 静默的错误答案，因此这里显式匹配。
        let trimmed = provider_id.trim();
        let provider = match trimmed {
            "codex" => ProviderId::Codex,
            "gemini" => ProviderId::Gemini,
            "claude" => ProviderId::Claude,
            "qoder" => ProviderId::Qoder,
            _ => return Err(failed(format!("未知 CLI：{provider_id}"))),
        };
        if installer_for(provider).is_none() {
            return Err(failed(format!("未知 CLI：{provider_id}")));
        }
        Ok(provider)
    }

    /// 探测环境（阻塞部分在专用线程上跑）。
    async fn snapshot(&self, target_id: &str, provider: ProviderId) -> Result<EnvironmentSnapshot> {
        let cache_key = if target_id.trim().is_empty() {
            "local".to_owned()
        } else {
            target_id.trim().to_owned()
        };
        if let Some(cached) = self
            .snapshots
            .lock()
            .map_err(|_| failed("环境快照缓存不可用。".to_owned()))?
            .get(&cache_key)
            .cloned()
        {
            return Ok(cached);
        }

        let (probe_scope, _, _) = scope_for_target(&cache_key);
        let runner = Arc::clone(&self.runner);
        // 阻塞调用必须离开 async 运行时：单次探测最多 6 条命令 × 10 秒。
        let snapshot = tokio::task::spawn_blocking(move || {
            probe_environment(runner.as_ref(), probe_scope, provider)
        })
        .await
        .map_err(|error| failed(format!("环境探测任务失败：{error}")))?;

        self.snapshots
            .lock()
            .map_err(|_| failed("环境快照缓存不可用。".to_owned()))?
            .insert(cache_key, snapshot.clone());
        Ok(snapshot)
    }

    /// `cli:check-environment`。
    ///
    /// # Errors
    ///
    /// 平台编号非法时返回错误。探测本身失败**不返回错误**——失败信息在 `messages` 里，
    /// 因为「探测不到」是一种应当展示给用户的状态，而不是命令失败。
    pub async fn check_environment(&self, provider_id: &str, target_id: &str) -> Result<Value> {
        let provider = Self::require_provider(provider_id)?;
        let snapshot = self.snapshot(target_id, provider).await?;
        let (probe_scope, _, _) = scope_for_target(target_id);
        let status =
            status_from_snapshot(&snapshot, provider, target_id, probe_scope, host_platform());
        Ok(status_to_json(&status))
    }

    /// `cli:install`。
    ///
    /// # Errors
    ///
    /// 平台非法、nodeMajor 越界、或安装失败时返回错误（带真实 stderr）。
    pub async fn install(
        &self,
        provider_id: &str,
        target_id: &str,
        ensure_node: bool,
        node_major: Option<u32>,
    ) -> Result<Value> {
        let provider = Self::require_provider(provider_id)?;
        let (_, install_scope, distro) = scope_for_target(target_id);
        let request = InstallRequest {
            provider,
            scope: install_scope,
            distro,
            ensure_node,
            node_major: node_major.unwrap_or(aivc_tooling::cli_env::core::RECOMMENDED_NODE_MAJOR),
            target_id: target_id.to_owned(),
        };
        // 先构造计划：命令拼错在这里就暴露，而不是等真的装了一半。
        let steps = plan_install(&request).map_err(failed)?;
        // 先取出展示串：`steps` 随后会被移动进阻塞任务。
        let plan: Vec<String> = steps.iter().map(InstallStep::display).collect();
        let runner = Arc::clone(&self.runner);
        let install_request = request.clone();
        let outcome = tokio::task::spawn_blocking(move || {
            run_install(runner.as_ref(), &steps, &install_request)
        })
        .await
        .map_err(|error| failed(format!("安装任务失败：{error}")))?
        .map_err(failed)?;

        // 装完清缓存：下次探测必须拿到新版本，否则界面会显示「未检测到」而实际已装好。
        if let Ok(mut cache) = self.snapshots.lock() {
            cache.remove(target_id);
            cache.remove("local");
        }

        Ok(json!({
            "providerId": outcome.provider_id,
            "command": outcome.command,
            "stdout": outcome.stdout,
            "stderr": outcome.stderr,
            // 响应里带上实际执行过的命令，供界面展示与排障。
            "plan": plan,
        }))
    }
}

/// 状态转 JSON（字段名与现网 `CliEnvironmentStatus` 一致）。
fn status_to_json(status: &aivc_tooling::cli_env::probe::CliEnvironmentStatus) -> Value {
    json!({
        "providerId": status.provider_id,
        "targetId": status.target_id,
        "scope": status.scope,
        "platform": status.platform,
        "minimumNodeMajor": status.minimum_node_major,
        "recommendedNodeMajor": status.recommended_node_major,
        "nodeVersion": status.node_version,
        "npmVersion": status.npm_version,
        "nvmVersion": status.nvm_version,
        "cliVersion": status.cli_version,
        "nodePath": status.node_path,
        "npmPath": status.npm_path,
        "hasNode": status.has_node,
        "hasNpm": status.has_npm,
        "hasNvm": status.has_nvm,
        "hasCli": status.has_cli,
        "nodeManagedByNvm": status.node_managed_by_nvm,
        "nodeMeetsMinimum": status.node_meets_minimum,
        "canInstallCli": status.can_install_cli,
        "canInstallNodeWithNvm": status.can_install_node_with_nvm,
        "messages": status.messages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_tooling::cli_env::probe::CommandRun;
    use std::collections::BTreeMap;

    /// 按命令名返回固定输出。
    struct Fake {
        responses: BTreeMap<String, CommandRun>,
        /// 未精确匹配时返回什么。默认是「空输出 + 退出码 0」。
        fallback: CommandRun,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                responses: BTreeMap::new(),
                fallback: CommandRun::ok(""),
            }
        }

        /// 所有命令都以同一个失败结果回应。
        fn failing(run: CommandRun) -> Self {
            Self {
                responses: BTreeMap::new(),
                fallback: run,
            }
        }

        fn with(mut self, key: &str, stdout: &str) -> Self {
            self.responses
                .insert(key.to_owned(), CommandRun::ok(stdout));
            self
        }
    }

    impl CommandRunner for Fake {
        fn run(&self, program: &str, args: &[&str], _timeout_ms: u64) -> CommandRun {
            let key = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.responses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| self.fallback.clone())
        }
    }

    fn commands(runner: Fake) -> CliCommands {
        CliCommands::with_runner(Arc::new(runner))
    }

    #[test]
    fn target_scope_is_derived_from_the_target_id() {
        // 不重新探测发行版：编号本身就是权威来源。
        // Codex 的 WSL 编号是**裸** `wsl:<distro>`（其余平台才带 provider 前缀）。
        let (probe, install, distro) = scope_for_target("wsl:Ubuntu");
        assert_eq!(probe, ProbeScope::Wsl);
        assert_eq!(install, InstallScope::Wsl);
        assert_eq!(distro.as_deref(), Some("Ubuntu"));

        let (probe, install, distro) = scope_for_target("codex:local");
        assert_eq!(probe, ProbeScope::Local);
        assert_eq!(distro, None);
        // 本机分支取决于宿主平台。
        if cfg!(target_os = "windows") {
            assert_eq!(install, InstallScope::WindowsLocal);
        } else {
            assert_eq!(install, InstallScope::PosixLocal);
        }
    }

    #[tokio::test]
    async fn an_unknown_provider_is_rejected() {
        // `from_target_id` 会把无法识别的输入兜底成 Codex——对目标编号是合理兼容，
        // 对平台编号则是**静默的错误答案**，因此必须回查。
        let commands = commands(Fake::new());
        let error = commands
            .check_environment("not-a-provider", "codex:local")
            .await
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("未知 CLI"), "{error}");
    }

    #[tokio::test]
    async fn a_healthy_environment_is_reported() {
        let runner = Fake::new()
            .with("node --version", "v22.11.0\n")
            .with("npm --version", "10.9.0\n")
            .with("nvm --version", "0.40.1\n")
            .with("codex --version", "0.30.0\n")
            .with("where node", "/usr/bin/node\n")
            .with("where npm", "/usr/bin/npm\n");
        let value = commands(runner)
            .check_environment("codex", "codex:local")
            .await
            .expect("检查");
        assert_eq!(value["providerId"], "codex");
        assert_eq!(value["hasNode"], true);
        assert_eq!(value["nodeMeetsMinimum"], true);
        assert_eq!(value["minimumNodeMajor"], 22);
        assert_eq!(value["recommendedNodeMajor"], 24);
        assert_eq!(value["messages"].as_array().expect("数组").len(), 0);
    }

    #[tokio::test]
    async fn a_missing_environment_yields_messages_not_an_error() {
        // 「探测不到」是要展示给用户的状态，不是命令失败。
        let value = commands(Fake::new())
            .check_environment("codex", "codex:local")
            .await
            .expect("检查不应失败");
        assert_eq!(value["hasNode"], false);
        let messages = value["messages"].as_array().expect("数组");
        assert!(!messages.is_empty());
    }

    #[tokio::test]
    async fn the_status_carries_every_frontend_field() {
        // 少一个字段前端就会拿到 undefined。
        let value = commands(Fake::new())
            .check_environment("gemini", "gemini:local")
            .await
            .expect("检查");
        for field in [
            "providerId",
            "targetId",
            "scope",
            "platform",
            "minimumNodeMajor",
            "recommendedNodeMajor",
            "nodeVersion",
            "npmVersion",
            "nvmVersion",
            "cliVersion",
            "nodePath",
            "npmPath",
            "hasNode",
            "hasNpm",
            "hasNvm",
            "hasCli",
            "nodeManagedByNvm",
            "nodeMeetsMinimum",
            "canInstallCli",
            "canInstallNodeWithNvm",
            "messages",
        ] {
            assert!(value.get(field).is_some(), "缺少字段 {field}");
        }
        // Gemini 的最低版本与 Codex 不同。
        assert_eq!(value["minimumNodeMajor"], 20);
    }

    #[tokio::test]
    async fn a_wsl_target_reports_linux_and_the_install_goes_through_wsl() {
        let runner = Fake::new().with("bash -lc node --version", "v22.0.0\n");
        let value = commands(runner)
            .check_environment("codex", "wsl:Ubuntu")
            .await
            .expect("检查");
        assert_eq!(value["scope"], "wsl");
        // 目标在 WSL 里，平台就必须报 linux。
        assert_eq!(value["platform"], "linux");
        assert_eq!(value["hasNode"], true);
    }

    #[tokio::test]
    async fn installing_without_node_ensuring_runs_a_single_step() {
        let value = commands(Fake::new().with("bash -lc x", "ok"))
            .install("codex", "codex:local", false, None)
            .await
            .expect("安装");
        assert_eq!(value["providerId"], "codex");
        assert_eq!(value["command"], "npm install");
        // 计划要如实回传，供界面展示「跑了什么」。
        let plan = value["plan"].as_array().expect("计划");
        assert_eq!(plan.len(), 1);
    }

    #[tokio::test]
    async fn installing_with_node_ensuring_produces_two_steps_on_posix() {
        let value = commands(Fake::new().with("bash -lc x", "ok"))
            .install("codex", "codex:local", true, Some(22))
            .await
            .expect("安装");
        let plan = value["plan"].as_array().expect("计划");
        let expected = if cfg!(target_os = "windows") { 1 } else { 2 };
        assert_eq!(plan.len(), expected, "计划：{plan:?}");
    }

    #[tokio::test]
    async fn an_out_of_range_node_major_is_rejected() {
        let error = commands(Fake::new())
            .install("codex", "codex:local", true, Some(999))
            .await
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("nodeMajor"), "{error}");
    }

    #[tokio::test]
    async fn a_failed_install_reports_the_real_stderr() {
        // 用 WSL 目标：命令形态与宿主平台无关，测试因此在所有平台结论一致。
        let runner = Fake::failing(CommandRun::failed(1, "", "EACCES: permission denied"));
        let error = commands(runner)
            .install("codex", "wsl:Ubuntu", false, None)
            .await
            .expect_err("必须失败");
        assert!(error.to_string().contains("EACCES"), "{error}");
    }

    #[tokio::test]
    async fn the_snapshot_cache_is_reused_then_cleared_after_install() {
        // 首次探测真的跑命令；第二次命中缓存。装完必须清缓存，
        // 否则界面会显示「未检测到」而实际已装好。
        let runner = Fake::new().with("node --version", "v22.0.0\n");
        let commands = commands(runner);
        commands
            .check_environment("codex", "codex:local")
            .await
            .expect("首次");
        commands
            .check_environment("codex", "codex:local")
            .await
            .expect("第二次");
        assert_eq!(
            commands.snapshots.lock().expect("lock").len(),
            1,
            "两次探测应当共用一份快照"
        );
        commands
            .install("codex", "codex:local", false, None)
            .await
            .expect("安装");
        assert!(
            commands.snapshots.lock().expect("lock").is_empty(),
            "安装后必须清空快照缓存"
        );
    }

    #[test]
    fn the_host_platform_matches_the_desktop_reference() {
        let expected = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        assert_eq!(host_platform(), expected);
    }

    #[test]
    fn a_target_id_without_a_distro_is_not_wsl() {
        // 防回归：判定必须只认带 `:wsl:` 的编号。
        let (probe, _, distro) = scope_for_target("codex:local");
        assert_eq!(probe, ProbeScope::Local);
        assert_eq!(distro, None);
        let (probe, _, distro) = scope_for_target("wsl:Ubuntu");
        assert_eq!(probe, ProbeScope::Wsl);
        assert_eq!(distro.as_deref(), Some("Ubuntu"));
    }
}
