//! CLI 安装：构造安装步骤并执行。
//!
//! # 为什么把「构造」与「执行」分开
//!
//! 安装会**真的改用户的全局环境**（装全局 npm 包、切 nvm 版本）。因此要把
//! 「打算跑什么命令」做成可断言的数据（[`plan_install`]），执行只是照着跑。
//! 这样：
//!
//! - 命令拼错、参数顺序错、平台分支错，都能在单测里发现，不需要真装一次；
//! - 界面可以在执行前把计划展示给用户（「将要执行：npm install -g @openai/codex」）；
//! - 出问题时能明确回答「到底跑了哪几条命令」。
//!
//! # 三个平台分支
//!
//! | 场景 | 命令 |
//! | --- | --- |
//! | Windows 本机 + 需要装 Node | `cmd /d /s /c "nvm install N && nvm use N && npm <args>"`（**一条**命令） |
//! | 其余 + 需要装 Node | 先 `nvm install N && nvm use N`，再 `npm <args>` |
//! | 不需要装 Node | 只跑 `npm <args>` |
//!
//! Windows 本机之所以合并成一条：`nvm use` 改变的是**当前 shell** 的 PATH。
//! 分成两条命令会各自起一个 `cmd`，第二条看不到第一条切换的结果，于是 `npm`
//! 还是旧版本甚至找不到。
//!
//! # POSIX 侧必须 source nvm
//!
//! `nvm` 是个 shell 函数而不是可执行文件，非登录 shell 里直接调 `nvm` 会
//! `command not found`。因此脚本先定位 `NVM_DIR` 并 source `nvm.sh`。
//!
//! # 失败必须带真实输出
//!
//! 跨 IPC 只有 `message` 会保留，自定义属性会被丢弃。若只抛「安装失败」，
//! 用户拿不到任何可行动的信息——真实原因（网络、权限、包名变更）都在 stderr 上。

use aivc_core::target::ProviderId;

use crate::cli_env::core::{CommandOutput, INSTALL_TIMEOUT_MS, build_install_error, merge_streams};
use crate::cli_env::probe::{CommandRun, CommandRunner};

/// Node 主版本的合法区间。
pub const MIN_NODE_MAJOR: u32 = 18;
/// Node 主版本的上限。
///
/// 上限是**防御性**的：这个值会被拼进 shell 命令，不限制就等于允许任意输入。
/// 40 远高于任何现实版本，同时把「故意构造的畸形输入」挡在外面。
pub const MAX_NODE_MAJOR: u32 = 40;

/// 一个平台的 CLI 安装定义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliInstaller {
    /// 平台。
    pub provider: ProviderId,
    /// 展示名。
    pub label: &'static str,
    /// 描述。
    pub description: &'static str,
    /// 传给 `npm` 的参数。
    pub install_args: &'static [&'static str],
    /// 文档地址。
    pub docs_url: &'static str,
    /// 图标文字。
    pub icon_label: &'static str,
}

/// 四个平台的安装定义，顺序与现网一致。
pub const CLI_INSTALLERS: &[CliInstaller] = &[
    CliInstaller {
        provider: ProviderId::Codex,
        label: "Codex",
        description: "OpenAI Codex CLI",
        install_args: &["install", "-g", "@openai/codex"],
        docs_url: "https://github.com/openai/codex",
        icon_label: "C",
    },
    CliInstaller {
        provider: ProviderId::Gemini,
        label: "Gemini",
        description: "Google Gemini CLI",
        install_args: &["install", "-g", "@google/gemini-cli"],
        docs_url: "https://github.com/google-gemini/gemini-cli",
        icon_label: "G",
    },
    CliInstaller {
        provider: ProviderId::Claude,
        label: "Claude Code",
        description: "Anthropic Claude Code CLI",
        install_args: &["install", "-g", "@anthropic-ai/claude-code"],
        docs_url: "https://docs.anthropic.com/en/docs/claude-code/setup",
        icon_label: "A",
    },
    CliInstaller {
        provider: ProviderId::Qoder,
        label: "Qoder CN",
        description: "Qoder CN CLI",
        install_args: &["install", "-g", "@qodercn-ai/qoderclicn"],
        docs_url: "https://docs.qoder.cn/cli/installation",
        icon_label: "Q",
    },
];

/// 取某个平台的安装定义。
#[must_use]
pub fn installer_for(provider: ProviderId) -> Option<&'static CliInstaller> {
    CLI_INSTALLERS
        .iter()
        .find(|installer| installer.provider == provider)
}

/// 一条待执行的安装步骤。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallStep {
    /// 程序。
    pub program: String,
    /// 参数。
    pub args: Vec<String>,
    /// 给用户看的一句话。
    pub description: &'static str,
}

impl InstallStep {
    /// 命令的展示形式（用于界面显示与错误文案）。
    #[must_use]
    pub fn display(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// 执行环境。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallScope {
    /// Windows 本机。
    WindowsLocal,
    /// POSIX 本机（macOS / Linux）。
    PosixLocal,
    /// WSL 发行版。
    Wsl,
}

/// 安装请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallRequest {
    /// 平台。
    pub provider: ProviderId,
    /// 执行环境。
    pub scope: InstallScope,
    /// WSL 发行版（`scope` 为 [`InstallScope::Wsl`] 时必需）。
    pub distro: Option<String>,
    /// 是否先确保 Node。
    pub ensure_node: bool,
    /// 目标 Node 主版本。
    pub node_major: u32,
    /// 对外展示的目标编号。
    pub target_id: String,
}

/// 把命令拼成 shell 脚本用的形式。
fn shell_join(args: &[&str]) -> String {
    args.join(" ")
}

/// POSIX 下 source nvm 的前缀。
///
/// `nvm` 是 shell 函数不是可执行文件，非登录 shell 里直接调会 `command not found`。
#[must_use]
pub fn nvm_shell_script(command: &str) -> String {
    [
        r#"export NVM_DIR="${NVM_DIR:-$HOME/.nvm}""#,
        r#"[ -s "$NVM_DIR/nvm.sh" ] && . "$NVM_DIR/nvm.sh""#,
        command,
    ]
    .join("; ")
}

/// 构造安装计划。
///
/// # Errors
///
/// `node_major` 越界、平台无安装定义、或 WSL 缺少发行版时返回错误。
pub fn plan_install(request: &InstallRequest) -> Result<Vec<InstallStep>, String> {
    let installer = installer_for(request.provider)
        .ok_or_else(|| format!("未知 CLI：{}", request.provider.as_str()))?;
    if request.ensure_node && !(MIN_NODE_MAJOR..=MAX_NODE_MAJOR).contains(&request.node_major) {
        return Err("参数无效：nodeMajor".to_owned());
    }
    let npm_args = shell_join(installer.install_args);

    // Windows 本机 + 需要装 Node：**一条**命令。
    // 分开跑会各自起一个 cmd，第二条看不到第一条 `nvm use` 的结果。
    if request.scope == InstallScope::WindowsLocal && request.ensure_node {
        return Ok(vec![InstallStep {
            program: "cmd".to_owned(),
            args: vec![
                "/d".to_owned(),
                "/s".to_owned(),
                "/c".to_owned(),
                format!(
                    "nvm install {n} && nvm use {n} && npm {npm_args}",
                    n = request.node_major
                ),
            ],
            description: "通过 nvm 安装指定 Node 版本并安装 CLI",
        }]);
    }

    let mut steps = Vec::new();
    if request.ensure_node {
        let script = nvm_shell_script(&format!(
            "nvm install {n} && nvm use {n}",
            n = request.node_major
        ));
        steps.push(posix_step(
            request.scope,
            request.distro.as_deref(),
            &script,
            "安装并切换 Node 版本",
        )?);
    }
    // **两条路径都要 source nvm**：安装 Node 与安装 CLI 是两次独立的 shell 调用，
    // 第二条看不到第一条 `nvm use` 的结果。不 source 的话 `npm` 会跑到系统 Node 上，
    // 于是包被装进另一个 Node 的全局目录而 CLI 命令找不到。
    let script = nvm_shell_script(&format!("npm {npm_args}"));
    steps.push(posix_step(
        request.scope,
        request.distro.as_deref(),
        &script,
        "安装 CLI",
    )?);
    Ok(steps)
}

/// 构造一条 POSIX 侧命令。
///
/// WSL 侧必须 `bash -lc`：版本管理器只在登录 shell 里注入 PATH。
fn posix_step(
    scope: InstallScope,
    distro: Option<&str>,
    script: &str,
    description: &'static str,
) -> Result<InstallStep, String> {
    if scope == InstallScope::Wsl {
        let distro = distro.ok_or_else(|| "目标不是 WSL 环境。".to_owned())?;
        return Ok(InstallStep {
            program: "wsl.exe".to_owned(),
            args: vec![
                "-d".to_owned(),
                distro.to_owned(),
                "--".to_owned(),
                "bash".to_owned(),
                "-lc".to_owned(),
                script.to_owned(),
            ],
            description,
        });
    }
    Ok(InstallStep {
        program: "bash".to_owned(),
        args: vec!["-lc".to_owned(), script.to_owned()],
        description,
    })
}

/// 安装结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallOutcome {
    /// 平台编号。
    pub provider_id: String,
    /// 展示用的命令。
    pub command: &'static str,
    /// 合并后的标准输出。
    pub stdout: String,
    /// 合并后的标准错误。
    pub stderr: String,
}

/// 执行安装计划。
///
/// # Errors
///
/// 任一步骤失败或超时时返回**带真实输出**的错误文案。
pub fn run_install(
    runner: &dyn CommandRunner,
    steps: &[InstallStep],
    request: &InstallRequest,
) -> Result<InstallOutcome, String> {
    let mut outputs: Vec<CommandOutput> = Vec::new();
    let mut failed: Option<CommandOutput> = None;

    for step in steps {
        let arg_refs: Vec<&str> = step.args.iter().map(String::as_str).collect();
        let run = runner.run(&step.program, &arg_refs, INSTALL_TIMEOUT_MS);
        let output = run.output();
        let ok = !run.timed_out && succeeded(&run);
        outputs.push(output.clone());
        if !ok {
            failed = Some(output);
            break;
        }
    }

    if let Some(output) = failed {
        return Err(build_install_error(Some(&output), &outputs, "安装失败。"));
    }

    Ok(InstallOutcome {
        provider_id: request.provider.as_str().to_owned(),
        command: "npm install",
        stdout: merge_streams(outputs.iter(), false),
        stderr: merge_streams(outputs.iter(), true),
    })
}

/// 命令是否成功。
///
/// 只看退出码。最初我用「有没有输出」近似判断，结果 **stderr 有内容时被当成成功**——
/// 而安装失败的诊断恰恰全在 stderr 上。退出码才是唯一可靠的依据。
fn succeeded(run: &CommandRun) -> bool {
    !run.timed_out && run.exit_code == Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli_env::probe::CommandRun;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    /// 按 `program args...` 返回固定输出的执行器。
    struct Fake {
        responses: BTreeMap<String, CommandRun>,
        calls: Mutex<Vec<String>>,
        /// 未配置的命令默认返回什么。
        fallback: CommandRun,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                responses: BTreeMap::new(),
                calls: Mutex::new(Vec::new()),
                fallback: CommandRun::ok("ok"),
            }
        }

        fn with(mut self, key: &str, run: CommandRun) -> Self {
            self.responses.insert(key.to_owned(), run);
            self
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl CommandRunner for Fake {
        fn run(&self, program: &str, args: &[&str], _timeout_ms: u64) -> CommandRun {
            let key = std::iter::once(program)
                .chain(args.iter().copied())
                .collect::<Vec<_>>()
                .join(" ");
            self.calls.lock().expect("lock").push(key.clone());
            self.responses
                .get(&key)
                .cloned()
                .unwrap_or_else(|| self.fallback.clone())
        }
    }

    fn base_request(scope: InstallScope) -> InstallRequest {
        InstallRequest {
            provider: ProviderId::Codex,
            scope,
            distro: None,
            ensure_node: false,
            node_major: 24,
            target_id: "codex:local".to_owned(),
        }
    }

    #[test]
    fn every_platform_has_an_installer_with_a_global_package() {
        assert_eq!(CLI_INSTALLERS.len(), 4);
        for installer in CLI_INSTALLERS {
            // 必须是全局安装：局部安装装完 CLI 不在 PATH 上。
            assert!(
                installer.install_args.contains(&"-g"),
                "{} 不是全局安装",
                installer.label
            );
            assert_eq!(installer.install_args.first(), Some(&"install"));
            assert!(!installer.docs_url.is_empty());
            assert_eq!(installer.icon_label.chars().count(), 1);
        }
        // 每个平台都能查到。
        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Claude,
            ProviderId::Qoder,
        ] {
            assert!(
                installer_for(provider).is_some(),
                "{provider:?} 缺少安装定义"
            );
        }
    }

    #[test]
    fn the_package_names_match_the_desktop_reference() {
        // 包名写错会装出一个不存在的包，而错误只在真实安装时才暴露。
        assert_eq!(
            installer_for(ProviderId::Codex)
                .expect("codex")
                .install_args,
            ["install", "-g", "@openai/codex"]
        );
        assert_eq!(
            installer_for(ProviderId::Gemini)
                .expect("gemini")
                .install_args,
            ["install", "-g", "@google/gemini-cli"]
        );
        assert_eq!(
            installer_for(ProviderId::Claude)
                .expect("claude")
                .install_args,
            ["install", "-g", "@anthropic-ai/claude-code"]
        );
        assert_eq!(
            installer_for(ProviderId::Qoder)
                .expect("qoder")
                .install_args,
            ["install", "-g", "@qodercn-ai/qoderclicn"]
        );
    }

    #[test]
    fn a_plain_install_on_posix_sources_nvm_first() {
        let steps = plan_install(&base_request(InstallScope::PosixLocal)).expect("计划");
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].program, "bash");
        let script = &steps[0].args[1];
        // `nvm` 是 shell 函数，不 source 会 command not found。
        assert!(script.contains("NVM_DIR"), "{script}");
        assert!(script.contains("nvm.sh"), "{script}");
        assert!(script.contains("npm install -g @openai/codex"), "{script}");
    }

    #[test]
    fn windows_local_with_node_uses_one_combined_command() {
        // 分成两条会各自起一个 cmd，第二条看不到第一条 `nvm use` 的结果。
        let mut request = base_request(InstallScope::WindowsLocal);
        request.ensure_node = true;
        let steps = plan_install(&request).expect("计划");
        assert_eq!(steps.len(), 1, "必须合并成一条命令");
        assert_eq!(steps[0].program, "cmd");
        assert_eq!(steps[0].args[0], "/d");
        assert_eq!(steps[0].args[1], "/s");
        assert_eq!(steps[0].args[2], "/c");
        let script = &steps[0].args[3];
        assert!(script.contains("nvm install 24"), "{script}");
        assert!(script.contains("nvm use 24"), "{script}");
        assert!(script.contains("npm install -g @openai/codex"), "{script}");
    }

    #[test]
    fn wsl_installs_two_steps_and_goes_through_a_login_shell() {
        let mut request = base_request(InstallScope::Wsl);
        request.distro = Some("Ubuntu".to_owned());
        request.ensure_node = true;
        let steps = plan_install(&request).expect("计划");
        assert_eq!(steps.len(), 2, "先装 Node 再装 CLI");
        for step in &steps {
            assert_eq!(step.program, "wsl.exe");
            assert_eq!(step.args[0], "-d");
            assert_eq!(step.args[1], "Ubuntu");
            assert_eq!(step.args[3], "bash");
            // 登录 shell：版本管理器只在登录 shell 里注入 PATH。
            assert_eq!(step.args[4], "-lc");
            assert!(step.args[5].contains("NVM_DIR"));
        }
        assert!(steps[0].args[5].contains("nvm install 24"));
        assert!(steps[1].args[5].contains("npm install -g @openai/codex"));
    }

    #[test]
    fn a_wsl_request_without_a_distro_is_rejected() {
        let mut request = base_request(InstallScope::Wsl);
        request.ensure_node = true;
        let error = plan_install(&request).expect_err("必须拒绝");
        assert_eq!(error, "目标不是 WSL 环境。");
    }

    #[test]
    fn an_out_of_range_node_major_is_rejected() {
        // 这个值会被拼进 shell 命令，不限制就等于允许任意输入。
        for value in [0, 17, 41, 999] {
            let mut request = base_request(InstallScope::PosixLocal);
            request.ensure_node = true;
            request.node_major = value;
            let error = plan_install(&request).expect_err("必须拒绝");
            assert_eq!(error, "参数无效：nodeMajor", "值 {value} 应当被拒绝");
        }
        // 边界值可用。
        for value in [MIN_NODE_MAJOR, MAX_NODE_MAJOR] {
            let mut request = base_request(InstallScope::PosixLocal);
            request.ensure_node = true;
            request.node_major = value;
            assert!(plan_install(&request).is_ok(), "值 {value} 应当可用");
        }
    }

    #[test]
    fn an_out_of_range_major_is_ignored_when_node_is_not_ensured() {
        // 不需要装 Node 时这个值根本不会被拼进命令，因此不必校验。
        let mut request = base_request(InstallScope::PosixLocal);
        request.node_major = 999;
        assert!(plan_install(&request).is_ok());
    }

    #[test]
    fn the_step_description_is_human_readable() {
        let mut request = base_request(InstallScope::WindowsLocal);
        request.ensure_node = true;
        let steps = plan_install(&request).expect("计划");
        assert!(steps[0].description.contains("Node"));
        // 展示形式要能把程序与参数分开，供界面显示。
        assert!(steps[0].display().starts_with("cmd /d /s /c "));
    }

    #[test]
    fn a_successful_install_merges_both_streams() {
        let runner = Fake::new()
            .with("bash -lc x", CommandRun::ok("added 1 package\n"))
            .with("bash -lc y", CommandRun::ok(""));
        let steps = vec![InstallStep {
            program: "bash".to_owned(),
            args: vec!["-lc".to_owned(), "x".to_owned()],
            description: "第一步",
        }];
        let outcome =
            run_install(&runner, &steps, &base_request(InstallScope::PosixLocal)).expect("安装");
        assert_eq!(outcome.command, "npm install");
        assert!(outcome.stdout.contains("added 1 package"));
        assert_eq!(outcome.provider_id, "codex");
    }

    #[test]
    fn a_failure_reports_the_real_output() {
        // 跨 IPC 只有 message 会保留，真实原因必须写进文案。
        let runner = Fake::new().with(
            "bash -lc boom",
            CommandRun::failed(1, "", "EACCES: permission denied"),
        );
        let steps = vec![InstallStep {
            program: "bash".to_owned(),
            args: vec!["-lc".to_owned(), "boom".to_owned()],
            description: "安装 CLI",
        }];
        let error = run_install(&runner, &steps, &base_request(InstallScope::PosixLocal))
            .expect_err("必须失败");
        assert!(error.contains("EACCES"), "{error}");
    }

    #[test]
    fn a_timeout_is_treated_as_a_failure() {
        let runner = Fake::new().with("bash -lc slow", CommandRun::timeout());
        let steps = vec![InstallStep {
            program: "bash".to_owned(),
            args: vec!["-lc".to_owned(), "slow".to_owned()],
            description: "安装 CLI",
        }];
        assert!(run_install(&runner, &steps, &base_request(InstallScope::PosixLocal)).is_err());
    }

    #[test]
    fn execution_stops_at_the_first_failure() {
        // 第一步失败后不该继续跑第二步：那会在一个没有 Node 的环境里装全局包。
        let runner = Fake::new()
            .with("bash -lc first", CommandRun::failed(1, "", "失败"))
            .with("bash -lc second", CommandRun::ok("不该执行"));
        let steps = vec![
            InstallStep {
                program: "bash".to_owned(),
                args: vec!["-lc".to_owned(), "first".to_owned()],
                description: "第一步",
            },
            InstallStep {
                program: "bash".to_owned(),
                args: vec!["-lc".to_owned(), "second".to_owned()],
                description: "第二步",
            },
        ];
        assert!(run_install(&runner, &steps, &base_request(InstallScope::PosixLocal)).is_err());
        assert_eq!(runner.calls(), vec!["bash -lc first"], "失败后不该继续执行");
    }

    #[test]
    fn success_is_decided_by_the_exit_code_not_by_output() {
        // 最初我用「有没有输出」判断成败，结果 stderr 有内容时被当成成功——
        // 而安装失败的诊断恰恰全在 stderr 上。退出码才是唯一可靠的依据。
        let runner = Fake::new().with(
            "bash -lc quiet",
            CommandRun {
                stdout: String::new(),
                stderr: "npm warn deprecated".to_owned(),
                timed_out: false,
                exit_code: Some(0),
            },
        );
        let steps = vec![InstallStep {
            program: "bash".to_owned(),
            args: vec!["-lc".to_owned(), "quiet".to_owned()],
            description: "安装 CLI",
        }];
        // 退出码 0 且有 stderr 警告：这是成功。
        assert!(run_install(&runner, &steps, &base_request(InstallScope::PosixLocal)).is_ok());
    }

    #[test]
    fn a_non_zero_exit_is_a_failure_even_with_output() {
        let runner = Fake::new().with("bash -lc noisy", CommandRun::failed(1, "看起来正常", ""));
        let steps = vec![InstallStep {
            program: "bash".to_owned(),
            args: vec!["-lc".to_owned(), "noisy".to_owned()],
            description: "安装 CLI",
        }];
        let error = run_install(&runner, &steps, &base_request(InstallScope::PosixLocal))
            .expect_err("必须失败");
        // 失败命令自己没有 stderr 时退回 stdout，最终落到兜底文案。
        assert!(!error.is_empty());
    }
    #[test]
    fn the_install_timeout_is_passed_through() {
        struct Spy(Mutex<u64>);
        impl CommandRunner for Spy {
            fn run(&self, _program: &str, _args: &[&str], timeout_ms: u64) -> CommandRun {
                *self.0.lock().expect("lock") = timeout_ms;
                CommandRun::ok("ok")
            }
        }
        let spy = Spy(Mutex::new(0));
        let steps = vec![InstallStep {
            program: "bash".to_owned(),
            args: vec!["-lc".to_owned(), "x".to_owned()],
            description: "安装 CLI",
        }];
        run_install(&spy, &steps, &base_request(InstallScope::PosixLocal)).expect("安装");
        assert_eq!(*spy.0.lock().expect("lock"), INSTALL_TIMEOUT_MS);
    }

    #[test]
    fn the_nvm_script_shape_is_stable() {
        let script = nvm_shell_script("nvm install 24");
        assert!(script.starts_with("export NVM_DIR="));
        assert!(script.contains("nvm.sh"));
        assert!(script.ends_with("; nvm install 24"));
    }
}
