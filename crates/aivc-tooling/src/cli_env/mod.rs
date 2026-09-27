//! CLI 环境探测与安装。
//!
//! 探测 node / npm / nvm 与各平台 CLI 的版本，并给出可安装性判断。推荐 Node 24；
//! Gemini 与 Qoder 最低 Node 20。安装命令为 `npm install -g …`，超时 15 分钟。
//!
//! WSL 侧必须用 `bash -lc` 探测：`nvm` / `fnm` 按 profile 注入 PATH，非登录 shell 看不到。

pub mod core;
pub mod install;
pub mod probe;

pub use core::{
    CommandOutput, EnvironmentFacts, INSTALL_TIMEOUT_MS, RECOMMENDED_NODE_MAJOR,
    build_environment_messages, build_install_error, cli_version_key, merge_output, merge_streams,
    minimum_node_major, node_managed_by_nvm, parse_node_major,
};

pub use probe::{
    CliEnvironmentStatus, CommandRun, CommandRunner, EnvironmentSnapshot, PROBE_TIMEOUT_MS,
    ProbeScope, SystemRunner, probe_environment, status_from_snapshot,
};

pub use install::{
    CLI_INSTALLERS, CliInstaller, InstallOutcome, InstallRequest, InstallScope, InstallStep,
    installer_for, nvm_shell_script, plan_install, run_install,
};
