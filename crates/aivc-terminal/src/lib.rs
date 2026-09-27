//! 终端运行时：PTY 会话、命令构造、状态检测器、供应商路由绑定。
//!
//! # 模块
//!
//! - [`command`]：各平台 CLI 的命令行与环境变量构造（含网关路由注入）。
//! - [`locate`]：让子进程找得到 npm 装的 CLI（补齐 PATH）。
//! - [`status`]：从 PTY 输出流中识别「等待确认 / 出错 / 本轮已完成」。
//! - `session`：PTY 会话生命周期与输出批处理 —— **需要 `pty` 特性**。
//!
//! # 为什么状态检测是纯函数
//!
//! 它的输入是「任意切分的输出批次」，输出是「要不要提醒用户」。把它做成纯状态机之后，
//! 所有分支都能构造出来断言——包括那些真实终端里很难复现的时序（提示符被切成两批、
//! 同一轮里的重复输入、启动界面上的空闲提示）。
//!
//! # `pty` 特性为什么默认关闭
//!
//! `portable-pty` 在受限沙箱里会让**测试二进制无法启动**（`STATUS_DLL_INIT_FAILED`），
//! 因为 PTY 需要创建控制台宿主进程，而沙箱的作业对象不允许。这会让「环境限制」被误报成
//! 「代码缺陷」，正是项目里对 WSL 测试采用同一种处理的原因。
//!
//! 关闭不等于放任腐坏：门禁额外跑一遍 `cargo check --all-features` 与
//! `cargo clippy --all-features`，因此 `session` 始终被编译与 Lint 检查，只是不在
//! 本沙箱内**运行**。真实 PTY 行为需在沙箱外验证。

#![forbid(unsafe_code)]

pub mod command;
pub mod locate;
#[cfg(feature = "pty")]
pub mod session;
pub mod shell;
pub mod status;

pub use command::{
    CommandParams, ResolvedRoute, TerminalCommand, build_codex_command, build_gateway_cli_command,
    build_qoder_command, codex_route_config_args, provider_environment, supports_gateway,
};
#[cfg(feature = "pty")]
pub use session::{FLUSH_INTERVAL_MS, OutputBatcher, PtySession, SessionRegistry};
pub use shell::{
    ShellChoice, ShellKind, auto_kind_for, build_system_shell_command, find_on_path,
    git_bash_candidates, resolve_shell,
};
pub use status::{
    TerminalAttention, TerminalStatus, TerminalStatusDetector, TerminalStatusEvent,
    detect_attention, has_user_input, strip_ansi,
};
