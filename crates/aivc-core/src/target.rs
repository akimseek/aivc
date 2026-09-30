//! 平台标识、目标（本机 / WSL 发行版）与 targetId 语法。
//!
//! # targetId 语法（必须逐字保持）
//!
//! 后端（`aivc-provider`）实际发出的 WSL 编号**都带平台前缀**：
//!
//! | 平台 | 本机 | WSL |
//! | --- | --- | --- |
//! | Codex | `local` / `codex:local:<配置目录>` | `codex:wsl:<distro>` |
//! | Gemini | `gemini:local` | `gemini:wsl:<distro>` |
//! | Claude Code | `claude:local` | `claude:wsl:<distro>` |
//! | Qoder CN | `qoder:local` | `qoder:wsl:<distro>` |
//!
//! Codex 的 WSL 编号在**旧平台（以及旧缓存）**里是不带平台前缀的裸 `wsl:<distro>`，
//! 两种形制都真实存在于用户数据里（旧编号还会出现在 `cachedTargets` 与数据库里），
//! 因此解析时都要认——见 [`ProviderId::wsl_distro_of`]；**生成**新编号一律用带
//! 平台前缀的权威形（[`ProviderId::wsl_target_id`]）。
//!
//! 另外 [`ProviderId::from_target_id`] 对未知前缀**兜底为 Codex**：这是现网的
//! 兼容行为（历史上只支持 Codex），刻意保留但不作为契约固化。

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::TargetId;

/// AI 平台标识。
///
/// 序列化后是小写短名（`codex` / `gemini` / `claude` / `qoder`），与现网一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    /// Codex。
    Codex,
    /// Gemini。
    Gemini,
    /// Claude Code。
    Claude,
    /// Qoder CN。
    Qoder,
}

impl ProviderId {
    /// 全部平台，顺序与现网注册顺序一致（Codex / Gemini / Claude / Qoder）。
    pub const ALL: [Self; 4] = [Self::Codex, Self::Gemini, Self::Claude, Self::Qoder];

    /// 小写短名，与 JSON 契约一致。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Claude => "claude",
            Self::Qoder => "qoder",
        }
    }

    /// 界面展示名，与现网 `label` 一致。
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::Claude => "Claude Code",
            Self::Qoder => "Qoder CN",
        }
    }

    /// WSL targetId 前缀（**权威形**：后端实际发出的编号）。
    ///
    /// 旧编号的兼容只保留在解析侧，见 [`Self::wsl_distro_of`]——生成新编号不能再用
    /// 旧形，否则造出来的编号谁也认不出来（会话与目标都按编号存取）。
    #[must_use]
    pub const fn wsl_prefix(self) -> &'static str {
        match self {
            Self::Codex => "codex:wsl:",
            Self::Gemini => "gemini:wsl:",
            Self::Claude => "claude:wsl:",
            Self::Qoder => "qoder:wsl:",
        }
    }

    /// 旧平台留下的 WSL 前缀，**只在解析时**接受。
    ///
    /// 旧平台给 Codex 发的是裸 `wsl:<distro>`（`buildWslProviderTargetId("codex", …)`
    /// 的产物，见 `shell-golden.json` 的 `wsl:Ubuntu` 用例），这些编号至今仍在用户的
    /// `cachedTargets` 与会话记录里；其余平台从没有过第二种形制。
    const fn legacy_wsl_prefix(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some("wsl:"),
            Self::Gemini | Self::Claude | Self::Qoder => None,
        }
    }

    /// 本机 targetId。Codex 为裸 `local`，其余为 `<provider>:local`。
    #[must_use]
    pub const fn local_target_id(self) -> &'static str {
        match self {
            Self::Codex => "local",
            Self::Gemini => "gemini:local",
            Self::Claude => "claude:local",
            Self::Qoder => "qoder:local",
        }
    }

    /// 由 targetId 解析平台，未知前缀兜底为 [`ProviderId::Codex`]。
    ///
    /// 兜底是现网的兼容行为：历史上只支持 Codex，未加前缀的目标都归它。
    #[must_use]
    pub fn from_target_id(target_id: &str) -> Self {
        if target_id.starts_with("gemini:") {
            Self::Gemini
        } else if target_id.starts_with("claude:") {
            Self::Claude
        } else if target_id.starts_with("qoder:") {
            Self::Qoder
        } else {
            // `codex:` 前缀与未知前缀都归 Codex。
            Self::Codex
        }
    }

    /// 解析**本平台**的 WSL 发行版名；非本平台目标返回空串。
    ///
    /// Codex 同时接受权威形与旧形（裸 `wsl:<distro>`）——后端发的就是前者，而用户的
    /// 旧缓存里还有后者，漏认任何一边都会把「WSL 里有」误判成「本机没有」。
    ///
    /// 只认自己的前缀，避免把别的平台的 WSL 目标误判为本平台目标。
    #[must_use]
    pub fn wsl_distro_of(self, target_id: &str) -> &str {
        if let Some(distro) = target_id.strip_prefix(self.wsl_prefix()) {
            return distro;
        }
        self.legacy_wsl_prefix()
            .and_then(|prefix| target_id.strip_prefix(prefix))
            .unwrap_or_default()
    }

    /// 生成 WSL targetId，与 [`Self::wsl_distro_of`] 互逆。
    #[must_use]
    pub fn wsl_target_id(self, distro: &str) -> TargetId {
        TargetId::new(format!("{}{distro}", self.wsl_prefix()))
    }

    /// 能力开关。
    ///
    /// 这是**对外契约**（IPC 返回给渲染层做菜单/按钮可用性判断），必须与现网
    /// `providerDefinitions` 逐项一致。实现层的能力由能力子 trait 在编译期保证，
    /// 见 `aivc-provider`。
    #[must_use]
    pub const fn capabilities(self) -> ProviderCapabilities {
        match self {
            Self::Codex => ProviderCapabilities {
                skills: true,
                branch: true,
                usage: true,
                trash: true,
                batch_actions: true,
                custom_cwd: true,
                export: true,
                session_settings: true,
                duplicate: true,
                vendor_management: true,
            },
            Self::Gemini | Self::Claude => ProviderCapabilities {
                skills: false,
                branch: true,
                usage: true,
                trash: true,
                batch_actions: true,
                custom_cwd: true,
                export: true,
                session_settings: false,
                duplicate: true,
                vendor_management: true,
            },
            Self::Qoder => ProviderCapabilities {
                skills: false,
                branch: false,
                usage: true,
                trash: true,
                batch_actions: true,
                custom_cwd: true,
                export: true,
                session_settings: false,
                duplicate: false,
                vendor_management: false,
            },
        }
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Default for ProviderId {
    /// 缺省平台为 Codex，与 [`ProviderId::from_target_id`] 的历史兜底一致。
    ///
    /// **仅供 Rust 侧构造默认值使用**：所有 wire 类型里的 `provider_id` 字段都是必填
    /// （没有 `#[serde(default)]`），因此这个默认值不会掩盖「请求里漏传平台」这类错误。
    fn default() -> Self {
        Self::Codex
    }
}

/// 平台能力开关。
///
/// 字段名与现网 `AiProviderCapabilities` 一一对应（JSON 为 camelCase）。
///
/// 刻意用 10 个布尔字段而非位标志或枚举集合：这是**对外 wire 契约**，渲染层按字段名
/// 读取（`capabilities.branch` 等），并会随 IPC 响应序列化。改成其他形状会同时破坏
/// 契约与可读性，因此显式豁免 `struct_excessive_bools`。
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderCapabilities {
    /// 是否支持 Codex Skill 管理。
    pub skills: bool,
    /// 是否支持「从此处分支」。
    pub branch: bool,
    /// 是否提供 Token / 上下文用量。
    pub usage: bool,
    /// 是否支持回收站（删除 / 恢复 / 永久删除）。
    pub trash: bool,
    /// 是否支持批量操作。
    pub batch_actions: bool,
    /// 是否支持自定义工作目录。
    pub custom_cwd: bool,
    /// 是否支持导出。
    pub export: bool,
    /// 是否支持会话设置（`cwd` / `codexHome`）。
    pub session_settings: bool,
    /// 是否支持复制会话。
    pub duplicate: bool,
    /// 是否支持供应商管理。
    pub vendor_management: bool,
}

/// 目标类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    /// 本机（Windows / macOS / Linux）。
    Local,
    /// WSL 发行版。
    Wsl,
}

/// 宿主操作系统，对应 Node 的 `process.platform`。
///
/// 刻意保留 Node 的字符串形式（`win32` / `darwin` / `linux`），因为现网把它拼进
/// 目标标签（`本机（win32）`）并落库到 `cachedTargets`，属于可见契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Windows。
    #[serde(rename = "win32")]
    Win32,
    /// macOS。
    #[serde(rename = "darwin")]
    Darwin,
    /// Linux。
    #[serde(rename = "linux")]
    Linux,
}

impl Platform {
    /// 当前宿主平台。
    #[must_use]
    pub const fn current() -> Self {
        #[cfg(target_os = "windows")]
        {
            Self::Win32
        }
        #[cfg(target_os = "macos")]
        {
            Self::Darwin
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Self::Linux
        }
    }

    /// Node `process.platform` 形式的小写名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Win32 => "win32",
            Self::Darwin => "darwin",
            Self::Linux => "linux",
        }
    }

    /// 是否为 Windows 宿主。WSL 目标只在 Windows 宿主上存在。
    #[must_use]
    pub const fn is_windows(self) -> bool {
        matches!(self, Self::Win32)
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 一个可操作的目标：本机环境或某个 WSL 发行版。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    /// 目标标识，语法见模块文档。
    pub id: TargetId,
    /// 所属平台。
    pub provider: ProviderId,
    /// 界面展示名。
    pub label: String,
    /// 本机还是 WSL。
    pub kind: TargetKind,
    /// WSL 发行版名（仅 `kind == Wsl`）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distro: Option<String>,
    /// 该平台的配置目录（Codex 为 `CODEX_HOME`，其余为各 CLI 配置目录）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex_home: Option<String>,
    /// CLI 命令是否可用（决定能否新建 / 恢复会话）。
    pub available: bool,
    /// 探测结果的人类可读说明。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Target {
    /// 该目标是否为 WSL 目标。
    #[must_use]
    pub const fn is_wsl(&self) -> bool {
        matches!(self.kind, TargetKind::Wsl)
    }

    /// WSL 发行版名；本机目标返回 [`None`]。
    #[must_use]
    pub fn distro(&self) -> Option<&str> {
        self.distro.as_deref()
    }
}

/// 构造「本机」目标标签。
///
/// - Codex：`本机（win32）`
/// - 其余平台：`Claude Code：本机（win32）`
#[must_use]
pub fn local_target_label(provider: ProviderId, platform: Platform) -> String {
    match provider {
        ProviderId::Codex => format!("本机（{platform}）"),
        _ => format!("{}：本机（{platform}）", provider.display_name()),
    }
}

/// 构造「WSL」目标标签。
///
/// - Codex：`WSL：Ubuntu`
/// - 其余平台：`Claude Code：WSL：Ubuntu`
#[must_use]
pub fn wsl_target_label(provider: ProviderId, distro: &str) -> String {
    match provider {
        ProviderId::Codex => format!("WSL：{distro}"),
        _ => format!("{}：WSL：{distro}", provider.display_name()),
    }
}

/// 构造「命令与配置目录都找到」的探测说明（非 Codex 平台）。
#[must_use]
pub fn cli_target_detail_found(display_name: &str, has_config_dir: bool) -> String {
    if has_config_dir {
        format!("找到 {display_name} 命令和配置目录")
    } else {
        format!("找到 {display_name} 命令")
    }
}

/// 构造「只有配置目录、没有命令」的探测说明（非 Codex 平台）。
#[must_use]
pub fn cli_target_detail_config_only(display_name: &str, command: &str) -> String {
    format!("找到 {display_name} 配置目录，未找到 {command} 命令")
}

/// 构造 Codex 本机目标的探测说明。
#[must_use]
pub fn codex_local_target_detail(has_sessions: bool, codex_found: bool) -> &'static str {
    if has_sessions {
        "找到 Codex 会话"
    } else if codex_found {
        "找到 Codex 命令"
    } else {
        "找到 Codex 目录"
    }
}

/// 构造 Codex WSL 目标的探测说明（含 `whoami` / `HOME` 补充信息）。
#[must_use]
pub fn codex_wsl_target_detail(
    has_sessions: bool,
    has_codex_home: bool,
    user: &str,
    home: &str,
) -> String {
    let base = if has_sessions {
        "找到 Codex 会话"
    } else if has_codex_home {
        "找到 Codex 目录"
    } else {
        "找到 Codex 命令"
    };
    let user = user.trim();
    let home = home.trim();
    if user.is_empty() || home.is_empty() {
        base.to_owned()
    } else {
        format!("{base}（{user}：{home}）")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_prefixes_match_the_ids_the_backend_emits() {
        // 这条此前钉的是「Codex 是裸 `wsl:`，其余带平台前缀」——那是**旧平台**的口径，
        // 与后端实际发出的编号不符：`aivc-provider` 给 Codex 发的是
        // `codex:wsl:<distro>`（`codex/provider.rs` 与实机 `cachedTargets` 都是这个形制）。
        // 前缀是**生成**新编号用的，必须与生产者一致，所以这里改成统一带平台前缀；
        // 旧编号的兼容只在解析侧保留（见 `codex_parses_the_emitted_id_and_the_legacy_bare_one`）。
        assert_eq!(ProviderId::Codex.wsl_prefix(), "codex:wsl:");
        assert_eq!(ProviderId::Gemini.wsl_prefix(), "gemini:wsl:");
        assert_eq!(ProviderId::Claude.wsl_prefix(), "claude:wsl:");
        assert_eq!(ProviderId::Qoder.wsl_prefix(), "qoder:wsl:");
    }

    #[test]
    fn wsl_target_id_round_trips_per_provider() {
        for provider in ProviderId::ALL {
            let id = provider.wsl_target_id("Ubuntu-22.04");
            assert_eq!(
                id.as_str(),
                format!("{}Ubuntu-22.04", provider.wsl_prefix())
            );
            assert_eq!(provider.wsl_distro_of(id.as_str()), "Ubuntu-22.04");
        }
        assert_eq!(
            ProviderId::Codex.wsl_target_id("Ubuntu").as_str(),
            "codex:wsl:Ubuntu"
        );
    }

    #[test]
    fn codex_parses_the_emitted_id_and_the_legacy_bare_one() {
        // 权威形：后端发出的编号必须认得。此前认不出来（前缀是裸 `wsl:`），于是
        // `require_local` 不拒绝、按本机路径判 `/home/...` 不存在——WSL 会话恢复
        // 每次都被弹「原目录丢失」，这就是 P0-C 的根因。
        assert_eq!(
            ProviderId::Codex.wsl_distro_of("codex:wsl:Ubuntu-22.04"),
            "Ubuntu-22.04"
        );
        // 旧形：旧平台（以及旧缓存）留下的裸编号仍要认得。
        assert_eq!(ProviderId::Codex.wsl_distro_of("wsl:Ubuntu"), "Ubuntu");
        // 兼容分支只属于 Codex——别家从没有过裸编号，不得借道认领。
        for provider in [ProviderId::Gemini, ProviderId::Claude, ProviderId::Qoder] {
            assert_eq!(provider.wsl_distro_of("wsl:Ubuntu"), "", "{provider}");
        }
        // 非 WSL 与空发行版一律空串。
        assert_eq!(ProviderId::Codex.wsl_distro_of("codex:local"), "");
        assert_eq!(ProviderId::Codex.wsl_distro_of("codex:wsl:"), "");
    }

    #[test]
    fn wsl_distro_of_rejects_other_providers_prefixes() {
        // Claude 不得把 Codex 的 `wsl:Ubuntu` 当成自己的 WSL 目标。
        assert_eq!(ProviderId::Claude.wsl_distro_of("wsl:Ubuntu"), "");
        assert_eq!(
            ProviderId::Claude.wsl_distro_of("claude:wsl:Ubuntu"),
            "Ubuntu"
        );
        // Codex 不得把带平台前缀的目标当成自己的。
        assert_eq!(ProviderId::Codex.wsl_distro_of("claude:wsl:Ubuntu"), "");
    }

    #[test]
    fn local_target_ids_match_existing_wire_values() {
        assert_eq!(ProviderId::Codex.local_target_id(), "local");
        assert_eq!(ProviderId::Gemini.local_target_id(), "gemini:local");
        assert_eq!(ProviderId::Claude.local_target_id(), "claude:local");
        assert_eq!(ProviderId::Qoder.local_target_id(), "qoder:local");
    }

    #[test]
    fn provider_resolution_falls_back_to_codex_for_unknown_prefix() {
        assert_eq!(
            ProviderId::from_target_id("gemini:wsl:Ubuntu"),
            ProviderId::Gemini
        );
        assert_eq!(
            ProviderId::from_target_id("claude:local"),
            ProviderId::Claude
        );
        assert_eq!(ProviderId::from_target_id("qoder:local"), ProviderId::Qoder);
        assert_eq!(ProviderId::from_target_id("codex:local"), ProviderId::Codex);
        // 兜底：裸 local 与任意未知前缀都归 Codex。
        assert_eq!(ProviderId::from_target_id("local"), ProviderId::Codex);
        assert_eq!(ProviderId::from_target_id("wsl:Ubuntu"), ProviderId::Codex);
        assert_eq!(ProviderId::from_target_id("whatever"), ProviderId::Codex);
    }

    #[test]
    fn capability_matrix_matches_existing_product_behaviour() {
        let codex = ProviderId::Codex.capabilities();
        assert!(codex.skills && codex.branch && codex.session_settings && codex.duplicate);
        assert!(codex.vendor_management);

        for provider in [ProviderId::Gemini, ProviderId::Claude] {
            let caps = provider.capabilities();
            assert!(!caps.skills, "{provider} 不支持 Skill");
            assert!(caps.branch && caps.duplicate && caps.vendor_management);
            assert!(!caps.session_settings);
        }

        let qoder = ProviderId::Qoder.capabilities();
        assert!(!qoder.branch && !qoder.duplicate && !qoder.vendor_management);
        assert!(qoder.trash && qoder.batch_actions && qoder.export && qoder.usage);
    }

    #[test]
    fn provider_serializes_to_lowercase_short_name() {
        assert_eq!(
            serde_json::to_string(&ProviderId::Claude).unwrap(),
            "\"claude\""
        );
        assert_eq!(ProviderId::Claude.display_name(), "Claude Code");
    }

    #[test]
    fn platform_keeps_node_platform_strings() {
        assert_eq!(Platform::Win32.as_str(), "win32");
        assert_eq!(Platform::Darwin.as_str(), "darwin");
        assert_eq!(Platform::Linux.as_str(), "linux");
        assert!(Platform::Win32.is_windows());
        assert!(!Platform::Linux.is_windows());
    }

    #[test]
    fn target_labels_match_existing_wording() {
        assert_eq!(
            local_target_label(ProviderId::Codex, Platform::Win32),
            "本机（win32）"
        );
        assert_eq!(
            local_target_label(ProviderId::Claude, Platform::Linux),
            "Claude Code：本机（linux）"
        );
        assert_eq!(wsl_target_label(ProviderId::Codex, "Ubuntu"), "WSL：Ubuntu");
        assert_eq!(
            wsl_target_label(ProviderId::Gemini, "Ubuntu"),
            "Gemini：WSL：Ubuntu"
        );
    }

    #[test]
    fn target_details_match_existing_wording() {
        assert_eq!(codex_local_target_detail(true, true), "找到 Codex 会话");
        assert_eq!(codex_local_target_detail(false, true), "找到 Codex 命令");
        assert_eq!(codex_local_target_detail(false, false), "找到 Codex 目录");
        assert_eq!(
            codex_wsl_target_detail(true, true, "akim", "/home/akim"),
            "找到 Codex 会话（akim：/home/akim）"
        );
        // 缺少 whoami / HOME 时不追加括号内容。
        assert_eq!(
            codex_wsl_target_detail(true, true, "", ""),
            "找到 Codex 会话"
        );
        assert_eq!(
            cli_target_detail_found("Gemini", true),
            "找到 Gemini 命令和配置目录"
        );
        assert_eq!(
            cli_target_detail_config_only("Gemini", "gemini"),
            "找到 Gemini 配置目录，未找到 gemini 命令"
        );
    }

    #[test]
    fn target_serializes_with_camel_case_keys() {
        let target = Target {
            id: ProviderId::Codex.wsl_target_id("Ubuntu"),
            provider: ProviderId::Codex,
            label: "WSL：Ubuntu".into(),
            kind: TargetKind::Wsl,
            distro: Some("Ubuntu".into()),
            codex_home: Some("/home/akim/.codex".into()),
            available: true,
            detail: Some("找到 Codex 会话".into()),
        };
        let json = serde_json::to_value(&target).unwrap();
        assert_eq!(json["codexHome"], "/home/akim/.codex");
        assert_eq!(json["kind"], "wsl");
        assert!(json.get("codex_home").is_none());
        assert!(target.is_wsl());
        assert_eq!(target.distro(), Some("Ubuntu"));
    }
}
