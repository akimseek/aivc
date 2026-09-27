//! 让子进程找得到 npm 装的 CLI。
//!
//! # 为什么需要
//!
//! 从资源管理器启动时，应用继承的是**那一刻**的 PATH。之后再装的 Node / CLI 不在里面，
//! 而 Windows 上 `cmd.exe` 只认 PATH——表现为终端里一句「不是内部或外部命令」，
//! 或者一闪而过。
//!
//! 老平台的 `resolveWindowsCodexCommand` 处理的是同一件事，它把候选**解析成绝对路径**：
//! `where` → PATH → `%APPDATA%\npm` → `%LOCALAPPDATA%\npm` → `Program Files\nodejs` →
//! `%NVM_HOME%\*`；都找不到就抛出「请在新电脑安装 Codex CLI，并重启应用使 PATH 生效」。
//!
//! # 这里换了一种做法
//!
//! 不解析绝对路径，而是把上列目录**追加**到子进程的 PATH。三条理由：
//!
//! 1. **不引入误判。** `cmd.exe` 自己的查找规则还包括 `PATHEXT` 与当前目录，
//!    手工枚举必然覆盖不全；一旦我们判「没找到」而它其实找得到，终端就白起了。
//! 2. **一次覆盖四个平台。** 老平台只解析了 Codex，Gemini / Claude / Qoder 没有——
//!    而那三个同样可能是 npm 装的 `.cmd` shim。
//! 3. **顺序与老平台一致。** 这些目录排在 PATH **之后**，所以 PATH 里已有的仍优先；
//!    老平台那边也是先查 PATH 候选、再查这些目录。
//!
//! 这也是**有意偏离**老平台实现的一点，理由如上。

use std::collections::BTreeMap;

/// Windows PATH 的分隔符。
pub const SEPARATOR: char = ';';

/// 目录拼接用的分隔符（本模块只在 Windows 上有实际作用）。
const DIR_SEPARATOR: char = '\\';

/// 定位所需的全部环境输入。
///
/// 抽成结构体是为了让 [`fallback_dirs`] / [`appended_path`] 这两个纯函数能脱离真实
/// 环境穷举测试——真实环境里有没有装 nvm 是碰运气的事。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathEnv {
    /// 当前 PATH。
    pub path: String,
    /// `%APPDATA%`。
    pub app_data: Option<String>,
    /// `%LOCALAPPDATA%`。
    pub local_app_data: Option<String>,
    /// `%ProgramFiles%`。
    pub program_files: Option<String>,
    /// `%ProgramFiles(x86)%`。
    pub program_files_x86: Option<String>,
    /// `%NVM_HOME%`。
    pub nvm_home: Option<String>,
    /// `%NVM_HOME%` 下的子目录（nvm-windows 的各个版本目录）。
    pub nvm_versions: Vec<String>,
}

/// 需要补进 PATH 的目录，顺序固定。
///
/// 顺序与老平台一致：npm 全局目录 → Node 安装目录 → nvm 各版本目录。
#[must_use]
pub fn fallback_dirs(env: &PathEnv) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut push = |base: Option<&String>, leaf: &str| {
        let Some(base) = base.map(String::as_str).map(str::trim) else {
            return;
        };
        if base.is_empty() {
            return;
        }
        dirs.push(join_dir(base, leaf));
    };
    push(env.app_data.as_ref(), "npm");
    push(env.local_app_data.as_ref(), "npm");
    push(env.program_files.as_ref(), "nodejs");
    push(env.program_files_x86.as_ref(), "nodejs");
    if let Some(home) = env
        .nvm_home
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        for version in &env.nvm_versions {
            dirs.push(join_dir(home, version));
        }
    }
    dirs
}

/// 拼接一个目录（按 Windows 的 `\`，并避免重复分隔符）。
#[must_use]
pub fn join_dir(base: &str, leaf: &str) -> String {
    format!(
        "{}{DIR_SEPARATOR}{leaf}",
        base.trim_end_matches(DIR_SEPARATOR)
    )
}

/// 追加后的 PATH；**没有新目录可加时返回 [`None`]**。
///
/// 已经出现在 PATH 里的目录不会重复追加——重复项会让 PATH 随终端开关不断变长。
#[must_use]
pub fn appended_path(env: &PathEnv) -> Option<String> {
    let existing: Vec<&str> = env
        .path
        .split(SEPARATOR)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect();
    let additions: Vec<String> = fallback_dirs(env)
        .into_iter()
        .filter(|dir| {
            let wanted = dir.trim_end_matches(DIR_SEPARATOR);
            // 两侧都要去掉末尾分隔符：`c:\program files\nodejs\` 与
            // `C:\Program Files\nodejs` 是同一个目录，不加这一条就会重复追加。
            !existing.iter().any(|item| {
                item.trim_end_matches(DIR_SEPARATOR)
                    .eq_ignore_ascii_case(wanted)
            })
        })
        .collect();
    if additions.is_empty() {
        return None;
    }
    let mut parts: Vec<String> = existing.iter().map(|value| (*value).to_owned()).collect();
    parts.extend(additions);
    Some(parts.join(&SEPARATOR.to_string()))
}

/// 从当前进程环境读一份输入。
#[must_use]
pub fn process_env() -> PathEnv {
    let var = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    };
    let nvm_home = var("NVM_HOME");
    let nvm_versions = nvm_home.as_deref().map(sub_directories).unwrap_or_default();
    PathEnv {
        path: current_path(),
        app_data: var("APPDATA"),
        local_app_data: var("LOCALAPPDATA"),
        program_files: var("ProgramFiles"),
        program_files_x86: var("ProgramFiles(x86)"),
        nvm_home,
        nvm_versions,
    }
}

/// 当前进程的 PATH。
///
/// Windows 上变量名大小写不敏感，而系统里通常是 `Path`；`std::env::var` 两种都能取到。
#[must_use]
pub fn current_path() -> String {
    std::env::var("PATH").unwrap_or_default()
}

/// 把 npm / nvm 目录追加到 `env` 的 PATH。
///
/// 非 Windows 上是空操作：那边的 npm shim 是可执行脚本，shell 自己找得到。
pub fn extend_path(env: &mut BTreeMap<String, String>) {
    if !cfg!(windows) {
        return;
    }
    // `env` 里可能已经有调用方设好的 PATH（目前没有），以它为准。
    let mut search = process_env();
    if let Some(current) = env.get("PATH") {
        current.clone_into(&mut search.path);
    }
    let Some(combined) = appended_path(&search) else {
        return;
    };
    let before = search
        .path
        .split(SEPARATOR)
        .filter(|value| !value.trim().is_empty())
        .count();
    tracing::debug!(
        added = combined.split(SEPARATOR).count() - before,
        total = combined.split(SEPARATOR).count(),
        "已把 npm / nvm 目录补进子进程 PATH"
    );
    env.insert("PATH".to_owned(), combined);
}

/// 列出目录下的子目录名（已排序，保证 PATH 可复现）。
fn sub_directories(home: &str) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(home) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_with(path: &str) -> PathEnv {
        PathEnv {
            path: path.to_owned(),
            app_data: Some("C:\\Users\\u\\AppData\\Roaming".to_owned()),
            local_app_data: Some("C:\\Users\\u\\AppData\\Local".to_owned()),
            program_files: Some("C:\\Program Files".to_owned()),
            program_files_x86: Some("C:\\Program Files (x86)".to_owned()),
            nvm_home: Some("D:\\tools\\nvm".to_owned()),
            nvm_versions: vec!["v20.11.0".to_owned(), "v22.4.0".to_owned()],
        }
    }

    #[test]
    fn the_fallback_directories_match_the_old_platform() {
        // 顺序与老平台的候选顺序一致：npm → Node 安装目录 → nvm 各版本。
        assert_eq!(
            fallback_dirs(&env_with("C:\\Windows")),
            vec![
                "C:\\Users\\u\\AppData\\Roaming\\npm",
                "C:\\Users\\u\\AppData\\Local\\npm",
                "C:\\Program Files\\nodejs",
                "C:\\Program Files (x86)\\nodejs",
                "D:\\tools\\nvm\\v20.11.0",
                "D:\\tools\\nvm\\v22.4.0",
            ]
        );
    }

    #[test]
    fn missing_environment_entries_are_skipped() {
        let env = PathEnv {
            path: "C:\\Windows".to_owned(),
            ..PathEnv::default()
        };
        assert!(fallback_dirs(&env).is_empty());
        assert_eq!(appended_path(&env), None, "没有可加的目录时不该改 PATH");
    }

    #[test]
    fn blank_entries_are_not_turned_into_directories() {
        // 环境变量存在但是空白串时不能拼出 `\npm` 这种相对路径。
        let env = PathEnv {
            path: "C:\\Windows".to_owned(),
            app_data: Some("   ".to_owned()),
            ..PathEnv::default()
        };
        assert!(fallback_dirs(&env).is_empty());
    }

    #[test]
    fn the_original_path_keeps_priority() {
        // **顺序就是行为**：PATH 里已有的必须先被 `cmd.exe` 看到，否则会把用户
        // 特意装在前面的版本顶掉。
        let env = env_with("D:\\tools\\nvm\\nodejs;C:\\Windows");
        let combined = appended_path(&env).expect("有可加的目录");
        assert!(combined.starts_with("D:\\tools\\nvm\\nodejs;C:\\Windows;"));
        assert!(combined.ends_with("\\v22.4.0"));
    }

    #[test]
    fn directories_already_on_the_path_are_not_appended_again() {
        // 重复追加会让 PATH 随终端开关不断变长。
        let env = PathEnv {
            path: "C:\\Windows;c:\\program files\\nodejs\\".to_owned(),
            program_files: Some("C:\\Program Files".to_owned()),
            ..PathEnv::default()
        };
        assert_eq!(
            appended_path(&env),
            None,
            "大小写不同、末尾多一个分隔符，都应当认作同一个目录"
        );
    }

    #[test]
    fn only_the_new_directories_are_added() {
        let env = PathEnv {
            path: "C:\\Windows;D:\\tools\\nvm\\v20.11.0".to_owned(),
            nvm_home: Some("D:\\tools\\nvm".to_owned()),
            nvm_versions: vec!["v20.11.0".to_owned(), "v22.4.0".to_owned()],
            ..PathEnv::default()
        };
        assert_eq!(
            appended_path(&env),
            Some("C:\\Windows;D:\\tools\\nvm\\v20.11.0;D:\\tools\\nvm\\v22.4.0".to_owned())
        );
    }

    #[test]
    fn joining_does_not_double_the_separator() {
        assert_eq!(
            join_dir("C:\\Program Files\\", "nodejs"),
            "C:\\Program Files\\nodejs"
        );
        assert_eq!(
            join_dir("D:\\tools\\nvm", "v22.4.0"),
            "D:\\tools\\nvm\\v22.4.0"
        );
    }

    #[test]
    fn a_non_windows_build_leaves_the_environment_alone() {
        let mut env = BTreeMap::new();
        extend_path(&mut env);
        if cfg!(windows) {
            // Windows 上至少会读到真实环境；这里只断言它没有 panic、也没有写出空值。
            assert!(env.get("PATH").is_none_or(|value| !value.trim().is_empty()));
        } else {
            assert!(env.is_empty(), "非 Windows 上必须是空操作");
        }
    }
}
