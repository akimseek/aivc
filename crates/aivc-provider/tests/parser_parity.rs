//! Codex 会话解析的差分测试：与现网 TypeScript 实现逐字段对拍。
//!
//! # 为什么这里比终端状态检测器更值得对拍
//!
//! 解析器的边界最多：非法 JSON、合成块过滤、CRLF、缺字段、多文本块拼接、标题截断……
//! 每一条都可能因为「我理解错了一个细节」而与现网产生细微差异，而单测只能验证
//! 「我实现的是我以为的规则」。
//!
//! golden 由 `generate-parser-golden.mjs` 用现网的 `src/shared/session-parser.ts` 生成，
//! 因此本测试**离线**：不需要 Node，也不需要现网源码。
//!
//! # 重新生成
//!
//! ```text
//! node --experimental-transform-types crates/aivc-provider/tests/generate-parser-golden.mjs
//! ```

// 集成测试是独立 crate，`clippy.toml` 的测试豁免只覆盖 `#[cfg(test)]` 模块。
// 这里的 panic 就是测试失败的表达方式。
#![allow(clippy::panic)]

use std::path::{Path, PathBuf};

use aivc_provider::codex::parser::{ParseMode, parse_content};
use aivc_provider::common::SessionFileInfo;
use serde::Deserialize;

/// 固定文件元信息，与生成脚本里的常量一致——否则 `updatedAt` 无法比较。
const FIXED_MTIME_MS: f64 = 1_758_794_400_000.0;

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    #[serde(rename = "filePath")]
    file_path: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct CaseFile {
    cases: Vec<Case>,
}

/// golden 里的预览条目。
#[derive(Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
struct GoldenPreview {
    #[serde(default)]
    role: Option<String>,
    text: String,
}

/// golden 里的会话。
#[derive(Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GoldenSession {
    id: String,
    title: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    cli_version: Option<String>,
    message_count: u64,
    preview_count: usize,
    preview: Vec<GoldenPreview>,
    #[serde(default)]
    usage: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldenCase {
    name: String,
    #[serde(default)]
    session: Option<GoldenSession>,
    /// 详情模式（保留全部消息）的结果。
    #[serde(default)]
    full_session: Option<GoldenSession>,
}

#[derive(Debug, Deserialize)]
struct GoldenFile {
    cases: Vec<GoldenCase>,
}

/// 夹具目录。
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn load<T: for<'de> Deserialize<'de>>(name: &str) -> T {
    let path = fixtures().join(name);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 {} 失败：{error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("解析 {} 失败：{error}", path.display()))
}

/// 把本实现的解析结果压成与 golden 同形的结构。
fn normalize(session: &aivc_core::session::Session) -> GoldenSession {
    GoldenSession {
        id: session.id.as_str().to_owned(),
        title: session.title.clone(),
        cwd: session.cwd.clone(),
        created_at: session
            .created_at
            .as_ref()
            .map(|item| item.as_str().to_owned()),
        updated_at: session
            .updated_at
            .as_ref()
            .map(|item| item.as_str().to_owned()),
        model: session.model.clone(),
        cli_version: session.cli_version.clone(),
        message_count: session.message_count,
        preview_count: session.preview.len(),
        preview: session
            .preview
            .iter()
            .map(|message| GoldenPreview {
                role: serde_json::to_value(message.role)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned)),
                text: message.text.clone(),
            })
            .collect(),
        usage: session
            .usage
            .as_ref()
            .and_then(|usage| serde_json::to_value(usage).ok()),
    }
}

#[test]
fn both_fixtures_describe_the_same_cases() {
    let cases: CaseFile = load("parser-scenarios.json");
    let golden: GoldenFile = load("parser-golden.json");
    assert_eq!(
        cases.cases.len(),
        golden.cases.len(),
        "用例数与 golden 不一致"
    );
    for (case, expected) in cases.cases.iter().zip(&golden.cases) {
        assert_eq!(case.name, expected.name, "用例名不一致");
    }
}

#[test]
fn the_parser_matches_the_node_oracle() {
    let cases: CaseFile = load("parser-scenarios.json");
    let golden: GoldenFile = load("parser-golden.json");
    assert!(!golden.cases.is_empty(), "golden 为空——夹具没有正确生成");

    let mut mismatches = Vec::new();
    for (case, expected) in cases.cases.iter().zip(&golden.cases) {
        let info = SessionFileInfo {
            file_path: case.file_path.clone(),
            mtime_ms: FIXED_MTIME_MS,
            change_ms: Some(FIXED_MTIME_MS),
            size: case.content.len() as u64,
        };
        let actual =
            parse_content(info, &case.content, ParseMode::List).map(|item| normalize(&item));
        if actual != expected.session {
            mismatches.push(format!(
                "用例「{}」\n  现网: {}\n  本实现: {}",
                case.name,
                serde_json::to_string(&expected.session).unwrap_or_default(),
                serde_json::to_string(&actual).unwrap_or_default()
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 个用例与现网不一致：\n\n{}",
        mismatches.len(),
        golden.cases.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn the_oracle_exercises_both_outcomes() {
    // 防止夹具退化成「全部解析失败」或「全部成功」——那样对拍会假通过。
    let golden: GoldenFile = load("parser-golden.json");
    assert!(
        golden.cases.iter().any(|case| case.session.is_none()),
        "没有「应当解析失败」的用例"
    );
    assert!(
        golden.cases.iter().any(|case| case.session.is_some()),
        "没有「应当解析成功」的用例"
    );
    // 至少有一个用例带用量，否则用量提取这条没被验证。
    assert!(
        golden
            .cases
            .iter()
            .any(|case| case.session.as_ref().is_some_and(|s| s.usage.is_some())),
        "没有带用量的用例"
    );
    // 至少有一个用例的预览被截断过（预览数少于消息数），否则截断逻辑没被验证。
    assert!(
        golden.cases.iter().any(|case| case
            .session
            .as_ref()
            .is_some_and(|s| s.preview_count < usize::try_from(s.message_count).unwrap_or(0))),
        "没有预览被截断的用例"
    );
}

#[test]
fn the_full_mode_parser_matches_the_node_oracle() {
    // 详情模式与列表模式的判定不同：列表截断预览但计数全量，详情保留全部消息。
    // 两者必须分开对拍——只测列表模式会漏掉「详情少了一条消息」这类问题。
    let cases: CaseFile = load("parser-scenarios.json");
    let golden: GoldenFile = load("parser-golden.json");
    let mut mismatches = Vec::new();
    for (case, expected) in cases.cases.iter().zip(&golden.cases) {
        let info = SessionFileInfo {
            file_path: case.file_path.clone(),
            mtime_ms: FIXED_MTIME_MS,
            change_ms: Some(FIXED_MTIME_MS),
            size: case.content.len() as u64,
        };
        let actual =
            parse_content(info, &case.content, ParseMode::Full).map(|item| normalize(&item));
        if actual != expected.full_session {
            mismatches.push(format!(
                "用例「{}」（详情模式）\n  现网: {}\n  本实现: {}",
                case.name,
                serde_json::to_string(&expected.full_session).unwrap_or_default(),
                serde_json::to_string(&actual).unwrap_or_default()
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 个用例的详情模式与现网不一致：\n\n{}",
        mismatches.len(),
        golden.cases.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn the_two_modes_differ_where_they_should() {
    // 元测试：如果两种模式的结果完全一样，说明夹具没有覆盖到「列表截断」这个差异，
    // 那么两个对拍测试其实是同一个测试，覆盖是假的。
    let golden: GoldenFile = load("parser-golden.json");
    let differing = golden.cases.iter().filter(|case| {
        let list = case.session.as_ref().map(|item| item.preview_count);
        let full = case.full_session.as_ref().map(|item| item.preview_count);
        list.is_some() && full.is_some() && list != full
    });
    assert!(
        differing.count() >= 1,
        "没有任何用例体现列表与详情模式的差别，对拍覆盖不足"
    );
    // 两种模式的消息计数应当一致（列表也做全量计数）。
    for case in &golden.cases {
        if let (Some(list), Some(full)) = (&case.session, &case.full_session) {
            assert_eq!(
                list.message_count, full.message_count,
                "用例「{}」的两种模式消息计数不一致",
                case.name
            );
        }
    }
}
