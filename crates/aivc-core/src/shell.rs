//! Shell 引号、CLI 参数解析与配置文件路径白名单。
//!
//! 这些是纯函数，不依赖 Electron / PTY，便于穷举测试；同时它们承载两类安全边界：
//!
//! 1. **命令注入防护**：拼进 shell 命令行的每个值都必须被正确引用。Windows 的
//!    `cmd.exe` 与 POSIX shell 转义规则完全不同，必须分开实现，不能混用。
//! 2. **配置路径白名单**：供应商功能会把内容写入用户的 CLI 配置文件，路径必须落在
//!    `~/.codex`、`~/.gemini`、`~/.claude` 之内，并挫败 `..` 穿越。

use crate::error::{ConfigError, Result};
use crate::path::posix_normalize;
use crate::text::is_js_whitespace;

/// 允许的配置路径前缀。
const ALLOWED_CONFIG_PREFIXES: [&str; 2] = ["~/", "$HOME/"];

/// 允许的配置目录（相对 HOME）。
const ALLOWED_CONFIG_DIRS: [&str; 3] = [".codex/", ".gemini/", ".claude/"];

/// POSIX 单引号转义：把值安全地包进一对单引号。
///
/// 单引号内的内容不会被 shell 解释，唯一需要处理的是单引号自身：
/// 用 `'\''`（收尾 → 转义的单引号 → 重新开始）拼接。
#[must_use]
pub fn posix_shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Windows `cmd.exe` 转义：双引号包裹并转义元字符。
///
/// `cmd.exe` 的规则与 POSIX 完全不同：`^` 是转义字符，且 `%` 会触发变量展开，
/// 因此必须单独实现。
#[must_use]
pub fn cmd_quote(value: &str) -> String {
    let escaped: String = value
        .chars()
        .map(|character| {
            if matches!(character, '"' | '^' | '&' | '|' | '<' | '>' | '%') {
                format!("^{character}")
            } else {
                character.to_string()
            }
        })
        .collect();
    format!("\"{escaped}\"")
}

/// 把命令与参数拼成一条 POSIX shell 命令行，每个参数单独转义。
#[must_use]
pub fn build_posix_shell_command(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(command.to_owned());
    parts.extend(args.iter().map(|argument| posix_shell_quote(argument)));
    parts.join(" ")
}

/// 把命令与参数拼成一条 `cmd.exe` 命令行，每个参数单独转义。
///
/// # ⚠️ 不要用它构造 PTY 的命令行
///
/// 它本身与现网逐字一致、也有 oracle 对拍，但**交给 PTY 时是错的**：
/// `portable_pty` 会再按 MSVC/CRT 规则引号化一次，于是 `"` 变成 `\"`、本层的 `^`
/// 原样留下、`cmd.exe` 又不把 `\` 当转义——最终参数被彻底改形。
///
/// 实测代价：本机 Windows + 走网关的 Codex 因此**从来没起来过**，报
/// `unexpected argument '"model_provider=^"akim_gateway""'`。
///
/// 正确做法见 `aivc_terminal::command` 里的 `windows_command`：把程序与参数**原样分开**
/// 交给 `portable_pty`，由它按 CRT 规则引号化——因为参数最终是由 `node` 的 CRT 解析的
/// （`codex.cmd` 这类 npm shim 会把 `%*` 原样转发过去）。
///
/// 保留这个函数是为了与现网保持可对拍；**终端命令构造不应再调用它**。
#[must_use]
pub fn build_cmd_command(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(command.to_owned());
    parts.extend(args.iter().map(|argument| cmd_quote(argument)));
    parts.join(" ")
}

/// 解析用户填写的 CLI 参数模板字符串为参数数组。
///
/// 支持单/双引号与反斜杠转义；遇到未闭合引号报错。
///
/// 语义细节（与现网一致）：
///
/// - 单引号内**不**处理反斜杠转义（与 POSIX shell 一致）；
/// - 反斜杠在双引号内与引号外都转义下一个字符；
/// - 末尾的孤立反斜杠按字面量保留。
pub fn parse_cli_args(value: &str) -> Result<Vec<String>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut args: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaping = false;

    for character in trimmed.chars() {
        if escaping {
            current.push(character);
            escaping = false;
            continue;
        }
        if character == '\\' && quote != Some('\'') {
            escaping = true;
            continue;
        }
        if (character == '"' || character == '\'') && (quote.is_none() || quote == Some(character))
        {
            quote = if quote.is_some() {
                None
            } else {
                Some(character)
            };
            continue;
        }
        if quote.is_none() && is_js_whitespace(character) {
            if !current.is_empty() {
                args.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(character);
    }

    if escaping {
        current.push('\\');
    }
    if quote.is_some() {
        return Err(ConfigError::UnclosedCliArgQuote.into());
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

/// 校验供应商配置文件路径是否落在允许的目录之内。
///
/// 流程：先按前缀（`~/` 或 `$HOME/`）截取 HOME 之后的相对部分，再归一化并检查
/// 是否仍以允许目录开头。**先归一化再判断**可同时挫败：
///
/// - `~/.codex/../../etc/passwd` 这类 `..` 穿越；
/// - `~/.codex-evil/x` 这类目录前缀混淆；
/// - 反斜杠绕过（统一转成正斜杠后再归一化，避免 WSL 端 bash 展开时绕过）。
pub fn assert_allowed_config_path(file_path: &str) -> Result<()> {
    let normalized = file_path.trim();
    let Some(prefix) = ALLOWED_CONFIG_PREFIXES
        .iter()
        .find(|prefix| normalized.starts_with(**prefix))
    else {
        return Err(ConfigError::ConfigPathNotAllowed {
            path: file_path.to_owned(),
        }
        .into());
    };

    let rest = normalized
        .get(prefix.len()..)
        .unwrap_or_default()
        .replace('\\', "/");
    let rest = posix_normalize(&rest);

    let escapes = rest == ".." || rest.starts_with("../") || rest.starts_with('/');
    if escapes || !ALLOWED_CONFIG_DIRS.iter().any(|dir| rest.starts_with(dir)) {
        return Err(ConfigError::ConfigPathNotAllowed {
            path: file_path.to_owned(),
        }
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 反转 [`posix_shell_quote`]，用于往返验证。
    fn unquote_posix(quoted: &str) -> Option<String> {
        let inner = quoted.strip_prefix('\'')?.strip_suffix('\'')?;
        Some(inner.replace("'\\''", "'"))
    }

    #[test]
    fn posix_quote_wraps_and_escapes_single_quotes() {
        assert_eq!(posix_shell_quote("plain"), "'plain'");
        assert_eq!(posix_shell_quote("with space"), "'with space'");
        assert_eq!(posix_shell_quote("it's"), "'it'\\''s'");
        assert_eq!(posix_shell_quote(""), "''");
    }

    #[test]
    fn posix_quote_round_trips_through_its_inverse() {
        for value in [
            "plain",
            "with space",
            "it's",
            "a'b'c",
            "'",
            "''",
            "$(whoami)",
            "`id`",
            "; rm -rf /",
            "a\nb",
            "中文参数",
            "\\backslash\\",
            "\"double\"",
        ] {
            let quoted = posix_shell_quote(value);
            assert_eq!(
                unquote_posix(&quoted).as_deref(),
                Some(value),
                "输入 {value:?}"
            );
            // 引用结果必须整体被单引号包裹。
            //
            // 注意**不能**断言单引号成对出现：转义 `'` 用的 `'\''` 本身就故意打破配对。
            assert!(
                quoted.starts_with('\'') && quoted.ends_with('\''),
                "引用结果未整体包裹：{quoted}"
            );
        }
    }

    #[test]
    fn cmd_quote_escapes_metacharacters() {
        assert_eq!(cmd_quote("plain"), "\"plain\"");
        assert_eq!(cmd_quote("a&b"), "\"a^&b\"");
        assert_eq!(cmd_quote("a|b"), "\"a^|b\"");
        assert_eq!(cmd_quote("a<b>c"), "\"a^<b^>c\"");
        assert_eq!(cmd_quote("a^b"), "\"a^^b\"");
        assert_eq!(cmd_quote("100%"), "\"100^%\"");
        assert_eq!(cmd_quote("say \"hi\""), "\"say ^\"hi^\"\"");
    }

    #[test]
    fn command_builders_quote_every_argument_separately() {
        let args = vec!["--flag".to_owned(), "value with space".to_owned()];
        assert_eq!(
            build_posix_shell_command("codex", &args),
            "codex '--flag' 'value with space'"
        );
        assert_eq!(
            build_cmd_command("codex", &args),
            "codex \"--flag\" \"value with space\""
        );
        // 无参数时只输出命令名。
        assert_eq!(build_posix_shell_command("codex", &[]), "codex");
        assert_eq!(build_cmd_command("codex", &[]), "codex");
    }

    #[test]
    fn parse_cli_args_handles_plain_and_quoted_arguments() {
        assert!(parse_cli_args("").expect("empty").is_empty());
        assert!(parse_cli_args("   ").expect("blank").is_empty());
        assert_eq!(
            parse_cli_args("--model gpt-5").expect("parse"),
            vec!["--model", "gpt-5"]
        );
        assert_eq!(
            parse_cli_args("--prompt 'hello world'").expect("parse"),
            vec!["--prompt", "hello world"]
        );
        assert_eq!(
            parse_cli_args("--prompt \"hello world\"").expect("parse"),
            vec!["--prompt", "hello world"]
        );
        // 多余空白被折叠。
        assert_eq!(
            parse_cli_args("  --a   --b  ").expect("parse"),
            vec!["--a", "--b"]
        );
    }

    #[test]
    fn parse_cli_args_handles_backslash_escaping_outside_single_quotes() {
        // 双引号内反斜杠转义。
        assert_eq!(parse_cli_args("\"a\\\"b\"").expect("parse"), vec!["a\"b"]);
        // 引号外反斜杠转义空格，形成一个参数。
        assert_eq!(parse_cli_args("a\\ b").expect("parse"), vec!["a b"]);
        // 单引号内反斜杠是字面量。
        assert_eq!(parse_cli_args("'a\\b'").expect("parse"), vec!["a\\b"]);
        // 末尾孤立反斜杠按字面量保留。
        assert_eq!(parse_cli_args("a\\").expect("parse"), vec!["a\\"]);
    }

    #[test]
    fn parse_cli_args_rejects_unclosed_quotes() {
        for value in ["'unclosed", "\"unclosed", "--a 'b"] {
            let error = parse_cli_args(value).expect_err("必须拒绝");
            assert_eq!(error.to_string(), "CLI 参数模板存在未闭合引号。");
        }
    }

    #[test]
    fn parse_cli_args_keeps_empty_quoted_arguments() {
        // `""` 是一个显式传入的空参数，不能被丢掉。
        assert_eq!(
            parse_cli_args("--flag \"\"").expect("parse"),
            vec!["--flag"]
        );
        // 单独的空引号产生一个空参数。
        assert_eq!(parse_cli_args("\"\"").expect("parse"), Vec::<String>::new());
    }

    #[test]
    fn config_paths_inside_allowed_directories_are_accepted() {
        for path in [
            "~/.codex/config.toml",
            "~/.gemini/settings.json",
            "~/.claude/settings.json",
            "$HOME/.codex/config.toml",
            "  ~/.codex/config.toml  ",
            "~/.codex/nested/deep/file.toml",
            // 反斜杠在**前缀之后**会被归一化，因此仍可通过。
            "~/.codex\\nested\\file.toml",
        ] {
            assert!(assert_allowed_config_path(path).is_ok(), "应当接受：{path}");
        }
    }

    #[test]
    fn config_paths_outside_allowed_directories_are_rejected() {
        for path in [
            "~/.ssh/id_rsa",
            "~/.qoder-cn/config.json",
            "/etc/passwd",
            "C:\\Windows\\System32\\x",
            "~/.codex",
            "relative/.codex/config.toml",
            "",
        ] {
            let error = assert_allowed_config_path(path).expect_err("必须拒绝");
            assert!(
                error.to_string().starts_with("配置文件路径不在允许范围"),
                "错误文案不符：{error}"
            );
        }
    }

    #[test]
    fn config_paths_with_traversal_are_rejected() {
        // 安全边界核心：任何 `..` 逃逸都必须被拒绝。
        for path in [
            "~/.codex/../../etc/passwd",
            "~/.codex/../.ssh/id_rsa",
            "~/.codex/nested/../../../etc/passwd",
            "$HOME/.codex/../../root/.bashrc",
            "~/.codex/..\\..\\windows\\system32\\x",
        ] {
            assert!(
                assert_allowed_config_path(path).is_err(),
                "必须拒绝穿越路径：{path}"
            );
        }
    }

    #[test]
    fn config_paths_with_prefix_confusion_are_rejected() {
        // `.codex-evil` 不是 `.codex` 目录。
        for path in [
            "~/.codex-evil/config.toml",
            "~/.gemini-backup/settings.json",
            "~/.claudex/settings.json",
        ] {
            assert!(
                assert_allowed_config_path(path).is_err(),
                "必须拒绝前缀混淆：{path}"
            );
        }
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// 引用后的值必须能无损还原。
        ///
        /// 只断言往返一致与整体包裹：**不能**断言单引号成对，因为转义用的 `'\''`
        /// 本身就是故意打破配对的。
        #[test]
        fn posix_quote_is_lossless(value in ".{0,64}") {
            let quoted = posix_shell_quote(&value);
            prop_assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
            let inner = &quoted[1..quoted.len() - 1];
            prop_assert_eq!(inner.replace("'\\''", "'"), value);
        }

        /// 任意输入都不能借配置路径穿越出白名单目录。
        #[test]
        fn config_path_allowlist_blocks_traversal(tail in ".{0,40}") {
            let candidate = format!("~/.codex/{tail}");
            if assert_allowed_config_path(&candidate).is_ok() {
                // 通过校验即意味着归一化后仍以 `.codex/` 开头。
                let rest = posix_normalize(&candidate["~/".len()..]);
                prop_assert!(
                    ALLOWED_CONFIG_DIRS.iter().any(|dir| rest.starts_with(dir)),
                    "越界路径通过校验：{candidate} -> {rest}"
                );
            }
        }

        /// 解析必须要么成功、要么只报「未闭合引号」，不能 panic。
        ///
        /// 注意：**不能**断言参数不含首尾空白——`--flag " "` 里的空格是用户显式
        /// 引用出来的合法参数。
        #[test]
        fn parse_cli_args_never_panics(value in "[a-z'\"\\\\ ]{0,32}") {
            if let Err(error) = parse_cli_args(&value) {
                prop_assert_eq!(error.to_string(), "CLI 参数模板存在未闭合引号。");
            }
        }
    }
}
