//! 工作区预设。
//!
//! 预设保存「工作目录 + 可选提示词 + 可选 CLI 参数」，用于一键以相同上下文新建会话。
//!
//! # 与现网一致的归一化规则
//!
//! - `cwd` 必填（`trim` 后为空即报错）；
//! - `name` 缺省取 `cwd`，截断到 60 字符；
//! - `prompt` 截断到 4000 字符，空白视为未设置；
//! - `cli_args` 截断到 400 字符，空白视为未设置；
//! - 保存时若已有**相同 `cwd` 且相同 `targetKind`** 的预设，旧记录被替换（去重）。

use serde::{Deserialize, Serialize};

use crate::error::{ConfigError, Result};
use crate::id::PresetId;
use crate::target::TargetKind;
use crate::time::Timestamp;

/// 名称长度上限。
pub const NAME_MAX_LENGTH: usize = 60;
/// 提示词长度上限。
pub const PROMPT_MAX_LENGTH: usize = 4_000;
/// CLI 参数长度上限。
pub const CLI_ARGS_MAX_LENGTH: usize = 400;

/// 一个工作区预设。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePreset {
    /// 预设编号。
    pub id: PresetId,
    /// 展示名。
    pub name: String,
    /// 工作目录。
    pub cwd: String,
    /// 目标类型（本机 / WSL）；未指定表示不限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<TargetKind>,
    /// 新建会话时预填的提示词。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// 附加的 CLI 参数模板。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_args: Option<String>,
    /// 最后更新时间。
    pub updated_at: Timestamp,
}

/// 保存预设时的输入。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspacePresetInput {
    /// 展示名；留空时取 `cwd`。
    #[serde(default)]
    pub name: String,
    /// 工作目录。
    #[serde(default)]
    pub cwd: String,
    /// 目标类型。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<TargetKind>,
    /// 提示词。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// CLI 参数模板。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_args: Option<String>,
}

impl WorkspacePresetInput {
    /// 归一化为可直接落库的字段（不含编号与时间）。
    ///
    /// 返回 `(name, cwd, target_kind, prompt, cli_args)`。
    pub fn normalize(&self) -> Result<NormalizedPresetInput> {
        let cwd = self.cwd.trim();
        if cwd.is_empty() {
            return Err(ConfigError::EmptyWorkingDirectory.into());
        }
        let name = self.name.trim();
        Ok(NormalizedPresetInput {
            name: clamp(if name.is_empty() { cwd } else { name }, NAME_MAX_LENGTH),
            cwd: cwd.to_owned(),
            target_kind: self.target_kind,
            prompt: optional_clamp(self.prompt.as_deref(), PROMPT_MAX_LENGTH),
            cli_args: optional_clamp(self.cli_args.as_deref(), CLI_ARGS_MAX_LENGTH),
        })
    }
}

/// 归一化后的预设输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPresetInput {
    /// 展示名（已截断，非空）。
    pub name: String,
    /// 工作目录（已 `trim`，非空）。
    pub cwd: String,
    /// 目标类型。
    pub target_kind: Option<TargetKind>,
    /// 提示词（已截断，空白为 [`None`]）。
    pub prompt: Option<String>,
    /// CLI 参数（已截断，空白为 [`None`]）。
    pub cli_args: Option<String>,
}

impl NormalizedPresetInput {
    /// 判断两个预设是否指向同一工作目录与目标类型（用于去重）。
    #[must_use]
    pub fn same_slot(&self, other: &Self) -> bool {
        self.cwd == other.cwd && self.target_kind == other.target_kind
    }

    /// 判断某个已有预设是否占用同一槽位。
    #[must_use]
    pub fn matches_preset(&self, preset: &WorkspacePreset) -> bool {
        self.cwd == preset.cwd && self.target_kind == preset.target_kind
    }
}

/// 按字符数截断（不产生非法 UTF-8）。
fn clamp(value: &str, max_length: usize) -> String {
    value.chars().take(max_length).collect()
}

/// 可选字段：空白视为未设置，否则截断。
fn optional_clamp(value: Option<&str>, max_length: usize) -> Option<String> {
    let trimmed = value?.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(clamp(trimmed, max_length))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, cwd: &str) -> WorkspacePresetInput {
        WorkspacePresetInput {
            name: name.to_owned(),
            cwd: cwd.to_owned(),
            ..WorkspacePresetInput::default()
        }
    }

    #[test]
    fn empty_working_directory_is_rejected() {
        let error = input("名字", "   ").normalize().expect_err("必须拒绝");
        assert_eq!(error.to_string(), "工作目录不能为空。");
    }

    #[test]
    fn working_directory_is_trimmed() {
        let normalized = input("名字", "  /work/a  ").normalize().expect("normalize");
        assert_eq!(normalized.cwd, "/work/a");
    }

    #[test]
    fn name_falls_back_to_the_working_directory() {
        let normalized = input("   ", "/work/a").normalize().expect("normalize");
        assert_eq!(normalized.name, "/work/a");
    }

    #[test]
    fn name_is_truncated_to_sixty_characters() {
        let normalized = input(&"n".repeat(100), "/w")
            .normalize()
            .expect("normalize");
        assert_eq!(normalized.name.chars().count(), NAME_MAX_LENGTH);
    }

    #[test]
    fn prompt_and_cli_args_are_trimmed_truncated_and_blank_becomes_unset() {
        let mut raw = input("n", "/w");
        raw.prompt = Some("   ".to_owned());
        raw.cli_args = Some("  --foo bar  ".to_owned());
        let normalized = raw.normalize().expect("normalize");
        assert_eq!(normalized.prompt, None, "空白提示词视为未设置");
        assert_eq!(normalized.cli_args.as_deref(), Some("--foo bar"));

        let mut long = input("n", "/w");
        long.prompt = Some("p".repeat(PROMPT_MAX_LENGTH + 500));
        long.cli_args = Some("c".repeat(CLI_ARGS_MAX_LENGTH + 500));
        let normalized = long.normalize().expect("normalize");
        assert_eq!(
            normalized
                .prompt
                .as_ref()
                .map(|value| value.chars().count()),
            Some(PROMPT_MAX_LENGTH)
        );
        assert_eq!(
            normalized
                .cli_args
                .as_ref()
                .map(|value| value.chars().count()),
            Some(CLI_ARGS_MAX_LENGTH)
        );
    }

    #[test]
    fn multibyte_text_is_truncated_on_character_boundaries() {
        // 中文与 emoji 都不得被截成半个字符。
        let normalized = input(&"中文标题".repeat(40), "/w")
            .normalize()
            .expect("normalize");
        assert_eq!(normalized.name.chars().count(), NAME_MAX_LENGTH);
        assert!(normalized.name.is_char_boundary(normalized.name.len()));
    }

    #[test]
    fn same_slot_requires_matching_cwd_and_target_kind() {
        let local = input("a", "/work").normalize().expect("normalize");
        let same = input("b", "/work").normalize().expect("normalize");
        let mut wsl = input("c", "/work").normalize().expect("normalize");
        wsl.target_kind = Some(TargetKind::Wsl);
        let other = input("d", "/other").normalize().expect("normalize");

        assert!(local.same_slot(&same));
        // 目标类型不同视为不同槽位。
        assert!(!local.same_slot(&wsl));
        assert!(!local.same_slot(&other));
    }

    #[test]
    fn matches_preset_uses_the_same_slot_rule() {
        let normalized = input("a", "/work").normalize().expect("normalize");
        let preset = WorkspacePreset {
            id: PresetId::from("p1"),
            name: "旧".to_owned(),
            cwd: "/work".to_owned(),
            target_kind: None,
            prompt: None,
            cli_args: None,
            updated_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
        };
        assert!(normalized.matches_preset(&preset));

        let wsl_preset = WorkspacePreset {
            target_kind: Some(TargetKind::Wsl),
            ..preset.clone()
        };
        assert!(!normalized.matches_preset(&wsl_preset));
    }

    #[test]
    fn preset_serializes_with_camel_case_and_skips_absent_optionals() {
        let preset = WorkspacePreset {
            id: PresetId::from("p1"),
            name: "名字".to_owned(),
            cwd: "/work".to_owned(),
            target_kind: None,
            prompt: Some("提示".to_owned()),
            cli_args: None,
            updated_at: Timestamp::from_raw("2026-01-01T00:00:00.000Z"),
        };
        let json = serde_json::to_value(&preset).expect("serialize");
        assert_eq!(json["cwd"], "/work");
        assert_eq!(json["prompt"], "提示");
        assert_eq!(json["updatedAt"], "2026-01-01T00:00:00.000Z");
        assert!(json.get("targetKind").is_none());
        assert!(json.get("cliArgs").is_none());
    }
}
