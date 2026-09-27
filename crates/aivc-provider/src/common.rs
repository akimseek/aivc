//! Provider 之间真正共享的纯逻辑。
//!
//! 只放**确实共享**的部分：目标探测骨架、会话排序、摘要级搜索匹配、有限并发。
//! 存储布局与文件格式**不**放这里——那是各平台的真实差异，抽成统一基类只会增加条件分支。

use aivc_core::session::Session;

/// 会话文件指纹与路径。
///
/// 各平台的存储布局不同，但「解析一个会话需要知道哪些文件属性」是共通的：路径、
/// mtime、ctime、大小。放在这里避免每个 Provider 各定义一份。
#[derive(Debug, Clone, PartialEq)]
pub struct SessionFileInfo {
    /// 文件绝对路径。
    pub file_path: String,
    /// 文件修改时间（毫秒）。
    pub mtime_ms: f64,
    /// 文件内容变更时间（毫秒）。
    pub change_ms: Option<f64>,
    /// 文件大小（字节）。
    pub size: u64,
}

impl SessionFileInfo {
    /// 由会话文件描述构造。
    #[must_use]
    pub fn from_session_file(file: &aivc_core::session::SessionFile) -> Self {
        Self {
            file_path: file.file_path.clone(),
            mtime_ms: file.mtime_ms,
            change_ms: file.change_ms,
            size: file.size,
        }
    }
}

/// 判断路径是否**严格**位于某个根目录之内（排除根目录本身）。
///
/// 「目录自身也算在根之内」会让一条伪造的路径把整个会话目录当成目标文件，
/// 因此所有变更操作都必须用这个判据而不是宽松的包含判断。
#[must_use]
pub fn is_strictly_inside(path: &str, root: &str, kind: aivc_core::ports::StoreKind) -> bool {
    let inside = match kind {
        aivc_core::ports::StoreKind::Local => aivc_core::path::is_inside_local_path(path, root),
        aivc_core::ports::StoreKind::Wsl => aivc_core::path::is_inside_posix_dir(path, root),
    };
    inside && relative_to(path, root).is_some()
}

/// 断言路径严格位于根目录之内，否则返回携带自定义文案的错误。
pub fn assert_session_file_inside(
    path: &str,
    root: &str,
    kind: aivc_core::ports::StoreKind,
    message: &str,
) -> aivc_core::error::Result<()> {
    if is_strictly_inside(path, root, kind) {
        return Ok(());
    }
    Err(aivc_core::error::SessionError::SessionPathNotAllowed {
        message: message.to_owned(),
    }
    .into())
}

/// 把源文件在源根目录下的相对位置映射到目标根目录。
///
/// 回收站与恢复共用这一条规则：两边都保持相同的相对结构，因此恢复不需要额外索引，
/// 「原位置已存在同名文件」这条冲突判断也才成立。
///
/// 先断言再算相对路径——顺序反了的话，越界输入会得到一个看似合理的垃圾路径，
/// 错误就从「明确拒绝」退化成「静默的数据损坏」。
pub fn relocate_session_path(
    file_path: &str,
    source_root: &str,
    destination_root: &str,
    kind: aivc_core::ports::StoreKind,
    message: &str,
) -> aivc_core::error::Result<String> {
    assert_session_file_inside(file_path, source_root, kind, message)?;
    let relative = relative_to(file_path, source_root).ok_or_else(|| {
        aivc_core::error::SessionError::SessionPathNotAllowed {
            message: message.to_owned(),
        }
    })?;
    Ok(join_path(kind, destination_root, &relative))
}

/// 计算路径相对某个根目录的部分；等于根目录本身时返回 [`None`]。
#[must_use]
pub fn relative_to(path: &str, root: &str) -> Option<String> {
    let root = root.trim_end_matches(['/', '\\']);
    let rest = path.strip_prefix(root)?;
    let rest = rest.trim_start_matches(['/', '\\']);
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_owned())
    }
}

/// 按后端拼接路径。
#[must_use]
pub fn join_path(kind: aivc_core::ports::StoreKind, base: &str, rest: &str) -> String {
    match kind {
        aivc_core::ports::StoreKind::Local => std::path::Path::new(base)
            .join(rest)
            .to_string_lossy()
            .into_owned(),
        aivc_core::ports::StoreKind::Wsl => {
            let base = base.trim_end_matches('/');
            let rest = rest.trim_start_matches('/');
            if base.is_empty() {
                rest.to_owned()
            } else if rest.is_empty() {
                base.to_owned()
            } else {
                format!("{base}/{rest}")
            }
        }
    }
}

/// 只读缓存，不触发扫描。
pub async fn list_cached_sessions(
    ctx: &crate::context::ProviderContext,
    cache_key: &str,
) -> aivc_core::error::Result<Vec<Session>> {
    let cached = ctx.session_cache().read(cache_key).await?;
    let mut sessions: Vec<Session> = cached.into_values().map(|entry| entry.session).collect();
    sort_sessions(&mut sessions);
    Ok(sessions)
}

/// 取路径所在目录（按后端语义）。
#[must_use]
pub fn dir_name(path: &str, kind: aivc_core::ports::StoreKind) -> String {
    match kind {
        aivc_core::ports::StoreKind::Local => std::path::Path::new(path)
            .parent()
            .map_or_else(String::new, |parent| parent.to_string_lossy().into_owned()),
        aivc_core::ports::StoreKind::Wsl => match path.rfind('/') {
            Some(0) => "/".to_owned(),
            Some(index) => path[..index].to_owned(),
            None => String::new(),
        },
    }
}
/// 本机 CLI 目标探测参数。
#[derive(Debug, Clone)]
pub struct LocalCliProbe {
    /// 平台标识。
    pub provider: aivc_core::target::ProviderId,
    /// 界面展示名（如「Claude Code」）。
    pub display_name: &'static str,
    /// Windows 上的命令名。
    pub windows_command: &'static str,
    /// 类 Unix 上的命令名。
    pub unix_command: &'static str,
    /// 配置目录。
    pub config_dir: String,
}

/// 探测本机 CLI 目标。
///
/// **命令与配置目录任一存在**即认为目标可用：用户可能装了 CLI 但还没跑过（没有配置
/// 目录），也可能有历史配置但命令不在 PATH 上。两者皆无才不展示。
///
/// `available` 只反映**命令**是否存在——它决定能否新建/恢复会话，而读取历史会话只需
/// 配置目录。把两者混为一谈会让「有配置没命令」的场景被误判为完全不可用。
pub async fn probe_local_cli_target(probe: &LocalCliProbe) -> Option<aivc_core::target::Target> {
    use aivc_core::target::{Platform, Target, TargetKind};

    let command = if cfg!(windows) {
        probe.windows_command
    } else {
        probe.unix_command
    };
    let found = command_exists_local(command);
    let has_config_dir = tokio::fs::try_exists(&probe.config_dir)
        .await
        .unwrap_or(false);
    if !found && !has_config_dir {
        return None;
    }

    let detail = match (found, has_config_dir) {
        (true, true) => format!("找到 {} 命令和配置目录", probe.display_name),
        (true, false) => format!("找到 {} 命令", probe.display_name),
        (false, _) => format!(
            "找到 {} 配置目录，未找到 {} 命令",
            probe.display_name, probe.unix_command
        ),
    };

    Some(Target {
        id: aivc_core::id::TargetId::from(format!("{}:local", probe.provider.as_str())),
        provider: probe.provider,
        label: format!(
            "{}：本机（{}）",
            probe.display_name,
            Platform::current().as_str()
        ),
        kind: TargetKind::Local,
        distro: None,
        codex_home: has_config_dir.then(|| probe.config_dir.clone()),
        available: found,
        detail: Some(detail),
    })
}

/// WSL CLI 目标探测参数。
#[derive(Debug, Clone)]
pub struct WslCliProbe {
    /// 平台标识。
    pub provider: aivc_core::target::ProviderId,
    /// 界面展示名。
    pub display_name: &'static str,
    /// 发行版内的命令名。
    pub command: &'static str,
    /// 配置目录相对 `$HOME` 的位置（如 `.claude`）。
    pub config_dir_suffix: &'static str,
}

/// 探测各 WSL 发行版内的 CLI 目标。
pub async fn probe_wsl_cli_targets(
    ctx: &crate::context::ProviderContext,
    probe: &WslCliProbe,
) -> Vec<aivc_core::target::Target> {
    use aivc_core::target::{Target, TargetKind};

    if !ctx.wsl().is_available() {
        return Vec::new();
    }
    let distros = ctx.wsl().list_distros().await;
    let mut targets = Vec::new();
    for distro in distros {
        let home = ctx.wsl().get_env(&distro, "HOME").await.unwrap_or_default();
        let home = home.trim().to_owned();
        let config_dir = if home.is_empty() {
            String::new()
        } else {
            format!("{}/{}", home.trim_end_matches('/'), probe.config_dir_suffix)
        };
        let command_found = ctx.wsl().command_exists(&distro, probe.command).await;
        let has_config_dir = !config_dir.is_empty() && ctx.wsl().exists(&distro, &config_dir).await;
        if !command_found && !has_config_dir {
            continue;
        }

        let detail = match (command_found, has_config_dir) {
            (true, true) => format!("找到 {} 命令和配置目录", probe.display_name),
            (true, false) => format!("找到 {} 命令", probe.display_name),
            (false, _) => format!(
                "找到 {} 配置目录，未找到 {} 命令",
                probe.display_name, probe.command
            ),
        };

        targets.push(Target {
            id: aivc_core::id::TargetId::from(format!("{}:wsl:{distro}", probe.provider.as_str())),
            provider: probe.provider,
            label: format!("{}：WSL：{distro}", probe.display_name),
            kind: TargetKind::Wsl,
            distro: Some(distro),
            codex_home: has_config_dir.then_some(config_dir),
            available: command_found,
            detail: Some(detail),
        });
    }
    targets
}

/// 本机命令是否存在（按 `PATH` 与 Windows 的 `PATHEXT` 查找）。
///
/// 自己实现而不引入 `which` 依赖：这里只需要「能否在 PATH 上找到」这一个判断，
/// 而多一个依赖会进入供应链与许可证审计范围。
#[must_use]
pub fn command_exists_local(command: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT").map_or_else(
            |_| vec![".exe".to_owned(), ".cmd".to_owned(), ".bat".to_owned()],
            |value| {
                value
                    .split(';')
                    .filter(|item| !item.is_empty())
                    .map(str::to_lowercase)
                    .collect()
            },
        )
    } else {
        Vec::new()
    };

    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(command);
        if candidate.is_file() {
            return true;
        }
        for extension in &extensions {
            let with_extension = directory.join(format!("{command}{extension}"));
            if with_extension.is_file() {
                return true;
            }
        }
    }
    false
}

/// 会话列表排序：`updatedAt` 优先，缺失时回退 `createdAt`，均为倒序。
#[must_use]
pub fn sort_sessions_by_recency(left: &Session, right: &Session) -> std::cmp::Ordering {
    let left_key = left
        .updated_at
        .as_ref()
        .or(left.created_at.as_ref())
        .and_then(aivc_core::time::Timestamp::to_unix_millis)
        .unwrap_or(0);
    let right_key = right
        .updated_at
        .as_ref()
        .or(right.created_at.as_ref())
        .and_then(aivc_core::time::Timestamp::to_unix_millis)
        .unwrap_or(0);
    right_key.cmp(&left_key)
}

/// 按 `updatedAt`（回退 `createdAt`）倒序排列。
pub fn sort_sessions(sessions: &mut [Session]) {
    sessions.sort_by(sort_sessions_by_recency);
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::SessionId;
    use aivc_core::time::Timestamp;

    fn session(id: &str, updated: Option<&str>, created: Option<&str>) -> Session {
        Session {
            id: SessionId::from(id),
            title: id.to_owned(),
            source_title: None,
            cwd: None,
            created_at: created.map(Timestamp::from_raw),
            updated_at: updated.map(Timestamp::from_raw),
            model: None,
            model_status: None,
            cli_version: None,
            file_path: format!("/{id}.jsonl"),
            file_mtime_ms: None,
            file_change_ms: None,
            file_size: None,
            message_count: 0,
            message_count_is_lower_bound: false,
            preview: Vec::new(),
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    #[test]
    fn sessions_sort_by_updated_at_descending() {
        let mut sessions = vec![
            session("old", Some("2026-01-01T00:00:00.000Z"), None),
            session("new", Some("2026-06-01T00:00:00.000Z"), None),
            session("mid", Some("2026-03-01T00:00:00.000Z"), None),
        ];
        sort_sessions(&mut sessions);
        let ids: Vec<&str> = sessions.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(ids, vec!["new", "mid", "old"]);
    }

    #[test]
    fn updated_at_missing_falls_back_to_created_at() {
        let mut sessions = vec![
            session("only-created", None, Some("2026-06-01T00:00:00.000Z")),
            session("stale-updated", Some("2026-01-01T00:00:00.000Z"), None),
        ];
        sort_sessions(&mut sessions);
        assert_eq!(sessions[0].id.as_str(), "only-created");
    }

    #[test]
    fn missing_timestamps_sort_last() {
        let mut sessions = vec![
            session("none", None, None),
            session("dated", Some("2026-01-01T00:00:00.000Z"), None),
        ];
        sort_sessions(&mut sessions);
        assert_eq!(sessions[0].id.as_str(), "dated");
        assert_eq!(sessions[1].id.as_str(), "none");
    }
}
