//! Claude Code `SessionProvider` 实现。
//!
//! # 与 Codex 的编排差异
//!
//! | 维度 | Codex | Claude |
//! | --- | --- | --- |
//! | 会话发现 | 递归扫描 `sessions/`（任意深度） | 只扫 `projects/` 的**第二层** |
//! | 编号校验 | 读文件头找 `session_meta.id` | 扫全部行找第一个 `sessionId` |
//! | 分支 | 行号 → 轮次 → 官方 `thread/fork`（可回退文件截断） | 直接按可见消息条数截断 |
//! | 复制 | 重写 `payload.id` + `payload.session_id` | 重写顶层 `sessionId` |
//!
//! # 为什么只扫第二层
//!
//! `projects/<项目目录>/<文件>.jsonl` 是 Claude 的固定两层结构。项目目录名是工作目录
//! 的编码形式（可能很长且含任意字符），再往下递归没有意义，只会误收用户放在里面的
//! 备份或附件。
//!
//! # 编号校验为什么扫全部行
//!
//! Claude 不保证 `sessionId` 出现在第一行（不同于 Codex 的 `session_meta`）。因此必须
//! 逐行找**第一个**带该字段的记录：找到就比对，找不到就算不匹配——「读不到编号」
//! 与「编号不符」在安全性上等价，都不能继续动文件。

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
use super::parser::{LIST_PREVIEW_LIMIT, compact_title, parse_content};
use super::paths::{ClaudeLayout, MESSAGE_OPERATE, SESSION_FILE_GLOB, SESSION_FILE_SUFFIX};
use crate::common::{
    LocalCliProbe, SessionFileInfo, WslCliProbe, probe_local_cli_target, probe_wsl_cli_targets,
    sort_sessions,
};
use crate::context::ProviderContext;
use crate::export::{format_session, sanitize_for_export};
use crate::paging::{LinePolicy, PageFolder};
use crate::registry::{Branchable, Duplicable, Exportable, SessionProvider};

/// 界面展示名。
pub const DISPLAY_NAME: &str = "Claude Code";
/// 会话文件头部读取上限（列表页只需要前几条消息）。
pub const LIST_HEAD_BYTES: usize = 64 * 1024;
/// 列表解析并发上限。
pub const LIST_PARSE_CONCURRENCY: usize = 6;
/// 搜索时逐会话扫描的并发上限。
pub const SEARCH_SCAN_CONCURRENCY: usize = 8;

/// Claude Code 平台实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeProvider;

impl ClaudeProvider {
    /// 构造。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 目标对应的存储布局。
    fn layout(target: &Target) -> ClaudeLayout {
        let kind = match target.kind {
            TargetKind::Local => StoreKind::Local,
            TargetKind::Wsl => StoreKind::Wsl,
        };
        ClaudeLayout::new(target.codex_home.clone().unwrap_or_default(), kind)
    }

    /// 缓存键：目标 + 视图。
    fn cache_key(ctx: &ProviderContext, target: &Target, view: SessionView) -> String {
        format!("{}-{}", ctx.cache_key(target), view.as_str())
    }

    /// 枚举某视图下的会话文件。
    async fn list_files(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        view: SessionView,
    ) -> Result<Vec<aivc_core::session::SessionFile>> {
        let layout = Self::layout(target);
        let root = match view {
            SessionView::Active => layout.projects_root(),
            SessionView::Trash => layout.trash_projects_root(),
        };
        match target.distro() {
            Some(distro) => {
                // 固定两层：`projects/<项目目录>/<文件>.jsonl`。
                ctx.wsl()
                    .list_session_files_at_depth(distro, &root, SESSION_FILE_GLOB, 2, 2)
                    .await
            }
            None => Ok(walk_two_level_files(Path::new(&root)).await),
        }
    }

    /// 按编号查找会话文件。
    async fn find_file(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        id: &SessionId,
        view: SessionView,
    ) -> Option<String> {
        let files = self.list_files(ctx, target, view).await.ok()?;
        files
            .into_iter()
            .find(|file| super::parser::session_id_from_file_path(&file.file_path) == id.as_str())
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
                    // 找到第一个编号即可判定，不必读完整个文件。
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

    /// 解析「要动哪个文件」：优先用提示路径，校验不过就按编号回退查找。
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
            // 提示路径来自渲染层，必须重新过围栏。
            let root = match view {
                SessionView::Active => layout.projects_root(),
                SessionView::Trash => layout.trash_projects_root(),
            };
            crate::common::assert_session_file_inside(hint, &root, layout.kind(), MESSAGE_OPERATE)?;
            if ctx.store_for(target).exists(hint).await.unwrap_or(false) {
                return Ok(Some(hint.to_owned()));
            }
        }
        Ok(self.find_file(ctx, target, id, view).await)
    }

    /// 清掉某视图缓存里的一批条目。
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
    ) -> Result<Option<Session>> {
        let store = ctx.store_for(target);
        let content = store.read_text(file_path).await?;
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
        let files = self.list_files(ctx, target, view).await?;
        let key = Self::cache_key(ctx, target, view);
        if files.is_empty() {
            ctx.session_cache().replace(&key, &[]).await?;
            return Ok(Vec::new());
        }
        let store = ctx.store_for(target);
        let store = Arc::new(store);
        let cached = ctx.session_cache().read(&key).await?;

        let results: Vec<Option<(aivc_infra::db::SessionCacheEntry, bool)>> =
            stream::iter(files.iter().cloned())
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
                        // 列表只读头部：会话正文可能几十 MB，列表页只需要标题与预览。
                        let Ok(head) = store.read_head(&file.file_path, LIST_HEAD_BYTES).await
                        else {
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

        let mut folder = PageFolder::new(
            cursor,
            u64::from(limit),
            anchor.as_ref(),
            LinePolicy::Claude,
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

    /// 本机 CLI 目标探测参数。
    fn local_probe(config_dir: String) -> LocalCliProbe {
        LocalCliProbe {
            provider: ProviderId::Claude,
            display_name: DISPLAY_NAME,
            windows_command: "claude.cmd",
            unix_command: "claude",
            config_dir,
        }
    }

    /// 默认配置目录：`~/.claude`。
    fn default_config_dir() -> Option<String> {
        for variable in ["USERPROFILE", "HOME"] {
            if let Ok(value) = std::env::var(variable) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Some(
                        Path::new(trimmed)
                            .join(".claude")
                            .to_string_lossy()
                            .into_owned(),
                    );
                }
            }
        }
        None
    }
}

/// 递归枚举**两层**目录下的会话文件。
///
/// `projects/<项目目录>/<文件>.jsonl` 是固定结构，因此不递归到第三层——项目目录里
/// 可能放着备份或附件，递归下去只会误收。
pub async fn walk_two_level_files(root: &Path) -> Vec<aivc_core::session::SessionFile> {
    let mut files = Vec::new();
    let Ok(mut projects) = tokio::fs::read_dir(root).await else {
        return files;
    };
    while let Ok(Some(project)) = projects.next_entry().await {
        let Ok(metadata) = project.metadata().await else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let Ok(mut entries) = tokio::fs::read_dir(project.path()).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !ClaudeLayout::is_session_file_name(&name) {
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

impl SessionProvider for ClaudeProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    fn label(&self) -> &'static str {
        DISPLAY_NAME
    }

    fn list_targets<'a>(&'a self, ctx: &'a ProviderContext) -> BoxFuture<'a, Result<Vec<Target>>> {
        Box::pin(async move {
            let mut targets = Vec::new();
            if let Some(config_dir) = Self::default_config_dir()
                && let Some(target) = probe_local_cli_target(&Self::local_probe(config_dir)).await
            {
                targets.push(target);
            }
            targets.extend(
                probe_wsl_cli_targets(
                    ctx,
                    &WslCliProbe {
                        provider: ProviderId::Claude,
                        display_name: DISPLAY_NAME,
                        command: "claude",
                        config_dir_suffix: ".claude",
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
        // 目标探测要访问文件系统，没有纯内存来源；返回空让上层回退到完整探测。
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
            crate::common::list_cached_sessions(ctx, &key).await
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
                        // 第一阶段：摘要匹配，命中就不读文件。
                        if matches_session_terms(&session, &terms) {
                            return Some(session);
                        }
                        // 第二阶段：读整个文件，用完整正文再匹配一次。
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
                    layout.assert_readable(hint)?;
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
                let cached = crate::common::list_cached_sessions(ctx, &key).await?;
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
            layout.assert_operable(&source)?;
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
                layout.assert_operable(&source)?;
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
            // 恢复必须拒绝覆盖：原位置已有同名文件时静默覆盖会毁掉那份会话。
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
            // 回收站围栏：源必须在 `.visual-console-trash/projects` 之内。
            crate::common::assert_session_file_inside(
                &source,
                &layout.trash_projects_root(),
                layout.kind(),
                super::paths::MESSAGE_RESTORE,
            )?;
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
                crate::common::assert_session_file_inside(
                    &source,
                    &layout.trash_projects_root(),
                    layout.kind(),
                    super::paths::MESSAGE_RESTORE,
                )?;
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

impl ClaudeProvider {
    /// 写入分支/复制产出的文件并解析回来。
    async fn write_derived(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        path: &str,
        text: &str,
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
            text,
            None,
        )
        .ok_or(not_found)?;
        self.evict_cache(ctx, target, SessionView::Active, &[path.to_owned()])
            .await;
        Ok(parsed)
    }
}

impl Branchable for ClaudeProvider {
    fn branch_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        message_line: u64,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            // 注意：Claude 的这个参数是**可见消息条数**，不是 JSONL 行号。
            let session = self.get_session_summary(ctx, target, id).await?;
            let layout = Self::layout(target);
            layout.assert_operable(&session.file_path)?;
            let source_text = ctx.store_for(target).read_text(&session.file_path).await?;
            let branch_id = uuid::Uuid::new_v4().to_string();
            let text = build_branch_text(&source_text, &branch_id, message_line)?;
            let path = layout.derived_path_for(&session.file_path, &branch_id)?;
            let branch = self
                .write_derived(ctx, target, &path, &text, SessionError::ClaudeBranchFailed)
                .await?;

            // 分支来源记在元数据里；Claude 用消息序号（不是行号）。
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

impl Duplicable for ClaudeProvider {
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
            let text = build_duplicate_text(&source_text, &duplicate_id)?;
            let path = layout.derived_path_for(&session.file_path, &duplicate_id)?;
            let duplicated = self
                .write_derived(
                    ctx,
                    target,
                    &path,
                    &text,
                    SessionError::ClaudeDuplicateFailed,
                )
                .await?;

            // 自定义标题由元数据承载，不改写 Claude 的文件内容。
            if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
                ctx.session_metadata()
                    .set_custom_title(&target.id, &duplicated.id, &compact_title(title))
                    .await?;
            }
            self.attach(ctx, target, duplicated).await
        })
    }
}

impl Exportable for ClaudeProvider {
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
            // 导出始终写宿主侧：目标文件由保存对话框决定。
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

/// 会话文件后缀（供上层拼装文件名）。
#[must_use]
pub const fn session_suffix() -> &'static str {
    SESSION_FILE_SUFFIX
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestContext;

    const SESSION_A: &str = "11111111-2222-3333-4444-555555555555";
    const SESSION_B: &str = "66666666-7777-8888-9999-000000000000";

    fn claude_target(ctx: &TestContext) -> Target {
        Target {
            id: aivc_core::id::TargetId::from("claude:local"),
            provider: ProviderId::Claude,
            label: "Claude Code：本机（win32）".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some(ctx.codex_home.to_string_lossy().into_owned()),
            available: true,
            detail: None,
        }
    }

    fn project_relative(session_id: &str) -> String {
        // Claude 的文件名就是会话编号，没有 Codex 那样的 `rollout-<时间戳>-` 前缀。
        format!("projects/-home-a-work/{session_id}.jsonl")
    }

    fn content(session_id: &str, title: &str) -> String {
        [
            format!(
                r#"{{"type":"user","sessionId":"{session_id}","timestamp":"2026-09-16T12:00:00.000Z","cwd":"/work","message":{{"role":"user","content":"{title}"}}}}"#
            ),
            format!(
                r#"{{"type":"assistant","sessionId":"{session_id}","timestamp":"2026-09-16T12:00:01.000Z","message":{{"role":"assistant","model":"claude-sonnet-4","content":"回答"}}}}"#
            ),
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
    fn provider_reports_claude_identity() {
        let provider = ClaudeProvider::new();
        assert_eq!(provider.id(), ProviderId::Claude);
        assert_eq!(provider.label(), "Claude Code");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_reads_two_level_project_directories() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "标题 A"));

        let sessions = ClaudeProvider::new()
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
    async fn listing_ignores_files_below_the_second_level() {
        // 第三层的文件不属于 Claude 的布局，不应被当成会话。
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(
            "projects/-home-a-work/nested/deeper/x.jsonl",
            &content(SESSION_A, "太深了"),
        );

        let sessions = ClaudeProvider::new()
            .list_sessions(&ctx.context, &target)
            .await
            .expect("list");
        assert!(sessions.is_empty(), "第三层文件不应被收录");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_moves_the_file_into_the_trash_tree() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let source = ctx.write_session(
            &project_relative(SESSION_A),
            &content(SESSION_A, "将被删除"),
        );

        let result = ClaudeProvider::new()
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");
        assert!(result.stale.is_none());
        assert!(!source.exists());
        // 回收站保持 projects 前缀。
        assert!(
            ctx.codex_home
                .join(".visual-console-trash")
                .join(project_relative(SESSION_A).replace('/', std::path::MAIN_SEPARATOR_STR))
                .exists()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_then_restore_round_trips() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let source = ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "往返"));

        let provider = ClaudeProvider::new();
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
    async fn restore_refuses_to_overwrite() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "原文件"));
        let provider = ClaudeProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("delete");
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "新文件"));

        let error = provider
            .restore_session(&ctx.context, &target, &SessionId::from(SESSION_A))
            .await
            .expect_err("必须拒绝覆盖");
        assert_eq!(error.to_string(), "原位置已存在同名会话文件，无法恢复。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn purge_removes_the_file_from_the_trash() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(
            &project_relative(SESSION_A),
            &content(SESSION_A, "彻底删除"),
        );
        let provider = ClaudeProvider::new();
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
    async fn delete_rejects_a_forged_path_outside_the_projects_root() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let outside = ctx.write_session("settings.json", "{}");

        let error = ClaudeProvider::new()
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
        assert!(outside.exists(), "被拒绝的文件不得被改动");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_rejects_a_path_whose_content_has_a_different_id() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let other = ctx.write_session(
            &project_relative(SESSION_B),
            &content(SESSION_B, "B 的会话"),
        );

        let error = ClaudeProvider::new()
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
    async fn deleting_a_stale_entry_reports_stale() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let result = ClaudeProvider::new()
            .delete_session(&ctx.context, &target, &mutation(SESSION_A))
            .await
            .expect("不得报错");
        assert_eq!(result.stale, Some(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paging_returns_the_tail_without_source_lines() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let mut lines = Vec::new();
        for index in 0..10 {
            lines.push(format!(
                r#"{{"type":"user","sessionId":"{SESSION_A}","timestamp":"2026-09-16T12:00:0{}.000Z","message":{{"role":"user","content":"消息{index}"}}}}"#,
                index % 10
            ));
        }
        ctx.write_session(&project_relative(SESSION_A), &lines.join("\n"));

        let page = ClaudeProvider::new()
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
        // Claude 的分支按消息条数定位，因此不填 source_line。
        assert!(page.messages.iter().all(|m| m.source_line.is_none()));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn branching_truncates_at_the_requested_message_count() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "第一条"));

        let branch = ClaudeProvider::new()
            .branch_session(&ctx.context, &target, &SessionId::from(SESSION_A), 1)
            .await
            .expect("branch");
        assert_ne!(branch.id.as_str(), SESSION_A, "分支必须有新编号");
        assert!(Path::new(&branch.file_path).exists());
        // 元数据记录父会话与消息序号。
        let metadata = branch.metadata.expect("metadata");
        let branch_meta = metadata.branch.expect("branch metadata");
        assert_eq!(
            branch_meta
                .parent_session_id
                .map(|id| id.as_str().to_owned()),
            Some(SESSION_A.to_owned())
        );
        assert_eq!(branch_meta.parent_message_index, Some(1));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn branching_with_too_many_messages_fails() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(
            &project_relative(SESSION_A),
            &content(SESSION_A, "只有两条"),
        );

        let error = ClaudeProvider::new()
            .branch_session(&ctx.context, &target, &SessionId::from(SESSION_A), 99)
            .await
            .expect_err("必须失败");
        assert_eq!(
            error.to_string(),
            "创建 Claude 分支失败：原始 jsonl 中可保留消息不足。"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicating_creates_an_independent_copy() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "原始"));

        let duplicate = ClaudeProvider::new()
            .duplicate_session(&ctx.context, &target, &SessionId::from(SESSION_A), None)
            .await
            .expect("duplicate");
        assert_ne!(duplicate.id.as_str(), SESSION_A);
        assert!(Path::new(&duplicate.file_path).exists());
        // 内容保留，编号已换。
        let text = std::fs::read_to_string(&duplicate.file_path).expect("read");
        assert!(text.contains("原始"));
        assert!(!text.contains(SESSION_A), "旧编号必须已被替换");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicating_with_a_custom_title_records_metadata() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "原始"));

        let duplicate = ClaudeProvider::new()
            .duplicate_session(
                &ctx.context,
                &target,
                &SessionId::from(SESSION_A),
                Some("自定义标题"),
            )
            .await
            .expect("duplicate");
        assert_eq!(duplicate.title, "自定义标题");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn exporting_writes_markdown_to_the_host() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(
            &project_relative(SESSION_A),
            &content(SESSION_A, "导出标题"),
        );
        let destination = ctx.root().join("export").join("session.md");

        let result = ClaudeProvider::new()
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
        let text = std::fs::read_to_string(&destination).expect("read");
        assert!(text.contains("导出标题"));
        assert!(text.contains("## 对话"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_matches_the_summary() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(
            &project_relative(SESSION_A),
            &content(SESSION_A, "重构会话列表"),
        );
        ctx.write_session(
            &project_relative(SESSION_B),
            &content(SESSION_B, "无关内容"),
        );

        let found = ClaudeProvider::new()
            .search_sessions(&ctx.context, &target, SessionView::Active, "重构")
            .await
            .expect("search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.as_str(), SESSION_A);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reading_a_missing_session_reports_not_found() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        let error = ClaudeProvider::new()
            .get_session(&ctx.context, &target, &SessionId::from(SESSION_A), None)
            .await
            .expect_err("必须失败");
        assert_eq!(error.to_string(), format!("未找到会话：{SESSION_A}"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trash_sessions_are_listed_separately() {
        let ctx = TestContext::new().await;
        let target = claude_target(&ctx);
        ctx.write_session(&project_relative(SESSION_A), &content(SESSION_A, "活跃"));
        ctx.write_session(
            &format!(".visual-console-trash/{}", project_relative(SESSION_B)),
            &content(SESSION_B, "回收站"),
        );

        let provider = ClaudeProvider::new();
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
