//! CLI 环境探测的纯逻辑：版本解析、最低版本要求、提示文案、输出合并。
//!
//! # 为什么先做这部分
//!
//! 真正的探测要启动 `node --version` / `npm --version` / `nvm --version` 等外部命令，
//! 还要分本机与 WSL 两条路径。但其中**判定**是纯的，而且决定了用户看到什么：
//!
//! - 最低 Node 版本按平台不同（Codex 要 22，Gemini/Qoder 要 20，Claude 要 18）；
//! - 提示文案要能组合出「缺 Node 但可用 nvm 安装」这类状态；
//! - 版本号解析要容忍 `v` 前缀与前后空白。
//!
//! 这些都能穷举测试，且不需要真的装一个 Node 环境。
//!
//! # 文案是契约
//!
//! 全部与现网 `cli-installer.ts` 逐字一致（含全角冒号与句末句号）——它们直接显示给用户。

use aivc_core::target::ProviderId;

/// 推荐的 Node 主版本。
pub const RECOMMENDED_NODE_MAJOR: u32 = 24;

/// 安装命令的超时（15 分钟）。
///
/// 全局安装要下载依赖，慢的时候可能几分钟。给足时间但**必须有上限**：
/// 没有超时的话，一次卡住的安装会让界面永远停在「安装中」。
pub const INSTALL_TIMEOUT_MS: u64 = 1000 * 60 * 15;

/// 某个平台要求的最低 Node 主版本。
///
/// 各平台 CLI 的引擎要求不同，因此这个值不能统一。
#[must_use]
pub const fn minimum_node_major(provider: ProviderId) -> u32 {
    match provider {
        ProviderId::Codex => 22,
        // Gemini 与 Qoder 的要求相同，但不合并成一个分支——它们将来可能分叉，
        // 而现在写清楚每个平台的值比省两行更重要。
        ProviderId::Gemini | ProviderId::Qoder => 20,
        ProviderId::Claude => 18,
    }
}

/// 某个平台的 CLI 版本在环境快照里的键名。
#[must_use]
pub const fn cli_version_key(provider: ProviderId) -> &'static str {
    match provider {
        ProviderId::Codex => "CODEX_VERSION",
        ProviderId::Gemini => "GEMINI_VERSION",
        ProviderId::Qoder => "QODER_VERSION",
        ProviderId::Claude => "CLAUDE_VERSION",
    }
}

/// 从版本字符串里解析主版本号。
///
/// 容忍 `v` 前缀与前后空白：`v22.11.0`、`22.11.0`、` 20 ` 都能解析。
/// 解析不出数字时返回 [`None`]——**不返回 0**，因为 0 会被当成「版本太低」
/// 而不是「没检测到」。
#[must_use]
pub fn parse_node_major(version: &str) -> Option<u32> {
    let trimmed = version.trim();
    let digits: String = trimmed
        .strip_prefix('v')
        .unwrap_or(trimmed)
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Node 是否由 nvm 管理。
///
/// 判断依据是**路径**：nvm 会把 Node 装在 `~/.nvm/versions/node/...` 下。
/// 这决定了界面要不要提示「可通过 nvm 安装推荐版本」。
#[must_use]
pub fn node_managed_by_nvm(node_path: &str) -> bool {
    if node_path.is_empty() {
        return false;
    }
    let lower = node_path.to_ascii_lowercase();
    ["/nvm/", "\\nvm\\", ".nvm/", ".nvm\\"]
        .iter()
        .any(|marker| lower.contains(marker))
}

/// 环境提示。
///
/// # 为什么允许四个 bool
///
/// 这四个是**互相独立的事实**，不是可以归并的状态机：可能装了 Node 但没装 npm，
/// 也可能两者都有但没有 nvm。把它们合成一个枚举会得到 2^4 种组合中的大量无意义分支，
/// 反而不如四个 bool 直白。
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct EnvironmentFacts {
    /// 是否检测到 Node。
    pub has_node: bool,
    /// 是否检测到 npm。
    pub has_npm: bool,
    /// 是否检测到 nvm。
    pub has_nvm: bool,
    /// 是否可以用 nvm 安装 Node。
    pub can_install_node_with_nvm: bool,
    /// 探测过程中的错误（空串表示没有）。
    ///
    /// 用借用而不是 `&'static str`：错误文案是**运行时构造**的
    /// （「命令超时：`NODE_VERSION`」这类），静态字符串表达不了它。
    pub detection_error: String,
}

/// 按顺序构造环境提示。
///
/// 顺序是契约：用户从上往下读，越靠前越该先处理。检测失败排第一，因为后面的
/// 「未检测到 X」可能只是失败的结果而不是真实缺失。
#[must_use]
pub fn build_environment_messages(facts: &EnvironmentFacts) -> Vec<String> {
    let mut messages = Vec::new();
    if !facts.detection_error.is_empty() {
        messages.push(format!("检测失败：{}", facts.detection_error));
    }
    if !facts.has_node {
        messages.push("未检测到 Node.js。".to_owned());
    }
    if !facts.has_npm {
        messages.push("未检测到 npm。".to_owned());
    }
    if (!facts.has_node || !facts.has_npm) && !facts.has_nvm {
        messages.push("未检测到 nvm，请先安装 nvm 后再由应用安装 Node.js。".to_owned());
    }
    if facts.can_install_node_with_nvm && (!facts.has_node || !facts.has_npm) {
        messages.push("可通过 nvm 安装推荐 Node.js 版本后继续安装 CLI。".to_owned());
    }
    messages
}

/// 一次命令的输出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandOutput {
    /// 标准输出。
    pub stdout: String,
    /// 标准错误。
    pub stderr: String,
}

impl CommandOutput {
    /// 构造。
    #[must_use]
    pub fn new(stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }
}

/// 把一次命令的两路输出合并成单段文本。
///
/// 丢弃空白段：外部命令经常往 stderr 写一行空内容，保留它会在快照里留下空行，
/// 让后续按行解析拿到噪声。
#[must_use]
pub fn merge_output(output: &CommandOutput) -> String {
    [output.stdout.trim(), output.stderr.trim()]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// 合并多段命令的同一路输出。
///
/// 用空行分隔：不同命令的输出属于不同的语义块，粘在一起会读不出来。
#[must_use]
pub fn merge_streams<'a, I>(outputs: I, use_stderr: bool) -> String
where
    I: IntoIterator<Item = &'a CommandOutput>,
{
    outputs
        .into_iter()
        .map(|output| {
            if use_stderr {
                output.stderr.trim()
            } else {
                output.stdout.trim()
            }
        })
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// 构造安装失败的错误文案。
///
/// 优先 stderr、其次 stdout：安装失败的**原因**几乎总在 stderr 上，而 stdout 多是
/// 进度输出。跨 IPC 只有 `message` 会保留，自定义属性会被丢弃，因此真实原因必须
/// 写进文案本身——否则用户只看到「安装失败」而不知道哪里失败。
#[must_use]
pub fn build_install_error(
    failed: Option<&CommandOutput>,
    prior: &[CommandOutput],
    fallback: &str,
) -> String {
    if let Some(output) = failed {
        let stderr = output.stderr.trim();
        if !stderr.is_empty() {
            return stderr.to_owned();
        }
        let stdout = output.stdout.trim();
        if !stdout.is_empty() {
            return stdout.to_owned();
        }
    }
    // 失败命令自己没有输出时，退回之前几段命令的输出——那里面常有线索
    // （例如 `nvm install` 成功了但 `npm install -g` 失败）。
    let merged = merge_streams(prior.iter(), true);
    if !merged.is_empty() {
        return merged;
    }
    let merged = merge_streams(prior.iter(), false);
    if !merged.is_empty() {
        return merged;
    }
    fallback.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_platform_has_its_own_minimum_node_major() {
        // 各平台 CLI 的引擎要求不同，统一成一个值会让某一方装不上或误判为满足。
        assert_eq!(minimum_node_major(ProviderId::Codex), 22);
        assert_eq!(minimum_node_major(ProviderId::Gemini), 20);
        assert_eq!(minimum_node_major(ProviderId::Qoder), 20);
        assert_eq!(minimum_node_major(ProviderId::Claude), 18);
        // 推荐版本要高于所有最低要求，否则「推荐安装」会装出一个不满足的平台。
        for provider in [
            ProviderId::Codex,
            ProviderId::Gemini,
            ProviderId::Qoder,
            ProviderId::Claude,
        ] {
            assert!(
                RECOMMENDED_NODE_MAJOR >= minimum_node_major(provider),
                "{provider:?} 的推荐版本低于最低要求"
            );
        }
    }

    #[test]
    fn the_version_keys_are_distinct() {
        let keys = [
            cli_version_key(ProviderId::Codex),
            cli_version_key(ProviderId::Gemini),
            cli_version_key(ProviderId::Qoder),
            cli_version_key(ProviderId::Claude),
        ];
        let unique: std::collections::BTreeSet<&&str> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len(), "版本键重复会让快照互相覆盖");
        assert_eq!(cli_version_key(ProviderId::Codex), "CODEX_VERSION");
    }

    #[test]
    fn version_parsing_tolerates_the_v_prefix_and_whitespace() {
        assert_eq!(parse_node_major("v22.11.0"), Some(22));
        assert_eq!(parse_node_major("22.11.0"), Some(22));
        assert_eq!(parse_node_major("  20  "), Some(20));
        assert_eq!(parse_node_major("v18"), Some(18));
        assert_eq!(parse_node_major("24"), Some(24));
    }

    #[test]
    fn an_unparseable_version_is_none_not_zero() {
        // 返回 0 会被当成「版本太低」，而真实情况是「没检测到」——
        // 两者的处理方式完全不同。
        for value in ["", "   ", "not a version", "v", "x22"] {
            assert_eq!(parse_node_major(value), None, "{value:?} 应当解析失败");
        }
    }

    #[test]
    fn nvm_managed_paths_are_recognized_on_both_platforms() {
        assert!(node_managed_by_nvm(
            "/home/u/.nvm/versions/node/v22/bin/node"
        ));
        assert!(node_managed_by_nvm(r"C:\Users\u\.nvm\v22\node.exe"));
        assert!(node_managed_by_nvm(
            "/usr/local/nvm/versions/node/v20/bin/node"
        ));
        assert!(node_managed_by_nvm("/NVM/versions/node/v22/bin/node"));
        // 系统安装的 Node 不算 nvm 管理。
        assert!(!node_managed_by_nvm("/usr/bin/node"));
        assert!(!node_managed_by_nvm(r"C:\Program Files\nodejs\node.exe"));
        assert!(!node_managed_by_nvm(""));
    }

    #[test]
    fn a_healthy_environment_produces_no_messages() {
        let messages = build_environment_messages(&EnvironmentFacts {
            has_node: true,
            has_npm: true,
            has_nvm: true,
            can_install_node_with_nvm: true,
            detection_error: String::new(),
        });
        assert!(messages.is_empty(), "一切正常时不该有提示：{messages:?}");
    }

    #[test]
    fn a_detection_error_is_reported_first() {
        // 检测失败时后面的「未检测到 X」可能只是失败的结果，因此失败信息要排第一。
        let messages = build_environment_messages(&EnvironmentFacts {
            has_node: false,
            has_npm: false,
            has_nvm: false,
            can_install_node_with_nvm: false,
            detection_error: "命令超时".to_owned(),
        });
        assert_eq!(messages[0], "检测失败：命令超时");
        assert!(messages.contains(&"未检测到 Node.js。".to_owned()));
        assert!(messages.contains(&"未检测到 npm。".to_owned()));
        assert!(
            messages.contains(&"未检测到 nvm，请先安装 nvm 后再由应用安装 Node.js。".to_owned())
        );
    }

    #[test]
    fn a_missing_node_with_nvm_suggests_installing_it() {
        let messages = build_environment_messages(&EnvironmentFacts {
            has_node: false,
            has_npm: true,
            has_nvm: true,
            can_install_node_with_nvm: true,
            detection_error: String::new(),
        });
        assert_eq!(
            messages,
            [
                "未检测到 Node.js。",
                "可通过 nvm 安装推荐 Node.js 版本后继续安装 CLI。",
            ]
        );
        // 有 nvm 时不该提示「请先安装 nvm」。
        assert!(
            !messages
                .iter()
                .any(|message| message.contains("请先安装 nvm"))
        );
    }

    #[test]
    fn a_missing_npm_with_nvm_suggests_installing_node() {
        let messages = build_environment_messages(&EnvironmentFacts {
            has_node: true,
            has_npm: false,
            has_nvm: true,
            can_install_node_with_nvm: true,
            detection_error: String::new(),
        });
        assert_eq!(
            messages,
            [
                "未检测到 npm。",
                "可通过 nvm 安装推荐 Node.js 版本后继续安装 CLI。",
            ]
        );
    }

    #[test]
    fn without_nvm_the_user_is_told_to_install_it_first() {
        let messages = build_environment_messages(&EnvironmentFacts {
            has_node: false,
            has_npm: false,
            has_nvm: false,
            can_install_node_with_nvm: false,
            detection_error: String::new(),
        });
        assert_eq!(
            messages,
            vec![
                "未检测到 Node.js。",
                "未检测到 npm。",
                "未检测到 nvm，请先安装 nvm 后再由应用安装 Node.js。",
            ]
        );
    }

    #[test]
    fn merge_output_drops_blank_streams() {
        assert_eq!(merge_output(&CommandOutput::new("out", "err")), "out\nerr");
        assert_eq!(merge_output(&CommandOutput::new("out", "   ")), "out");
        assert_eq!(merge_output(&CommandOutput::new("  ", "err")), "err");
        assert_eq!(merge_output(&CommandOutput::new("", "")), "");
        // 两路都有内容时按 stdout 在前、stderr 在后。
        assert_eq!(merge_output(&CommandOutput::new("  a  ", "  b  ")), "a\nb");
    }

    #[test]
    fn merge_streams_separates_blocks_with_a_blank_line() {
        let outputs = [
            CommandOutput::new("one", ""),
            CommandOutput::new("", ""),
            CommandOutput::new("two", ""),
        ];
        assert_eq!(merge_streams(outputs.iter(), false), "one\n\ntwo");
        // 空白段被丢弃，不会留下多余空行。
        let outputs = [CommandOutput::new("one", ""), CommandOutput::new("   ", "")];
        assert_eq!(merge_streams(outputs.iter(), false), "one");
    }

    #[test]
    fn merge_streams_can_pick_the_other_stream() {
        let outputs = [CommandOutput::new("out", "err")];
        assert_eq!(merge_streams(outputs.iter(), true), "err");
        assert_eq!(merge_streams(outputs.iter(), false), "out");
    }

    #[test]
    fn the_install_error_prefers_stderr() {
        // 安装失败的原因几乎总在 stderr 上。
        let failed = CommandOutput::new("进度输出", "真正的错误");
        assert_eq!(
            build_install_error(Some(&failed), &[], "安装失败"),
            "真正的错误"
        );
    }

    #[test]
    fn the_install_error_falls_back_to_stdout_then_prior_output() {
        let failed = CommandOutput::new("只有 stdout", "");
        assert_eq!(
            build_install_error(Some(&failed), &[], "安装失败"),
            "只有 stdout"
        );
        // 失败命令自己没有输出时，退回之前几段——那里面常有线索。
        let prior = [CommandOutput::new("", "之前的错误")];
        let empty = CommandOutput::new("", "");
        assert_eq!(
            build_install_error(Some(&empty), &prior, "安装失败"),
            "之前的错误"
        );
    }

    #[test]
    fn the_install_error_falls_back_to_the_caller_message() {
        // 完全没有输出时也不能给一个空文案。
        assert_eq!(build_install_error(None, &[], "安装失败"), "安装失败");
        let empty = CommandOutput::new("", "");
        assert_eq!(
            build_install_error(Some(&empty), &[], "安装失败"),
            "安装失败"
        );
    }

    #[test]
    fn the_install_timeout_is_fifteen_minutes() {
        // 必须有上限：没有超时的话，一次卡住的安装会让界面永远停在「安装中」。
        assert_eq!(INSTALL_TIMEOUT_MS, 900_000);
    }
}
