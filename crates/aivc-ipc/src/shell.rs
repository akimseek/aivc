//! Shell 命令：路径存在性检查与「用系统程序打开」。
//!
//! # 打开动作是一个 trait
//!
//! 与终端、CLI、HTTP 同理：测试不能真的去启动系统文件管理器（会弹窗、会依赖桌面环境）。
//! 注入 [`Opener`] 之后，「先校验再打开」「不存在的路径不打开」这些**编排语义**
//! 都能测，真实实现只负责调用系统命令。
//!
//! # 为什么「不存在的路径不打开」
//!
//! 把不存在的路径交给 `explorer` / `open` 会得到一个**系统级错误弹窗**，
//! 文案通常是英文且与用户的操作对不上（「Windows 找不到文件」）。
//! 先检查再打开，报错就能是我们自己的、说人话的那句。
//!
//! # WSL 目标当前显式拒绝
//!
//! `shell:path-exists` 与 `shell:open-path` 在现网都按目标分流：WSL 目标要经
//! `wsl.exe` 检查、并用宿主程序打开 `\\wsl$` 路径。这条分支尚未接线，
//! 因此**显式报错**而不是悄悄按本机路径检查——后者会把「WSL 里有」误判成「没有」。

use std::path::{Path, PathBuf};

use aivc_core::error::{Error, IpcError, Result};
use aivc_core::target::{ProviderId, TargetKind};
use serde_json::{Value, json};

/// 用系统程序打开一个路径。
pub trait Opener: Send + Sync {
    /// 打开。
    ///
    /// # Errors
    ///
    /// 系统调用失败时返回错误文案。
    fn open(&self, path: &str) -> std::result::Result<(), String>;
}

/// 丢弃打开请求（用于不需要真实打开的场合）。
pub struct NullOpener;

impl Opener for NullOpener {
    fn open(&self, _path: &str) -> std::result::Result<(), String> {
        Ok(())
    }
}

/// 真实打开器：交给系统默认程序。
pub struct SystemOpener;

impl Opener for SystemOpener {
    fn open(&self, path: &str) -> std::result::Result<(), String> {
        // 目录用文件管理器、文件用默认关联程序——两者在三个平台上都是同一条命令。
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut command = std::process::Command::new("explorer");
            command.arg(path);
            command
        };
        #[cfg(target_os = "macos")]
        let mut command = {
            let mut command = std::process::Command::new("open");
            command.arg(path);
            command
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let mut command = {
            let mut command = std::process::Command::new("xdg-open");
            command.arg(path);
            command
        };
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|error| format!("打开失败：{error}"))
    }
}

/// Shell 命令处理器。
pub struct ShellCommands {
    opener: std::sync::Arc<dyn Opener>,
}

impl ShellCommands {
    /// 构造。
    #[must_use]
    pub fn new(opener: std::sync::Arc<dyn Opener>) -> Self {
        Self { opener }
    }

    /// 路径是否存在。
    ///
    /// # Errors
    ///
    /// 参数非法或目标是 WSL 时返回错误。
    pub fn path_exists(&self, args: &Value) -> Result<Value> {
        let path = require_path(args)?;
        require_local(args)?;
        // 不做规范化：现网同样按原样判断，且 `..` 在这类检查里没有安全含义
        // （用户本来就能用系统文件管理器访问任何路径）。
        Ok(json!(Path::new(&path).exists()))
    }

    /// 用系统程序打开路径。
    ///
    /// # Errors
    ///
    /// 参数非法、目标是 WSL、路径不存在或打开失败时返回错误。
    pub fn open_path(&self, args: &Value) -> Result<Value> {
        let path = require_path(args)?;
        require_local(args)?;
        // 先校验：把不存在的路径交给系统会得到与用户操作对不上的英文弹窗。
        if !Path::new(&path).exists() {
            return Err(failed(format!("路径不存在：{path}")));
        }
        self.opener.open(&path).map_err(failed)?;
        Ok(Value::Null)
    }
}

/// 取 `path` 参数。
fn require_path(args: &Value) -> Result<String> {
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| invalid("path"))
}

/// 要求目标是本机。
///
/// 见模块文档：WSL 分支尚未接线，显式拒绝好过按本机路径误判。
fn require_local(args: &Value) -> Result<()> {
    let target_id = args
        .get("targetId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim();
    if target_id.is_empty() {
        return Ok(());
    }
    // 与 `scope_for_target` 一致：用 `ProviderId::wsl_distro_of` 判定，
    // 而不是自己再解析一遍字符串。
    let distro = ProviderId::from_target_id(target_id).wsl_distro_of(target_id);
    if !distro.is_empty() {
        return Err(failed("暂不支持 WSL 目标的路径操作。".to_owned()));
    }
    let _ = TargetKind::Local;
    Ok(())
}

/// 把路径规范化成绝对路径（供将来的 WSL 分支使用）。
///
/// 保留这个函数是为了让「本机路径原样、WSL 路径需要转换」这个差异在代码里可见。
#[must_use]
pub fn normalize_open_path(path: &str) -> PathBuf {
    PathBuf::from(path)
}

/// 构造一个「参数无效」错误。
fn invalid(name: &str) -> Error {
    Error::Ipc(IpcError::InvalidParameter {
        name: name.to_owned(),
    })
}

/// 把字符串错误包成命令面错误。
fn failed(message: String) -> Error {
    Error::Ipc(IpcError::CommandFailed { message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// 记录被打开过的路径。
    #[derive(Default)]
    struct Recording {
        opened: Mutex<Vec<String>>,
    }

    impl Recording {
        fn opened(&self) -> Vec<String> {
            self.opened.lock().expect("lock").clone()
        }
    }

    impl Opener for Recording {
        fn open(&self, path: &str) -> std::result::Result<(), String> {
            self.opened.lock().expect("lock").push(path.to_owned());
            Ok(())
        }
    }

    /// 总是失败的打开器。
    struct Failing;

    impl Opener for Failing {
        fn open(&self, _path: &str) -> std::result::Result<(), String> {
            Err("系统调用失败".to_owned())
        }
    }

    fn commands(opener: std::sync::Arc<dyn Opener>) -> ShellCommands {
        ShellCommands::new(opener)
    }

    #[test]
    fn an_existing_path_is_reported_as_existing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let value = commands
            .path_exists(&json!({ "path": dir.path().to_string_lossy() }))
            .expect("查询");
        assert_eq!(value, json!(true));
    }

    #[test]
    fn a_missing_path_is_reported_as_missing() {
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let value = commands
            .path_exists(&json!({ "path": "/aivc-definitely-missing-xyz" }))
            .expect("查询");
        assert_eq!(value, json!(false));
    }

    #[test]
    fn a_blank_path_is_rejected_by_name() {
        // 参数错误要指名道姓，否则调用方无从下手。
        let commands = commands(std::sync::Arc::new(Recording::default()));
        for args in [json!({}), json!({ "path": "" }), json!({ "path": "   " })] {
            let error = commands.path_exists(&args).expect_err("必须拒绝");
            assert_eq!(error.to_string(), "参数无效：path");
        }
    }

    #[test]
    fn opening_an_existing_path_delegates_to_the_opener() {
        let dir = tempfile::tempdir().expect("tempdir");
        let recording = std::sync::Arc::new(Recording::default());
        let commands = commands(recording.clone());
        commands
            .open_path(&json!({ "path": dir.path().to_string_lossy() }))
            .expect("打开");
        assert_eq!(
            recording.opened(),
            vec![dir.path().to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn opening_a_missing_path_is_refused_before_touching_the_system() {
        // 把不存在的路径交给系统会得到与用户操作对不上的英文弹窗。
        let recording = std::sync::Arc::new(Recording::default());
        let commands = commands(recording.clone());
        let error = commands
            .open_path(&json!({ "path": "/aivc-definitely-missing-xyz" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("路径不存在"), "{error}");
        assert!(recording.opened().is_empty(), "不该调用打开器");
    }

    #[test]
    fn an_opener_failure_is_reported_as_is() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Failing));
        let error = commands
            .open_path(&json!({ "path": dir.path().to_string_lossy() }))
            .expect_err("必须失败");
        assert!(error.to_string().contains("系统调用失败"), "{error}");
    }

    #[test]
    fn a_wsl_target_is_refused_explicitly() {
        // 按本机路径检查会把「WSL 里有」误判成「没有」——静默出错比报错更糟。
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let error = commands
            .path_exists(&json!({ "targetId": "wsl:Ubuntu", "path": "/home/u/x" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("暂不支持 WSL"), "{error}");
        // 带平台前缀的 WSL 目标同样拒绝。
        let error = commands
            .path_exists(&json!({ "targetId": "gemini:wsl:Ubuntu", "path": "/x" }))
            .expect_err("必须拒绝");
        assert!(error.to_string().contains("暂不支持 WSL"), "{error}");
    }

    #[test]
    fn a_local_target_id_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Recording::default()));
        let value = commands
            .path_exists(&json!({
                "targetId": "codex:local",
                "path": dir.path().to_string_lossy(),
            }))
            .expect("查询");
        assert_eq!(value, json!(true));
    }

    #[test]
    fn a_missing_target_id_is_treated_as_local() {
        // 现网用 `targetId || ""`，缺省即本机。
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(Recording::default()));
        assert_eq!(
            commands
                .path_exists(&json!({ "path": dir.path().to_string_lossy() }))
                .expect("查询"),
            json!(true)
        );
    }

    #[test]
    fn the_null_opener_succeeds_without_side_effects() {
        let dir = tempfile::tempdir().expect("tempdir");
        let commands = commands(std::sync::Arc::new(NullOpener));
        commands
            .open_path(&json!({ "path": dir.path().to_string_lossy() }))
            .expect("打开");
    }

    #[test]
    fn path_normalization_is_identity_for_local_paths() {
        // 本机路径原样使用；这个函数存在的意义是让「WSL 需要转换」这个差异可见。
        assert_eq!(normalize_open_path("/tmp/x"), PathBuf::from("/tmp/x"));
    }
}
