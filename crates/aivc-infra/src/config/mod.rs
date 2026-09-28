//! 运行根、路径派生与 `settings.json`。
//!
//! 本模块回答两个问题：
//!
//! 1. **应用把数据写到哪里** —— [`runtime_root`] 与 [`paths`]。核心约束是应用自有产物
//!    绝不写入系统用户目录，全部从运行根派生。
//! 2. **用户的持久化偏好是什么** —— [`settings`]。网关端口、失败阈值、熔断参数、
//!    WSL 路径覆盖与目标缓存都在这里，且读取路径必须**逐字段容错**（见该模块文档）。
//!
//! 官方 CLI 自己的目录（`~/.codex`、`~/.claude`、`~/.gemini`、`~/.qoder-cn`）是产品
//! 明确依赖的**用户数据边界**，由 `aivc-provider` 负责读写，不属于本模块的路径模型。

pub mod paths;
pub mod runtime_root;
pub mod settings;

pub use paths::AppPaths;
pub use runtime_root::{
    RUNTIME_ROOT_ENV, RuntimeRoot, RuntimeShape, is_unpacked_build_directory, overridden_root,
    resolve_runtime_storage_root,
};
pub use settings::{AppSettings, RawSettings, SettingsStore};
