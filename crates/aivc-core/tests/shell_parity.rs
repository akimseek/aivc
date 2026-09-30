//! shell 参数处理与目标编号解析的差分测试：与现网 TypeScript 实现逐值对拍。
//!
//! # 为什么这几组函数最需要对拍
//!
//! 它们都是**纯字符串变换**，而且边界极密：引号、空格、反斜杠、多字节字符、空串、
//! 允许列表之外的路径。这类函数的单测容易写成「我自己认为应该怎样」，而对拍能证明
//! 「与现网一致」——本项目里 `posix_shell_quote` 的实际行为就与我的第一直觉不同
//! （它给以 `-` 开头的值也加引号）。
//!
//! golden 由 `generate-shell-golden.mjs` 用现网的 `src/shared/shell-args.ts`、
//! `target-ids.ts`、`wsl-paths.ts` 生成，因此本测试离线。
//!
//! # 重新生成
//!
//! ```text
//! node --experimental-transform-types crates/aivc-core/tests/generate-shell-golden.mjs
//! ```
//!
//! ⚠ 重新生成后必须把 **Codex 的 WSL 编号两列**改回新契约（当前 golden 里已是新契约）：
//! 后端发出的编号是 `codex:wsl:<distro>`，旧平台造的是裸 `wsl:<distro>`、也认不出
//! 带前缀的那种——对齐旧平台会把这个缺陷带回来。说明见 `shell-golden.json` 的
//! `_comment` 与 `generate-shell-golden.mjs` 文件头。

// 集成测试是独立 crate，`clippy.toml` 的测试豁免只覆盖 `#[cfg(test)]` 模块。
#![allow(clippy::panic)]

use std::path::{Path, PathBuf};

use aivc_core::path::{
    is_inside_local_path, is_inside_posix_dir, sanitize_wsl_distro, wsl_mount_path_to_windows_path,
};
use aivc_core::shell::{
    build_cmd_command, build_posix_shell_command, cmd_quote, parse_cli_args, posix_shell_quote,
};
use aivc_core::target::ProviderId;
use serde::Deserialize;

/// 现网一次调用的结果：成功值或错误消息。
///
/// 「抛错」本身也是可比对的结果——两侧是否都不接受某个输入，正是需要验证的行为。
#[derive(Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
struct Outcome<T> {
    ok: bool,
    #[serde(default)]
    value: Option<T>,
    #[serde(default)]
    error: Option<String>,
}

/// 本实现的结果，与 [`Outcome`] 同形。
fn outcome<T>(result: Result<T, String>) -> Outcome<T> {
    match result {
        Ok(value) => Outcome {
            ok: true,
            value: Some(value),
            error: None,
        },
        Err(error) => Outcome {
            ok: false,
            value: None,
            error: Some(error),
        },
    }
}

#[derive(Debug, Deserialize)]
struct QuoteCase {
    value: String,
    posix: Outcome<String>,
    cmd: Outcome<String>,
    #[serde(rename = "wslShell")]
    wsl_shell: Outcome<String>,
}

#[derive(Debug, Deserialize)]
struct CommandCase {
    command: String,
    args: Vec<String>,
    posix: Outcome<String>,
    cmd: Outcome<String>,
}

#[derive(Debug, Deserialize)]
struct ParseCase {
    value: String,
    args: Outcome<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TargetCase {
    target_id: String,
    provider: Outcome<String>,
    distro: Outcome<String>,
    codex_distro: Outcome<String>,
}

#[derive(Debug, Deserialize)]
struct DistroCase {
    distro: String,
    sanitized: Outcome<String>,
    #[serde(rename = "builtCodex")]
    built_codex: Outcome<String>,
    #[serde(rename = "builtClaude")]
    built_claude: Outcome<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FenceCase {
    file_path: String,
    dir_path: String,
    posix: Outcome<bool>,
    local: Outcome<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MountCase {
    file_path: String,
    converted: Outcome<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Golden {
    quote_values: Vec<QuoteCase>,
    command_cases: Vec<CommandCase>,
    parse_inputs: Vec<ParseCase>,
    target_ids: Vec<TargetCase>,
    distro_names: Vec<DistroCase>,
    fences: Vec<FenceCase>,
    mount_paths: Vec<MountCase>,
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn golden() -> Golden {
    let path = fixtures().join("shell-golden.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 {} 失败：{error}", path.display()))
}

fn collect<T: PartialEq + std::fmt::Debug>(
    mismatches: &mut Vec<String>,
    label: &str,
    input: &str,
    expected: &Outcome<T>,
    actual: &Outcome<T>,
) {
    if expected != actual {
        mismatches.push(format!(
            "{label}（输入 {input:?}）\n  现网: {expected:?}\n  本实现: {actual:?}"
        ));
    }
}

#[test]
fn quotes_match_the_node_oracle() {
    let golden = golden();
    assert!(!golden.quote_values.is_empty(), "夹具为空");
    let mut mismatches = Vec::new();
    for case in &golden.quote_values {
        collect(
            &mut mismatches,
            "posixShellQuote",
            &case.value,
            &case.posix,
            &outcome(Ok(posix_shell_quote(&case.value))),
        );
        collect(
            &mut mismatches,
            "cmdQuote",
            &case.value,
            &case.cmd,
            &outcome(Ok(cmd_quote(&case.value))),
        );
        // 现网的 WSL 路径模块用的是同一套 POSIX 引号规则。
        collect(
            &mut mismatches,
            "wsl shellQuote",
            &case.value,
            &case.wsl_shell,
            &outcome(Ok(posix_shell_quote(&case.value))),
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 条引号输入不一致：\n\n{}",
        mismatches.len(),
        golden.quote_values.len() * 3,
        mismatches.join("\n\n")
    );
}

#[test]
fn command_building_matches_the_node_oracle() {
    let golden = golden();
    let mut mismatches = Vec::new();
    for case in &golden.command_cases {
        collect(
            &mut mismatches,
            "buildPosixShellCommand",
            &case.command,
            &case.posix,
            &outcome(Ok(build_posix_shell_command(&case.command, &case.args))),
        );
        collect(
            &mut mismatches,
            "buildCmdCommand",
            &case.command,
            &case.cmd,
            &outcome(Ok(build_cmd_command(&case.command, &case.args))),
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 条命令不一致：\n\n{}",
        mismatches.len(),
        golden.command_cases.len() * 2,
        mismatches.join("\n\n")
    );
}

#[test]
fn cli_argument_parsing_matches_the_node_oracle() {
    let golden = golden();
    let mut mismatches = Vec::new();
    for case in &golden.parse_inputs {
        let actual = match parse_cli_args(&case.value) {
            Ok(args) => outcome(Ok(args)),
            Err(error) => Outcome {
                ok: false,
                value: None,
                error: Some(error.to_string()),
            },
        };
        // 错误消息文本两侧措辞可能不同，因此只比较「是否成功」与成功时的值。
        let matches =
            case.args.ok == actual.ok && (!case.args.ok || case.args.value == actual.value);
        if !matches {
            mismatches.push(format!(
                "parseCliArgs（输入 {:?}）\n  现网: {:?}\n  本实现: {:?}",
                case.value, case.args, actual
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 条解析输入不一致：\n\n{}",
        mismatches.len(),
        golden.parse_inputs.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn target_id_parsing_matches_the_node_oracle() {
    let golden = golden();
    let mut mismatches = Vec::new();
    for case in &golden.target_ids {
        let provider = ProviderId::from_target_id(&case.target_id);
        let actual_provider = outcome(Ok(provider.as_str().to_owned()));
        collect(
            &mut mismatches,
            "getProviderIdFromTargetId",
            &case.target_id,
            &case.provider,
            &actual_provider,
        );

        // 现网的 `getWslDistroFromTargetId` 等价于「本平台的 WSL 发行版」，
        // 而 `getWslDistroFromProviderTarget("codex", …)` 是「Codex 视角的发行版」。
        let codex = ProviderId::Codex;
        collect(
            &mut mismatches,
            "getWslDistroFromProviderTarget(codex)",
            &case.target_id,
            &case.codex_distro,
            &outcome(Ok(codex.wsl_distro_of(&case.target_id).to_owned())),
        );
        // `getWslDistroFromTargetId` 先按前缀认出平台再取发行版。
        collect(
            &mut mismatches,
            "getWslDistroFromTargetId",
            &case.target_id,
            &case.distro,
            &outcome(Ok(provider.wsl_distro_of(&case.target_id).to_owned())),
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} 条目标编号不一致：\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn distro_sanitizing_and_building_match_the_node_oracle() {
    let golden = golden();
    let mut mismatches = Vec::new();
    for case in &golden.distro_names {
        collect(
            &mut mismatches,
            "sanitizeWslDistro",
            &case.distro,
            &case.sanitized,
            &outcome(Ok(sanitize_wsl_distro(&case.distro))),
        );
        let codex = outcome(Ok(ProviderId::Codex
            .wsl_target_id(&case.distro)
            .as_str()
            .to_owned()));
        collect(
            &mut mismatches,
            "buildWslProviderTargetId(codex)",
            &case.distro,
            &case.built_codex,
            &codex,
        );
        let claude = outcome(Ok(ProviderId::Claude
            .wsl_target_id(&case.distro)
            .as_str()
            .to_owned()));
        collect(
            &mut mismatches,
            "buildWslProviderTargetId(claude)",
            &case.distro,
            &case.built_claude,
            &claude,
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} 条发行版不一致：\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn the_oracle_covers_awkward_inputs() {
    // 防止夹具退化成「全是简单输入」——那样对拍会假通过。
    let golden = golden();
    let has_awkward = |values: &[String]| {
        values.iter().any(|value| {
            value.is_empty()
                || value.contains('\'')
                || value.contains('"')
                || value.contains('\\')
                || value.contains(' ')
                || !value.is_ascii()
        })
    };
    assert!(
        has_awkward(
            &golden
                .quote_values
                .iter()
                .map(|c| c.value.clone())
                .collect::<Vec<_>>()
        ),
        "引号输入里没有边界值"
    );
    assert!(
        golden
            .parse_inputs
            .iter()
            .any(|case| !case.args.ok || case.args.value.as_ref().is_some_and(|v| v.len() > 1)),
        "解析输入里没有值得一提的用例"
    );
    // 注意：`wsl_mount_path_to_windows_path` 与 `is_inside_posix_dir` 目前**没有**
    // 纳入本对拍——它们的输入不是简单字符串，需要单独设计场景。这里不写「顺手断言」：
    // 未经 oracle 验证的断言正是对拍要取代的东西。
}

#[test]
fn path_fencing_matches_the_node_oracle() {
    // 这是**安全关键**的行为：围栏判错会让「拒绝把 sessions 目录之外的文件移入回收站」
    // 这道防线失效。180 对（文件 × 目录）覆盖了遍历、前缀欺骗（sessions-evil）与
    // 相对片段（..、.）。
    let golden = golden();
    assert!(!golden.fences.is_empty(), "夹具里没有围栏用例");
    let mut mismatches = Vec::new();
    for case in &golden.fences {
        let label = format!("{} in {}", case.file_path, case.dir_path);
        collect(
            &mut mismatches,
            "isInsidePosixDir",
            &label,
            &case.posix,
            &outcome(Ok(is_inside_posix_dir(&case.file_path, &case.dir_path))),
        );
        collect(
            &mut mismatches,
            "isInsidePath",
            &label,
            &case.local,
            &outcome(Ok(is_inside_local_path(&case.file_path, &case.dir_path))),
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 条围栏判定不一致：\n\n{}",
        mismatches.len(),
        golden.fences.len() * 2,
        mismatches.join("\n\n")
    );
}

#[test]
fn mount_path_conversion_matches_the_node_oracle() {
    let golden = golden();
    let mut mismatches = Vec::new();
    for case in &golden.mount_paths {
        collect(
            &mut mismatches,
            "wslMountPathToWindowsPath",
            &case.file_path,
            &case.converted,
            &outcome(Ok(wsl_mount_path_to_windows_path(&case.file_path))),
        );
    }
    assert!(
        mismatches.is_empty(),
        "{} 条挂载路径不一致：\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn the_fence_fixture_covers_traversal_attempts() {
    // 防止围栏夹具退化成「全是合法路径」——那样对拍会假通过。
    let golden = golden();
    assert!(
        golden
            .fences
            .iter()
            .any(|case| case.file_path.contains("..")),
        "围栏夹具里没有 `..` 遍历尝试"
    );
    assert!(
        golden
            .fences
            .iter()
            .any(|case| case.file_path.contains("sessions-evil")),
        "围栏夹具里没有前缀欺骗（sessions-evil）"
    );
    assert!(
        golden
            .fences
            .iter()
            .any(|case| case.posix.value == Some(false)),
        "围栏夹具里没有「应当拒绝」的用例"
    );
    assert!(
        golden
            .fences
            .iter()
            .any(|case| case.posix.value == Some(true)),
        "围栏夹具里没有「应当接受」的用例"
    );
}
