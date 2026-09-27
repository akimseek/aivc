//! 路径处理：POSIX 归一化、目录边界判定、WSL 挂载路径转换。
//!
//! # 为什么自己实现 POSIX 归一化
//!
//! [`is_inside_posix_dir`] 是**安全边界**（拒绝 `..` 逃逸与目录前缀混淆）。Rust 标准库
//! 没有 `normalize`，而 `std::path` 在本机（Windows）用的是 Win32 语义，与 WSL 内的
//! POSIX 语义不同——用错会让 `/a/sessions-evil` 被误判为位于 `/a/sessions` 之内。
//!
//! 因此这里逐字移植 Node `path.posix.normalize` 的算法（含 `allowAboveRoot` 的处理），
//! 并用 Node 生成的黄金数据对拍。

/// 判断字符是否为路径分隔符（POSIX）。
fn is_separator(value: char) -> bool {
    value == '/'
}

/// 移植自 Node `path.posix.normalize`。
///
/// 行为要点（与 Node 一致）：
///
/// - 空串 → `.`
/// - 折叠重复斜杠；保留单个尾随斜杠
/// - 以 `/` 开头时绝对化，`..` 不会越过根
/// - 非绝对路径保留前导 `..`
#[must_use]
pub fn posix_normalize(value: &str) -> String {
    if value.is_empty() {
        return ".".to_owned();
    }
    let is_absolute = value.starts_with('/');
    let trailing_separator = value.ends_with('/');

    let mut normalized = normalize_string(value, !is_absolute);
    if normalized.is_empty() {
        if is_absolute {
            return "/".to_owned();
        }
        return if trailing_separator {
            "./".to_owned()
        } else {
            ".".to_owned()
        };
    }
    if trailing_separator {
        normalized.push('/');
    }
    if is_absolute {
        format!("/{normalized}")
    } else {
        normalized
    }
}

/// 移植自 Node `path.posix` 内部的 `normalizeString`。
///
/// `allow_above_root` 为假时（绝对路径）`..` 会被丢弃而不是保留。
fn normalize_string(value: &str, allow_above_root: bool) -> String {
    let characters: Vec<char> = value.chars().collect();
    let mut result = String::new();
    let mut last_segment_length = 0usize;
    let mut last_slash: isize = -1;
    let mut dots: isize = 0;

    // 多迭代一轮（i == len）以便处理结尾段。
    for index in 0..=characters.len() {
        let code = if index < characters.len() {
            characters[index]
        } else if characters.last().copied().is_some_and(is_separator) {
            break;
        } else {
            '/'
        };

        if is_separator(code) {
            if last_slash == index.cast_signed() - 1 || dots == 1 {
                // 重复分隔符或单点段：忽略。
            } else if dots == 2 {
                let ends_with_dot_dot =
                    result.len() >= 2 && last_segment_length == 2 && result.ends_with("..");
                if !ends_with_dot_dot {
                    if result.len() > 2 {
                        if let Some(position) = result.rfind('/') {
                            result.truncate(position);
                            last_segment_length = position;
                        } else {
                            result.clear();
                            last_segment_length = 0;
                        }
                        last_slash = index.cast_signed();
                        dots = 0;
                        continue;
                    } else if result.len() == 2 || result.len() == 1 {
                        result.clear();
                        last_segment_length = 0;
                        last_slash = index.cast_signed();
                        dots = 0;
                        continue;
                    }
                }
                if allow_above_root {
                    if result.is_empty() {
                        result.push_str("..");
                    } else {
                        result.push_str("/..");
                    }
                    last_segment_length = 2;
                }
            } else {
                let segment: String = characters
                    .get((last_slash + 1).cast_unsigned()..index)
                    .unwrap_or_default()
                    .iter()
                    .collect();
                if result.is_empty() {
                    result.push_str(&segment);
                } else {
                    result.push('/');
                    result.push_str(&segment);
                }
                last_segment_length = segment.chars().count();
            }
            last_slash = index.cast_signed();
            dots = 0;
        } else if code == '.' && dots != -1 {
            dots += 1;
        } else {
            dots = -1;
        }
    }

    result
}

/// 判断 `file_path` 是否**严格位于**（或等于）`dir_path` 之内（POSIX 语义）。
///
/// 先归一化再判断，因此可同时挫败两类攻击：
///
/// - `..` 逃逸：`/a/sessions/../../etc/passwd`；
/// - 目录前缀混淆：`/a/sessions-evil/x` 不属于 `/a/sessions`。
#[must_use]
pub fn is_inside_posix_dir(file_path: &str, dir_path: &str) -> bool {
    let normalized_file = posix_normalize(file_path);
    let normalized_dir = posix_normalize(dir_path);
    let trimmed_dir = normalized_dir.trim_end_matches('/');
    if trimmed_dir.is_empty() {
        // 根目录：归一化后为 `/`，去掉尾随斜杠会变成空串。任何绝对路径都在根之下。
        // 必须单独处理，否则拼出的前缀会变成 `//`，永远匹配不上。
        return normalized_file.starts_with('/');
    }
    normalized_file == trimmed_dir || normalized_file.starts_with(&format!("{trimmed_dir}/"))
}

/// 判断 `file_path` 是否位于 `dir_path` 之内（本机 / Win32 语义）。
///
/// 与 [`is_inside_posix_dir`] 的区别：本机路径大小写不敏感、分隔符可能是 `\`。
#[must_use]
pub fn is_inside_local_path(file_path: &str, dir_path: &str) -> bool {
    let file = std::path::Path::new(file_path);
    let dir = std::path::Path::new(dir_path);
    // 用绝对化后的路径做相对计算，避免相对路径与 `..` 绕过约束。
    let Ok(relative) = path_relative(&absolute(dir), &absolute(file)) else {
        return false;
    };
    relative.is_empty()
        || (!relative.starts_with("..") && !std::path::Path::new(&relative).is_absolute())
}

/// 把路径转为绝对路径（不做文件系统访问）。
fn absolute(path: &std::path::Path) -> std::path::PathBuf {
    if path.is_absolute() {
        // 仍要消掉 `.` 与 `..`，否则 `C:\a\..\b` 不会被识别为 `C:\b`。
        return std::path::PathBuf::from(normalize_local_lexically(&path.to_string_lossy()));
    }
    let base = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    std::path::PathBuf::from(normalize_local_lexically(
        &base.join(path).to_string_lossy(),
    ))
}

/// 词法归一化本机路径（不访问文件系统，不解析符号链接）。
fn normalize_local_lexically(value: &str) -> String {
    use std::path::Component;
    let path = std::path::Path::new(value);
    let mut result = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // 根之上无法再退，直接丢弃。
                if !result.pop() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result.to_string_lossy().into_owned()
}

/// 计算 `path` 相对 `base` 的词法相对路径。
fn path_relative(base: &std::path::Path, path: &std::path::Path) -> Result<String, ()> {
    let base_components: Vec<_> = base.components().collect();
    let path_components: Vec<_> = path.components().collect();

    let common = base_components
        .iter()
        .zip(path_components.iter())
        .take_while(|(left, right)| left == right)
        .count();

    // 完全不相交（例如不同盘符）时无法表达相对路径。
    if common == 0 && (base_components.is_empty() || path_components.is_empty()) {
        return Err(());
    }

    let mut parts: Vec<String> = Vec::new();
    for _ in common..base_components.len() {
        parts.push("..".to_owned());
    }
    for component in &path_components[common..] {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    Ok(parts.join(std::path::MAIN_SEPARATOR_STR))
}

/// 把 WSL 的 `/mnt/<盘符>/...` 挂载路径转换为 Windows 原生路径；非挂载路径原样返回。
#[must_use]
pub fn wsl_mount_path_to_windows_path(file_path: &str) -> String {
    let Some(rest) = file_path.strip_prefix("/mnt/") else {
        return file_path.to_owned();
    };
    let mut parts = rest.splitn(2, '/');
    let Some(drive) = parts.next() else {
        return file_path.to_owned();
    };
    // 盘符必须是单个 ASCII 字母。
    if drive.len() != 1
        || !drive
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return file_path.to_owned();
    }
    let drive = drive.to_ascii_uppercase();
    let remainder = parts.next().unwrap_or_default().replace('/', "\\");
    if remainder.is_empty() {
        format!("{drive}:\\")
    } else {
        format!("{drive}:\\{remainder}")
    }
}

/// 把 WSL 发行版名净化为可安全用作文件名的形式。
///
/// 阻断借 distro 名注入路径分隔符（`Ubuntu/../../etc`）。
#[must_use]
pub fn sanitize_wsl_distro(distro: &str) -> String {
    distro
        .chars()
        .map(|value| {
            if value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '-') {
                value
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_normalize_matches_node_behaviour() {
        // 期望值由 Node `path.posix.normalize` 生成。
        for (input, expected) in [
            ("", "."),
            (".", "."),
            ("..", ".."),
            ("/", "/"),
            ("//", "/"),
            ("///a", "/a"),
            ("a/b/c", "a/b/c"),
            ("a//b", "a/b"),
            ("a/b/", "a/b/"),
            ("a/b/.", "a/b"),
            ("a/b/..", "a"),
            ("a/b/../..", "."),
            ("a/b/../../..", ".."),
            ("/a/../../b", "/b"),
            ("/..", "/"),
            ("/../..", "/"),
            ("./a", "a"),
            ("../a", "../a"),
            ("a/./b", "a/b"),
            ("a/../b", "b"),
            ("/a/sessions/", "/a/sessions/"),
            ("/a/sessions/../sessions-evil", "/a/sessions-evil"),
            ("a/b/../../c", "c"),
            ("a/../../c", "../c"),
        ] {
            assert_eq!(posix_normalize(input), expected, "输入 {input:?}");
        }
    }

    #[test]
    fn inside_posix_dir_accepts_children_and_the_directory_itself() {
        assert!(is_inside_posix_dir("/a/sessions/x.jsonl", "/a/sessions"));
        assert!(is_inside_posix_dir(
            "/a/sessions/sub/x.jsonl",
            "/a/sessions"
        ));
        assert!(is_inside_posix_dir("/a/sessions", "/a/sessions"));
        assert!(is_inside_posix_dir("/a/sessions/", "/a/sessions"));
    }

    #[test]
    fn inside_posix_dir_rejects_traversal_escapes() {
        // 这是安全边界的核心：任何 `..` 逃逸都必须被拒绝。
        assert!(!is_inside_posix_dir(
            "/a/sessions/../../etc/passwd",
            "/a/sessions"
        ));
        assert!(!is_inside_posix_dir(
            "/a/sessions/../other/x",
            "/a/sessions"
        ));
        assert!(!is_inside_posix_dir("/etc/passwd", "/a/sessions"));
        assert!(!is_inside_posix_dir("/a", "/a/sessions"));
    }

    #[test]
    fn inside_posix_dir_rejects_directory_prefix_confusion() {
        // `/a/sessions-evil` 不是 `/a/sessions` 的子目录。
        assert!(!is_inside_posix_dir(
            "/a/sessions-evil/x.jsonl",
            "/a/sessions"
        ));
        assert!(!is_inside_posix_dir("/a/sessions2", "/a/sessions"));
    }

    #[test]
    fn inside_posix_dir_handles_the_root_directory() {
        assert!(is_inside_posix_dir("/etc/passwd", "/"));
        assert!(is_inside_posix_dir("/", "/"));
    }

    #[test]
    fn wsl_mount_paths_convert_to_windows_drive_paths() {
        assert_eq!(
            wsl_mount_path_to_windows_path("/mnt/c/Users/akim/x.jsonl"),
            "C:\\Users\\akim\\x.jsonl"
        );
        assert_eq!(wsl_mount_path_to_windows_path("/mnt/d"), "D:\\");
        assert_eq!(wsl_mount_path_to_windows_path("/mnt/e/"), "E:\\");
        // 盘符大小写归一为大写。
        assert_eq!(wsl_mount_path_to_windows_path("/mnt/z/a"), "Z:\\a");
    }

    #[test]
    fn non_mount_paths_are_returned_unchanged() {
        for value in [
            "/home/akim/.codex/sessions",
            "/mnt/",
            "/mnt/cc/x",
            "/mnt/1/x",
            "relative/path",
            "",
            "C:\\Users\\akim",
        ] {
            assert_eq!(
                wsl_mount_path_to_windows_path(value),
                value,
                "输入 {value:?}"
            );
        }
    }

    #[test]
    fn distro_names_are_sanitized_against_path_injection() {
        assert_eq!(sanitize_wsl_distro("Ubuntu-22.04"), "Ubuntu-22.04");
        assert_eq!(sanitize_wsl_distro("Ubuntu_20.04"), "Ubuntu_20.04");
        // 路径分隔符与 `..` 都必须被净化。
        assert_eq!(sanitize_wsl_distro("Ubuntu/../../etc"), "Ubuntu_.._.._etc");
        assert_eq!(sanitize_wsl_distro("a\\b"), "a_b");
        assert_eq!(sanitize_wsl_distro("with space"), "with_space");
        assert_eq!(sanitize_wsl_distro("中文发行版"), "_____"); // 5 个字符 -> 5 个下划线
    }

    #[test]
    fn local_path_containment_rejects_traversal() {
        let root = if cfg!(windows) {
            "C:\\work\\sessions"
        } else {
            "/work/sessions"
        };
        let child = if cfg!(windows) {
            "C:\\work\\sessions\\a.jsonl"
        } else {
            "/work/sessions/a.jsonl"
        };
        assert!(is_inside_local_path(child, root));
        assert!(is_inside_local_path(root, root));

        let escape = if cfg!(windows) {
            "C:\\work\\sessions\\..\\..\\Windows\\System32"
        } else {
            "/work/sessions/../../etc/passwd"
        };
        assert!(!is_inside_local_path(escape, root));

        // 目录前缀混淆。
        let sibling = if cfg!(windows) {
            "C:\\work\\sessions-evil\\a.jsonl"
        } else {
            "/work/sessions-evil/a.jsonl"
        };
        assert!(!is_inside_local_path(sibling, root));
    }

    #[test]
    fn local_path_normalization_removes_dot_segments() {
        assert_eq!(normalize_local_lexically("C:\\a\\.\\b"), "C:\\a\\b");
        assert_eq!(normalize_local_lexically("C:\\a\\..\\b"), "C:\\b");
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// 归一化结果中不得残留 `.` 或 `..` 段（除非是合法的前导 `..`）。
        #[test]
        fn normalize_never_leaves_interior_dot_segments(value in ". /a\\._-{0,24}") {
            let normalized = posix_normalize(&value);
            let segments: Vec<&str> = normalized.split('/').collect();
            for (index, segment) in segments.iter().enumerate() {
                if *segment == ".." {
                    // 只允许出现在最前面（相对路径向上引用）。
                    prop_assert!(index == 0, "`..` 出现在中间：{normalized}");
                }
                if *segment == "." {
                    prop_assert!(index == 0 && segments.len() == 1, "残留 `.` 段：{normalized}");
                }
            }
        }

        /// 绝对路径归一化后必须仍以 `/` 开头。
        #[test]
        fn absolute_paths_stay_absolute(value in "/[a-z./]{0,32}") {
            prop_assert!(posix_normalize(&value).starts_with('/'));
        }

        /// 无论输入如何，`..` 都不能让路径逃出给定的根目录。
        #[test]
        fn traversal_never_escapes_the_root(tail in "[a-z./]{0,24}") {
            let root = "/root/sessions";
            let candidate = format!("{root}/{tail}");
            if is_inside_posix_dir(&candidate, root) {
                // 命中即意味着归一化后仍在根之下——用归一化结果再验一次。
                let normalized = posix_normalize(&candidate);
                prop_assert!(
                    normalized == root || normalized.starts_with(&format!("{root}/")),
                    "越界路径被判为内部：{candidate} -> {normalized}"
                );
            }
        }
    }
}
