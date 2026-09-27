//! 终端状态检测器的差分测试：与现网 TypeScript 实现逐事件对拍。
//!
//! # 这个测试证明什么
//!
//! 单元测试证明「我的实现符合我以为的规则」；差分测试证明「我的实现与现网一致」。
//! 后者才是「功能一致」这条要求的证据——单元测试写得再多，也可能整体地理解错了规则。
//!
//! # golden 从哪来
//!
//! `fixtures/status-golden.json` 由 `generate-status-golden.mjs` 用**现网的
//! `terminal-status-detector.ts`** 跑同一批场景生成。因此本测试是**离线**的：不需要
//! Node，也不需要现网源码，跑得和普通单测一样快。
//!
//! 场景定义在 `fixtures/status-scenarios.json`，生成脚本与这里读的是同一个文件——
//! 两边不可能漂移。
//!
//! # 重新生成
//!
//! ```text
//! node --experimental-transform-types crates/aivc-terminal/tests/generate-status-golden.mjs
//! ```

// 集成测试是独立 crate，`clippy.toml` 里的测试豁免（allow-panic-in-tests）只覆盖
// `#[cfg(test)]` 模块，不覆盖 `tests/` 目录。这里的 panic 就是测试失败的表达方式，
// 因此整个文件放开该 lint。
#![allow(clippy::panic)]

use std::path::{Path, PathBuf};

use aivc_core::target::ProviderId;
use aivc_terminal::{TerminalStatus, TerminalStatusDetector, TerminalStatusEvent};
use serde::Deserialize;

/// 一个场景步骤。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Step {
    #[serde(default)]
    input: Option<String>,
    #[serde(default)]
    output: Option<String>,
    #[serde(default)]
    exit: Option<i32>,
}

/// 一个场景。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Scenario {
    name: String,
    provider_id: String,
    steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
struct ScenarioFile {
    scenarios: Vec<Scenario>,
}

/// golden 里的一条事件。
#[derive(Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GoldenAttention {
    kind: String,
    title: String,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GoldenEvent {
    status: String,
    #[serde(default)]
    attention: Option<GoldenAttention>,
    #[serde(default)]
    turn_id: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoldenScenario {
    name: String,
    provider_id: String,
    events: Vec<GoldenEvent>,
}

#[derive(Debug, Deserialize)]
struct GoldenFile {
    scenarios: Vec<GoldenScenario>,
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

/// 解析平台标识。
fn provider(value: &str) -> ProviderId {
    match value {
        "codex" => ProviderId::Codex,
        "claude" => ProviderId::Claude,
        "gemini" => ProviderId::Gemini,
        "qoder" => ProviderId::Qoder,
        other => panic!("场景里有未知平台：{other}"),
    }
}

/// 把本实现发出的事件转成可比较的形状。
fn normalize(event: &TerminalStatusEvent) -> GoldenEvent {
    GoldenEvent {
        status: event.status.as_str().to_owned(),
        attention: event.attention.as_ref().map(|item| GoldenAttention {
            kind: item.kind.as_str().to_owned(),
            title: item.title.clone(),
            detail: item.detail.clone(),
        }),
        turn_id: event.turn_id,
    }
}

/// 跑一个场景。
fn run(scenario: &Scenario) -> Vec<GoldenEvent> {
    let mut detector = TerminalStatusDetector::new(provider(&scenario.provider_id));
    let mut events = Vec::new();
    for step in &scenario.steps {
        let event = if let Some(input) = &step.input {
            detector.mark_input(input)
        } else if let Some(output) = &step.output {
            detector.inspect_output(output)
        } else if let Some(code) = step.exit {
            detector.mark_exit(code)
        } else {
            panic!(
                "场景「{}」里有既不是 input/output 也不是 exit 的步骤",
                scenario.name
            );
        };
        if let Some(event) = event {
            events.push(normalize(&event));
        }
    }
    events
}

#[test]
fn both_fixtures_describe_the_same_scenarios() {
    // 两个文件必须一一对应，否则「对拍」会在错误的场景上比较。
    let scenarios: ScenarioFile = load("status-scenarios.json");
    let golden: GoldenFile = load("status-golden.json");
    assert_eq!(
        scenarios.scenarios.len(),
        golden.scenarios.len(),
        "场景数与 golden 不一致"
    );
    for (scenario, expected) in scenarios.scenarios.iter().zip(&golden.scenarios) {
        assert_eq!(scenario.name, expected.name, "场景名不一致");
        assert_eq!(
            scenario.provider_id, expected.provider_id,
            "场景「{}」的平台不一致",
            scenario.name
        );
    }
}

#[test]
fn the_detector_matches_the_node_oracle() {
    let scenarios: ScenarioFile = load("status-scenarios.json");
    let golden: GoldenFile = load("status-golden.json");
    assert!(
        !golden.scenarios.is_empty(),
        "golden 为空——夹具没有正确生成"
    );

    let mut mismatches = Vec::new();
    for (scenario, expected) in scenarios.scenarios.iter().zip(&golden.scenarios) {
        let actual = run(scenario);
        if actual != expected.events {
            mismatches.push(format!(
                "场景「{}」（{}）\n  现网: {}\n  本实现: {}",
                scenario.name,
                scenario.provider_id,
                serde_json::to_string(&expected.events).unwrap_or_default(),
                serde_json::to_string(&actual).unwrap_or_default()
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} / {} 个场景与现网不一致：\n\n{}",
        mismatches.len(),
        golden.scenarios.len(),
        mismatches.join("\n\n")
    );
}

#[test]
fn the_oracle_actually_exercises_every_outcome() {
    // 防止夹具退化成「所有场景都什么都不产生」——那种情况下对拍会假通过。
    let golden: GoldenFile = load("status-golden.json");
    let statuses: std::collections::BTreeSet<&str> = golden
        .scenarios
        .iter()
        .flat_map(|scenario| scenario.events.iter())
        .map(|event| event.status.as_str())
        .collect();
    for expected in ["running", "awaiting-confirmation", "completed", "error"] {
        assert!(
            statuses.contains(expected),
            "golden 里没有 {expected} 状态，对拍覆盖不足"
        );
    }
    // 至少有一个场景不产生任何事件（空输入/控制序列），否则「不误报」这条没被验证。
    assert!(
        golden
            .scenarios
            .iter()
            .any(|scenario| scenario.events.is_empty()),
        "没有「应当保持沉默」的场景"
    );
}

#[test]
fn the_outcome_enum_covers_every_golden_status() {
    // 如果现网新增了一种状态而本实现没有，这里会失败而不是静默忽略。
    let golden: GoldenFile = load("status-golden.json");
    for scenario in &golden.scenarios {
        for event in &scenario.events {
            let known = [
                TerminalStatus::Running.as_str(),
                TerminalStatus::AwaitingConfirmation.as_str(),
                TerminalStatus::Completed.as_str(),
                TerminalStatus::Error.as_str(),
            ];
            assert!(
                known.contains(&event.status.as_str()),
                "golden 里有本实现不认识的状态：{}",
                event.status
            );
        }
    }
}
