//! Gemini CLI `SessionProvider` 实现。
//!
//! # 与 Claude 的编排差异
//!
//! | 维度 | Claude | Gemini |
//! | --- | --- | --- |
//! | 会话发现 | `projects/<项目目录>/<文件>` | `tmp/<项目标识>/chats/<文件>` |
//! | 工作目录 | 记录里直接有 `cwd` | 靠 `projects.json` **反查** |
//! | 编号校验 | 扫第一个 `sessionId` | 扫第一个 `sessionId` |
//! | 分页 | 一行一条消息 | 一行可产出多条（`$set.messages`） |
//!
//! # `cwd` 为什么靠反查
//!
//! 会话文件里**不含**工作目录，而 `projects.json` 记录的是「工作目录 → 项目标识」。
//! 因此列表时要先读一次这张表并翻转方向。表缺失或损坏时 `cwd` 为空——用户可能删过
//! 这个文件，但历史会话仍应可读，不能因为辅助信息缺失就让列表整体失败。

use std::path::Path;
use std::sync::Arc;

use futures::stream::{self, StreamExt};

use aivc_core::error::{Error, Result, SessionError};
use aivc_core::id::SessionId;
use aivc_core::ports::{
    BoxFuture, ConflictPolicy, LineControl, ReadOptions, ReadStart, SessionStore, StoreKind,
};
use aivc_core::session::{
    BranchMetadata, BranchOrigin, ExportFormat, ExportResult, MessagePage, PageCursor, Session,
    SessionBatchResult, SessionDeleteResult, SessionMutationEntry, SessionMutationRef, SessionView,
};
use aivc_core::target::{ProviderId, Target, TargetKind};
use aivc_core::text::{matches_session_terms, normalize_search_terms};
use aivc_infra::db::{MessageIndexEntry, SessionIndexKey};

use super::branch::{build_branch_text, build_duplicate_text};
use super::parser::{LIST_PREVIEW_LIMIT, parse_content, session_id_from_file_path};
use super::paths::{GeminiLayout, MESSAGE_OPERATE, SESSION_PATH_PATTERN, parse_projects};
use crate::common::{
    LocalCliProbe, SessionFileInfo, WslCliProbe, list_cached_sessions, probe_local_cli_target,
    probe_wsl_cli_targets, sort_sessions,
};
use crate::context::ProviderContext;
use crate::export::{format_session, sanitize_for_export};
use crate::paging::{LinePolicy, PageFolder};
use crate::registry::{Branchable, Duplicable, Exportable, SessionProvider};

/// 界面展示名。
pub const DISPLAY_NAME: &str = "Gemini";
/// 会话文件头部读取上限。
pub const LIST_HEAD_BYTES: usize = 64 * 1024;
/// 列表解析并发上限。
pub const LIST_PARSE_CONCURRENCY: usize = 6;
/// 搜索扫描并发上限。
pub const SEARCH_SCAN_CONCURRENCY: usize = 8;

/// Gemini 平台实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct GeminiProvider;

impl GeminiProvider {
    /// 构造。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 目标对应的存储布局。
    fn layout(target: &Target) -> GeminiLayout {
        let kind = match target.kind {
            TargetKind::Local => StoreKind::Local,
            TargetKind::Wsl => StoreKind::Wsl,
        };
        GeminiLayout::new(target.codex_home.clone().unwrap_or_default(), kind)
    }

    /// 缓存键。
    fn cache_key(ctx: &ProviderContext, target: &Target, view: SessionView) -> String {
        format!("{}-{}", ctx.cache_key(target), view.as_str())
    }

    /// 读取「项目标识 → 工作目录」映射。
    async fn project_map(
        &self,
        ctx: &ProviderContext,
        target: &Target,
    ) -> std::collections::BTreeMap<String, String> {
        let layout = Self::layout(target);
        let store = ctx.store_for(target);
        // 读不到就返回空表：辅助信息缺失不应让列表整体失败。
        match store.read_text(&layout.projects_file()).await {
            Ok(content) => parse_projects(&content),
            Err(_) => std::collections::BTreeMap::new(),
        }
    }

    /// 枚举某视图下的会话文件（含 `cwd`）。
    async fn list_files(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        view: SessionView,
    ) -> Vec<(aivc_core::session::SessionFile, Option<String>)> {
        let layout = Self::layout(target);
        let root = match view {
            SessionView::Active => layout.active_root(),
            SessionView::Trash => layout.trash_active_root(),
        };
        let files = match target.distro() {
            Some(distro) => ctx
                .wsl()
                .list_session_files_matching(distro, &root, SESSION_PATH_PATTERN)
                .await
                .unwrap_or_default(),
            None => walk_chats_files(Path::new(&root)).await,
        };
        let projects = self.project_map(ctx, target).await;
        files
            .into_iter()
            .map(|file| {
                let key = layout.project_key_of(&file.file_path);
                let cwd = projects.get(&key).cloned();
                (file, cwd)
            })
            .collect()
    }

    /// 按编号查找会话文件。
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
            .find(|(file, _)| session_id_from_file_path(&file.file_path) == id.as_str())
            .map(|(file, _)| file.file_path)
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
            let root = match view {
                SessionView::Active => layout.active_root(),
                SessionView::Trash => layout.trash_active_root(),
            };
            crate::common::assert_session_file_inside(hint, &root, layout.kind(), MESSAGE_OPERATE)?;
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

    /// 完整加载一个会话文件。
    async fn load_full(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
        cwd: Option<String>,
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
            cwd,
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
            .map(|(file, cwd)| {
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
                    let mut session = parse_content(info, cwd, &head, Some(LIST_PREVIEW_LIMIT))?;
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

        let mut folder = PageFolder::new(
            cursor,
            u64::from(limit),
            anchor.as_ref(),
            LinePolicy::Gemini,
        );
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

    /// 本机配置目录：`~/.gemini`。
    fn default_config_dir() -> Option<String> {
        for variable in ["USERPROFILE", "HOME"] {
            if let Ok(value) = std::env::var(variable) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(
                        Path::new(trimmed)
                            .join(".gemini")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        None
    }
}

/// 枚举 `tmp/<项目标识>/chats/*.jsonl`。
///
/// 形状固定为三层，因此不递归到第四层——项目目录里可能放着附件或备份。
pub async fn walk_chats_files(root: &Path) -> Vec<aivc_core::session::SessionFile> {
    let mut files = Vec::new();
    let Ok(mut projects) = tokio::fs::read_dir(root).await else {
        return files;
    };
    while let Ok(Some(project)) = projects.next_entry().await {
        if !project.metadata().await.is_ok_and(|item| item.is_dir()) {
            continue;
        }
        let chats = project.path().join("chats");
        let Ok(mut entries) = tokio::fs::read_dir(&chats).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !super::parser::is_session_file_name(&name) {
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

impl SessionProvider for GeminiProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn label(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn list_targets<'a>(&'a self, ctx: &'a ProviderContext) -> BoxFuture<'a, Result<Vec<Target>>> {
        Box::pin(async move {
            let mut targets = Vec::new();
            if let Some(config_dir) = Self::default_config_dir()
                && let Some(target) = probe_local_cli_target(&LocalCliProbe {
                    provider: ProviderId::Gemini,
                    display_name: DISPLAY_NAME,
                    windows_command: "gemini",
                    unix_command: "gemini",
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
                        provider: ProviderId::Gemini,
                        display_name: DISPLAY_NAME,
                        command: "gemini",
                        config_dir_suffix: ".gemini",
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
                            session.cwd.clone(),
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
            let (path, cwd) = match file_hint.filter(|value| !value.is_empty()) {
                Some(hint) => {
                    layout.assert_readable(hint)?;
                    let key = layout.project_key_of(hint);
                    let cwd = self.project_map(ctx, target).await.get(&key).cloned();
                    (hint.to_owned(), cwd)
                }
                None => self
                    .find_entry(ctx, target, id, SessionView::Active)
                    .await
                    .or(self.find_entry(ctx, target, id, SessionView::Trash).await)
                    .ok_or_else(|| SessionError::NotFound {
                        id: id.as_str().to_owned(),
                    })?,
            };
            let session = self
                .load_full(ctx, target, &path, cwd)
                .await?
                .ok_or_else(|| SessionError::NotFound {
                    id: id.as_str().to_owned(),
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
                if let Some((path, cwd)) = self.find_entry(ctx, target, id, view).await {
                    let store = ctx.store_for(target);
                    let head = store.read_head(&path, LIST_HEAD_BYTES).await?;
                    if let Some(session) = parse_content(
                        SessionFileInfo {
                            file_path: path,
                            mtime_ms: 0.0,
                            change_ms: None,
                            size: 0,
                        },
                        cwd,
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
        ctx: &'a ProviderContext,
        target: &'a Target,
        parent: &'a SessionId,
    ) -> BoxFuture<'a, Result<Vec<Session>>> {
        Box::pin(async move {
            let ids = ctx
                .session_metadata()
                .ids_by_parent(&target.id, parent)
                .await?;
            let mut sessions = Vec::with_capacity(ids.len());
            for id in ids {
                if let Ok(session) = self.get_session_summary(ctx, target, &id).await {
                    sessions.push(session);
                }
            }
            Ok(sessions)
        })
    }

    fn get_session_folder_path<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
    ) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            let (path, _) = self
                .find_entry(ctx, target, id, SessionView::Active)
                .await
                .or(self.find_entry(ctx, target, id, SessionView::Trash).await)
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
            let moved_to = layout.trash_path_for(&source)?;
            ctx.store_for(target)
                .move_file(&source, &moved_to, ConflictPolicy::Overwrite)
                .await?;
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
                let moved_to = layout.trash_path_for(&source)?;
                ctx.store_for(target)
                    .move_file(&source, &moved_to, ConflictPolicy::Overwrite)
                    .await?;
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
            let restored_to = layout.restore_path_for(&source)?;
            ctx.store_for(target)
                .move_file(&source, &restored_to, ConflictPolicy::FailIfExists)
                .await?;
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
            layout.assert_purgeable(&source)?;
            self.verify_session_id(ctx, target, &source, &reference.id)
                .await?;
            ctx.store_for(target).remove(&source).await?;
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
                layout.assert_purgeable(&source)?;
                self.verify_session_id(ctx, target, &source, &reference.id)
                    .await?;
                ctx.store_for(target).remove(&source).await?;
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

impl GeminiProvider {
    /// 按编号查找会话文件并附带 `cwd`。
    async fn find_entry(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        id: &SessionId,
        view: SessionView,
    ) -> Option<(String, Option<String>)> {
        self.list_files(ctx, target, view)
            .await
            .into_iter()
            .find(|(file, _)| session_id_from_file_path(&file.file_path) == id.as_str())
            .map(|(file, cwd)| (file.file_path, cwd))
    }

    /// 写入派生文件并解析回来。
    async fn write_derived(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        path: &str,
        text: &str,
        cwd: Option<String>,
        not_found: SessionError,
    ) -> Result<Session> {
        let store = ctx.store_for(target);
        store.write_text(path, text).await?;
        let parsed = parse_content(
            SessionFileInfo {
                file_path: path.to_owned(),
                mtime_ms: 0.0,
                change_ms: None,
                size: text.len() as u64,
            },
            cwd,
            text,
            None,
        )
        .ok_or(not_found)?;
        self.evict_cache(ctx, target, SessionView::Active, &[path.to_owned()])
            .await;
        Ok(parsed)
    }
}

impl Branchable for GeminiProvider {
    fn branch_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        message_line: u64,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            // 注意：Gemini 的这个参数是**可见消息条数**，不是 JSONL 行号。
            let session = self.get_session_summary(ctx, target, id).await?;
            let layout = Self::layout(target);
            layout.assert_operable(&session.file_path)?;
            let source_text = ctx.store_for(target).read_text(&session.file_path).await?;
            let branch_id = uuid::Uuid::new_v4().to_string();
            let now = ctx.clock().now();
            let text = build_branch_text(&source_text, &branch_id, &now, message_line)?;
            let path = layout.derived_path_for(&session.file_path, &branch_id, &now)?;
            let branch = self
                .write_derived(
                    ctx,
                    target,
                    &path,
                    &text,
                    session.cwd.clone(),
                    SessionError::GeminiInsufficientContext,
                )
                .await?;

            let metadata = BranchMetadata {
                parent_target_id: Some(target.id.clone()),
                parent_session_id: Some(session.id.clone()),
                parent_message_index: Some(message_line),
                parent_message_line: None,
                created_by: Some(BranchOrigin::Branch),
            };
            ctx.session_metadata()
                .set_branch(&target.id, &branch.id, &metadata)
                .await?;
            self.attach(ctx, target, branch).await
        })
    }
}

impl Duplicable for GeminiProvider {
    fn duplicate_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        title: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let session = self.get_session(ctx, target, id, None).await?;
            let layout = Self::layout(target);
            layout.assert_operable(&session.file_path)?;
            let source_text = ctx.store_for(target).read_text(&session.file_path).await?;
            let duplicate_id = uuid::Uuid::new_v4().to_string();
            let now = ctx.clock().now();
            let text = build_duplicate_text(&source_text, &duplicate_id, &now)?;
            let path = layout.derived_path_for(&session.file_path, &duplicate_id, &now)?;
            let duplicated = self
                .write_derived(
                    ctx,
                    target,
                    &path,
                    &text,
                    session.cwd.clone(),
                    SessionError::GeminiDuplicateMissingMetadata,
                )
                .await?;

            if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
                ctx.session_metadata()
                    .set_custom_title(&target.id, &duplicated.id, title)
                    .await?;
            }
            self.attach(ctx, target, duplicated).await
        })
    }
}

impl Exportable for GeminiProvider {
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

/// 便于上层构造「不支持」错误。
#[must_use]
pub fn unsupported(capability: &str) -> Error {
    Error::Unsupported {
        provider: DISPLAY_NAME.to_owned(),
        capability: capability.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestContext;

    const SESSION_A: &str = "aaaaaaaa";
    const SESSION_B: &str = "bbbbbbbb";

    fn gemini_target(ctx: &TestContext) -> Target {
        Target {
            id: aivc_core::id::TargetId::from("gemini:local"),
            provider: ProviderId::Gemini,
            label: "Gemini：本机（win32）".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some(ctx.codex_home.to_string_lossy().into_owned()),
            available: true,
            detail: None,
        }
    }

    fn relative(session_id: &str) -> String {
        format!("tmp/proj-{session_id}/chats/session-2026-09-16T12-34-{session_id}.jsonl")
    }

    fn content(session_id: &str, title: &str) -> String {
        [
            format!(
                r#"{{"sessionId":"{session_id}","projectHash":"hash-{session_id}","startTime":"2026-09-16T12:00:00.000Z","lastUpdated":"2026-09-16T12:05:00.000Z","kind":"main"}}"#
            ),
            format!(r#"{{"type":"user","content":"{title}"}}"#),
            r#"{"type":"gemini","content":"回答","model":"gemini-2.5-pro"}"#.to_owned(),
        ]
        .join("\n")
    }

    fn mutation(id: &str) -> SessionMutationRef {
        SessionMutationRef {
            id: SessionId::from(id),
            file_path: None,
        }
    }

    #[test]
    fn provider_reports_gemini_identity() {
        let provider = GeminiProvider::new();
        assert_eq!(provider.id(), ProviderId::Gemini);
        assert_eq!(provider.label(), "Gemini");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_reads_three_level_chats_directories() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "标题 A"));

        let sessions = GeminiProvider::new()
            .list_sessions(&ctx.context, &target)
            .await
            .expect("list");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_str(), SESSION_A);
        assert_eq!(sessions[0].title, "标题 A");
        // 见 codex 同名断言的说明：四个 provider 各有一条列表路径，漏一个就会让那一个
        // 把下界当成精确值报出去。
        assert!(
            sessions[0].message_count_is_lower_bound,
            "列表路径必须标注条数是下界"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cwd_is_resolved_through_projects_json() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "标题"));
        // projects.json 的方向是「工作目录 → 项目标识」，需要反查。
        ctx.write_session(
            "projects.json",
            &format!(r#"{{"projects":{{"/home/a/work":"proj-{SESSION_A}"}}}}"#),
        );

        let sessions = GeminiProvider::new()
            .list_sessions(&ctx.context, &target)
            .await
            .expect("list");
        assert_eq!(sessions[0].cwd.as_deref(), Some("/home/a/work"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_missing_projects_file_leaves_cwd_empty_without_failing() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "标题"));

        let sessions = GeminiProvider::new()
            .list_sessions(&ctx.context, &target)
            .await
            .expect("列表不应因缺少 projects.json 而失败");
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].cwd.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_moves_the_file_into_the_trash_tree() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        let source = ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "删除"));

        let result = GeminiProvider::new()
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");
        assert!(result.stale.is_none());
        assert!(!source.exists());
        assert!(
            ctx.codex_home
                .join(".visual-console-trash")
                .join(relative(SESSION_A).replace('/', std::path::MAIN_SEPARATOR_STR))
                .exists()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_then_restore_round_trips() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        let source = ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "往返"));

        let provider = GeminiProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");
        let restored = provider
            .restore_session(&ctx.context, &target, &SessionId::from(SESSION_A))
            .await
            .expect("restore");
        assert_eq!(Path::new(&restored), source.as_path());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn purge_removes_the_file_from_the_trash() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "彻底删除"));
        let provider = GeminiProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");

        let deleted = provider
            .purge_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("purge");
        assert!(!Path::new(&deleted).exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_rejects_a_forged_path_outside_the_tmp_root() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        let outside = ctx.write_session("projects.json", "{}");

        let error = GeminiProvider::new()
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
    async fn delete_rejects_a_path_whose_content_has_a_different_id() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        let other = ctx.write_session(&relative(SESSION_B), &content(SESSION_B, "B"));

        let error = GeminiProvider::new()
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
        let target = gemini_target(&ctx);
        let mut lines = vec![format!(
            r#"{{"sessionId":"{SESSION_A}","projectHash":"h","kind":"main"}}"#
        )];
        for index in 0..10 {
            lines.push(format!(r#"{{"type":"user","content":"消息{index}"}}"#));
        }
        ctx.write_session(&relative(SESSION_A), &lines.join("\n"));

        let page = GeminiProvider::new()
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
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn branching_truncates_at_the_requested_message_count() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "第一条"));

        let branch = GeminiProvider::new()
            .branch_session(&ctx.context, &target, &SessionId::from(SESSION_A), 1)
            .await
            .expect("branch");
        assert_ne!(branch.id.as_str(), SESSION_A);
        assert!(Path::new(&branch.file_path).exists());
        // 分支文件必须带新编号与结尾的 lastUpdated 补丁。
        let text = std::fs::read_to_string(&branch.file_path).expect("read");
        assert!(text.contains(&branch.id.as_str().to_owned()));
        assert!(text.contains("lastUpdated"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn branching_with_too_many_messages_fails() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "只有两条"));

        let error = GeminiProvider::new()
            .branch_session(&ctx.context, &target, &SessionId::from(SESSION_A), 99)
            .await
            .expect_err("必须失败");
        assert_eq!(error.to_string(), "Gemini 源会话上下文不足，无法创建分支。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicating_creates_an_independent_copy() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "原始"));

        let duplicate = GeminiProvider::new()
            .duplicate_session(&ctx.context, &target, &SessionId::from(SESSION_A), None)
            .await
            .expect("duplicate");
        assert_ne!(duplicate.id.as_str(), SESSION_A);
        let text = std::fs::read_to_string(&duplicate.file_path).expect("read");
        assert!(text.contains("原始"));
        // 精确检查 sessionId 字段：projectHash 里也含源编号，不能用整串包含判断。
        let meta: serde_json::Value = text
            .lines()
            .map(|line| serde_json::from_str(line).expect("每行都是 JSON"))
            .find(|value: &serde_json::Value| value.get("sessionId").is_some())
            .expect("元数据记录");
        assert_eq!(meta["sessionId"], duplicate.id.as_str());
        assert_ne!(meta["sessionId"], SESSION_A);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exporting_writes_markdown_to_the_host() {
        let ctx = TestContext::new().await;
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "导出标题"));
        let destination = ctx.root().join("export").join("gemini.md");

        let result = GeminiProvider::new()
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
        let target = gemini_target(&ctx);
        ctx.write_session(&relative(SESSION_A), &content(SESSION_A, "活跃"));
        ctx.write_session(
            &format!(".visual-console-trash/{}", relative(SESSION_B)),
            &content(SESSION_B, "回收站"),
        );

        let provider = GeminiProvider::new();
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
