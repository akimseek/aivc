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
//! # 不可写回退（2026-10-02 决策）
//!
//! 打包形态（[`RuntimeShape::Installed`] / [`RuntimeShape::UnpackedBuild`]）下运行根
//! 可能落在**不可写**的位置——per-machine MSI 装进 `C:\Program Files`、macOS 的
//! `.app` 包内部都是这样。实测（`docs/rust-rewrite/44-p8-packaging.md` §八）：
//! 普通用户启动即报「创建目录失败：…\data」，应用根本起不来。
//!
//! 现在启动时探测运行根可写性（[`root_is_writable`]），不可写且有用户级目录可用
//! （[`fallback_user_root`]：Windows `%LOCALAPPDATA%\aivc`、macOS
//! `~/Library/Application Support/aivc`、Linux `$XDG_DATA_HOME`（缺省
//! `~/.local/share`）`/aivc`）时回退过去（[`RuntimeShape::Fallback`]），并以
//! warn 日志同时记下两个路径。**开发根与显式覆盖不参与回退**：前者该大声失败
//! （那是开发机的配置问题），后者是探针的隔离手段，不许被静默挪走。

use std::path::{Path, PathBuf};

use aivc_core::target::Platform;

/// 覆盖运行根的环境变量名。
///
/// 设了它就以它的值为运行根，`is_packaged` 与可执行文件位置都不再参与判断。
pub const RUNTIME_ROOT_ENV: &str = "AIVC_RUNTIME_ROOT";

/// 回退目录名（用户数据目录下的子目录）。
///
/// 用 `aivc` 而不是产品名（`AI 可视化控制台`）：回退路径要跨终端与脚本好写，
/// 避免中文路径在旧编码工具链里的各种坑；`AIVC_` 前缀的环境变量已开了先例。
pub const FALLBACK_DIR_NAME: &str = "aivc";

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
    /// 打包形态的原始运行根不可写，已回退到用户级目录（见 [`effective_root`]）。
    Fallback,
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

/// 运行根可写性探测：**真的写一次**再删掉。
///
/// 不用「目录是否存在」「只读属性」这类间接判据——「目录已存在但不可写」
/// （Program Files 的典型状态）才是要抓的那种。探测文件写在根目录下，
/// 与 `data/`、`logs/` 同级；任何一步失败都算不可写。
///
/// 探测有副作用（在可写的根里瞬时创建一个文件），因此只在打包形态下使用
/// （见 [`effective_root`]：开发根与显式覆盖根本不走探测）。
#[must_use]
pub fn root_is_writable(root: &Path) -> bool {
    let probe = root.join(".aivc-write-probe");
    if std::fs::write(&probe, b"probe").is_err() {
        return false;
    }
    let _ = std::fs::remove_file(&probe);
    true
}

/// 打包形态运行根不可写时的用户级回退目录。
///
/// - Windows：`%LOCALAPPDATA%\aivc`（机器本地数据；与 NSIS per-user 安装落点同层）；
/// - macOS：`$HOME/Library/Application Support/aivc`；
/// - Linux：`$XDG_DATA_HOME/aivc`，未设时 `$HOME/.local/share/aivc`。
///
/// 环境变量经参数注入（与 [`overridden_root`] 同一理由）：测试不碰进程环境。
/// 空串与全空白视为「没设」。
#[must_use]
pub fn fallback_user_root(
    platform: Platform,
    env: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    let non_empty = |key: &str| env(key).filter(|value| !value.trim().is_empty());
    let base = match platform {
        Platform::Win32 => PathBuf::from(non_empty("LOCALAPPDATA")?),
        Platform::Darwin => PathBuf::from(non_empty("HOME")?).join("Library/Application Support"),
        Platform::Linux => match non_empty("XDG_DATA_HOME") {
            Some(xdg) => PathBuf::from(xdg),
            None => PathBuf::from(non_empty("HOME")?).join(".local/share"),
        },
    };
    Some(base.join(FALLBACK_DIR_NAME))
}

/// 打包形态下运行根不可写时的最终决策（纯函数，探测与回退目录都由参数注入）。
///
/// - **只处理打包形态**（`Installed` / `UnpackedBuild`）：开发根不可写该大声失败
///   （开发机的配置问题），显式覆盖是探针的隔离手段、不许被静默挪走——这两类
///   形态原样返回，**连探测都不做**；
/// - 可写 → 原样返回；
/// - 不可写且有回退目录 → `Fallback`；
/// - 不可写且拿不到回退目录（环境变量缺失）→ 原样返回，让「创建目录失败」如实浮出。
#[must_use]
pub fn effective_root(
    resolved: RuntimeRoot,
    is_writable: impl Fn(&Path) -> bool,
    fallback: Option<PathBuf>,
) -> RuntimeRoot {
    if !matches!(
        resolved.shape,
        RuntimeShape::Installed | RuntimeShape::UnpackedBuild
    ) {
        return resolved;
    }
    if is_writable(&resolved.root) {
        return resolved;
    }
    match fallback {
        Some(root) => RuntimeRoot {
            root,
            shape: RuntimeShape::Fallback,
        },
        None => resolved,
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

    // ── 不可写回退（2026-10-02）────────────────────────────

    /// 只认识这几个键的假环境。
    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let owned: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key: &str| owned.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    }

    #[test]
    fn fallback_on_windows_is_local_app_data() {
        let root = fallback_user_root(
            Platform::Win32,
            env_from(&[("LOCALAPPDATA", "C:/Users/x/AppData/Local")]),
        )
        .expect("应当有回退目录");
        assert_eq!(root, p("C:/Users/x/AppData/Local/aivc"));
    }

    #[test]
    fn fallback_on_macos_is_application_support() {
        let root = fallback_user_root(Platform::Darwin, env_from(&[("HOME", "/Users/x")]))
            .expect("应当有回退目录");
        assert_eq!(root, p("/Users/x/Library/Application Support/aivc"));
    }

    #[test]
    fn fallback_on_linux_prefers_xdg_then_home() {
        let xdg = fallback_user_root(
            Platform::Linux,
            env_from(&[("XDG_DATA_HOME", "/home/x/.data"), ("HOME", "/home/x")]),
        )
        .expect("应当有回退目录");
        assert_eq!(xdg, p("/home/x/.data/aivc"));

        let home = fallback_user_root(Platform::Linux, env_from(&[("HOME", "/home/x")]))
            .expect("应当有回退目录");
        assert_eq!(home, p("/home/x/.local/share/aivc"));

        // XDG_DATA_HOME 是空串时按「没设」处理（空串当路径会得到比不设更难查的故障）。
        let blank = fallback_user_root(
            Platform::Linux,
            env_from(&[("XDG_DATA_HOME", "  "), ("HOME", "/home/x")]),
        )
        .expect("应当有回退目录");
        assert_eq!(blank, p("/home/x/.local/share/aivc"));
    }

    #[test]
    fn fallback_is_none_without_environment() {
        assert!(fallback_user_root(Platform::Win32, env_from(&[])).is_none());
        assert!(fallback_user_root(Platform::Darwin, env_from(&[])).is_none());
        assert!(fallback_user_root(Platform::Linux, env_from(&[])).is_none());
    }

    #[test]
    fn an_unwritable_installed_root_falls_back() {
        // MSI 装进 Program Files 的场景：探测为不可写 → 回退到用户目录。
        let resolved = RuntimeRoot {
            root: p("C:/Program Files/AIVC"),
            shape: RuntimeShape::Installed,
        };
        let effective = effective_root(
            resolved,
            |_| false,
            Some(p("C:/Users/x/AppData/Local/aivc")),
        );
        assert_eq!(effective.shape, RuntimeShape::Fallback);
        assert_eq!(effective.root, p("C:/Users/x/AppData/Local/aivc"));
    }

    #[test]
    fn a_writable_root_is_kept_as_is() {
        // NSIS per-user 的场景：可写 → 原样，不回退。
        let resolved = RuntimeRoot {
            root: p("C:/Users/x/AppData/Local/AIVC"),
            shape: RuntimeShape::Installed,
        };
        let effective = effective_root(resolved.clone(), |_| true, Some(p("X:/unused")));
        assert_eq!(effective, resolved);
    }

    #[test]
    fn without_a_fallback_directory_the_original_root_is_kept() {
        // 环境变量缺失时原样返回——让「创建目录失败」如实浮出，而不是挪到一个
        // 凭空拼出来的路径上。
        let resolved = RuntimeRoot {
            root: p("C:/Program Files/AIVC"),
            shape: RuntimeShape::Installed,
        };
        let effective = effective_root(resolved.clone(), |_| false, None);
        assert_eq!(effective, resolved);
    }

    #[test]
    fn development_and_overridden_roots_are_neither_probed_nor_moved() {
        // 判别力：探测闭包一旦被调用就 panic——实现若把形态闸门去掉，此用例红。
        let development = RuntimeRoot {
            root: p("D:/code/ai/aivc-rs"),
            shape: RuntimeShape::Development,
        };
        let kept = effective_root(
            development.clone(),
            |_| panic!("开发形态不该被探测"),
            Some(p("X:/should/not/be/used")),
        );
        assert_eq!(kept, development);

        let overridden = RuntimeRoot {
            root: p("C:/tmp/aivc-probe-1"),
            shape: RuntimeShape::Overridden,
        };
        let kept = effective_root(
            overridden.clone(),
            |_| panic!("显式覆盖不该被探测"),
            Some(p("X:/should/not/be/used")),
        );
        assert_eq!(kept, overridden);
    }

    #[test]
    fn root_is_writable_says_yes_for_a_writable_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(root_is_writable(dir.path()));
        // 探测文件不留痕。
        assert!(!dir.path().join(".aivc-write-probe").exists());
    }

    #[test]
    fn root_is_writable_says_no_when_the_root_is_a_file() {
        // 便携模拟「不可写」：根路径其实是个文件，写入必然失败。
        // （真实的权限拒绝在测试里没法可移植地造，交给装机实测——见 44 号 §八。）
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("blocked");
        std::fs::write(&file, b"x").expect("写文件");
        assert!(!root_is_writable(&file));
    }
}
