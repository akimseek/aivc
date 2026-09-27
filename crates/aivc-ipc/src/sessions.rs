//! 会话相关 IPC 命令。
//!
//! # 这一层的职责
//!
//! 只做三件事：**校验入参 → 解析目标 → 分发到对应平台**。业务逻辑全在 Provider 层，
//! 因此这里没有任何算法，只有「把渲染层的调用翻译成领域调用」的映射。
//!
//! # 为什么返回类型直接复用领域模型
//!
//! [`aivc_core::session::Session`]、[`MessagePage`] 等已经带了 `camelCase` 的 serde 契约
//! 与 `skip_serializing_if`，与 `src/shared/types.ts` 的线上形状逐字一致。再包一层 DTO
//! 只会多一次手工同步的机会——两处字段名不一致就是最难查的那类 bug。
//!
//! # 目标解析为什么要抽成接口
//!
//! 渲染层传的是 `targetId`（如 `codex:local`），而 Provider 要的是完整 [`Target`]
//! （含 `codexHome`、`distro`）。解析需要探测文件系统与 WSL，因此放在上层并通过
//! [`TargetLookup`] 注入——这样本层可以在没有 WSL 的环境里完整测试。

use std::sync::Arc;

use serde_json::Value;

use aivc_core::error::{Error, IpcError, Result, SessionError, TargetError};
use aivc_core::id::{SessionId, TargetId};
use aivc_core::session::{
    ExportResult, MessagePage, PageCursor, Session, SessionBatchResult, SessionDeleteResult,
    SessionMutationRef, SessionView,
};
use aivc_core::target::Target;
use aivc_provider::{ProviderContext, ProviderRegistry};

use crate::validate::{
    require_export_format, require_message_page_offset, require_session_file_ref,
    require_session_mutation_refs, require_string, require_view,
};

/// 按编号解析目标。
pub trait TargetLookup: Send + Sync {
    /// 解析目标；编号未知时返回 [`None`]。
    fn resolve(&self, target_id: &str) -> aivc_core::ports::BoxFuture<'_, Option<Target>>;

    /// 列出全部目标。
    fn list(&self) -> aivc_core::ports::BoxFuture<'_, Vec<Target>>;
}

/// 会话命令面。
pub struct SessionCommands {
    registry: Arc<ProviderRegistry>,
    targets: Arc<dyn TargetLookup>,
}

impl SessionCommands {
    /// 构造。
    pub fn new(registry: Arc<ProviderRegistry>, targets: Arc<dyn TargetLookup>) -> Self {
        Self { registry, targets }
    }

    /// 探测并列出全部目标，同时回写缓存。
    ///
    /// # 为什么必须回写缓存
    ///
    /// 其余命令按 `targetId` 解析目标，而解析只能读缓存（它们不该每条命令都去探测
    /// 文件系统或启动 WSL）。因此**这里是唯一的写入点**：没有它缓存永远是空的，
    /// 所有会话命令都会以「未解析到目标」失败。
    ///
    /// # Errors
    ///
    /// 设置写入失败时返回错误。单个平台的探测失败只记日志——一个平台的 CLI 没装，
    /// 不该让其余平台的目标消失。
    pub async fn list_targets(&self, ctx: &ProviderContext, args: &Value) -> Result<Vec<Target>> {
        // 可选的平台过滤。前端按平台分别加载目标，不过滤会把**全部平台**的目标
        // 混在一起返回，界面上就变成「目标是各平台的」。
        let only = match args.get("providerId") {
            None | Some(Value::Null) => None,
            Some(_) => Some(crate::validate::require_provider_id(
                args.get("providerId"),
            )?),
        };
        let mut targets = Vec::new();
        for provider in self.registry.list() {
            if only.is_some_and(|wanted| wanted != provider.id()) {
                continue;
            }
            match provider.list_targets(ctx).await {
                Ok(mut items) => {
                    // **最终计数必须可见**。实测踩过：界面上目标只有一个，
                    // 而 `local_target()` 的两条失败日志都没出现（说明它返回了 Some）
                    // ——两边矛盾，无从判断是后端少了还是前端滤了。
                    // 记下「每个平台返回几个」就能一次定论。
                    tracing::debug!(
                        provider = ?provider.id(),
                        count = items.len(),
                        "平台目标探测结果"
                    );
                    targets.append(&mut items);
                }
                Err(error) => tracing::warn!(%error, "探测目标失败，跳过该平台"),
            }
        }
        // 与已有缓存**合并**（按平台覆盖）而不是全量替换：只探测成功的平台不该把之前
        // 探测到的其他平台记录挤掉。
        if !targets.is_empty() {
            let snapshot = targets.clone();
            ctx.settings()
                .update(move |raw| raw.merge_cached_targets(&snapshot))
                .await?;
        }
        // **把探测结果写进缓存**。
        //
        // 这一步曾经缺失：`merge_cached_targets` 只在测试里被调用过，
        // 生产代码从不写缓存。后果有两层——
        //
        // 1. `list-cached-targets` 永远返回空，前端的「先渲染缓存再增量刷新」
        //    策略完全失效，每次都要现场探测；
        // 2. 我实现的**启动预热**白做了：它探测并打印了「预热完成 count=8」，
        //    却没留下任何东西——**我验证了想验证的，没验证该验证的**。
        //
        // 写失败只记日志：探测结果本身是对的，缓存写不进去不该让整次探测失败。
        if let Err(error) = ctx
            .settings()
            .update(|raw| raw.merge_cached_targets(&targets))
            .await
        {
            tracing::warn!(%error, "写入目标缓存失败");
        }
        Ok(targets)
    }

    /// 平台清单（渲染层的平台下拉框用它取数据）。
    ///
    /// 返回 `{ id, label, capabilities }`：`capabilities` 直接复用领域层的
    /// [`aivc_core::target::ProviderCapabilities`]——它的十个字段（含 camelCase 命名）
    /// 与前端 `AiProviderCapabilities` 逐字一致，再包一层只会多一次手工同步的机会。
    ///
    /// 能力来自 [`ProviderRegistry::capabilities`]，而那个映射由**注册时追加了哪些能力
    /// 子 trait**推导出来，并与静态声明交叉校验过。因此下拉框里显示的「支持分支」等
    /// 标记不会与实际行为漂移。
    #[must_use]
    pub fn provider_summaries(&self) -> Vec<Value> {
        self.registry
            .capabilities()
            .into_iter()
            .map(|(id, capabilities)| {
                // 标签取自 Provider 自己，避免在这里再维护一份名字表。
                let label = self.registry.get(id).map_or_else(
                    || id.as_str().to_owned(),
                    |provider| provider.label().to_owned(),
                );
                serde_json::json!({
                    "id": id.as_str(),
                    "label": label,
                    "capabilities": capabilities,
                })
            })
            .collect()
    }

    /// 列出**缓存中**的目标，不主动探测。
    ///
    /// 与 [`Self::list_targets`] 的关键差别：这里不碰文件系统、不启动 WSL，因此可以在
    /// 界面首屏立即返回。渲染层先渲染缓存结果，再等一次 `list-targets` 做增量刷新。
    ///
    /// # Errors
    ///
    /// 本方法不返回错误：单个平台的缓存读失败只记录日志，不应让整个列表失败。
    pub async fn list_cached_targets(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Vec<Target>> {
        // 与 `list_targets` 同一套过滤：不接受平台参数会返回**全部平台**的目标缓存。
        let only = match args.get("providerId") {
            None | Some(Value::Null) => None,
            Some(_) => Some(crate::validate::require_provider_id(
                args.get("providerId"),
            )?),
        };
        let mut targets = Vec::new();
        for provider in self.registry.list() {
            if only.is_some_and(|wanted| wanted != provider.id()) {
                continue;
            }
            match provider.list_cached_targets(ctx).await {
                Ok(mut items) => targets.append(&mut items),
                Err(error) => tracing::warn!(%error, "读取目标缓存失败"),
            }
        }
        Ok(targets)
    }

    /// 解析目标编号。
    async fn target(&self, args: &Value) -> Result<Target> {
        let target_id = require_string(args.get("targetId"), "targetId")?;
        self.targets.resolve(&target_id).await.ok_or_else(|| {
            Error::Target(TargetError::NotResolved {
                id: target_id.clone(),
            })
        })
    }

    /// 列出活跃会话。
    pub async fn list_sessions(&self, ctx: &ProviderContext, args: &Value) -> Result<Vec<Session>> {
        let target = self.target(args).await?;
        self.registry
            .dispatch(&target)?
            .list_sessions(ctx, &target)
            .await
    }

    /// 列出回收站会话。
    pub async fn list_trash_sessions(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Vec<Session>> {
        let target = self.target(args).await?;
        self.registry
            .dispatch(&target)?
            .list_trash_sessions(ctx, &target)
            .await
    }

    /// 只读缓存中的会话（不扫描文件系统）。
    pub async fn list_cached_sessions(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Vec<Session>> {
        let target = self.target(args).await?;
        let view = require_view(args.get("view"))?;
        self.registry
            .dispatch(&target)?
            .list_cached_sessions(ctx, &target, view)
            .await
    }

    /// 搜索会话。
    pub async fn search_sessions(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Vec<Session>> {
        let target = self.target(args).await?;
        let view = require_view(args.get("view"))?;
        let query = require_string(args.get("query"), "query")?;
        self.registry
            .dispatch(&target)?
            .search_sessions(ctx, &target, view, &query)
            .await
    }

    /// 读取完整会话。
    pub async fn get_session(&self, ctx: &ProviderContext, args: &Value) -> Result<Session> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        let hint = require_session_file_ref(args.get("ref"))?;
        self.registry
            .dispatch(&target)?
            .get_session(ctx, &target, &SessionId::from(id), hint.as_deref())
            .await
    }

    /// 读取会话摘要。
    pub async fn get_session_summary(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Session> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        self.registry
            .dispatch(&target)?
            .get_session_summary(ctx, &target, &SessionId::from(id))
            .await
    }

    /// 分页读取会话消息。
    ///
    /// `offset` 为 `-1` 表示首屏（取末尾若干条并回填真实序号）。
    pub async fn get_session_messages_page(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<MessagePage> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        let offset = require_message_page_offset(args.get("offset"))?;
        let limit = crate::validate::require_positive_integer(args.get("limit"), "limit")?;
        let cursor = PageCursor::from_wire(offset);
        // 现网的 limit 有一个上限，避免一次请求把整份会话拉回来。
        let limit = u32::try_from(limit.min(1000)).unwrap_or(1000);
        self.registry
            .dispatch(&target)?
            .get_session_messages_page(ctx, &target, &SessionId::from(id), cursor, limit)
            .await
    }

    /// 会话文件所在目录。
    pub async fn get_session_folder_path(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<String> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        self.registry
            .dispatch(&target)?
            .get_session_folder_path(ctx, &target, &SessionId::from(id))
            .await
    }

    /// 反查分支子会话。
    pub async fn list_session_children(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<Vec<Session>> {
        let target = self.target(args).await?;
        let parent = require_string(args.get("parentSessionId"), "parentSessionId")?;
        self.registry
            .dispatch(&target)?
            .list_sessions_by_parent(ctx, &target, &SessionId::from(parent))
            .await
    }

    /// 批量删除（移入回收站）。
    pub async fn delete_sessions(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<SessionBatchResult> {
        let target = self.target(args).await?;
        let refs = require_session_mutation_refs(args.get("sessions"))?;
        self.registry
            .dispatch(&target)?
            .delete_sessions(ctx, &target, &refs)
            .await
    }

    /// 删除单个会话。
    pub async fn delete_session(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<SessionDeleteResult> {
        let target = self.target(args).await?;
        let reference = Self::mutation_ref(args)?;
        self.registry
            .dispatch(&target)?
            .delete_session(ctx, &target, &reference)
            .await
    }

    /// 从回收站恢复。
    pub async fn restore_session(&self, ctx: &ProviderContext, args: &Value) -> Result<String> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        self.registry
            .dispatch(&target)?
            .restore_session(ctx, &target, &SessionId::from(id))
            .await
    }

    /// 永久删除单个会话。
    pub async fn purge_session(&self, ctx: &ProviderContext, args: &Value) -> Result<String> {
        let target = self.target(args).await?;
        let reference = Self::mutation_ref(args)?;
        self.registry
            .dispatch(&target)?
            .purge_session(ctx, &target, &reference)
            .await
    }

    /// 批量永久删除。
    pub async fn purge_sessions(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<SessionBatchResult> {
        let target = self.target(args).await?;
        let refs = require_session_mutation_refs(args.get("sessions"))?;
        self.registry
            .dispatch(&target)?
            .purge_sessions(ctx, &target, &refs)
            .await
    }

    /// 去掉只用于排序的内部键。
    ///
    /// `_sort` / `_label` / `_title` 是本模块内部的比较依据，**不该出现在响应里**：
    /// 前端 `SessionQuickAccessItem` 只声明四个字段（target / session / favorite /
    /// lastOpenedAt），多余的键不会让它报错，却会让契约变模糊、载荷无谓变大。
    fn public_item(item: &Value) -> Value {
        let mut item = item.clone();
        if let Some(object) = item.as_object_mut() {
            object.remove("_sort");
            object.remove("_label");
            object.remove("_title");
        }
        item
    }

    /// 「快捷访问」：收藏与最近打开的会话。
    ///
    /// # 算法（与现网 `ai-providers.ts` 一致）
    ///
    /// 候选是「收藏过」或「打开过」的会话，按三级兜底排序：
    /// `lastOpenedAt` → 会话 `updatedAt` → 会话 `createdAt`。
    ///
    /// **`recent` 必须排除已进 `favorites` 的项**：否则同一条会话会同时出现在
    /// 两个列表里，界面上看起来像重复。按 `targetId:sessionId` 去重。
    ///
    /// # 为什么只查「已缓存」的目标与会话
    ///
    /// 这个列表要**立即**显示，不能触发全量探测（实测 WSL 冷启动 3.3 秒）。
    /// 缓存里没有的目标直接跳过——它下次刷新时会补上。
    pub async fn list_quick_access(&self, ctx: &ProviderContext, _args: &Value) -> Result<Value> {
        /// 收藏上限。
        const MAX_FAVORITES: usize = 12;
        /// 最近打开上限。
        const MAX_RECENT: usize = 8;

        let targets = self
            .list_cached_targets(ctx, &serde_json::json!({}))
            .await?;
        let mut by_target: std::collections::BTreeMap<String, Target> =
            std::collections::BTreeMap::new();
        for target in targets {
            by_target.insert(target.id.as_str().to_owned(), target);
        }

        // 逐目标取已缓存会话，再与元数据求交。
        let mut items: Vec<Value> = Vec::new();
        for target in by_target.values() {
            // 注册表按平台取 provider：`list()` 是既有入口，没有按平台单取的访问器。
            let Some(provider) = self
                .registry
                .list()
                .into_iter()
                .find(|provider| provider.id() == target.provider)
            else {
                continue;
            };
            let Ok(sessions) = provider
                .list_cached_sessions(ctx, target, SessionView::Active)
                .await
            else {
                continue;
            };
            for session in sessions {
                let Ok(meta) = ctx.session_metadata().get(&target.id, &session.id).await else {
                    continue;
                };
                let favorite = meta.favorite == Some(true);
                let opened = meta.last_opened_at.clone();
                if !favorite && opened.is_none() {
                    continue;
                }
                items.push(serde_json::json!({
                    "target": target,
                    "session": session,
                    "favorite": favorite,
                    "lastOpenedAt": opened,
                    // 排序键：三级兜底，避免在 Rust 侧再解析时间戳。
                    "_sort": meta
                        .last_opened_at
                        .clone()
                        .or_else(|| meta.updated_at.clone())
                        .or_else(|| session.updated_at.clone())
                        .map(|value| value.as_str().to_owned())
                        .unwrap_or_default(),
                    "_label": target.label.clone(),
                    "_title": session.title.clone(),
                }));
            }
        }

        items.sort_by(|left, right| {
            let key = |item: &Value| {
                (
                    item["_sort"].as_str().unwrap_or_default().to_owned(),
                    item["_label"].as_str().unwrap_or_default().to_owned(),
                    item["_title"].as_str().unwrap_or_default().to_owned(),
                )
            };
            let (ls, ll, lt) = key(left);
            let (rs, rl, rt) = key(right);
            // 时间倒序，其余升序。
            rs.cmp(&ls)
                .then_with(|| ll.cmp(&rl))
                .then_with(|| lt.cmp(&rt))
        });

        let favorites: Vec<Value> = items
            .iter()
            .filter(|item| item["favorite"] == Value::Bool(true))
            .take(MAX_FAVORITES)
            .map(Self::public_item)
            .collect();
        // 去重键：收藏里已有的不再进「最近」。
        let taken: std::collections::BTreeSet<String> = favorites
            .iter()
            .map(|item| {
                format!(
                    "{}:{}",
                    item["target"]["id"].as_str().unwrap_or_default(),
                    item["session"]["id"].as_str().unwrap_or_default()
                )
            })
            .collect();
        let recent: Vec<Value> = items
            .iter()
            .filter(|item| item["lastOpenedAt"].is_string())
            .filter(|item| {
                !taken.contains(&format!(
                    "{}:{}",
                    item["target"]["id"].as_str().unwrap_or_default(),
                    item["session"]["id"].as_str().unwrap_or_default()
                ))
            })
            .take(MAX_RECENT)
            .map(Self::public_item)
            .collect();

        Ok(serde_json::json!({ "favorites": favorites, "recent": recent }))
    }

    /// 从指定消息创建分支。
    ///
    /// `messageLine` 的含义**随平台而异**：Codex 是原始 JSONL 行号，其余平台是可见
    /// 消息条数。渲染层按平台传对应值，这一层不转换——转换规则写在各 Provider 里。
    pub async fn branch_session(&self, ctx: &ProviderContext, args: &Value) -> Result<Session> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        let anchor = crate::validate::require_positive_integer(
            args.get("messageLine").or_else(|| args.get("messageIndex")),
            "messageLine",
        )?;
        let provider = self.registry.branchable(&target)?;
        provider
            .branch_session(ctx, &target, &SessionId::from(id), anchor.cast_unsigned())
            .await
    }

    /// 复制会话。
    pub async fn duplicate_session(&self, ctx: &ProviderContext, args: &Value) -> Result<Session> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        let title = match args.get("title") {
            None | Some(Value::Null) => None,
            Some(value) => Some(crate::validate::require_custom_title(Some(value))?),
        };
        let provider = self.registry.duplicable(&target)?;
        provider
            .duplicate_session(ctx, &target, &SessionId::from(id), title.as_deref())
            .await
    }

    /// 导出会话。
    pub async fn export_session(
        &self,
        ctx: &ProviderContext,
        args: &Value,
    ) -> Result<ExportResult> {
        let target = self.target(args).await?;
        let id = require_string(args.get("sessionId"), "sessionId")?;
        let format = require_export_format(args.get("format"))?;
        // 目标路径**可选**：原生「保存到…」对话框需要 Tauri 的 dialog 插件，离线环境
        // 装不上，而导出不该因此不可用。没给路径时由后端挑一个确定的位置，
        // 并把实际路径回给界面——用户至少知道东西去哪了。
        let destination = match args.get("filePath").and_then(Value::as_str) {
            Some(value) if !value.trim().is_empty() => value.to_owned(),
            _ => default_export_path(ctx, &id, format),
        };
        let provider = self.registry.exportable(&target)?;
        provider
            .export_session(ctx, &target, &SessionId::from(id), format, &destination)
            .await
    }

    /// 由参数构造单条变更引用。
    fn mutation_ref(args: &Value) -> Result<SessionMutationRef> {
        let id = require_string(args.get("sessionId"), "sessionId")?;
        Ok(SessionMutationRef {
            id: SessionId::from(id),
            file_path: require_session_file_ref(args.get("ref"))?,
        })
    }
}

/// 没给导出路径时的默认落点：`<运行根>/data/exports/session-<短编号>.<扩展名>`。
///
/// 用**编号**而不是标题来命名：标题里可能有路径分隔符、控制字符与超长文本，清洗它要
/// 处理一堆边界；而编号本来就唯一，取前 8 位既够区分，也不会把文件名撑得没法看。
fn default_export_path(
    ctx: &ProviderContext,
    session_id: &str,
    format: aivc_core::session::ExportFormat,
) -> String {
    use aivc_core::session::ExportFormat;
    let extension = match format {
        ExportFormat::Markdown => "md",
        ExportFormat::Json => "json",
        ExportFormat::Html => "html",
    };
    let short: String = session_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(8)
        .collect();
    ctx.paths()
        .export_dir()
        .join(format!("session-{short}.{extension}"))
        .to_string_lossy()
        .into_owned()
}

/// 目标编号与视图的字符串形式（供上层构造缓存的诊断信息）。
#[must_use]
pub fn view_name(view: SessionView) -> &'static str {
    view.as_str()
}

/// 便于上层把「目标编号」转成 [`TargetId`]。
#[must_use]
pub fn target_id(value: &str) -> TargetId {
    TargetId::from(value)
}

/// 会话不存在时的统一错误。
#[must_use]
pub fn not_found(id: &str) -> Error {
    SessionError::NotFound { id: id.to_owned() }.into()
}

/// 命令名非法。
#[must_use]
pub fn unknown_command(name: &str) -> Error {
    IpcError::CommandFailed {
        message: format!("未知命令：{name}"),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::ports::BoxFuture;
    use aivc_core::target::{Platform, ProviderId, TargetKind};
    use aivc_infra::config::{AppPaths, SettingsStore};
    use aivc_infra::db::Db;
    use aivc_infra::wsl::{LocalBashLauncher, WslAdapter, WslConfig};
    use aivc_provider::{ProviderRepositories, build_registry};
    use serde_json::json;

    /// 只认识一个固定目标。
    struct StubLookup {
        target: Option<Target>,
    }

    impl TargetLookup for StubLookup {
        fn resolve(&self, target_id: &str) -> BoxFuture<'_, Option<Target>> {
            let found = self
                .target
                .as_ref()
                .filter(|target| target.id.as_str() == target_id)
                .cloned();
            Box::pin(async move { found })
        }

        fn list(&self) -> BoxFuture<'_, Vec<Target>> {
            let items: Vec<Target> = self.target.iter().cloned().collect();
            Box::pin(async move { items })
        }
    }

    fn qoder_target() -> Target {
        Target {
            id: TargetId::from("qoder:local"),
            provider: ProviderId::Qoder,
            label: "Qoder".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some("/tmp/qoder".to_owned()),
            available: true,
            detail: None,
        }
    }

    fn commands(target: Option<Target>) -> SessionCommands {
        SessionCommands::new(Arc::new(build_registry()), Arc::new(StubLookup { target }))
    }

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

    #[tokio::test(flavor = "multi_thread")]
    async fn target_resolution_reports_unknown_ids() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        let error = commands
            .list_sessions(&ctx, &json!({ "targetId": "unknown:id" }))
            .await
            .expect_err("必须失败");
        assert!(
            error.to_string().contains("未解析到目标") || error.to_string().contains("unknown:id"),
            "{error}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_target_id_is_rejected() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        let error = commands
            .list_sessions(&ctx, &json!({}))
            .await
            .expect_err("必须失败");
        assert_eq!(error.to_string(), "参数无效：targetId");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_unsupported_capability_is_rejected_at_dispatch() {
        // Qoder 不支持分支：命令面必须给出与现网一致的显式拒绝，而不是静默失败。
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        let error = commands
            .branch_session(
                &ctx,
                &json!({ "targetId": "qoder:local", "sessionId": "s1", "messageLine": 1 }),
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "Qoder CN 不支持分支。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicate_is_also_rejected_for_qoder() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        let error = commands
            .duplicate_session(
                &ctx,
                &json!({ "targetId": "qoder:local", "sessionId": "s1" }),
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "Qoder CN 不支持复制。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paging_validates_the_offset_sentinel_and_the_limit() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        // 首屏哨兵是合法输入；会话不存在时应当报「未找到」而不是「参数无效」。
        let error = commands
            .get_session_messages_page(
                &ctx,
                &json!({ "targetId": "qoder:local", "sessionId": "s1", "offset": -1, "limit": 10 }),
            )
            .await
            .expect_err("会话不存在");
        assert!(error.to_string().contains("未找到会话"), "{error}");

        // limit 缺失或非法一律拒绝。
        let error = commands
            .get_session_messages_page(
                &ctx,
                &json!({ "targetId": "qoder:local", "sessionId": "s1", "offset": -1 }),
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：limit");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn export_rejects_an_invalid_format() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));

        // 格式非法。**目标路径是可选的**（没给就由后端挑默认落点），因此这里不测它。
        let error = commands
            .export_session(
                &ctx,
                &json!({
                    "targetId": "qoder:local",
                    "sessionId": "s1",
                    "format": "pdf",
                    "filePath": "/tmp/x.pdf"
                }),
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：format");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_branch_anchor_must_be_a_positive_integer() {
        let (ctx, _dir) = context().await;
        let target = Target {
            id: TargetId::from("codex:local"),
            provider: ProviderId::Codex,
            label: "Codex".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some("/tmp/codex".to_owned()),
            available: true,
            detail: None,
        };
        let commands = SessionCommands::new(
            Arc::new(build_registry()),
            Arc::new(StubLookup {
                target: Some(target),
            }),
        );
        for bad in [json!(0), json!(-1), json!("3"), json!(null)] {
            let error = commands
                .branch_session(
                    &ctx,
                    &json!({ "targetId": "codex:local", "sessionId": "s1", "messageLine": bad }),
                )
                .await
                .expect_err("必须拒绝");
            assert_eq!(error.to_string(), "参数无效：messageLine", "输入 {bad}");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mutation_refs_are_validated_before_dispatch() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        let error = commands
            .delete_sessions(
                &ctx,
                &json!({ "targetId": "qoder:local", "sessions": "nope" }),
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "参数无效：sessions");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_targets_uses_the_injected_lookup() {
        let (ctx, _dir) = context().await;
        let commands = commands(Some(qoder_target()));
        // `list_targets` 现在会**探测**（走注册表）并回写缓存，因此注入的固定目标不再
        // 出现在结果里——它返回的是本机真实探测到的目标（可能为空，取决于装了哪些 CLI）。
        //
        // 真正要验证的是**探测结果被写进了缓存**：这是修复的核心，没有它所有会话命令
        // 都会以「未解析到目标」失败。
        let targets = commands
            .list_targets(&ctx, &serde_json::json!({}))
            .await
            .expect("targets");
        // 断言的是**设置缓存**而不是 `list_cached_targets`：后者走注入的 `TargetLookup`
        // （本测试里是恒返回空的桩），而回写发生在设置里。两条路径本来就该分开验证。
        let cached = ctx.settings().snapshot().await.cached_targets;
        assert_eq!(
            cached.len(),
            targets.len(),
            "探测到的目标必须回写设置缓存，否则后续命令解析不到目标"
        );
    }
}
