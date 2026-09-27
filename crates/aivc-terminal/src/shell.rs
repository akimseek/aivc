//! 系统终端：解析要用哪个 shell，并拼出可执行的命令。
//!
//! # 与 Codex 终端的区别
//!
//! Codex 终端跑的是固定的一套（`codex ...`，带网关路由与配置注入）。系统终端跑的
//! 是**用户本来就在用的那个 shell**——因此这里只做「选哪个可执行文件、带什么参数」，
//! 不碰配置、不碰路由。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use aivc_core::error::{IpcError, Result, TerminalError};

use crate::command::TerminalCommand;

/// 系统终端的种类。
///
/// 与前端 `SystemTerminalKind` 逐字对应（`auto` / `powershell` / `cmd` / `git-bash` /
/// `wsl` / `wsl-default` / `bash` / `zsh` / `fish`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    /// 按平台选默认值。
    Auto,
    /// Windows 命令提示符。
    Cmd,
    /// Windows PowerShell。
    PowerShell,
    /// Git for Windows 自带的 bash。
    GitBash,
    /// 指定发行版的 WSL。
    Wsl,
    /// 默认发行版的 WSL。
    WslDefault,
    /// POSIX bash。
    Bash,
    /// zsh。
    Zsh,
    /// fish。
    Fish,
}

impl ShellKind {
    /// 由前端的字符串解析。
    ///
    /// # Errors
    ///
    /// 未知种类时返回错误——**不要静默回退到 auto**：用户明确选了 PowerShell 却拿到
    /// cmd，会以为是程序出错。
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "auto" => Ok(Self::Auto),
            "powershell" => Ok(Self::PowerShell),
            "cmd" => Ok(Self::Cmd),
            "git-bash" => Ok(Self::GitBash),
            "wsl" => Ok(Self::Wsl),
            "wsl-default" => Ok(Self::WslDefault),
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "fish" => Ok(Self::Fish),
            other => Err(IpcError::InvalidValue {
                message: format!("未知的系统终端类型：{other}"),
            }
            .into()),
        }
    }

    /// 协议里的名字。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Cmd => "cmd",
            Self::PowerShell => "powershell",
            Self::GitBash => "git-bash",
            Self::Wsl => "wsl",
            Self::WslDefault => "wsl-default",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
        }
    }
}

/// 选中了一个 shell，但还没找它的可执行文件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellChoice {
    /// 可执行文件名（不含路径）。
    pub program: &'static str,
    /// 固定参数。
    pub args: Vec<String>,
    /// 是否需要在 PATH 里找它（`wsl.exe`、`cmd.exe` 这类系统自带的不用找）。
    pub search_path: bool,
}

/// 决定用哪个 shell。
///
/// **纯函数**：不碰文件系统，因此可以直接断言「Windows 上 auto 选 PowerShell」
/// 这类规则，不用先在某台机器上装齐所有 shell。
///
/// `distro` 只在 `Wsl` 下有意义；`WslDefault` 会忽略它。
#[must_use]
pub fn resolve_shell(kind: ShellKind, is_windows: bool, distro: Option<&str>) -> ShellChoice {
    match kind {
        // Windows 上 auto 选 PowerShell：它比 cmd 好用得多，且每台 Windows 都有。
        ShellKind::Auto if is_windows => ShellChoice {
            program: "powershell.exe",
            args: vec!["-NoLogo".to_owned()],
            search_path: false,
        },
        ShellKind::Auto => ShellChoice {
            program: "bash",
            args: Vec::new(),
            search_path: true,
        },
        ShellKind::Cmd => ShellChoice {
            program: "cmd.exe",
            args: Vec::new(),
            search_path: false,
        },
        ShellKind::PowerShell => ShellChoice {
            program: "powershell.exe",
            args: vec!["-NoLogo".to_owned()],
            search_path: false,
        },
        ShellKind::GitBash => ShellChoice {
            program: "bash.exe",
            args: vec!["--login".to_owned(), "-i".to_owned()],
            search_path: true,
        },
        ShellKind::Wsl => ShellChoice {
            program: "wsl.exe",
            args: distro
                .filter(|value| !value.trim().is_empty())
                .map(|value| vec!["-d".to_owned(), value.to_owned()])
                .unwrap_or_default(),
            search_path: false,
        },
        ShellKind::WslDefault => ShellChoice {
            program: "wsl.exe",
            args: Vec::new(),
            search_path: false,
        },
        ShellKind::Bash => ShellChoice {
            program: "bash",
            args: vec!["-l".to_owned()],
            search_path: true,
        },
        ShellKind::Zsh => ShellChoice {
            program: "zsh",
            args: vec!["-l".to_owned()],
            search_path: true,
        },
        ShellKind::Fish => ShellChoice {
            program: "fish",
            args: vec!["-l".to_owned()],
            search_path: true,
        },
    }
}

/// 目标是 WSL 目标时，`auto` 该换成 [`ShellKind::Wsl`]。
///
/// [`resolve_shell`] 对 `Auto` 只按**平台**选（Windows → PowerShell），它不知道
/// 「这个终端属于哪个目标」。而用户在一个 WSL 目标上开终端，要的显然是那个发行版里的
/// shell——在本机开一个 PowerShell 是答非所问。
///
/// 本函数只加这一条规则，不含平台判断：平台那一层由 [`resolve_shell`] 负责，
/// 两处都判会让「到底谁说了算」变得难查。
#[must_use]
pub fn auto_kind_for(distro: Option<&str>) -> ShellKind {
    if distro.is_some() {
        ShellKind::Wsl
    } else {
        ShellKind::Auto
    }
}

/// 在 PATH 里找一个可执行文件。
///
/// 找不到时返回 `None`，由调用方决定是报错还是回退——**本函数不猜**。
#[must_use]
pub fn find_on_path(program: &str, path_env: &str) -> Option<PathBuf> {
    for directory in path_env.split([';', ':']) {
        let directory = directory.trim();
        if directory.is_empty() {
            continue;
        }
        let candidate = Path::new(directory).join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Git for Windows 自带的 bash 的常见位置。
///
/// 之所以要单独找：Git Bash 通常**不在 PATH 里**（安装器默认只加 `cmd` 目录），
/// 而它恰恰是这个应用里最常用的系统终端之一。
#[must_use]
pub fn git_bash_candidates(
    program_files: Option<&str>,
    local_app_data: Option<&str>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(root) = program_files {
        candidates.push(Path::new(root).join("Git").join("bin").join("bash.exe"));
    }
    if let Some(root) = local_app_data {
        candidates.push(
            Path::new(root)
                .join("Programs")
                .join("Git")
                .join("bin")
                .join("bash.exe"),
        );
    }
    candidates
}

/// 拼出系统终端要跑的命令。
///
/// `program` 是已经定位好的可执行文件（可能带路径）；传 `None` 表示没找到，
/// 此时**如实报错**而不是退化成别的 shell——用户选的是它。
///
/// # Errors
///
/// 指定的 shell 在本机找不到时返回错误。
pub fn build_system_shell_command(
    kind: ShellKind,
    program_override: Option<PathBuf>,
    cwd: &str,
    distro: Option<&str>,
    is_windows: bool,
) -> Result<TerminalCommand> {
    let choice = resolve_shell(kind, is_windows, distro);
    let file = match program_override {
        Some(path) => path.to_string_lossy().into_owned(),
        None if choice.search_path => {
            // Git Bash 单独给一句更准的话：它通常不在 PATH 里（安装器默认不加），
            // 「未找到 bash.exe」对用户没什么指导意义。
            return Err(if kind == ShellKind::GitBash {
                TerminalError::GitBashNotFound.into()
            } else {
                TerminalError::SpawnFailed {
                    message: format!("本机找不到 {}（{}）", choice.program, kind.as_str()),
                }
                .into()
            });
        }
        None => choice.program.to_owned(),
    };
    Ok(TerminalCommand {
        file,
        args: choice.args,
        cwd: cwd.to_owned(),
        // 系统终端**不注入任何环境**：它就该是用户平时那个 shell。
        env: BTreeMap::new(),
        remove_env: BTreeSet::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_picks_powershell_on_windows_and_bash_elsewhere() {
        assert_eq!(
            resolve_shell(ShellKind::Auto, true, None).program,
            "powershell.exe"
        );
        assert_eq!(resolve_shell(ShellKind::Auto, false, None).program, "bash");
    }

    #[test]
    fn wsl_carries_the_distro_only_when_given() {
        let with = resolve_shell(ShellKind::Wsl, true, Some("Ubuntu-24.04"));
        assert_eq!(with.program, "wsl.exe");
        assert_eq!(with.args, vec!["-d".to_owned(), "Ubuntu-24.04".to_owned()]);

        // 默认发行版：不加 `-d`。加一个空串会让 wsl.exe 报错。
        let without = resolve_shell(ShellKind::Wsl, true, Some("  "));
        assert!(without.args.is_empty());
    }

    #[test]
    fn an_unknown_kind_is_rejected_instead_of_silently_falling_back() {
        // 静默回退到 auto 会让「我明明选了 PowerShell」变成查出半天的怪事。
        assert!(ShellKind::parse("nushell").is_err());
        assert_eq!(
            ShellKind::parse("git-bash").expect("git-bash"),
            ShellKind::GitBash
        );
    }

    #[test]
    fn a_missing_shell_is_reported_rather_than_substituted() {
        // zsh 在多数 Windows 上不存在。这时必须报错——退化成 bash 会让用户以为
        // 自己启动的是 zsh，而配置文件全都没生效。
        let outcome = build_system_shell_command(ShellKind::Zsh, None, "/tmp", None, false);
        assert!(outcome.is_err());
    }

    #[test]
    fn a_found_shell_is_used_verbatim() {
        let command = build_system_shell_command(
            ShellKind::GitBash,
            Some(PathBuf::from("C:/Program Files/Git/bin/bash.exe")),
            "/work",
            None,
            true,
        )
        .expect("命令");
        assert_eq!(command.file, "C:/Program Files/Git/bin/bash.exe");
        assert_eq!(command.args, vec!["--login".to_owned(), "-i".to_owned()]);
        assert_eq!(command.cwd, "/work");
        assert!(command.env.is_empty(), "系统终端不注入环境");
    }

    #[test]
    fn path_lookup_ignores_empty_entries() {
        // PATH 里空条目在 Windows 上表示「当前目录」，去那里找可执行文件是危险的。
        assert!(find_on_path("definitely-not-a-real-program", ";;;").is_none());
    }

    #[test]
    fn a_wsl_target_makes_auto_mean_wsl() {
        // 目标带发行版时 `auto` 要变成 `wsl`；不带就交回给平台判断。
        assert_eq!(auto_kind_for(Some("Ubuntu")), ShellKind::Wsl);
        assert_eq!(auto_kind_for(None), ShellKind::Auto);
    }
}
