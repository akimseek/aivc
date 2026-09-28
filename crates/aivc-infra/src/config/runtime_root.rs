//! 运行根派生。
//!
//! 应用自身的数据（`data/`、`logs/`、SQLite、日志）**绝不写入系统用户目录**，
//! 而是全部从「运行根」派生。
//!
//! # 为什么不再用「当前工作目录」
//!
//! 早先开发构建的运行根取的是**当前工作目录**。这看起来方便，实际是错的：
//! 同一个可执行文件，从 `aivc-rs` 启动、从 `D:\code\ai` 启动、被探针从临时目录启动，
//! 会得到**三份互不相干的数据**（供应商、会话缓存、设置）。
//!
//! 真出过一次事：用户报「终端没走网关」，排查时**说不清供应商到底在哪一份数据里**，
//! 因为候选目录有三个，而其中一个已经被清理过。
//!
//! 现在运行根只看**可执行文件的位置**，与工作目录无关：
//!
//! | 形态 | 运行根 | 例 |
//! | --- | --- | --- |
//! | 开发构建（`debug_assertions`） | 编译期已知的**工作区根** | `aivc-rs/` |
//! | Windows/Linux 未打包目录 | 该目录的**父目录** | `release/` |
//! | 安装包 | 可执行文件所在目录 | `…/AIVC/` |
//!
//! 开发构建为什么不用「可执行文件的祖父目录」？那要靠「祖父目录恰好叫 `target`」
//! 这个约定去猜，而 `CARGO_TARGET_DIR` 一改就不成立。**编译期已知的路径**没有这个
//! 不确定性，而且它天然满足「数据独立于编译产物」——`cargo clean` 不会连数据一起删。
//!
//! 调用方给「开发根」时要注意层级：`CARGO_MANIFEST_DIR` 指向的是 **crate** 目录
//! （`<工作区>/crates/aivc-app`），因此要往上**两级**才是工作区根。
//! 只往上取一级会得到 `<工作区>/crates`——路径看着正常，位置却是错的（踩过）。
//!
//! 未打包形态回退到父目录，是为了让 `release/` 下的临时构建与最终安装包共用同一层
//! 数据目录，避免调试时反复重建会话缓存。
//!
//! # 覆盖：`AIVC_RUNTIME_ROOT`
//!
//! 设了这个环境变量就以它为准（见 [`overridden_root`]）。它的用途是**把数据挪开**：
//! 界面探针必须在一个临时目录里跑，否则它会碰用户真实的数据目录
//! （这一点也是事故换来的，见 `scripts/app-probe.ps1` 的说明）。
//!
//! # 已知限制（刻意保留现网行为）
//!
//! macOS 打包形态下运行根等于 `…/App.app/Contents/MacOS`，即数据目录落在 `.app`
//! 包内部。已签名的应用包不可写，这会失败。现网即如此，此处不擅自改变行为；
//! 若需修正，应作为独立决策并同步更新 parity 矩阵。

use std::path::{Path, PathBuf};

use aivc_core::target::Platform;

/// 覆盖运行根的环境变量名。
///
/// 设了它就以它的值为运行根，`is_packaged` 与可执行文件位置都不再参与判断。
pub const RUNTIME_ROOT_ENV: &str = "AIVC_RUNTIME_ROOT";

/// 运行根形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeShape {
    /// 开发构建：运行根为编译期已知的**工作区根**。
    ///
    /// 注意它**不是**工作目录——那正是这次改掉的东西。
    Development,
    /// 未打包的构建目录：运行根为其父目录。
    UnpackedBuild,
    /// 已安装：运行根为可执行文件所在目录。
    Installed,
    /// 由 [`RUNTIME_ROOT_ENV`] 显式指定。
    Overridden,
}

/// 运行根解析结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeRoot {
    /// 运行根绝对路径。
    pub root: PathBuf,
    /// 形态，便于诊断日志与测试断言。
    pub shape: RuntimeShape,
}

/// 读环境变量给出的运行根覆盖。
///
/// 空串与全空白视为「没设」——把空串当成路径会让运行根变成 `""`，
/// 那是比不设更难查的故障。
#[must_use]
pub fn overridden_root(value: Option<&str>) -> Option<RuntimeRoot> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(RuntimeRoot {
        root: PathBuf::from(trimmed),
        shape: RuntimeShape::Overridden,
    })
}

/// 解析运行根。
///
/// - `is_packaged`：是否已打包（等价于现网 `app.isPackaged`）。
/// - `executable_path`：可执行文件路径（等价于 `app.getPath("exe")`）。
/// - `development_root`：开发构建使用的根（由调用方从编译期常量算出，
///   见 `main.rs` 的 `CARGO_MANIFEST_DIR`）。**传工作目录是错的**。
/// - `platform`：宿主平台，仅用于判断未打包目录名。
///
/// 环境变量覆盖**不在这里**：那个判断放在 [`overridden_root`]，调用方先问它。
/// 这样本函数保持纯粹，测试不需要碰进程环境。
#[must_use]
pub fn resolve_runtime_storage_root(
    is_packaged: bool,
    executable_path: &Path,
    development_root: &Path,
    platform: Platform,
) -> RuntimeRoot {
    if !is_packaged {
        return RuntimeRoot {
            root: development_root.to_path_buf(),
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
    fn development_uses_the_workspace_root_not_the_working_directory() {
        // **这条是本模块存在的理由。**
        //
        // 传入的工作区根是 `D:/code/ai/aivc-rs`；函数签名里**根本没有 `cwd`**——
        // 于是「从哪儿启动」不再影响数据落点。此前开发构建取工作目录，
        // 同一台机器上跑出过三份互不相干的数据。
        let resolved = resolve_runtime_storage_root(
            false,
            &p("D:/code/ai/aivc-rs/target/debug/aivc-app.exe"),
            &p("D:/code/ai/aivc-rs"),
            Platform::Win32,
        );
        assert_eq!(resolved.root, p("D:/code/ai/aivc-rs"));
        assert_eq!(resolved.shape, RuntimeShape::Development);
    }

    #[test]
    fn development_root_does_not_depend_on_the_executable_location() {
        // 换一个 `CARGO_TARGET_DIR` 也只是换了个 exe 路径，数据落点不变。
        let a = resolve_runtime_storage_root(
            false,
            &p("D:/code/ai/aivc-rs/target/debug/aivc-app.exe"),
            &p("D:/code/ai/aivc-rs"),
            Platform::Win32,
        );
        let b = resolve_runtime_storage_root(
            false,
            &p("E:/builds/custom-target/debug/aivc-app.exe"),
            &p("D:/code/ai/aivc-rs"),
            Platform::Win32,
        );
        assert_eq!(a.root, b.root);
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
    fn the_development_root_is_ignored_once_packaged() {
        // 打包之后不该再看编译期那个路径——否则安装包会把数据写回开发机的源码目录。
        let resolved = resolve_runtime_storage_root(
            true,
            &p("C:/Program Files/AIVC/AIVC.exe"),
            &p("D:/code/ai/aivc-rs"),
            Platform::Win32,
        );
        assert_eq!(resolved.root, p("C:/Program Files/AIVC"));
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

    #[test]
    fn an_explicit_root_overrides_everything() {
        // 覆盖是探针隔离数据的手段，必须无条件生效。
        let overridden = overridden_root(Some("C:/tmp/aivc-probe-1")).expect("应当生效");
        assert_eq!(overridden.root, p("C:/tmp/aivc-probe-1"));
        assert_eq!(overridden.shape, RuntimeShape::Overridden);
    }

    #[test]
    fn a_blank_override_counts_as_not_set() {
        // 把空串当路径会让运行根变成 `""`——那比不设更难查。
        assert!(overridden_root(None).is_none());
        assert!(overridden_root(Some("")).is_none());
        assert!(overridden_root(Some("   ")).is_none());
    }

    #[test]
    fn an_override_is_trimmed() {
        // 环境变量里带尾随空格是常见的（尤其从脚本里设），不该变成路径的一部分。
        let overridden = overridden_root(Some("  C:/tmp/probe  ")).expect("应当生效");
        assert_eq!(overridden.root, p("C:/tmp/probe"));
    }
}
