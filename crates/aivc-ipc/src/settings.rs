//! 会话设置与工作区预设命令。
//!
//! # 为什么这两类放在一起
//!
//! 它们都不触碰会话文件，只改**本应用自己的数据库**：会话设置写 `session_metadata`，
//! 预设写 `workspace_preset`。因此不需要目标探测——预设有各自的编号空间。
//!
//! # 会话设置为什么不需要解析目标
//!
//! 反过来，会话设置**需要**目标编号：元数据按 `(target_id, session_id)` 存储，同一个
//! 会话编号在不同目标下是不同记录。目标编号直接作为键使用，不必解析成完整 [`Target`]
//! ——因为我们不读文件系统。

use serde_json::Value;

use aivc_core::error::Result;
use aivc_core::id::{PresetId, SessionId, TargetId};
use aivc_core::preset::WorkspacePresetInput;
use aivc_core::session::SessionMetadata;
use aivc_provider::ProviderContext;

use crate::validate::{
    require_boolean, require_custom_title, require_string, require_string as required,
};

/// 会话设置命令。
pub struct SessionSettings;

impl SessionSettings {
    /// 设置自定义会话名。
    ///
    /// 空串表示**清除**自定义名（回退到会话原文标题），因此这里不拒绝空串——
    /// 拒绝会让用户无法撤销重命名。
    pub async fn set_custom_title(ctx: &ProviderContext, args: &Value) -> Result<SessionMetadata> {
        let target_id = TargetId::from(required(args.get("targetId"), "targetId")?);
        let session_id = SessionId::from(required(args.get("sessionId"), "sessionId")?);
        let title = require_custom_title(args.get("title"))?;
        ctx.session_metadata()
            .set_custom_title(&target_id, &session_id, &title)
            .await
    }

    /// 设置收藏状态。
    pub async fn set_favorite(ctx: &ProviderContext, args: &Value) -> Result<SessionMetadata> {
        let target_id = TargetId::from(required(args.get("targetId"), "targetId")?);
        let session_id = SessionId::from(required(args.get("sessionId"), "sessionId")?);
        let favorite = require_boolean(args.get("favorite"), "favorite")?;
        ctx.session_metadata()
            .set_favorite(&target_id, &session_id, favorite)
            .await
    }

    /// 记录会话被打开过（用于「最近打开」排序）。
    pub async fn mark_opened(ctx: &ProviderContext, args: &Value) -> Result<SessionMetadata> {
        let target_id = TargetId::from(required(args.get("targetId"), "targetId")?);
        let session_id = SessionId::from(required(args.get("sessionId"), "sessionId")?);
        ctx.session_metadata()
            .mark_opened(&target_id, &session_id)
            .await
    }

    /// 设置某 WSL 发行版的人工 Codex 配置目录覆盖。
    ///
    /// 自动探测在某些发行版上会失败（例如 Codex 装在非标准位置），因此需要人工覆盖。
    pub async fn set_wsl_codex_home(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let distro = require_string(args.get("distro"), "distro")?;
        let codex_home = require_string(args.get("codexHome"), "codexHome")?;
        ctx.settings()
            .update(|raw| raw.set_wsl_codex_home(&distro, &codex_home))
            .await?;
        Ok(serde_json::json!({ "distro": distro, "codexHome": codex_home }))
    }

    /// 清除某 WSL 发行版的人工覆盖，回到自动探测。
    pub async fn clear_wsl_codex_home(ctx: &ProviderContext, args: &Value) -> Result<Value> {
        let distro = require_string(args.get("distro"), "distro")?;
        ctx.settings()
            .update(|raw| raw.clear_wsl_codex_home(&distro))
            .await?;
        Ok(serde_json::json!({ "distro": distro }))
    }
}

/// 工作区预设命令。
pub struct WorkspaceCommands;

impl WorkspaceCommands {
    /// 列出全部预设。
    pub async fn list_presets(
        ctx: &ProviderContext,
    ) -> Result<Vec<aivc_core::preset::WorkspacePreset>> {
        ctx.preset().list().await
    }

    /// 保存预设（新建或更新）。
    ///
    /// 返回本次使用的预设编号：新建时由这里生成。
    ///
    /// 注意仓储的返回值语义：它返回的是「被本次写入替换掉的重复项编号」（去重键是
    /// `cwd` + 目标类型），**不是**新编号。因此新建场景必须自己生成编号，不能把
    /// 仓储的返回值当成新 id。
    pub async fn save_preset(ctx: &ProviderContext, args: &Value) -> Result<PresetId> {
        let input = preset_input(args)?;
        let normalized = input.normalize()?;
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map_or_else(
                || PresetId::from(uuid::Uuid::new_v4().to_string()),
                PresetId::from,
            );

        let preset = aivc_core::preset::WorkspacePreset {
            id: id.clone(),
            name: normalized.name,
            cwd: normalized.cwd,
            target_kind: normalized.target_kind,
            prompt: normalized.prompt,
            cli_args: normalized.cli_args,
            updated_at: aivc_core::time::Timestamp::now(),
        };
        ctx.preset().save(&preset).await?;
        Ok(id)
    }

    /// 删除预设。
    pub async fn delete_preset(ctx: &ProviderContext, args: &Value) -> Result<bool> {
        let id = PresetId::from(require_string(args.get("id"), "id")?);
        ctx.preset().delete(&id).await
    }
}

/// 从参数构造预设输入。
///
/// `cwd` 是必填项：空工作目录的预设没有意义，领域层也会拒绝。名称留空时由领域层
/// 回退到 `cwd`，因此这里不强制非空。
fn preset_input(args: &Value) -> Result<WorkspacePresetInput> {
    let optional = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_owned);
    Ok(WorkspacePresetInput {
        name: optional("name").unwrap_or_default(),
        cwd: require_string(args.get("cwd"), "cwd")?,
        target_kind: match args.get("targetKind").and_then(Value::as_str) {
            Some("local") => Some(aivc_core::target::TargetKind::Local),
            Some("wsl") => Some(aivc_core::target::TargetKind::Wsl),
            _ => None,
        },
        prompt: optional("prompt"),
        cli_args: optional("cliArgs"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::target::{Platform, ProviderId, Target, TargetKind};
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
    use aivc_provider::{ProviderRegistry, ProviderRepositories};
    use serde_json::json;
    use std::sync::Arc;

    /// 构造一个只含数据库的上下文（这些命令不需要目标探测）。
    async fn context() -> (ProviderContext, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::new(dir.path().join("runtime"));
        for required in paths.required_directories() {
            std::fs::create_dir_all(&required).expect("dirs");
        }
        let db = Db::open(paths.database_path()).await.expect("db");
        let wsl = WslAdapter::new(
            Arc::new(LocalBashLauncher::new("definitely-not-a-real-bash")),
            WslConfig {
                use_worker: false,
                ..WslConfig::default()
            },
        );
        let ctx = ProviderContext::new(
            paths,
            SettingsStore::new(dir.path().join("runtime/settings.json")),
            ProviderRepositories::from_db(db),
            wsl,
            None,
            Platform::current(),
        );
        (ctx, dir)
    }

    #[allow(dead_code)]
    fn unused_registry() -> ProviderRegistry {
        aivc_provider::build_registry()
    }

    #[allow(dead_code)]
    fn unused_target() -> Target {
        Target {
            id: TargetId::from("codex:local"),
            provider: ProviderId::Codex,
            label: "Codex".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: None,
            available: true,
            detail: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn custom_titles_round_trip_and_can_be_cleared() {
        let (ctx, _dir) = context().await;
        let args = json!({ "targetId": "codex:local", "sessionId": "s1", "title": "  我的会话  " });
        let saved = SessionSettings::set_custom_title(&ctx, &args)
            .await
            .expect("save");
        assert_eq!(saved.custom_title.as_deref(), Some("我的会话"));

        // 空串表示清除：必须允许，否则用户无法撤销重命名。
        let args = json!({ "targetId": "codex:local", "sessionId": "s1", "title": "" });
        let cleared = SessionSettings::set_custom_title(&ctx, &args)
            .await
            .expect("clear");
        assert!(cleared.custom_title.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_over_long_title_is_rejected() {
        let (ctx, _dir) = context().await;
        let long = "字".repeat(121);
        let args = json!({ "targetId": "codex:local", "sessionId": "s1", "title": long });
        let error = SessionSettings::set_custom_title(&ctx, &args)
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "会话名称不能超过 120 个字符。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn favorite_requires_a_boolean() {
        let (ctx, _dir) = context().await;
        let args = json!({ "targetId": "codex:local", "sessionId": "s1", "favorite": true });
        assert_eq!(
            SessionSettings::set_favorite(&ctx, &args)
                .await
                .expect("favorite")
                .favorite,
            Some(true)
        );

        // 字符串 "true" 不是布尔值。
        let args = json!({ "targetId": "codex:local", "sessionId": "s1", "favorite": "true" });
        let error = SessionSettings::set_favorite(&ctx, &args)
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：favorite");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn marking_opened_records_a_timestamp() {
        let (ctx, _dir) = context().await;
        let args = json!({ "targetId": "codex:local", "sessionId": "s1" });
        let metadata = SessionSettings::mark_opened(&ctx, &args)
            .await
            .expect("opened");
        assert!(metadata.last_opened_at.is_some());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metadata_is_scoped_per_target() {
        // 同一会话编号在不同目标下是不同记录。
        let (ctx, _dir) = context().await;
        for (target, title) in [("codex:local", "A"), ("claude:local", "B")] {
            SessionSettings::set_custom_title(
                &ctx,
                &json!({ "targetId": target, "sessionId": "same", "title": title }),
            )
            .await
            .expect("save");
        }
        let a = ctx
            .session_metadata()
            .get(&TargetId::from("codex:local"), &SessionId::from("same"))
            .await
            .expect("a");
        let b = ctx
            .session_metadata()
            .get(&TargetId::from("claude:local"), &SessionId::from("same"))
            .await
            .expect("b");
        assert_eq!(a.custom_title.as_deref(), Some("A"));
        assert_eq!(b.custom_title.as_deref(), Some("B"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn presets_round_trip() {
        let (ctx, _dir) = context().await;
        let args = json!({
            "name": "默认工作区",
            "description": "常用",
            "cwd": "/work",
            "cliArgs": "--model gpt-5",
            "codexHome": "/home/a/.codex"
        });
        let id = WorkspaceCommands::save_preset(&ctx, &args)
            .await
            .expect("save");
        assert!(!id.as_str().is_empty(), "新建预设必须得到编号");

        let presets = WorkspaceCommands::list_presets(&ctx).await.expect("list");
        assert_eq!(presets.len(), 1);
        assert_eq!(presets[0].name, "默认工作区");
        assert_eq!(presets[0].cwd, "/work");

        assert!(
            WorkspaceCommands::delete_preset(&ctx, &json!({ "id": id.as_str() }))
                .await
                .expect("delete")
        );
        assert!(
            WorkspaceCommands::list_presets(&ctx)
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn preset_name_is_required() {
        let (ctx, _dir) = context().await;
        let error = WorkspaceCommands::save_preset(&ctx, &json!({ "name": "无目录" }))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：cwd");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn saving_an_existing_preset_updates_it() {
        let (ctx, _dir) = context().await;
        let id = WorkspaceCommands::save_preset(&ctx, &json!({ "name": "旧名", "cwd": "/work" }))
            .await
            .expect("create");
        let updated = WorkspaceCommands::save_preset(
            &ctx,
            &json!({ "id": id.as_str(), "name": "新名", "cwd": "/work" }),
        )
        .await
        .expect("update");
        assert_eq!(updated.as_str(), id.as_str());

        let presets = WorkspaceCommands::list_presets(&ctx).await.expect("list");
        assert_eq!(presets.len(), 1, "更新不应新增一条");
        assert_eq!(presets[0].name, "新名");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_an_unknown_preset_reports_false() {
        let (ctx, _dir) = context().await;
        assert!(
            !WorkspaceCommands::delete_preset(&ctx, &json!({ "id": "missing" }))
                .await
                .expect("delete")
        );
    }
}
