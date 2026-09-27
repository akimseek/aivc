//! 运行根派生。
//!
//! 应用自身的数据（`data/`、`logs/`、SQLite、日志）**绝不写入系统用户目录**，
//! 而是全部从「运行根」派生。运行根有三种形态，必须逐字保持：
//!
//! | 形态 | 运行根 |
//! | --- | --- |
//! | 开发（设置 `VITE_DEV_SERVER_URL` 等价条件） | 当前工作目录 |
//! | Windows/Linux 未打包目录（`release/win-unpacked`、`linux-unpacked`） | 该目录的**父目录** |
//! | 安装包 | 可执行文件所在目录 |
//!
//! 未打包形态回退到父目录，是为了让 `release/` 下的临时构建与最终安装包使用同一层
//! 数据目录，避免调试时反复重建会话缓存。
//!
//! # 已知限制（刻意保留现网行为）
//!
//! macOS 打包形态下运行根等于 `…/App.app/Contents/MacOS`，即数据目录落在 `.app`
//! 包内部。已签名的应用包不可写，这会失败。现网即如此，此处不擅自改变行为；
//! 若需修正，应作为独立决策并同步更新 parity 矩阵。

use std::path::{Path, PathBuf};

use aivc_core::target::Platform;

/// 运行根形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeShape {
    /// 开发运行：运行根为工作目录。
    Development,
    /// 未打包的构建目录：运行根为其父目录。
    UnpackedBuild,
    /// 已安装：运行根为可执行文件所在目录。
    Installed,
}

/// 运行根解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRoot {
    /// 运行根绝对路径。
    pub root: PathBuf,
    /// 形态，便于诊断日志与测试断言。
    pub shape: RuntimeShape,
}

/// 解析运行根。
///
/// - `is_packaged`：是否已打包（等价于现网 `app.isPackaged`）。
/// - `executable_path`：可执行文件路径（等价于 `app.getPath("exe")`）。
/// - `cwd`：当前工作目录。
/// - `platform`：宿主平台，仅用于判断未打包目录名。
#[must_use]
pub fn resolve_runtime_storage_root(
    is_packaged: bool,
    executable_path: &Path,
    cwd: &Path,
    platform: Platform,
) -> RuntimeRoot {
    if !is_packaged {
        return RuntimeRoot {
            root: cwd.to_path_buf(),
            shape: RuntimeShape::Development,
        };
    }

    let executable_dir = executable_path
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

    if is_unpacked_build_directory(&executable_dir, platform) {
        let root = executable_dir
            .parent()
            .map_or_else(|| executable_dir.clone(), Path::to_path_buf);
        return RuntimeRoot {
            root,
            shape: RuntimeShape::UnpackedBuild,
        };
    }

    RuntimeRoot {
        root: executable_dir,
        shape: RuntimeShape::Installed,
    }
}

/// 目录名是否为未打包构建目录。
///
/// 只按目录名判断，不做文件系统访问，因此可跨平台确定性测试。
#[must_use]
pub fn is_unpacked_build_directory(directory: &Path, platform: Platform) -> bool {
    let Some(name) = directory.file_name().and_then(|value| value.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    match platform {
        Platform::Win32 => name == "win-unpacked",
        Platform::Linux => name == "linux-unpacked",
        // macOS 的打包产物是 `.app` 包，没有对应的 unpacked 目录约定。
        Platform::Darwin => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[test]
    fn development_uses_working_directory() {
        let resolved = resolve_runtime_storage_root(
            false,
            &p("D:/code/ai/node_modules/electron/dist/electron.exe"),
            &p("D:/code/ai"),
            Platform::Win32,
        );
        assert_eq!(resolved.root, p("D:/code/ai"));
        assert_eq!(resolved.shape, RuntimeShape::Development);
    }

    #[test]
    fn windows_unpacked_build_falls_back_to_parent_directory() {
        // release/win-unpacked 下的构建与最终安装包共用 release/ 这一层数据目录。
        let resolved = resolve_runtime_storage_root(
            true,
            &p("D:/code/ai/release/win-unpacked/App.exe"),
            &p("D:/code/ai"),
            Platform::Win32,
        );
        assert_eq!(resolved.root, p("D:/code/ai/release"));
        assert_eq!(resolved.shape, RuntimeShape::UnpackedBuild);
    }

    #[test]
    fn linux_unpacked_build_falls_back_to_parent_directory() {
        let resolved = resolve_runtime_storage_root(
            true,
            &p("/opt/app/release/linux-unpacked/app"),
            &p("/opt/app"),
            Platform::Linux,
        );
        assert_eq!(resolved.root, p("/opt/app/release"));
        assert_eq!(resolved.shape, RuntimeShape::UnpackedBuild);
    }

    #[test]
    fn installed_build_uses_executable_directory() {
        let resolved = resolve_runtime_storage_root(
            true,
            &p("C:/Program Files/AIVC/AIVC.exe"),
            &p("C:/somewhere/else"),
            Platform::Win32,
        );
        assert_eq!(resolved.root, p("C:/Program Files/AIVC"));
        assert_eq!(resolved.shape, RuntimeShape::Installed);
    }

    #[test]
    fn unpacked_directory_name_matching_is_case_insensitive() {
        assert!(is_unpacked_build_directory(
            &p("D:/release/WIN-UNPACKED"),
            Platform::Win32
        ));
        assert!(is_unpacked_build_directory(
            &p("/x/Linux-Unpacked"),
            Platform::Linux
        ));
    }

    #[test]
    fn unpacked_directory_matching_is_platform_specific() {
        // win-unpacked 在 Linux 上不是未打包目录名，反之亦然。
        assert!(!is_unpacked_build_directory(
            &p("/x/win-unpacked"),
            Platform::Linux
        ));
        assert!(!is_unpacked_build_directory(
            &p("/x/linux-unpacked"),
            Platform::Win32
        ));
        // macOS 没有 unpacked 约定。
        assert!(!is_unpacked_build_directory(
            &p("/x/win-unpacked"),
            Platform::Darwin
        ));
    }

    #[test]
    fn root_without_parent_is_handled_without_panicking() {
        // 防御性：路径没有父目录或文件名时不得 panic。
        let resolved = resolve_runtime_storage_root(true, &p("App.exe"), &p("."), Platform::Win32);
        assert_eq!(resolved.shape, RuntimeShape::Installed);
        assert!(!is_unpacked_build_directory(&p("/"), Platform::Linux));
    }
}
