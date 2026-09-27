//! WSL 适配器：输出解码、进程启动、持久 worker。
//!
//! 现网每次文件操作都拉起一次 `wsl.exe`（单次 50–150 ms 固定开销），会话列表遍历
//! 数百个文件时开销线性放大。这里的核心改进是**每个发行版一个常驻 worker**
//! （见 [`worker`]），把单次操作降到一次 stdin/stdout 往返。
//!
//! # 子模块
//!
//! - [`decode`]：`wsl.exe` 输出的 UTF-16/UTF-8 解码与 `find -printf` 解析。
//! - [`launcher`]：如何进入目标环境的 bash。抽成接口是为了能用本机 bash 验证协议。
//! - [`worker`]：常驻 worker 的帧协议、客户端与连接池。
//!
//! # 必须保留的现网语义
//!
//! - 脚本经 base64 透传，绕开 `wsl.exe` 对引号与反斜杠的再解析；
//! - 写文件走 `cat > file` 语义（这里用 `base64 -d > file`，同样不经过 shell 解析）；
//! - 输出上限（shell 16 MB / 读文件 32 MB / stderr 1 MB）；
//! - 60 s 兜底超时；
//! - 排除 Docker Desktop 内部发行版；
//! - `-l -q` 按 UTF-16 解码。

pub mod adapter;
pub mod decode;
pub mod launcher;
pub mod script_transfer;
pub mod worker;

pub use adapter::{
    HEAD_BATCH_SIZE, INTERNAL_WSL_DISTROS, WslAdapter, WslConfig, WslSessionStore,
    codex_home_probe_script, gateway_host_probe_script, parse_distro_list, parse_head_batch,
    read_heads_script, wrap_script_for_transport,
};
pub use decode::{decode_wsl_output, parse_wsl_session_file_list};
pub use launcher::{LocalBashLauncher, WslExeLauncher, WslLauncher};
pub use worker::{CommandOutput, WorkerOp, WslWorker, WslWorkerPool};
