//! IPC 命令名注册表。
//!
//! # 为什么把命令名做成显式表
//!
//! 前端通过 `window.api.invoke("codex:list-sessions", …)` 调用，命令名是**跨进程契约**。
//! 名字写错不会在编译期暴露，只会表现为「某个按钮点了没反应」——最难定位的那类问题。
//! 把它们集中成一张表之后：
//!
//! - 命令总数可断言（漏接一个会立刻发现）；
//! - 命令名 → 所属域的映射可测试（便于按域分批注册）；
//! - 前端期望的名字与本表的差异可以用一条脚本比对出来。
//!
//! # 命名规律
//!
//! `<域>:<动作>`，动作部分一律 **kebab-case**（`get-session-messages-page`）。
//! 全部 81 条命令与现网 `ipcMain.handle` 的注册名逐字一致。

/// 命令所属的域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommandDomain {
    /// 应用级（版本、退出、命令执行）。
    App,
    /// 平台清单。
    Ai,
    /// CLI 环境检测与安装。
    Cli,
    /// 剪贴板。
    Clipboard,
    /// 会话资产（四平台共用同一批命令名）。
    Session,
    /// 对话框。
    Dialog,
    /// 本地网关。
    Gateway,
    /// 模型列表。
    Models,
    /// 性能打点。
    Performance,
    /// Shell 与路径。
    Shell,
    /// Skill 管理。
    Skill,
    /// 终端。
    Terminal,
    /// 供应商管理。
    Vendor,
    /// 工作区预设。
    Workspace,
}

impl CommandDomain {
    /// 域前缀。
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Ai => "ai",
            Self::Cli => "cli",
            Self::Clipboard => "clipboard",
            Self::Session => "codex",
            Self::Dialog => "dialog",
            Self::Gateway => "gateway",
            Self::Models => "models",
            Self::Performance => "performance",
            Self::Shell => "shell",
            Self::Skill => "skill",
            Self::Terminal => "terminal",
            Self::Vendor => "vendor",
            Self::Workspace => "workspace",
        }
    }
}

/// 全部命令名。
///
/// # 会话域为什么前缀是 `codex`
///
/// 现网的会话命令统一挂在 `codex:` 前缀下，但它们**服务全部四个平台**——平台由
/// `targetId` 区分（如 `claude:local`）。这是历史命名，改名会破坏前端与既有文档，
/// 因此保留并在此说明。
pub const COMMANDS: [(&str, CommandDomain); 94] = [
    ("app:command", CommandDomain::App),
    ("app:exit-choice", CommandDomain::App),
    ("app:get-version", CommandDomain::App),
    // 平台清单：渲染层的平台下拉框靠它取数据。
    ("ai:list-providers", CommandDomain::Ai),
    ("clipboard:copy-text", CommandDomain::Clipboard),
    ("clipboard:read-text", CommandDomain::Clipboard),
    // 只写不读：粘贴图片走的是 DOM 的 `paste` 事件（webview 直接给出 `Blob`），
    // 不需要绕一圈从剪贴板读。少一条命令就少一个能读系统剪贴板的入口。
    ("clipboard:write-image", CommandDomain::Clipboard),
    // 平台环境检测与安装（P6，未实现）。
    ("cli:check-environment", CommandDomain::Cli),
    ("cli:install", CommandDomain::Cli),
    ("codex:clear-wsl-codex-home", CommandDomain::Session),
    ("codex:branch-session", CommandDomain::Session),
    ("codex:delete-session", CommandDomain::Session),
    ("codex:delete-sessions", CommandDomain::Session),
    ("codex:duplicate-session", CommandDomain::Session),
    ("codex:export-session", CommandDomain::Session),
    ("codex:get-session", CommandDomain::Session),
    ("codex:get-session-messages-page", CommandDomain::Session),
    ("codex:get-session-summary", CommandDomain::Session),
    ("codex:list-cached-sessions", CommandDomain::Session),
    ("codex:list-cached-targets", CommandDomain::Session),
    ("codex:list-session-children", CommandDomain::Session),
    ("codex:list-session-quick-access", CommandDomain::Session),
    ("codex:list-sessions", CommandDomain::Session),
    ("codex:list-targets", CommandDomain::Session),
    ("codex:list-trash-sessions", CommandDomain::Session),
    ("codex:mark-session-opened", CommandDomain::Session),
    ("codex:purge-session", CommandDomain::Session),
    ("codex:purge-sessions", CommandDomain::Session),
    ("codex:restore-session", CommandDomain::Session),
    ("codex:search-sessions", CommandDomain::Session),
    ("codex:set-session-custom-title", CommandDomain::Session),
    ("codex:set-session-favorite", CommandDomain::Session),
    ("codex:set-wsl-codex-home", CommandDomain::Session),
    ("dialog:choose-directory", CommandDomain::Dialog),
    // 单个文件版：技能既可以是目录，也可以是单个 SKILL.md。
    ("dialog:choose-file", CommandDomain::Dialog),
    // 多选版：自管输入的附件一次可以挑好几个。与单文件版分开而不是加个 `multiple`
    // 开关，是因为**返回类型不同**（这里是数组），而单文件版已经被技能导入用着。
    ("dialog:choose-files", CommandDomain::Dialog),
    ("gateway:clear-logs", CommandDomain::Gateway),
    ("gateway:delete-failover-rule", CommandDomain::Gateway),
    ("gateway:delete-log-entries", CommandDomain::Gateway),
    ("gateway:get-external-api-status", CommandDomain::Gateway),
    ("gateway:get-failure-diagnostics", CommandDomain::Gateway),
    ("gateway:get-port", CommandDomain::Gateway),
    ("gateway:get-recent-failures", CommandDomain::Gateway),
    ("gateway:get-usage-report", CommandDomain::Gateway),
    ("gateway:get-usage-summary", CommandDomain::Gateway),
    ("gateway:get-vendor-health", CommandDomain::Gateway),
    ("gateway:list-failover-rules", CommandDomain::Gateway),
    ("gateway:query-logs", CommandDomain::Gateway),
    ("gateway:reset-vendor-health", CommandDomain::Gateway),
    ("gateway:rotate-external-api-token", CommandDomain::Gateway),
    ("gateway:save-failover-rule", CommandDomain::Gateway),
    ("gateway:set-enabled", CommandDomain::Gateway),
    // 三个韧性阈值。**实现一直都在**（含边界校验），只是从没登记进这张表，
    // 因此命令面里查不到、界面也就没法调这三个设置。
    ("gateway:set-failure-threshold", CommandDomain::Gateway),
    (
        "gateway:set-circuit-failure-threshold",
        CommandDomain::Gateway,
    ),
    (
        "gateway:set-circuit-duration-seconds",
        CommandDomain::Gateway,
    ),
    ("gateway:set-external-api-enabled", CommandDomain::Gateway),
    ("gateway:set-failover-rule-enabled", CommandDomain::Gateway),
    ("gateway:set-port", CommandDomain::Gateway),
    // 试一条**还没保存**的规则会不会命中。
    //
    // 有它才不必在前端重写一份匹配逻辑——规则引擎只有 Rust 那一份，
    // 两份一旦分叉就会出现「测试说会命中、实际没切换」。
    ("gateway:test-failover-rule", CommandDomain::Gateway),
    ("models:list", CommandDomain::Models),
    ("performance:log", CommandDomain::Performance),
    ("shell:open-path", CommandDomain::Shell),
    ("shell:open-session-folder", CommandDomain::Shell),
    ("shell:path-exists", CommandDomain::Shell),
    // 图片预览与粘贴图片落盘。**不走平台层**：只用到标准库与 base64，
    // 因此留在命令层就能被测到（平台层那几条要真实 Tauri 才跑得起来）。
    ("shell:read-image", CommandDomain::Shell),
    ("shell:save-image", CommandDomain::Shell),
    ("skill:delete", CommandDomain::Skill),
    ("skill:import", CommandDomain::Skill),
    ("skill:list", CommandDomain::Skill),
    ("skill:list-trash", CommandDomain::Skill),
    ("skill:open-folder", CommandDomain::Skill),
    ("skill:purge", CommandDomain::Skill),
    ("skill:restore", CommandDomain::Skill),
    ("skill:set-enabled", CommandDomain::Skill),
    ("terminal:attention-notification", CommandDomain::Terminal),
    ("terminal:resize", CommandDomain::Terminal),
    ("terminal:start", CommandDomain::Terminal),
    ("terminal:start-system", CommandDomain::Terminal),
    ("terminal:stop", CommandDomain::Terminal),
    ("terminal:write", CommandDomain::Terminal),
    ("vendor:delete", CommandDomain::Vendor),
    ("vendor:enable", CommandDomain::Vendor),
    ("vendor:list", CommandDomain::Vendor),
    ("vendor:list-models", CommandDomain::Vendor),
    ("vendor:read-configs", CommandDomain::Vendor),
    ("vendor:refresh-balance", CommandDomain::Vendor),
    ("vendor:refresh-balances", CommandDomain::Vendor),
    ("vendor:route-mode", CommandDomain::Vendor),
    ("vendor:route-switch", CommandDomain::Vendor),
    ("vendor:save", CommandDomain::Vendor),
    ("vendor:set-enabled", CommandDomain::Vendor),
    ("workspace:delete-preset", CommandDomain::Workspace),
    ("workspace:list-presets", CommandDomain::Workspace),
    ("workspace:save-preset", CommandDomain::Workspace),
];

/// 命令总数（前端契约的一部分）。
pub const COMMAND_COUNT: usize = COMMANDS.len();

/// 已知命令名列表。
#[must_use]
pub fn command_names() -> Vec<&'static str> {
    COMMANDS.iter().map(|(name, _)| *name).collect()
}

/// 按域筛选命令名。
#[must_use]
pub fn commands_of(domain: CommandDomain) -> Vec<&'static str> {
    COMMANDS
        .iter()
        .filter(|(_, item)| *item == domain)
        .map(|(name, _)| *name)
        .collect()
}

/// 判断命令名是否已注册。
#[must_use]
pub fn is_known(command: &str) -> bool {
    COMMANDS.iter().any(|(name, _)| *name == command)
}

/// 解析命令名所属的域。
#[must_use]
pub fn domain_of(command: &str) -> Option<CommandDomain> {
    COMMANDS
        .iter()
        .find(|(name, _)| *name == command)
        .map(|(_, domain)| *domain)
}

/// 命令名前缀与声明域是否一致。
///
/// 用于自检「表里的域和名字前缀对得上」——手工维护 81 条时最容易写错的就是这个。
#[must_use]
pub fn prefix_matches_domain(command: &str, domain: CommandDomain) -> bool {
    command
        .split_once(':')
        .is_some_and(|(prefix, _)| prefix == domain.prefix())
}

/// 未知命令时的错误。
#[must_use]
pub fn unknown_command(command: &str) -> aivc_core::Error {
    aivc_core::error::IpcError::CommandFailed {
        message: format!("未知命令：{command}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn the_command_count_is_stable() {
        // 前端契约的一部分：漏接或多接一条都会在这里暴露。
        assert_eq!(COMMAND_COUNT, 94);
        assert_eq!(command_names().len(), 94);
    }

    #[test]
    fn command_names_are_unique() {
        let unique: BTreeSet<&str> = COMMANDS.iter().map(|(name, _)| *name).collect();
        assert_eq!(unique.len(), COMMAND_COUNT, "存在重复命令名");
    }

    #[test]
    fn every_command_prefix_matches_its_declared_domain() {
        for (name, domain) in COMMANDS {
            assert!(
                prefix_matches_domain(name, domain),
                "{name} 的前缀与声明域 {domain:?} 不一致"
            );
        }
    }

    #[test]
    fn command_names_use_the_documented_shape() {
        for (name, _) in COMMANDS {
            let (prefix, action) = name.split_once(':').expect("必须含冒号");
            assert!(!prefix.is_empty(), "{name} 缺少域前缀");
            assert!(!action.is_empty(), "{name} 缺少动作名");
            // 动作部分一律 kebab-case：不出现下划线或大写。
            assert!(
                !action.contains('_') && !action.chars().any(char::is_uppercase),
                "{name} 的动作部分不是 kebab-case"
            );
        }
    }

    #[test]
    fn session_commands_serve_every_provider_despite_the_codex_prefix() {
        // `codex:` 是历史命名，实际服务四个平台；平台由 targetId 区分。
        let session = commands_of(CommandDomain::Session);
        assert_eq!(session.len(), 24);
        assert!(session.iter().all(|name| name.starts_with("codex:")));
    }

    #[test]
    fn each_domain_exposes_its_commands() {
        assert_eq!(commands_of(CommandDomain::App).len(), 3);
        assert_eq!(commands_of(CommandDomain::Ai).len(), 1);
        assert_eq!(commands_of(CommandDomain::Cli).len(), 2);
        assert_eq!(commands_of(CommandDomain::Clipboard).len(), 3);
        assert_eq!(commands_of(CommandDomain::Dialog).len(), 3);
        assert_eq!(commands_of(CommandDomain::Gateway).len(), 23);
        assert_eq!(commands_of(CommandDomain::Models).len(), 1);
        assert_eq!(commands_of(CommandDomain::Performance).len(), 1);
        assert_eq!(commands_of(CommandDomain::Shell).len(), 5);
        assert_eq!(commands_of(CommandDomain::Skill).len(), 8);
        assert_eq!(commands_of(CommandDomain::Terminal).len(), 6);
        assert_eq!(commands_of(CommandDomain::Vendor).len(), 11);
        assert_eq!(commands_of(CommandDomain::Workspace).len(), 3);

        // 各域加起来必须正好是全部命令。
        let total: usize = [
            CommandDomain::App,
            CommandDomain::Ai,
            CommandDomain::Cli,
            CommandDomain::Clipboard,
            CommandDomain::Session,
            CommandDomain::Dialog,
            CommandDomain::Gateway,
            CommandDomain::Models,
            CommandDomain::Performance,
            CommandDomain::Shell,
            CommandDomain::Skill,
            CommandDomain::Terminal,
            CommandDomain::Vendor,
            CommandDomain::Workspace,
        ]
        .into_iter()
        .map(|domain| commands_of(domain).len())
        .sum();
        assert_eq!(total, COMMAND_COUNT);
    }

    #[test]
    fn lookup_helpers_agree_with_the_table() {
        assert!(is_known("codex:list-sessions"));
        assert!(!is_known("codex:not-a-command"));
        assert_eq!(domain_of("gateway:set-port"), Some(CommandDomain::Gateway));
        assert_eq!(domain_of("nope"), None);
        assert!(
            unknown_command("nope")
                .to_string()
                .contains("未知命令：nope")
        );
    }

    #[test]
    fn the_expected_wire_names_are_present() {
        // 逐条钉死前端调用面里最容易写错的名字。
        for name in [
            "codex:get-session-messages-page",
            "codex:list-cached-sessions",
            "gateway:rotate-external-api-token",
            "gateway:set-failover-rule-enabled",
            "vendor:refresh-balances",
            "terminal:attention-notification",
            "workspace:delete-preset",
        ] {
            assert!(is_known(name), "{name} 缺失");
        }
    }
}
