//! Qoder CN `SessionProvider` 实现。
//!
//! # 能力最窄，但变更最复杂
//!
//! Qoder 不支持分支与复制（这两项必须在它自己的 CLI 内完成），因此只实现
//! `SessionProvider` 与 `Exportable`。但它的**变更操作**反而是四个平台里最复杂的：
//! 一份会话由五个产物组成，移动必须作为一组原子处理。
//!
//! # 移动的两道保护
//!
//! 1. **整体预检**：动任何东西之前检查全部目标位置，只要有一个已存在就整体拒绝。
//!    部分移动会让会话处于「transcript 在回收站、关联目录在活跃目录」的分裂状态，
//!    比直接失败更难收拾。
//! 2. **失败回滚**：逐个移动，中途失败时按**逆序**把已移动的移回去。
//!
//! 现网对本机与 WSL 各写了一套（WSL 那套用 shell 的 `trap rollback ERR`）。这里用
//! 一份 Rust 实现覆盖两者——`ProviderStore` 已经把两个后端的差异收敛掉了，
//! 再分两套只会让回滚逻辑有两处可能不一致。

use std::path::Path;
use std::sync::Arc;

use futures::stream::{self, StreamExt};

use aivc_core::error::{Error, Result, SessionError};
use aivc_core::id::SessionId;
use aivc_core::ports::{
    BoxFuture, ConflictPolicy, LineControl, ReadOptions, ReadStart, SessionStore, StoreKind,
};
use aivc_core::session::{
    ExportFormat, ExportResult, MessagePage, PageCursor, Session, SessionBatchResult,
    SessionDeleteResult, SessionMutationEntry, SessionMutationRef, SessionView,
};
use aivc_core::target::{ProviderId, Target, TargetKind};
use aivc_core::text::{matches_session_terms, normalize_search_terms};
use aivc_infra::db::{MessageIndexEntry, SessionIndexKey};

use super::parser::{LIST_PREVIEW_LIMIT, parse_content};
use super::paths::{
    CONFIG_DIR_ENV, MESSAGE_OPERATE, QoderLayout, QoderStoragePath, SESSION_FILE_GLOB,
    session_id_from_file_path,
};
use crate::common::{
    LocalCliProbe, SessionFileInfo, WslCliProbe, list_cached_sessions, probe_local_cli_target,
    probe_wsl_cli_targets, sort_sessions,
};
use crate::context::ProviderContext;
use crate::export::{format_session, sanitize_for_export};
use crate::paging::{LinePolicy, PageFolder};
use crate::registry::{Exportable, SessionProvider};

/// 界面展示名。
pub const DISPLAY_NAME: &str = "Qoder";
/// CLI 命令名。
pub const COMMAND: &str = "qodercn";
/// 会话文件头部读取上限。
pub const LIST_HEAD_BYTES: usize = 64 * 1024;
/// 列表解析并发上限。
pub const LIST_PARSE_CONCURRENCY: usize = 6;
/// 搜索扫描并发上限。
pub const SEARCH_SCAN_CONCURRENCY: usize = 8;

/// Qoder 平台实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct QoderProvider;

impl QoderProvider {
    /// 构造。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 目标对应的存储布局。
    fn layout(target: &Target) -> QoderLayout {
        let kind = match target.kind {
            TargetKind::Local => StoreKind::Local,
            TargetKind::Wsl => StoreKind::Wsl,
        };
        QoderLayout::new(target.codex_home.clone().unwrap_or_default(), kind)
    }

    /// 缓存键。
    fn cache_key(ctx: &ProviderContext, target: &Target, view: SessionView) -> String {
        format!("{}-{}", ctx.cache_key(target), view.as_str())
    }

    /// 默认配置目录：`QODERCN_CONFIG_DIR` 优先，其次 `~/.qoder-cn`。
    fn default_config_dir() -> Option<String> {
        if let Ok(value) = std::env::var(CONFIG_DIR_ENV) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_owned());
            }
        }
        for variable in ["USERPROFILE", "HOME"] {
            if let Ok(value) = std::env::var(variable) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(
                        Path::new(trimmed)
                            .join(".qoder-cn")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        None
    }

    /// 枚举某视图下的会话文件。
    async fn list_files(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        view: SessionView,
    ) -> Vec<aivc_core::session::SessionFile> {
        let layout = Self::layout(target);
        let root = layout.projects_root(view);
        match target.distro() {
            Some(distro) => ctx
                .wsl()
                .list_session_files_at_depth(distro, &root, SESSION_FILE_GLOB, 2, 2)
                .await
                .unwrap_or_default(),
            None => walk_projects_files(Path::new(&root)).await,
        }
    }

    /// 按编号查找 transcript 路径。
    async fn find_file(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        id: &SessionId,
        view: SessionView,
    ) -> Option<String> {
        self.list_files(ctx, target, view)
            .await
            .into_iter()
            .find(|file| session_id_from_file_path(&file.file_path) == id.as_str())
            .map(|file| file.file_path)
    }

    /// 校验文件里的 `sessionId` 与请求的编号一致。
    async fn verify_session_id(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
        id: &SessionId,
    ) -> Result<()> {
        let mut found = false;
        let mut mismatch = false;
        ctx.store_for(target)
            .read_lines(
                file_path,
                ReadStart::BEGINNING,
                ReadOptions::none(),
                Box::new(|line: &str, _number: u64, _offset: u64| {
                    let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                        return LineControl::Continue;
                    };
                    let candidate = value
                        .get("sessionId")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if candidate.is_empty() {
                        return LineControl::Continue;
                    }
                    found = true;
                    if candidate != id.as_str() {
                        mismatch = true;
                    }
                    LineControl::Stop
                }),
            )
            .await?;
        if mismatch || !found {
            return Err(SessionError::SessionIdMismatch {
                id: id.as_str().to_owned(),
            }
            .into());
        }
        Ok(())
    }

    /// 解析「要动哪个文件」。
    async fn resolve_source(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        id: &SessionId,
        hint: Option<&str>,
        view: SessionView,
    ) -> Result<Option<String>> {
        let layout = Self::layout(target);
        if let Some(hint) = hint.filter(|value| !value.is_empty()) {
            layout.assert_operable(hint, view)?;
            if ctx.store_for(target).exists(hint).await.unwrap_or(false) {
                return Ok(Some(hint.to_owned()));
            }
        }
        Ok(self.find_file(ctx, target, id, view).await)
    }

    /// 清掉缓存条目。
    async fn evict_cache(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        view: SessionView,
        paths: &[String],
    ) {
        if paths.is_empty() {
            return;
        }
        let key = Self::cache_key(ctx, target, view);
        if let Err(error) = ctx.session_cache().remove_entries(&key, paths).await {
            tracing::warn!(%error, "清理会话缓存失败");
        }
    }

    /// 完整加载一个会话。
    async fn load_full(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
    ) -> Result<Option<Session>> {
        let content = ctx.store_for(target).read_text(file_path).await?;
        let size = content.len() as u64;
        Ok(parse_content(
            SessionFileInfo {
                file_path: file_path.to_owned(),
                mtime_ms: 0.0,
                change_ms: None,
                size,
            },
            &content,
            None,
        ))
    }

    /// 附着元数据。
    async fn attach(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        session: Session,
    ) -> Result<Session> {
        ctx.session_metadata().attach(&target.id, session).await
    }

    /// 列出某个视图下的会话。
    async fn list_view(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        view: SessionView,
    ) -> Result<Vec<Session>> {
        let files = self.list_files(ctx, target, view).await;
        let key = Self::cache_key(ctx, target, view);
        if files.is_empty() {
            ctx.session_cache().replace(&key, &[]).await?;
            return Ok(Vec::new());
        }
        let store = Arc::new(ctx.store_for(target));
        let cached = ctx.session_cache().read(&key).await?;

        let results: Vec<Option<(aivc_infra::db::SessionCacheEntry, bool)>> = stream::iter(files)
            .map(|file| {
                let cached = cached.clone();
                let store = Arc::clone(&store);
                async move {
                    let change_ms = file.change_ms.unwrap_or(file.mtime_ms);
                    if let Some(entry) = cached.get(&file.file_path)
                        && entry.is_fresh_for(file.mtime_ms, change_ms, file.size)
                    {
                        return Some((entry.clone(), true));
                    }
                    let info = SessionFileInfo::from_session_file(&file);
                    let Ok(head) = store.read_head(&file.file_path, LIST_HEAD_BYTES).await else {
                        return None;
                    };
                    let mut session = parse_content(info, &head, Some(LIST_PREVIEW_LIMIT))?;
                    // 只读了头部窗口，数出来的条数是**下界**——必须随数据一起标注，
                    // 否则消费方会把它当成精确总数。见 `codex/listing.rs` 的说明。
                    session.message_count_is_lower_bound = true;
                    Some((
                        aivc_infra::db::SessionCacheEntry {
                            file_path: file.file_path.clone(),
                            mtime_ms: file.mtime_ms,
                            change_ms,
                            size: file.size,
                            session,
                        },
                        false,
                    ))
                }
            })
            .buffered(LIST_PARSE_CONCURRENCY)
            .collect()
            .await;

        let mut entries = Vec::with_capacity(results.len());
        let mut sessions = Vec::with_capacity(results.len());
        for (entry, _hit) in results.into_iter().flatten() {
            sessions.push(entry.session.clone());
            entries.push(entry);
        }
        ctx.session_cache().replace(&key, &entries).await?;
        sort_sessions(&mut sessions);
        ctx.session_metadata()
            .attach_list(&target.id, sessions)
            .await
    }

    /// 读取并分页。
    async fn read_page(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        session: &Session,
        cursor: PageCursor,
        limit: u32,
    ) -> Result<MessagePage> {
        let key = SessionIndexKey::from_session(&target.id, session);
        let anchor = if cursor.is_latest() {
            None
        } else {
            ctx.message_index()
                .find_anchor_at_or_before(&key, cursor.to_wire().max(0).cast_unsigned())
                .await?
        };
        let mut folder =
            PageFolder::new(cursor, u64::from(limit), anchor.as_ref(), LinePolicy::Qoder);
        let store = ctx.store_for(target);
        let start = match folder.start_byte() {
            Some(offset) => ReadStart::Byte {
                offset,
                line_number: folder.start_line(),
            },
            None => ReadStart::Line(folder.start_line()),
        };
        store
            .read_lines(
                &session.file_path,
                start,
                ReadOptions::none(),
                Box::new(|line: &str, line_number: u64, byte_offset: u64| {
                    if folder.push_line(line, line_number, byte_offset) {
                        LineControl::Continue
                    } else {
                        LineControl::Stop
                    }
                }),
            )
            .await?;
        let outcome = folder.finish(session.message_count);
        if let Err(error) = ctx
            .message_index()
            .save(&MessageIndexEntry {
                key,
                anchors: outcome.anchors.clone(),
                message_count: outcome.message_count,
            })
            .await
        {
            tracing::warn!(%error, "写入消息索引失败");
        }
        Ok(outcome.page)
    }

    /// 把一份会话的全部产物从一个视图移到另一个视图。
    ///
    /// 见模块文档：整体预检 + 失败回滚。
    async fn move_artifacts(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        paths: &[QoderStoragePath],
    ) -> Result<()> {
        let store = ctx.store_for(target);

        // 第一道保护：整体预检。任一目标已存在就整体拒绝。
        for item in paths {
            let source_exists = store.exists(&item.source).await.unwrap_or(false);
            if !source_exists {
                continue;
            }
            if store.exists(&item.destination).await.unwrap_or(false) {
                return Err(SessionError::MoveTargetExists.into());
            }
        }

        // 第二道保护：逐个移动，失败时按逆序回滚。
        let mut moved: Vec<&QoderStoragePath> = Vec::new();
        for item in paths {
            if !store.exists(&item.source).await.unwrap_or(false) {
                continue;
            }
            match store
                .move_file(&item.source, &item.destination, ConflictPolicy::Overwrite)
                .await
            {
                Ok(()) => moved.push(item),
                Err(error) => {
                    for done in moved.iter().rev() {
                        // 回滚本身失败无法补救，只能记录——此时原始错误更重要。
                        if let Err(rollback) = store
                            .move_file(&done.destination, &done.source, ConflictPolicy::Overwrite)
                            .await
                        {
                            tracing::warn!(%rollback, path = %done.source, "回滚会话产物失败");
                        }
                    }
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    /// 删除一份会话的全部产物。
    async fn remove_artifacts(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        paths: &[QoderStoragePath],
    ) -> Result<()> {
        let store = ctx.store_for(target);
        for item in paths {
            // 目录产物需要递归删除；`rm -f` 对目录无效。
            store.remove_tree(&item.source).await?;
        }
        Ok(())
    }
}

/// 枚举 `projects/<项目键>/*.jsonl`（固定两层）。
pub async fn walk_projects_files(root: &Path) -> Vec<aivc_core::session::SessionFile> {
    let mut files = Vec::new();
    let Ok(mut projects) = tokio::fs::read_dir(root).await else {
        return files;
    };
    while let Ok(Some(project)) = projects.next_entry().await {
        if !project.metadata().await.is_ok_and(|item| item.is_dir()) {
            continue;
        }
        let Ok(mut entries) = tokio::fs::read_dir(project.path()).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !super::paths::is_session_file_name(&name) {
                continue;
            }
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0.0, |delta| delta.as_millis() as f64);
            files.push(aivc_core::session::SessionFile {
                file_path: entry.path().to_string_lossy().into_owned(),
                mtime_ms,
                change_ms: Some(mtime_ms),
                size: metadata.len(),
            });
        }
    }
    files
}

impl SessionProvider for QoderProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Qoder
    }

    fn label(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn list_targets<'a>(&'a self, ctx: &'a ProviderContext) -> BoxFuture<'a, Result<Vec<Target>>> {
        Box::pin(async move {
            let mut targets = Vec::new();
            if let Some(config_dir) = Self::default_config_dir()
                && let Some(target) = probe_local_cli_target(&LocalCliProbe {
                    provider: ProviderId::Qoder,
                    display_name: DISPLAY_NAME,
                    windows_command: "qodercn.cmd",
                    unix_command: COMMAND,
                    config_dir,
                })
                .await
            {
                targets.push(target);
            }
            targets.extend(
                probe_wsl_cli_targets(
                    ctx,
                    &WslCliProbe {
                        provider: ProviderId::Qoder,
                        display_name: DISPLAY_NAME,
                        command: COMMAND,
                        config_dir_suffix: ".qoder-cn",
                    },
                )
                .await,
            );
            Ok(targets)
        })
    }

    fn list_cached_targets<'a>(
        &'a self,
        _ctx: &'a ProviderContext,
    ) -> BoxFuture<'a, Result<Vec<Target>>> {
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn list_cached_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        view: SessionView,
    ) -> BoxFuture<'a, Result<Vec<Session>>> {
        Box::pin(async move {
            let key = Self::cache_key(ctx, target, view);
            list_cached_sessions(ctx, &key).await
        })
    }

    fn list_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
    ) -> BoxFuture<'a, Result<Vec<Session>>> {
        Box::pin(async move { self.list_view(ctx, target, SessionView::Active).await })
    }

    fn list_trash_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
    ) -> BoxFuture<'a, Result<Vec<Session>>> {
        Box::pin(async move { self.list_view(ctx, target, SessionView::Trash).await })
    }

    fn search_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        view: SessionView,
        query: &'a str,
    ) -> BoxFuture<'a, Result<Vec<Session>>> {
        Box::pin(async move {
            let terms = normalize_search_terms(query);
            let list = self.list_view(ctx, target, view).await?;
            if terms.is_empty() {
                return Ok(list);
            }
            let store = ctx.store_for(target);
            let matched: Vec<Option<Session>> = stream::iter(list)
                .map(|session| {
                    let store = &store;
                    let terms = terms.clone();
                    async move {
                        if matches_session_terms(&session, &terms) {
                            return Some(session);
                        }
                        let content = store.read_text(&session.file_path).await.ok()?;
                        let full = parse_content(
                            SessionFileInfo {
                                file_path: session.file_path.clone(),
                                mtime_ms: session.file_mtime_ms.unwrap_or(0.0),
                                change_ms: session.file_change_ms,
                                size: session.file_size.unwrap_or(0),
                            },
                            &content,
                            None,
                        )?;
                        let candidate = Session {
                            title: session.title.clone(),
                            source_title: session.source_title.clone(),
                            metadata: session.metadata.clone(),
                            ..full
                        };
                        matches_session_terms(&candidate, &terms).then_some(candidate)
                    }
                })
                .buffered(SEARCH_SCAN_CONCURRENCY)
                .collect()
                .await;
            Ok(matched.into_iter().flatten().collect())
        })
    }

    fn get_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        file_hint: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let layout = Self::layout(target);
            let path = match file_hint.filter(|value| !value.is_empty()) {
                Some(hint) => {
                    layout.view_for_path(hint)?;
                    hint.to_owned()
                }
                None => self
                    .find_file(ctx, target, id, SessionView::Active)
                    .await
                    .or(self.find_file(ctx, target, id, SessionView::Trash).await)
                    .ok_or_else(|| SessionError::NotFound {
                        id: id.as_str().to_owned(),
                    })?,
            };
            let session = self.load_full(ctx, target, &path).await?.ok_or_else(|| {
                SessionError::NotFound {
                    id: id.as_str().to_owned(),
                }
            })?;
            self.attach(ctx, target, session).await
        })
    }

    fn get_session_messages_page<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        cursor: PageCursor,
        limit: u32,
    ) -> BoxFuture<'a, Result<MessagePage>> {
        Box::pin(async move {
            let session = self.get_session_summary(ctx, target, id).await?;
            self.read_page(ctx, target, &session, cursor, limit).await
        })
    }

    fn get_session_summary<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            for view in [SessionView::Active, SessionView::Trash] {
                let key = Self::cache_key(ctx, target, view);
                let cached = list_cached_sessions(ctx, &key).await?;
                if let Some(session) = cached.into_iter().find(|item| item.id == *id) {
                    return Ok(session);
                }
            }
            for view in [SessionView::Active, SessionView::Trash] {
                if let Some(path) = self.find_file(ctx, target, id, view).await {
                    let store = ctx.store_for(target);
                    let head = store.read_head(&path, LIST_HEAD_BYTES).await?;
                    if let Some(session) = parse_content(
                        SessionFileInfo {
                            file_path: path,
                            mtime_ms: 0.0,
                            change_ms: None,
                            size: 0,
                        },
                        &head,
                        Some(LIST_PREVIEW_LIMIT),
                    ) {
                        return self.attach(ctx, target, session).await;
                    }
                }
            }
            Err(SessionError::NotFound {
                id: id.as_str().to_owned(),
            }
            .into())
        })
    }

    fn list_sessions_by_parent<'a>(
        &'a self,
        _ctx: &'a ProviderContext,
        _target: &'a Target,
        _parent: &'a SessionId,
    ) -> BoxFuture<'a, Result<Vec<Session>>> {
        // Qoder 没有分支概念，因此不存在子会话。返回空列表而不是报错：
        // 调用方只需要「有没有子会话」这个答案。
        Box::pin(async move { Ok(Vec::new()) })
    }

    fn get_session_folder_path<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let path = self
                .find_file(ctx, target, id, SessionView::Active)
                .await
                .or(self.find_file(ctx, target, id, SessionView::Trash).await)
                .ok_or_else(|| SessionError::NotFound {
                    id: id.as_str().to_owned(),
                })?;
            Ok(crate::common::dir_name(&path, Self::layout(target).kind()))
        })
    }

    fn delete_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        reference: &'a SessionMutationRef,
    ) -> BoxFuture<'a, Result<SessionDeleteResult>> {
        Box::pin(async move {
            let layout = Self::layout(target);
            let source = self
                .resolve_source(
                    ctx,
                    target,
                    &reference.id,
                    reference.file_path.as_deref(),
                    SessionView::Active,
                )
                .await?;
            let Some(source) = source else {
                if let Some(hint) = reference.file_path.as_deref() {
                    self.evict_cache(ctx, target, SessionView::Active, &[hint.to_owned()])
                        .await;
                }
                return Ok(SessionDeleteResult {
                    moved_to: None,
                    stale: Some(true),
                });
            };
            self.verify_session_id(ctx, target, &source, &reference.id)
                .await?;
            let paths = layout.storage_paths(
                &source,
                reference.id.as_str(),
                SessionView::Active,
                Some(SessionView::Trash),
            )?;
            let moved_to = paths
                .iter()
                .find(|item| item.primary)
                .map_or_else(|| source.clone(), |item| item.destination.clone());
            self.move_artifacts(ctx, target, &paths).await?;

            self.evict_cache(ctx, target, SessionView::Active, &[source])
                .await;
            self.evict_cache(
                ctx,
                target,
                SessionView::Trash,
                std::slice::from_ref(&moved_to),
            )
            .await;
            Ok(SessionDeleteResult {
                moved_to: Some(moved_to),
                stale: None,
            })
        })
    }

    fn delete_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        references: &'a [SessionMutationRef],
    ) -> BoxFuture<'a, Result<SessionBatchResult>> {
        Box::pin(async move {
            let layout = Self::layout(target);
            let mut processed = Vec::with_capacity(references.len());
            let mut moved_paths = Vec::new();
            let mut trashed_paths = Vec::new();
            for reference in references {
                let source = self
                    .resolve_source(
                        ctx,
                        target,
                        &reference.id,
                        reference.file_path.as_deref(),
                        SessionView::Active,
                    )
                    .await?;
                let Some(source) = source else {
                    processed.push(SessionMutationEntry {
                        id: reference.id.clone(),
                        file_path: reference.file_path.clone(),
                        moved_to: None,
                        deleted: reference.file_path.clone(),
                    });
                    continue;
                };
                self.verify_session_id(ctx, target, &source, &reference.id)
                    .await?;
                let paths = layout.storage_paths(
                    &source,
                    reference.id.as_str(),
                    SessionView::Active,
                    Some(SessionView::Trash),
                )?;
                let moved_to = paths
                    .iter()
                    .find(|item| item.primary)
                    .map_or_else(|| source.clone(), |item| item.destination.clone());
                self.move_artifacts(ctx, target, &paths).await?;
                moved_paths.push(source.clone());
                trashed_paths.push(moved_to.clone());
                processed.push(SessionMutationEntry {
                    id: reference.id.clone(),
                    file_path: Some(source),
                    moved_to: Some(moved_to),
                    deleted: None,
                });
            }
            self.evict_cache(ctx, target, SessionView::Active, &moved_paths)
                .await;
            self.evict_cache(ctx, target, SessionView::Trash, &trashed_paths)
                .await;
            Ok(SessionBatchResult { processed })
        })
    }

    fn restore_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let layout = Self::layout(target);
            let source = self
                .find_file(ctx, target, id, SessionView::Trash)
                .await
                .ok_or_else(|| SessionError::NotFoundInTrash {
                    id: id.as_str().to_owned(),
                })?;
            let paths = layout.storage_paths(
                &source,
                id.as_str(),
                SessionView::Trash,
                Some(SessionView::Active),
            )?;
            let restored_to = paths
                .iter()
                .find(|item| item.primary)
                .map_or_else(|| source.clone(), |item| item.destination.clone());
            self.move_artifacts(ctx, target, &paths).await?;
            self.evict_cache(ctx, target, SessionView::Trash, &[source])
                .await;
            self.evict_cache(
                ctx,
                target,
                SessionView::Active,
                std::slice::from_ref(&restored_to),
            )
            .await;
            Ok(restored_to)
        })
    }

    fn purge_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        reference: &'a SessionMutationRef,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let layout = Self::layout(target);
            let source = self
                .resolve_source(
                    ctx,
                    target,
                    &reference.id,
                    reference.file_path.as_deref(),
                    SessionView::Trash,
                )
                .await?;
            let Some(source) = source else {
                if let Some(hint) = reference.file_path.as_deref() {
                    self.evict_cache(ctx, target, SessionView::Trash, &[hint.to_owned()])
                        .await;
                }
                return Err(SessionError::NotFoundInTrash {
                    id: reference.id.as_str().to_owned(),
                }
                .into());
            };
            self.verify_session_id(ctx, target, &source, &reference.id)
                .await?;
            let paths =
                layout.storage_paths(&source, reference.id.as_str(), SessionView::Trash, None)?;
            self.remove_artifacts(ctx, target, &paths).await?;
            self.evict_cache(
                ctx,
                target,
                SessionView::Trash,
                std::slice::from_ref(&source),
            )
            .await;
            Ok(source)
        })
    }

    fn purge_sessions<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        references: &'a [SessionMutationRef],
    ) -> BoxFuture<'a, Result<SessionBatchResult>> {
        Box::pin(async move {
            let layout = Self::layout(target);
            let mut processed = Vec::with_capacity(references.len());
            let mut deleted_paths = Vec::new();
            for reference in references {
                let source = self
                    .resolve_source(
                        ctx,
                        target,
                        &reference.id,
                        reference.file_path.as_deref(),
                        SessionView::Trash,
                    )
                    .await?;
                let Some(source) = source else {
                    processed.push(SessionMutationEntry {
                        id: reference.id.clone(),
                        file_path: reference.file_path.clone(),
                        moved_to: None,
                        deleted: reference.file_path.clone(),
                    });
                    continue;
                };
                self.verify_session_id(ctx, target, &source, &reference.id)
                    .await?;
                let paths = layout.storage_paths(
                    &source,
                    reference.id.as_str(),
                    SessionView::Trash,
                    None,
                )?;
                self.remove_artifacts(ctx, target, &paths).await?;
                deleted_paths.push(source.clone());
                processed.push(SessionMutationEntry {
                    id: reference.id.clone(),
                    file_path: Some(source.clone()),
                    moved_to: None,
                    deleted: Some(source),
                });
            }
            self.evict_cache(ctx, target, SessionView::Trash, &deleted_paths)
                .await;
            Ok(SessionBatchResult { processed })
        })
    }
}

impl Exportable for QoderProvider {
    fn export_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        format: ExportFormat,
        destination: &'a str,
    ) -> BoxFuture<'a, Result<ExportResult>> {
        Box::pin(async move {
            let session = self.get_session(ctx, target, id, None).await?;
            let content = format_session(&sanitize_for_export(&session), format);
            if let Some(parent) = Path::new(destination).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(destination, content).await?;
            Ok(ExportResult {
                file_path: destination.to_owned(),
                format,
            })
        })
    }
}

/// 构造「Qoder CLI 不支持该操作」的错误。
///
/// 与其他平台的「不支持」不同：这里给出的是**引导性提示**——操作要在 Qoder 自己的
/// CLI 内完成，而不是「本应用做不到」。
#[must_use]
pub fn unsupported(action: &str) -> Error {
    SessionError::QoderUnsupported {
        action: action.to_owned(),
    }
    .into()
}

/// 供上层展示的围栏文案。
#[must_use]
pub const fn operate_message() -> &'static str {
    MESSAGE_OPERATE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestContext;

    const SESSION_A: &str = "sess-aaaa";
    const SESSION_B: &str = "sess-bbbb";

    fn qoder_target(ctx: &TestContext) -> Target {
        Target {
            id: aivc_core::id::TargetId::from("qoder:local"),
            provider: ProviderId::Qoder,
            label: "Qoder：本机（win32）".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some(ctx.codex_home.to_string_lossy().into_owned()),
            available: true,
            detail: None,
        }
    }

    fn relative(session_id: &str) -> String {
        format!("projects/proj-{session_id}/{session_id}.jsonl")
    }

    fn content(session_id: &str, title: &str) -> String {
        [
            format!(r#"{{"type":"session-meta","sessionId":"{session_id}","cwd":"/work"}}"#),
            format!(
                r#"{{"type":"user","timestamp":"2026-09-16T12:00:00.000Z","message":{{"content":"{title}"}}}}"#
            ),
            r#"{"type":"assistant","timestamp":"2026-09-16T12:00:01.000Z","message":{"content":"回答"}}"#.to_owned(),
        ]
        .join("\n")
    }

    /// 写出一份会话的全部五个产物。
    fn write_all_artifacts(ctx: &TestContext, session_id: &str, title: &str) {
        ctx.write_session(&relative(session_id), &content(session_id, title));
        ctx.write_session(
            &format!("projects/proj-{session_id}/{session_id}/notes.txt"),
            "关联目录",
        );
        ctx.write_session(&format!("tasks/{session_id}/task.json"), "{}");
        ctx.write_session(&format!("file-history/{session_id}/h.json"), "{}");
        ctx.write_session(
            &format!("logs/sessions/proj-{session_id}/{session_id}/log.txt"),
            "日志",
        );
    }

    fn mutation(id: &str) -> SessionMutationRef {
        SessionMutationRef {
            id: SessionId::from(id),
            file_path: None,
        }
    }

    #[test]
    fn provider_reports_qoder_identity() {
        let provider = QoderProvider::new();
        assert_eq!(provider.id(), ProviderId::Qoder);
        assert_eq!(provider.label(), "Qoder");
    }

    #[test]
    fn unsupported_operations_give_guidance_rather_than_a_flat_refusal() {
        // 与「本应用做不到」不同：操作要在 Qoder 自己的 CLI 内完成。
        assert_eq!(
            unsupported("分支会话").to_string(),
            "Qoder CLI 当前不支持在本应用中分支会话，请在 Qoder CLI 内执行。"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_reads_two_level_project_directories() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "标题 A"));

        let sessions = QoderProvider::new()
            .list_sessions(&ctx.context, &target)
            .await
            .expect("list");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_str(), SESSION_A);
        assert_eq!(sessions[0].title, "标题 A");
        // 见 codex 那条同名断言的说明：四个 provider 各有一条列表路径，漏一个就会让
        // 那一个把下界当成精确值报出去。
        assert!(
            sessions[0].message_count_is_lower_bound,
            "列表路径必须标注条数是下界"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_moves_every_artifact_into_the_trash() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        write_all_artifacts(&ctx, SESSION_A, "删除");

        let result = QoderProvider::new()
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");
        assert!(result.stale.is_none());

        // 五个产物必须**全部**进入回收站，不能留下孤儿目录。
        let trash = ctx.codex_home.join(".visual-console-trash");
        for suffix in [
            format!("projects/proj-{SESSION_A}/{SESSION_A}.jsonl"),
            format!("projects/proj-{SESSION_A}/{SESSION_A}"),
            format!("tasks/{SESSION_A}"),
            format!("file-history/{SESSION_A}"),
            format!("logs/sessions/proj-{SESSION_A}/{SESSION_A}"),
        ] {
            let path = trash.join(suffix.replace('/', std::path::MAIN_SEPARATOR_STR));
            assert!(path.exists(), "{} 未进入回收站", path.display());
        }
        // 活跃目录里不应留下任何产物。
        assert!(
            !ctx.codex_home
                .join(relative(SESSION_A).replace('/', std::path::MAIN_SEPARATOR_STR))
                .exists()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_then_restore_round_trips_all_artifacts() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        write_all_artifacts(&ctx, SESSION_A, "往返");

        let provider = QoderProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");
        let restored = provider
            .restore_session(&ctx.context, &target, &SessionId::from(SESSION_A))
            .await
            .expect("restore");

        assert!(Path::new(&restored).exists());
        // 关联产物也要回来。
        for suffix in [
            format!("projects/proj-{SESSION_A}/{SESSION_A}"),
            format!("tasks/{SESSION_A}"),
            format!("file-history/{SESSION_A}"),
            format!("logs/sessions/proj-{SESSION_A}/{SESSION_A}"),
        ] {
            let path = ctx
                .codex_home
                .join(suffix.replace('/', std::path::MAIN_SEPARATOR_STR));
            assert!(path.exists(), "{} 未恢复", path.display());
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_refuses_when_a_destination_already_exists() {
        // 整体预检：任一目标已存在就拒绝，避免会话处于分裂状态。
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        write_all_artifacts(&ctx, SESSION_A, "冲突");
        // 预先在回收站放一个同名产物。
        ctx.write_session(
            &format!(".visual-console-trash/tasks/{SESSION_A}/task.json"),
            "{}",
        );

        let error = QoderProvider::new()
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "目标会话已存在，已拒绝覆盖。");
        // 拒绝后不得改动任何产物。
        assert!(
            ctx.codex_home
                .join(relative(SESSION_A).replace('/', std::path::MAIN_SEPARATOR_STR))
                .exists(),
            "被拒绝时不得移动任何产物"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn purge_removes_files_and_directories_recursively() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        write_all_artifacts(&ctx, SESSION_A, "彻底删除");
        let provider = QoderProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");

        let deleted = provider
            .purge_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("purge");
        assert!(!Path::new(&deleted).exists());
        // 目录产物也必须被递归删除，而不是留下空目录。
        let trash = ctx.codex_home.join(".visual-console-trash");
        assert!(!trash.join("tasks").join(SESSION_A).exists());
        assert!(!trash.join("file-history").join(SESSION_A).exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_rejects_a_forged_path_outside_the_projects_root() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        let outside = ctx.write_session("config.json", "{}");

        let error = QoderProvider::new()
            .delete_session(
                &ctx.context,
                &target,
                &SessionMutationRef {
                    id: SessionId::from(SESSION_A),
                    file_path: Some(outside.to_string_lossy().into_owned()),
                },
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), MESSAGE_OPERATE);
        assert!(outside.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_rejects_a_mismatched_session_id() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        let other = ctx.write_session(&relative(SESSION_B), &content(SESSION_B, "B"));

        let error = QoderProvider::new()
            .delete_session(
                &ctx.context,
                &target,
                &SessionMutationRef {
                    id: SessionId::from(SESSION_A),
                    file_path: Some(other.to_string_lossy().into_owned()),
                },
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            format!("会话文件与会话编号不匹配：{SESSION_A}")
        );
        assert!(other.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paging_returns_the_tail() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        let mut lines = vec![format!(
            r#"{{"type":"session-meta","sessionId":"{SESSION_A}"}}"#
        )];
        for index in 0..10 {
            lines.push(format!(
                r#"{{"type":"user","timestamp":"2026-09-16T12:00:0{}.000Z","message":{{"content":"消息{index}"}}}}"#,
                index % 10
            ));
        }
        ctx.write_session(&relative(SESSION_A), &lines.join("\n"));

        let page = QoderProvider::new()
            .get_session_messages_page(
                &ctx.context,
                &target,
                &SessionId::from(SESSION_A),
                PageCursor::Latest,
                3,
            )
            .await
            .expect("page");
        let texts: Vec<&str> = page.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, vec!["消息7", "消息8", "消息9"]);
        assert_eq!(page.offset, 7);
        assert!(page.messages.iter().all(|m| m.source_line.is_none()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_sessions_by_parent_is_always_empty() {
        // Qoder 没有分支概念；调用方只需要「有没有子会话」这个答案。
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        let children = QoderProvider::new()
            .list_sessions_by_parent(&ctx.context, &target, &SessionId::from(SESSION_A))
            .await
            .expect("children");
        assert!(children.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exporting_writes_markdown_to_the_host() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "导出标题"));
        let destination = ctx.root().join("export").join("qoder.md");

        let result = QoderProvider::new()
            .export_session(
                &ctx.context,
                &target,
                &SessionId::from(SESSION_A),
                ExportFormat::Markdown,
                &destination.to_string_lossy(),
            )
            .await
            .expect("export");
        assert_eq!(Path::new(&result.file_path), destination.as_path());
        assert!(
            std::fs::read_to_string(&destination)
                .expect("read")
                .contains("导出标题")
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trash_sessions_are_listed_separately() {
        let ctx = TestContext::new().await;
        let target = qoder_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "活跃"));
        ctx.write_session(
            &format!(".visual-console-trash/{}", relative(SESSION_B)),
            &content(SESSION_B, "回收站"),
        );

        let provider = QoderProvider::new();
        let active = provider
            .list_sessions(&ctx.context, &target)
            .await
            .expect("active");
        let trash = provider
            .list_trash_sessions(&ctx.context, &target)
            .await
            .expect("trash");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id.as_str(), SESSION_A);
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id.as_str(), SESSION_B);
    }
}
