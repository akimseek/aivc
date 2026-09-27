//! Skill 命令：把领域层的能力接到命令面上。
//!
//! # 为什么只让 Codex 目标通过
//!
//! Skill 是 Codex 的机制（`$CODEX_HOME/skills`）。其余平台没有对应目录，
//! 若放行会让命令去拼一个不存在的路径，报出的是「目录不存在」而不是
//! 「这个平台不支持」——后者才是用户能理解并据此行动的信息。
//!
//! # 返回形状为什么是 `{ movedTo }` 这类包装对象
//!
//! 命令面返回 JSON。直接返回字符串更简单，但**无法在不破坏兼容的情况下追加信息**。
//! 包一层对象的成本极低，换来的是可演进性；这与现网的返回形状也一致。
//!
//! # 未接线的两条
//!
//! - `skill:import` 需要**原生文件选择对话框**（Tauri 下要走 dialog 插件）；
//! - `skill:open-folder` 需要调用系统文件管理器打开目录。
//!
//! 两者留在 PENDING 里显式报「未接线」，而不是假装成功。

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use aivc_core::error::IpcError;
use aivc_core::target::{ProviderId, Target};
use aivc_core::{Error, Result};
use aivc_tooling::skills::core::{skill_directory_name, strip_disabled_suffix};
use aivc_tooling::skills::import::{ImportKind, ImportRequest, import_skill};
use aivc_tooling::skills::store::{
    InstalledSkill, delete_skill, list_skills, list_trash_skills, purge_skill, restore_skill,
    set_enabled, skill_exists,
};
use serde_json::{Value, json};

use crate::sessions::TargetLookup;

/// 不支持 Skill 的目标的拒绝文案。
pub const UNSUPPORTED: &str = "当前目标不支持 Codex Skill。";

/// 把领域层的字符串错误包成命令面的错误。
///
/// 领域层（文件操作、元数据校验）用 `String` 表达失败原因就够了，但命令面需要统一的
/// 错误类型才能被 `?` 串起来。`CommandFailed` 会原样透出文案，因此用户看到的仍是
/// 领域层那句具体的话——不会被包成「命令执行失败」这种没有信息量的消息。
fn failed(message: String) -> Error {
    Error::Ipc(IpcError::CommandFailed { message })
}

/// Skill 命令处理器。
pub struct SkillCommands {
    targets: Arc<dyn TargetLookup>,
}

impl SkillCommands {
    /// 构造。
    pub fn new(targets: Arc<dyn TargetLookup>) -> Self {
        Self { targets }
    }

    /// 解析目标，并要求它支持 Skill。
    async fn require_codex_target(&self, target_id: &str) -> Result<Target> {
        let target = self
            .targets
            .resolve(target_id)
            .await
            .ok_or_else(|| failed("未解析到目标。".to_owned()))?;
        if target.provider != ProviderId::Codex {
            return Err(failed(UNSUPPORTED.to_owned()));
        }
        if target.codex_home.is_none() {
            // 解析到了 Codex 目标却没有 codex_home：这是目标数据的问题，
            // 不是「平台不支持」，因此给不同的文案——否则用户会去换平台，
            // 而真正该做的是重新探测目标。
            return Err(failed("目标缺少 Codex 主目录。".to_owned()));
        }
        Ok(target)
    }

    /// `codex_home`（已确认存在）。
    fn home(target: &Target) -> &str {
        target.codex_home.as_deref().unwrap_or_default()
    }

    /// Skill 根目录（`$CODEX_HOME/skills`）。
    ///
    /// 不复用 `list_skills` 的结果反推路径：那需要读目录，而这里只需要一个路径。
    /// 目录**可能不存在**（从未装过 skill）——交给打开方处理，不在这里预判。
    pub async fn skill_root(&self, target_id: &str) -> Result<String> {
        let target = self.require_codex_target(target_id).await?;
        Ok(std::path::Path::new(Self::home(&target))
            .join("skills")
            .display()
            .to_string())
    }

    /// 列出活跃 skill。
    pub async fn list(&self, target_id: &str) -> Result<Vec<Value>> {
        let target = self.require_codex_target(target_id).await?;
        Ok(to_values(list_skills(Self::home(&target))))
    }

    /// 列出回收站 skill。
    pub async fn list_trash(&self, target_id: &str) -> Result<Vec<Value>> {
        let target = self.require_codex_target(target_id).await?;
        Ok(to_values(list_trash_skills(Self::home(&target))))
    }

    /// 启用或停用。
    pub async fn set_enabled(
        &self,
        target_id: &str,
        skill_name: &str,
        enabled: bool,
    ) -> Result<Value> {
        let target = self.require_codex_target(target_id).await?;
        set_enabled(Self::home(&target), skill_name, enabled).map_err(failed)?;
        // 返回**归一后**的目标名：调用方传进来的可能是带 `.disabled` 的目录名，
        // 回传它会让界面以为「名字没变」。
        let renamed_to = skill_directory_name(&strip_disabled_suffix(skill_name.trim()), enabled);
        Ok(json!({ "renamedTo": renamed_to }))
    }

    /// 移入回收站。
    pub async fn delete(&self, target_id: &str, skill_name: &str) -> Result<Value> {
        let target = self.require_codex_target(target_id).await?;
        let moved_to = delete_skill(Self::home(&target), skill_name, now_ms()).map_err(failed)?;
        Ok(json!({ "movedTo": moved_to }))
    }

    /// 从回收站恢复。
    pub async fn restore(&self, target_id: &str, skill_name: &str) -> Result<Value> {
        let target = self.require_codex_target(target_id).await?;
        let restored_to = restore_skill(Self::home(&target), skill_name).map_err(failed)?;
        Ok(json!({ "restoredTo": restored_to }))
    }

    /// 彻底删除。
    pub async fn purge(&self, target_id: &str, skill_name: &str) -> Result<Value> {
        let target = self.require_codex_target(target_id).await?;
        let deleted = purge_skill(Self::home(&target), skill_name).map_err(failed)?;
        Ok(json!({ "deleted": deleted }))
    }

    /// 导入一个 skill（源路径由调用方给出）。
    ///
    /// 界面上的 `skill:import` 走原生文件选择对话框，因此没有接到命令面；但导入逻辑
    /// 本身要先能被驱动，否则它无法被端到端验证——这条是它的入口。
    pub async fn import_from_path(
        &self,
        target_id: &str,
        source: &str,
        overwrite: bool,
    ) -> Result<Value> {
        let target = self.require_codex_target(target_id).await?;
        let path = PathBuf::from(source);
        let kind = if path.is_dir() {
            ImportKind::Directory
        } else {
            ImportKind::File
        };
        let outcome = import_skill(&ImportRequest {
            source: path,
            codex_home: Self::home(&target).to_owned(),
            kind,
            overwrite,
            suffix: unique_suffix(),
        })
        .map_err(failed)?;
        Ok(json!({
            "skillName": outcome.skill_name,
            "destinationPath": outcome.destination_path,
        }))
    }

    /// 某个 skill 是否已存在（启用或停用都算）。
    pub async fn exists(&self, target_id: &str, skill_name: &str) -> Result<bool> {
        let target = self.require_codex_target(target_id).await?;
        Ok(skill_exists(Self::home(&target), skill_name))
    }
}

/// 把领域类型转成命令面的 JSON。
///
/// 字段名与现网 `InstalledSkill` 一致：`name` / `description` / `path` /
/// `enabled` / `sourceName`。少一个前端就会拿到 `undefined`。
fn to_values(skills: Vec<InstalledSkill>) -> Vec<Value> {
    skills
        .into_iter()
        .map(|skill| {
            json!({
                "name": skill.name,
                "description": skill.description,
                "path": skill.path,
                "enabled": skill.enabled,
                "sourceName": skill.source_name,
            })
        })
        .collect()
}

/// 当前毫秒时间戳。
///
/// 取不到系统时间时退回 0：回收站条目名只需要唯一，时间戳为 0 仍能工作
/// （同一毫秒内的多次删除会因目录名冲突而失败，但这比整个操作不可用要好）。
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

/// 进程内唯一的临时目录后缀。
fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{}-{}", std::process::id(), now_ms(), sequence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::id::TargetId;
    use aivc_core::target::TargetKind;

    /// 固定返回一个目标的查找器。
    struct Fixed(Option<Target>);

    impl TargetLookup for Fixed {
        fn resolve(&self, _target_id: &str) -> aivc_core::ports::BoxFuture<'_, Option<Target>> {
            let value = self.0.clone();
            Box::pin(async move { value })
        }

        fn list(&self) -> aivc_core::ports::BoxFuture<'_, Vec<Target>> {
            let value: Vec<Target> = self.0.clone().into_iter().collect();
            Box::pin(async move { value })
        }
    }

    fn codex_target(home: &str) -> Target {
        Target {
            id: TargetId::from("codex:local"),
            provider: ProviderId::Codex,
            label: "Codex".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some(home.to_owned()),
            available: true,
            detail: None,
        }
    }

    fn commands(target: Option<Target>) -> SkillCommands {
        SkillCommands::new(Arc::new(Fixed(target)))
    }

    /// 建一个带 `SKILL.md` 的 skill 目录。
    fn write_skill(home: &std::path::Path, dir_name: &str, name: &str) {
        let dir = home.join("skills").join(dir_name);
        std::fs::create_dir_all(&dir).expect("目录");
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: 描述-{name}\n---\n"),
        )
        .expect("写入");
    }

    #[tokio::test]
    async fn a_qoder_target_is_rejected_with_the_documented_message() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut target = codex_target(&dir.path().to_string_lossy());
        target.provider = ProviderId::Qoder;
        let error = commands(Some(target))
            .list("qoder:local")
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), UNSUPPORTED);
    }

    #[tokio::test]
    async fn an_unresolved_target_is_rejected() {
        let error = commands(None)
            .list("codex:local")
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "未解析到目标。");
    }

    #[tokio::test]
    async fn a_codex_target_without_a_home_is_reported_differently() {
        // 「目标数据有问题」与「平台不支持」是两回事，文案必须能区分。
        let mut target = codex_target("/tmp");
        target.codex_home = None;
        let error = commands(Some(target))
            .list("codex:local")
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "目标缺少 Codex 主目录。");
    }

    #[tokio::test]
    async fn listing_returns_the_frontend_shape() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        write_skill(&home, "alpha", "alpha");
        let listed = commands(Some(codex_target(&home.to_string_lossy())))
            .list("codex:local")
            .await
            .expect("列表");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], "alpha");
        assert_eq!(listed[0]["description"], "描述-alpha");
        assert_eq!(listed[0]["enabled"], true);
        // 少一个字段前端就会拿到 undefined，因此逐个确认。
        assert_eq!(listed[0]["sourceName"], "alpha");
        assert!(listed[0]["path"].is_string());
    }

    #[tokio::test]
    async fn the_full_lifecycle_works_through_the_command_layer() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        write_skill(&home, "alpha", "alpha");
        let commands = commands(Some(codex_target(&home.to_string_lossy())));

        let value = commands
            .set_enabled("codex:local", "alpha", false)
            .await
            .expect("停用");
        assert_eq!(value["renamedTo"], "alpha.disabled");
        assert_eq!(
            commands.list("codex:local").await.expect("列表")[0]["enabled"],
            false
        );

        let value = commands
            .delete("codex:local", "alpha.disabled")
            .await
            .expect("删除");
        let moved_to = value["movedTo"].as_str().expect("路径").to_owned();
        assert!(moved_to.contains("alpha.disabled-"));
        assert!(commands.list("codex:local").await.expect("列表").is_empty());
        assert_eq!(
            commands
                .list_trash("codex:local")
                .await
                .expect("回收站")
                .len(),
            1
        );

        let trash_name = moved_to
            .rsplit(['/', '\\'])
            .next()
            .expect("名字")
            .to_owned();
        commands
            .restore("codex:local", &trash_name)
            .await
            .expect("恢复");
        assert_eq!(commands.list("codex:local").await.expect("列表").len(), 1);
    }

    #[tokio::test]
    async fn purging_removes_it_from_the_trash() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        write_skill(&home, "alpha", "alpha");
        let commands = commands(Some(codex_target(&home.to_string_lossy())));
        let value = commands.delete("codex:local", "alpha").await.expect("删除");
        let name = value["movedTo"]
            .as_str()
            .expect("路径")
            .rsplit(['/', '\\'])
            .next()
            .expect("名字")
            .to_owned();
        commands.purge("codex:local", &name).await.expect("清除");
        assert!(
            commands
                .list_trash("codex:local")
                .await
                .expect("回收站")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn the_enabled_reply_is_normalized() {
        // 调用方可能传带后缀的目录名；回传它会让界面以为「名字没变」。
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        write_skill(&home, "alpha", "alpha");
        let value = commands(Some(codex_target(&home.to_string_lossy())))
            .set_enabled("codex:local", " alpha.disabled ", true)
            .await
            .expect("启用");
        assert_eq!(value["renamedTo"], "alpha");
    }

    #[tokio::test]
    async fn import_from_path_makes_a_skill_visible() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = dir.path().join("src");
        std::fs::create_dir_all(&source).expect("目录");
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: imported\ndescription: 来自导入\n---\n",
        )
        .expect("写入");
        let commands = commands(Some(codex_target(&home.to_string_lossy())));
        let value = commands
            .import_from_path("codex:local", &source.to_string_lossy(), false)
            .await
            .expect("导入");
        assert_eq!(value["skillName"], "imported");
        assert_eq!(
            commands.list("codex:local").await.expect("列表")[0]["description"],
            "来自导入"
        );
    }

    #[tokio::test]
    async fn import_refuses_to_overwrite_without_permission() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        let source = dir.path().join("src");
        std::fs::create_dir_all(&source).expect("目录");
        std::fs::write(
            source.join("SKILL.md"),
            "---\nname: imported\ndescription: d\n---\n",
        )
        .expect("写入");
        let commands = commands(Some(codex_target(&home.to_string_lossy())));
        commands
            .import_from_path("codex:local", &source.to_string_lossy(), false)
            .await
            .expect("首次");
        let error = commands
            .import_from_path("codex:local", &source.to_string_lossy(), false)
            .await
            .expect_err("必须拒绝");
        // 领域层的文案必须原样透出，而不是被包成「命令执行失败」。
        assert_eq!(error.to_string(), "skill 已存在：imported");
    }

    #[tokio::test]
    async fn existence_reflects_both_states() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().to_path_buf();
        write_skill(&home, "alpha", "alpha");
        let commands = commands(Some(codex_target(&home.to_string_lossy())));
        assert!(commands.exists("codex:local", "alpha").await.expect("查询"));
        assert!(!commands.exists("codex:local", "other").await.expect("查询"));
    }

    #[test]
    fn the_unique_suffix_changes_between_calls() {
        // 同一次导入的两次尝试必须用不同的临时目录，否则会互相踩。
        assert_ne!(unique_suffix(), unique_suffix());
    }
}
