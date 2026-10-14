//! Codex `SessionProvider` 实现：目标探测、列表/详情编排与变更操作。
//!
//! # 编排而不是实现
//!
//! 解析、分页、路径与列表构建都已经有独立模块（且各自有测试）。这里只负责**把它们
//! 接到一起**：解析目标、选存储、读缓存、写索引、落盘。因此本文件里几乎没有算法，
//! 只有顺序与错误处理。
//!
//! # 变更操作的三道关
//!
//! 每次删除/恢复/清除都按固定顺序做三件事：
//!
//! 1. **目录围栏**：路径必须严格位于对应根目录之内（防伪造 `filePath`）；
//! 2. **编号校验**：读文件头，确认 `session_meta.id` 与请求的编号一致（防用 A 的
//!    编号删掉 B 的文件——列表项可能已经陈旧）；
//! 3. **移动/删除**：带上冲突保护（恢复时不覆盖原位置已有文件）。
//!
//! 顺序不能换：先校验再动文件，否则失败时已经改坏了数据。
//!
//! # 陈旧列表项
//!
//! 渲染层可能拿着已删除会话的路径来请求。此时**不报错**，而是回退到按编号重新查找；
//! 仍找不到就返回「陈旧」结果并清掉缓存条目。列表与文件系统之间必然存在时间差，
//! 把这种情况当错误会让用户看到莫名其妙的失败。

use std::path::{Path, PathBuf};

use futures::stream::{self, StreamExt};
use serde_json::Value;

use aivc_core::error::{Error, Result, SessionError};
use aivc_core::id::SessionId;
use aivc_core::ports::{
    BoxFuture, ConflictPolicy, ReadOptions, ReadStart, SessionStore, StoreKind,
};
use aivc_core::session::{
    BranchMetadata, BranchOrigin, ExportFormat, ExportResult, Message, MessagePage, PageCursor,
    Session, SessionBatchResult, SessionDeleteResult, SessionMutationEntry, SessionMutationRef,
    SessionView,
};
use aivc_core::target::{Platform, ProviderId, Target, TargetKind};
use aivc_core::text::{matches_session_terms, normalize_search_terms};
use aivc_infra::db::{MessageIndexEntry, SessionIndexKey};

use super::appserver::{APP_SERVER_TIMEOUT, AppServerLauncher, with_app_server};
use super::branch::{
    ForkResult, build_duplicate_session_text, canonicalize_forked_session, find_branch_turn_id,
};
use super::listing::{LIST_HEAD_BYTES, list_sessions_from_files, walk_local_session_files};
use super::parser::{
    ParseMode, SessionFileInfo, SessionParser, parse_content, session_id_from_file_path,
};
use super::paths::{CodexLayout, SESSION_FILE_GLOB, build_session_file_name};
use crate::common::list_cached_sessions;
use crate::context::ProviderContext;
use crate::export::{format_session, sanitize_for_export};
use crate::paging::{LinePolicy, PageFolder};
use crate::registry::{Branchable, Duplicable, Exportable, SessionProvider};

/// 搜索时逐会话扫描文件内容的并发上限。
///
/// 搜索的第二阶段会读整个文件，比列表重得多，因此并发比列表更低（现网取 8）。
pub const SEARCH_SCAN_CONCURRENCY: usize = 8;

/// Codex 平台实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct CodexProvider;

/// 一个 WSL 发行版的探测结果。
///
/// 存在的唯一目的，是把「**进不去**」和「进去了但没找到 Codex 目录」分开。这两件事在
/// `resolve_codex_home` 的返回值里都表现为空串，混在一起会让界面把一个进不去的发行版
/// 描述成可以进入——比不显示它更糟。详见 [`CodexProvider::wsl_target`]。
#[derive(Debug, Clone, PartialEq, Eq)]
enum WslProbe {
    /// 发行版可达。
    Reachable {
        /// 解析出的 Codex 配置目录；空串表示没找到。
        home: String,
        /// `<home>/sessions` 是否存在。
        has_sessions: bool,
    },
    /// 无法进入发行版（`wsl.exe` 失败、VHDX 挂载被拒、发行版未初始化等）。
    Unreachable {
        /// 人能读的原因（已截断）。
        reason: String,
    },
}

impl WslProbe {
    /// 构造「可达」结果。
    fn reachable(home: impl Into<String>, has_sessions: bool) -> Self {
        Self::Reachable {
            home: home.into(),
            has_sessions,
        }
    }

    /// 构造「不可达」结果。
    fn unreachable(reason: impl Into<String>) -> Self {
        Self::Unreachable {
            reason: reason.into(),
        }
    }
}

impl CodexProvider {
    /// 构造。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// 目标对应的存储布局。
    fn layout(target: &Target) -> CodexLayout {
        let kind = match target.kind {
            TargetKind::Local => StoreKind::Local,
            TargetKind::Wsl => StoreKind::Wsl,
        };
        CodexLayout::new(target.codex_home.clone().unwrap_or_default(), kind)
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
        // 配置目录尚未解析出来时**不发起任何查询**。
        //
        // 空字符串会让路径拼接产出**相对路径**（`sessions`），于是 `find` 会在发行版的
        // 当前工作目录下乱找——既慢，又可能命中毫不相关的文件。原平台的
        // `wslListSessionFiles` 开头就有这道 guard（`if (!codexHome) return []`）。
        //
        // 这道 guard 是随「列出全部发行版」一起被激活的：在此之前只有解析出配置目录的
        // 发行版才会成为目标，空 home 根本不可达。
        if layout.codex_home().is_empty() {
            return Ok(Vec::new());
        }
        let root = match view {
            SessionView::Active => layout.sessions_root(),
            SessionView::Trash => layout.trash_sessions_root(),
        };
        match target.distro() {
            Some(distro) => {
                ctx.wsl()
                    .list_session_files(distro, &root, SESSION_FILE_GLOB)
                    .await
            }
            None => Ok(walk_local_session_files(Path::new(&root)).await),
        }
    }

    /// 按编号在指定视图中查找会话文件。
    async fn find_file(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        id: &SessionId,
        view: SessionView,
    ) -> Option<String> {
        let files = self.list_files(ctx, target, view).await.ok()?;
        // 文件名里就带编号，因此绝大多数情况不需要读文件。
        files
            .into_iter()
            .find(|file| session_id_from_file_path(&file.file_path) == id.as_str())
            .map(|file| file.file_path)
    }

    /// 校验文件头的 `session_meta.id` 与请求的编号一致。
    ///
    /// 渲染层传来的路径可能已经陈旧（文件被替换或列表项过期）。只靠路径做删除会让
    /// 「用 A 的编号删掉 B 的文件」成为可能，因此动文件之前必须核对内容。
    async fn verify_session_id(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
        id: &SessionId,
    ) -> Result<()> {
        let store = ctx.store_for(target);
        let head = store.read_head(file_path, LIST_HEAD_BYTES).await?;
        let info = SessionFileInfo {
            file_path: file_path.to_owned(),
            mtime_ms: 0.0,
            change_ms: None,
            size: 0,
        };
        let parsed = parse_content(info, &head, ParseMode::List);
        match parsed {
            Some(session) if session.id.as_str() == id.as_str() => Ok(()),
            _ => Err(SessionError::SessionIdMismatch {
                id: id.as_str().to_owned(),
            }
            .into()),
        }
    }

    /// 解析「要动哪个文件」：优先用提示路径，校验不过就按编号回退查找。
    ///
    /// 返回 [`None`] 表示会话确实不存在（陈旧列表项），调用方据此返回 stale 结果。
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
            // 提示路径同样要过围栏：它来自渲染层。
            let fenced = match view {
                SessionView::Active => layout.assert_deletable(hint),
                SessionView::Trash => layout.assert_purgeable(hint),
            };
            fenced?;
            if ctx.store_for(target).exists(hint).await.unwrap_or(false) {
                return Ok(Some(hint.to_owned()));
            }
        }
        Ok(self.find_file(ctx, target, id, view).await)
    }

    /// 会话文件的指纹。
    async fn file_fingerprint(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
    ) -> (f64, f64, u64) {
        let store = ctx.store_for(target);
        let files = self
            .list_files(ctx, target, SessionView::Active)
            .await
            .unwrap_or_default();
        if let Some(file) = files.iter().find(|file| file.file_path == file_path) {
            return (
                file.mtime_ms,
                file.change_ms.unwrap_or(file.mtime_ms),
                file.size,
            );
        }
        let size = store.size(file_path).await.unwrap_or(0);
        (0.0, 0.0, size)
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
        // 缓存清不掉不应让删除失败：文件已经动过了，报错只会让用户以为没删掉。
        if let Err(error) = ctx.session_cache().remove_entries(&key, paths).await {
            tracing::warn!(%error, "清理会话缓存失败");
        }
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
            // 非首屏游标一定非负（负数在构造时已归为首屏）。
            ctx.message_index()
                .find_anchor_at_or_before(&key, cursor.to_wire().max(0).cast_unsigned())
                .await?
        };

        let mut folder =
            PageFolder::new(cursor, u64::from(limit), anchor.as_ref(), LinePolicy::Codex);
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
                        aivc_core::ports::LineControl::Continue
                    } else {
                        aivc_core::ports::LineControl::Stop
                    }
                }),
            )
            .await?;

        let mut outcome = folder.finish(session.message_count);
        outcome.page.file_revision =
            Some(format!("{}:{}:{}", key.mtime_ms, key.change_ms, key.size));
        // 索引写入失败只记日志：分页结果已经拿到了，不该因为索引写不进去而报错。
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

    /// 完整加载一个会话文件。
    async fn load_full(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
    ) -> Result<Option<Session>> {
        let store = ctx.store_for(target);
        let (mtime_ms, change_ms, size) = self.file_fingerprint(ctx, target, file_path).await;
        let info = SessionFileInfo {
            file_path: file_path.to_owned(),
            mtime_ms,
            change_ms: Some(change_ms),
            size,
        };
        let mut parser = SessionParser::new(info, ParseMode::Full);
        store
            .read_lines(
                file_path,
                ReadStart::BEGINNING,
                ReadOptions::none(),
                Box::new(|line, line_number, _offset| {
                    parser.push(line, line_number);
                    aivc_core::ports::LineControl::Continue
                }),
            )
            .await?;
        Ok(parser.finish())
    }

    /// Load the metadata and preview for a native fork without retaining its
    /// full transcript in memory. The complete transcript is loaded on demand
    /// by the paginated message endpoint.
    async fn load_fork_summary(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        file_path: &str,
    ) -> Result<Option<Session>> {
        let store = ctx.store_for(target);
        let (mtime_ms, change_ms, size) = self.file_fingerprint(ctx, target, file_path).await;
        let info = SessionFileInfo {
            file_path: file_path.to_owned(),
            mtime_ms,
            change_ms: Some(change_ms),
            size,
        };
        let head = store.read_head(file_path, LIST_HEAD_BYTES).await?;
        Ok(
            parse_content(info, &head, ParseMode::List).map(|mut session| {
                session.message_count_is_lower_bound = size > head.len() as u64;
                session
            }),
        )
    }

    /// 附着元数据（标题、收藏、分支信息）。
    async fn attach(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        session: Session,
    ) -> Result<Session> {
        ctx.session_metadata().attach(&target.id, session).await
    }

    /// 批量附着元数据。
    async fn attach_list(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        sessions: Vec<Session>,
    ) -> Result<Vec<Session>> {
        ctx.session_metadata()
            .attach_list(&target.id, sessions)
            .await
    }
}

/// 本机 Codex 配置目录：`CODEX_HOME` 优先，其次 `~/.codex`。
#[must_use]
pub fn local_codex_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    home_directory().map(|home| home.join(".codex"))
}

/// 用户主目录。
fn home_directory() -> Option<PathBuf> {
    for variable in ["USERPROFILE", "HOME"] {
        if let Ok(value) = std::env::var(variable) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(PathBuf::from(trimmed));
            }
        }
    }
    None
}

impl SessionProvider for CodexProvider {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    fn label(&self) -> &'static str {
        ProviderId::Codex.display_name()
    }

    fn list_targets<'a>(&'a self, ctx: &'a ProviderContext) -> BoxFuture<'a, Result<Vec<Target>>> {
        Box::pin(async move {
            let mut targets = Vec::new();
            if let Some(target) = Self::local_target() {
                targets.push(target);
            }
            targets.extend(self.wsl_targets(ctx).await);
            Ok(targets)
        })
    }

    fn list_cached_targets<'a>(
        &'a self,
        ctx: &'a ProviderContext,
    ) -> BoxFuture<'a, Result<Vec<Target>>> {
        // **读设置里持久化的目标缓存**。
        //
        // 这里曾经无条件返回空列表，理由是「没有纯内存的来源，让上层回退到完整探测」。
        // 那个决定是错的，后果有两层：
        //
        // 1. `codex:list-cached-targets` **永远是空的**，前端「先渲染缓存、再增量刷新」
        //    的策略完全失效——每次选平台都要现场探测（实测 2.1 秒，WSL 冷启动时更久）；
        // 2. 启动预热白做：它探测并打印了结果，却没留下任何可复用的东西。
        //
        // 缓存由 `codex:list-targets` 在探测后写入，因此它反映的是**上次探测**的结果。
        // 这正是「先显示旧的、后台刷新」需要的语义——不是「伪造」，是**有意的过期数据**。
        Box::pin(async move {
            Ok(ctx
                .settings()
                .snapshot()
                .await
                .cached_targets
                .into_iter()
                .filter(|target| target.provider == ProviderId::Codex)
                .collect())
        })
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
            // 空查询等价于「列出全部」。
            if terms.is_empty() {
                return Ok(list);
            }
            let store = ctx.store_for(target);
            let matched: Vec<Option<Session>> = stream::iter(list)
                .map(|session| {
                    let store = &store;
                    let terms = terms.clone();
                    async move {
                        // 第一阶段：摘要匹配（标题、cwd、编号、预览）。命中就不读文件。
                        if matches_session_terms(&session, &terms) {
                            return Some(session);
                        }
                        // 第二阶段：读整个文件，用完整正文再匹配一次。
                        let content = store.read_text(&session.file_path).await.ok()?;
                        let info = SessionFileInfo {
                            file_path: session.file_path.clone(),
                            mtime_ms: session.file_mtime_ms.unwrap_or(0.0),
                            change_ms: session.file_change_ms,
                            size: session.file_size.unwrap_or(0),
                        };
                        let full = parse_content(info, &content, ParseMode::Full)?;
                        // 轻量解析得到的标题与元数据更权威（元数据来自数据库），
                        // 因此用完整正文替换预览，但保留原来的标题。
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
            let mut session = self.get_session_summary(ctx, target, id).await?;
            // Refresh the fingerprint before reusing message anchors: an open
            // terminal can append to the file while the list cache stays valid.
            let (mtime, change, size) =
                self.file_fingerprint(ctx, target, &session.file_path).await;
            session.file_mtime_ms = Some(mtime);
            session.file_change_ms = Some(change);
            session.file_size = Some(size);
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
            // 摘要优先从缓存取：列表页刚加载过，缓存里通常就有。
            for view in [SessionView::Active, SessionView::Trash] {
                let key = Self::cache_key(ctx, target, view);
                let cached = list_cached_sessions(ctx, &key).await?;
                if let Some(session) = cached.into_iter().find(|item| item.id == *id) {
                    return Ok(session);
                }
            }
            // 缓存未命中：按编号找文件并做一次轻量解析。
            for view in [SessionView::Active, SessionView::Trash] {
                if let Some(path) = self.find_file(ctx, target, id, view).await {
                    let store = ctx.store_for(target);
                    let head = store.read_head(&path, LIST_HEAD_BYTES).await?;
                    let (mtime_ms, change_ms, size) =
                        self.file_fingerprint(ctx, target, &path).await;
                    let info = SessionFileInfo {
                        file_path: path,
                        mtime_ms,
                        change_ms: Some(change_ms),
                        size,
                    };
                    if let Some(mut session) = parse_content(info, &head, ParseMode::List) {
                        session.message_count_is_lower_bound = size > head.len() as u64;
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
            // 分支关系记在元数据里，因此先反查编号，再逐个取摘要。
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
                // 陈旧列表项：清掉缓存条目并如实告知，而不是报错。
                if let Some(hint) = reference.file_path.as_deref() {
                    self.evict_cache(ctx, target, SessionView::Active, &[hint.to_owned()])
                        .await;
                }
                return Ok(SessionDeleteResult {
                    moved_to: None,
                    stale: Some(true),
                });
            };

            layout.assert_deletable(&source)?;
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
                // 批量操作逐条围栏与校验：一条不合法不应该连累其余条目，
                // 但也不能跳过校验直接移动。
                layout.assert_deletable(&source)?;
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
                .ok_or_else(|| SessionError::NotFound {
                    id: id.as_str().to_owned(),
                })?;
            layout.assert_in_trash(&source)?;
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
                // 已经不存在：清理缓存条目即可，重复删除不算错误。
                if let Some(hint) = reference.file_path.as_deref() {
                    self.evict_cache(ctx, target, SessionView::Trash, &[hint.to_owned()])
                        .await;
                }
                return Err(SessionError::NotFound {
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

impl CodexProvider {
    /// 探测本机目标。
    fn local_target() -> Option<Target> {
        // **每一处提前返回都要留下痕迹**。
        //
        // 实测踩过：实机上本机目标没出现在下拉框里，而
        // `C:\Users\<用户>\.codex\sessions` **确实存在**（有 1 个条目）。
        // 这个函数当时完全静默，于是「为什么没有本机目标」无从查起——
        // 与本工程已修的多处「静默失败」是同一形态。
        let Some(home) = local_codex_home() else {
            tracing::debug!("解析不出本机 Codex 主目录，跳过本机目标");
            return None;
        };
        // **判据是「配置目录存在」，不是「已经有会话」**。
        //
        // 这里曾经要求 `<home>/sessions` 必须是个目录，否则整个本机目标消失。后果是
        // **全新安装**——CLI 装好了、还没跑过任何一次会话——的用户在界面上根本看不到
        // 本机目标。原平台的判据是 `codexHome` 存在即可（`available: hasCodexHome`），
        // `sessions` 只用来决定文案措辞。
        //
        // 「列表为空」与「根本没有这个目标」对用户是两件完全不同的事，不能混为一谈。
        if !home.is_dir() {
            tracing::debug!(
                home = %home.display(),
                "本机 Codex 配置目录不存在，跳过本机目标"
            );
            return None;
        }
        let home_text = home.to_string_lossy().into_owned();
        let has_sessions = home.join("sessions").is_dir();
        Some(Target {
            id: aivc_core::id::TargetId::from(format!("codex:local:{home_text}")),
            provider: ProviderId::Codex,
            label: "本机 Codex".to_owned(),
            kind: TargetKind::Local,
            distro: None,
            codex_home: Some(home_text.clone()),
            available: true,
            detail: Some(if has_sessions {
                format!("找到 Codex 会话（{home_text}）")
            } else {
                format!("找到 Codex 目录（{home_text}）")
            }),
        })
    }

    /// 探测全部 WSL 发行版里的 Codex。
    ///
    /// # 每一个发行版都必须出现在结果里
    ///
    /// 这里曾经有**两道 `continue`**：解析不出 Codex 主目录就跳过、`sessions` 目录不存在
    /// 就跳过。那是错的，而且是我自己「优化」出来的——原平台的
    /// `probeAllTargets` **无条件**把每个发行版都变成一个目标（`available: true`），
    /// 只把探测结果写进 `detail` 文案。
    ///
    /// 后果在实机上直接可见：`wsl -l -q` 给出 Ubuntu-22.04 与 Ubuntu-24.04，
    /// Ubuntu-22.04 没有 `~/.codex/sessions`，于是它**从列表里消失了**；
    /// 用户看到的目标比原平台少一个，而界面没有任何提示说明为什么。
    ///
    /// 更关键的是，被跳过的发行版**连带失去了一条恢复路径**：WSL 内的 Codex 装在
    /// 非标准位置时，用户要靠「手动指定 Codex 目录」把它接进来，而那个入口只对
    /// **已经出现在列表里**的目标开放。跳过 = 永远无法配置。
    ///
    /// 所以现在的规则是：**列出全部候选，逐个标注它的状态**，而不是「只列出好用的」。
    /// `detail` 承担说明职责：
    ///
    /// - `找到 Codex 会话`——可以直接用；
    /// - `找到 Codex 目录`——目录在，还没有会话；
    /// - `可进入 WSL，未找到 Codex 目录`——发行版可达，Codex 尚未初始化。
    ///
    /// `available` 一律为 `true`：它表达的是「这个目标可以进入、可以在此安装或指定
    /// Codex」，而不是「这里已经有会话」。这与原平台一致，也与渲染层的用法一致
    /// （`available` 只用来决定「继续会话」是否可点，而没有会话时那条目本就为空）。
    async fn wsl_targets(&self, ctx: &ProviderContext) -> Vec<Target> {
        // **每一道门都要留下痕迹**：静默返回空会让「没有 WSL 目标」变成黑盒——
        // 实测中正是如此：Ubuntu-24.04 的 `~/.codex/sessions` 确实存在，
        // 而界面上一个 WSL 目标都没有，却没有任何线索说明是哪一步失败的。
        if !ctx.wsl().is_available() {
            tracing::warn!("WSL 不可用，跳过 WSL 目标探测");
            return Vec::new();
        }
        let distros = ctx.wsl().list_distros().await;
        tracing::debug!(?distros, count = distros.len(), "WSL 发行版列表");
        let mut targets = Vec::with_capacity(distros.len());
        for distro in distros {
            let probe = match ctx.wsl().probe_codex_home(&distro).await {
                Ok(home) => {
                    // 探测脚本的第二个分支要求候选**目录存在**，因此非空即代表目录真实
                    // 存在；这里只需再确认一次 `sessions` 子目录。每个发行版因此只多花
                    // 一次 `wsl.exe` 调用（原平台为同样的信息要跑五六次）。
                    let has_sessions = if home.is_empty() {
                        false
                    } else {
                        let layout = CodexLayout::new(home.clone(), StoreKind::Wsl);
                        ctx.wsl().exists(&distro, &layout.sessions_root()).await
                    };
                    tracing::debug!(%distro, %home, has_sessions, "WSL 发行版探测结果");
                    WslProbe::reachable(home, has_sessions)
                }
                Err(reason) => {
                    tracing::warn!(%distro, %reason, "发行版不可进入");
                    WslProbe::unreachable(reason)
                }
            };
            // **无条件 push**：循环体里没有任何 `continue`，因为「是否展示这个发行版」
            // 已经不是一个判断了。
            targets.push(Self::wsl_target(&distro, probe));
        }
        targets
    }

    /// 由探测结果构造一个 WSL 目标。
    ///
    /// # 为什么把它单独拿出来
    ///
    /// 这个函数的签名里**没有「不构造」这条返回路径**——没有 `Option`，没有 `Result`。
    /// 这是刻意的：它把「某个发行版该不该出现在列表里」这个曾经出错的判断**从结构上
    /// 删除了**。探测结果只影响 `codex_home`、`available` 与 `detail` 三样东西，
    /// 不能影响目标是否存在。
    ///
    /// 之前的两道 `continue` 正是「用一个可能失败的环境探测结果，去决定要不要展示
    /// 一个入口」——这个模式在本工程里反复出现，且每次都让用户少看到东西。
    ///
    /// # 四种状态，四句互不相同的说明
    ///
    /// `detail` 承担全部说明职责，因此文案必须逐字区分：
    ///
    /// - `找到 Codex 会话`——可以直接用；
    /// - `找到 Codex 目录`——目录在，但还没有会话；
    /// - `可进入 WSL，未找到 Codex 目录`——发行版**进得去**，Codex 尚未初始化；
    /// - `无法进入 WSL：<原因>`——发行版**进不去**，此时 `available` 为 `false`。
    ///
    /// 前三种沿用了原平台的措辞。第四种是本工程新增的，原平台没有：它对所有发行版都写
    /// `available: true`，于是**进不去的发行版在界面上与可用的发行版长得一模一样**，
    /// 用户点下去只会收到一个失败。实测就有这种发行版——注册在册、VHDX 却挂载不上
    /// （`Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED`）。
    ///
    /// 把它标成不可用还有一个直接好处：前端「自动选中第一个可用目标」不会选中它。
    #[must_use]
    fn wsl_target(distro: &str, probe: WslProbe) -> Target {
        let (codex_home, available, detail) = match probe {
            WslProbe::Reachable { home, has_sessions } => {
                let detail = if has_sessions {
                    "找到 Codex 会话"
                } else if home.is_empty() {
                    "可进入 WSL，未找到 Codex 目录"
                } else {
                    "找到 Codex 目录"
                };
                // 解析不出来就**不编造**路径：空 `codex_home` 由 `list_files` 的 guard
                // 兜住（直接返回空列表），绝不会退化成相对路径查询。
                let codex_home = (!home.is_empty()).then_some(home);
                (codex_home, true, detail.to_owned())
            }
            WslProbe::Unreachable { reason } => (None, false, format!("无法进入 WSL：{reason}")),
        };
        Target {
            id: aivc_core::id::TargetId::from(format!("codex:wsl:{distro}")),
            provider: ProviderId::Codex,
            label: format!("WSL · {distro}"),
            kind: TargetKind::Wsl,
            distro: Some(distro.to_owned()),
            codex_home,
            available,
            detail: Some(detail),
        }
    }

    /// 列出某个视图下的会话。
    async fn list_view(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        view: SessionView,
    ) -> Result<Vec<Session>> {
        let files = self.list_files(ctx, target, view).await?;
        if files.is_empty() {
            // 目录不存在或为空：仍然写一次空缓存，让已删除的文件条目被淘汰。
            let key = Self::cache_key(ctx, target, view);
            ctx.session_cache().replace(&key, &[]).await?;
            return Ok(Vec::new());
        }
        let key = Self::cache_key(ctx, target, view);
        let outcome = list_sessions_from_files(ctx, target, &files, &key, true).await?;
        self.attach_list(ctx, target, outcome.sessions).await
    }
}

/// 平台与目录的便捷构造（供测试与注册表使用）。
#[must_use]
pub fn codex_target(id: &str, codex_home: &str, distro: Option<&str>) -> Target {
    let kind = if distro.is_some() {
        TargetKind::Wsl
    } else {
        TargetKind::Local
    };
    Target {
        id: aivc_core::id::TargetId::from(id),
        provider: ProviderId::Codex,
        label: format!("Codex {id}"),
        kind,
        distro: distro.map(str::to_owned),
        codex_home: Some(codex_home.to_owned()),
        available: true,
        detail: None,
    }
}

impl CodexProvider {
    /// 通过官方 `app-server` 创建分支线程。
    async fn fork_via_app_server(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        session: &Session,
        turn_id: &str,
    ) -> Result<ForkResult> {
        let launcher = match target.distro() {
            Some(distro) => {
                AppServerLauncher::wsl(ctx.wsl().clone(), distro, target.codex_home.clone())
            }
            None => AppServerLauncher::local(target.codex_home.clone()),
        };
        let parent_id = session.id.as_str().to_owned();
        let turn = turn_id.to_owned();

        with_app_server(&launcher, async |client| {
            let result = client
                .request(
                    "thread/fork",
                    // `lastTurnId` 在创建原生分支的同时原子地完成截断；
                    // 分页线程不接受 `thread/rollback`，因此只能走这个参数。
                    serde_json::json!({ "threadId": parent_id, "lastTurnId": turn, "excludeTurns": true, "deferGoalContinuation": true }),
                    APP_SERVER_TIMEOUT,
                )
                .await?;
            let thread_id = result
                .get("thread")
                .and_then(|thread| thread.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let file_path = result
                .get("thread")
                .and_then(|thread| thread.get("path"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if thread_id.is_empty() || file_path.is_empty() {
                return Err(SessionError::ForkUnavailable.into());
            }
            let fork = ForkResult {
                thread_id: thread_id.to_owned(),
                file_path: file_path.to_owned(),
            };
            // Validate the receipt before reading any returned path.
            canonicalize_forked_session(&Self::layout(target), session, session.clone(), &fork)?;
            // A CLI that silently ignores lastTurnId must not open a fork of
            // the entire thread. Verify with one metadata-only official page.
            let turns = client.request(
                "thread/turns/list",
                serde_json::json!({"threadId": fork.thread_id, "sortDirection":"desc", "limit":1, "itemsView":"notLoaded"}),
                APP_SERVER_TIMEOUT,
            ).await;
            let confirmed = turns.as_ref().ok().and_then(|value| value["data"].as_array())
                .and_then(|data| data.first())
                .is_some_and(|last| last["id"].as_str() == Some(turn.as_str()) && last["status"].as_str() == Some("completed"));
            if !confirmed {
                tracing::warn!(thread_id = %fork.thread_id, ?turns, "无法确认官方分支轮次边界，保留已创建的原生会话");
                return Err(SessionError::ForkBoundaryUnconfirmed { id: fork.thread_id }.into());
            }
            Ok(fork)
        })
        .await
    }

    /// 把分支来源写进元数据，并刷新缓存。
    async fn record_branch(
        &self,
        ctx: &ProviderContext,
        target: &Target,
        parent: &Session,
        parent_message_line: u64,
        branch: Session,
    ) -> Result<Session> {
        let metadata = BranchMetadata {
            parent_target_id: Some(target.id.clone()),
            parent_session_id: Some(parent.id.clone()),
            parent_message_index: None,
            parent_message_line: Some(parent_message_line),
            created_by: Some(BranchOrigin::Branch),
        };
        let file_path = branch.file_path.clone();
        if let Err(error) = ctx
            .session_metadata()
            .set_branch(&target.id, &branch.id, &metadata)
            .await
        {
            // The official fork already exists. A failure in our local metadata
            // must not delete it or create a second replacement fork.
            tracing::warn!(%error, "保存分支关系失败");
        }
        let branch = Session {
            metadata: Some(aivc_core::session::SessionMetadata {
                branch: Some(metadata),
                ..branch.metadata.clone().unwrap_or_default()
            }),
            ..branch
        };
        // 新分支要立刻出现在列表里，因此清掉活跃视图的缓存条目。
        self.evict_cache(ctx, target, SessionView::Active, &[file_path])
            .await;
        Ok(branch)
    }
}

impl Branchable for CodexProvider {
    fn branch_session<'a>(
        &'a self,
        ctx: &'a ProviderContext,
        target: &'a Target,
        id: &'a SessionId,
        message_line: u64,
    ) -> BoxFuture<'a, Result<Session>> {
        Box::pin(async move {
            let session = self.get_session_summary(ctx, target, id).await?;
            Self::layout(target).assert_readable(&session.file_path)?;
            self.verify_session_id(ctx, target, &session.file_path, id)
                .await?;
            let turn_id =
                find_branch_turn_id(&ctx.store_for(target), &session.file_path, message_line)
                    .await?;
            let fork = self
                .fork_via_app_server(ctx, target, &session, &turn_id)
                .await?;
            // Keep the receipt recoverable even if the new rollout is not yet
            // readable. Never delete a valid native fork after a metadata error.
            let parsed = match self.load_fork_summary(ctx, target, &fork.file_path).await {
                Ok(Some(summary)) => summary,
                result => {
                    tracing::warn!(?result, thread_id = %fork.thread_id, "原生分支摘要暂不可读");
                    Session {
                        message_count: 0,
                        message_count_is_lower_bound: true,
                        preview: Vec::new(),
                        preview_offset: None,
                        metadata: None,
                        file_mtime_ms: None,
                        file_change_ms: None,
                        file_size: None,
                        ..session.clone()
                    }
                }
            };
            let branch =
                canonicalize_forked_session(&Self::layout(target), &session, parsed, &fork)?;
            self.record_branch(ctx, target, &session, message_line, branch)
                .await
        })
    }
}

impl Duplicable for CodexProvider {
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
            let store = ctx.store_for(target);
            let source_text = store.read_text(&session.file_path).await?;
            let duplicate_id = uuid::Uuid::new_v4().to_string();
            let now = ctx.clock().now();
            let text = build_duplicate_session_text(&source_text, &duplicate_id, &now)?;
            // 源在回收站时复制品落到活跃目录：复制品不该留在回收站。
            let path = layout.branch_path_for(&session.file_path, &duplicate_id, &now)?;
            store.write_text(&path, &text).await?;

            let duplicated = parse_content(
                SessionFileInfo {
                    file_path: path,
                    mtime_ms: 0.0,
                    change_ms: None,
                    size: text.len() as u64,
                },
                &text,
                ParseMode::Full,
            )
            .ok_or(SessionError::DuplicateFailed)?;

            // 自定义标题由元数据承载，不改写 Codex 的文件内容。
            if let Some(title) = title.filter(|value| !value.trim().is_empty()) {
                ctx.session_metadata()
                    .set_custom_title(&target.id, &duplicated.id, title)
                    .await?;
            }
            self.evict_cache(
                ctx,
                target,
                SessionView::Active,
                std::slice::from_ref(&duplicated.file_path),
            )
            .await;
            self.attach(ctx, target, duplicated).await
        })
    }
}

impl Exportable for CodexProvider {
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
            // 导出始终写宿主侧：目标文件由保存对话框决定，与 Codex 存储无关。
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

/// 当前宿主平台（供上层拼装上下文）。
#[must_use]
pub const fn current_platform() -> Platform {
    Platform::current()
}

/// 一个会话的最小可见消息数（用于能力自检与文档）。
#[must_use]
pub fn preview_messages(session: &Session) -> &[Message] {
    &session.preview
}

/// 目标是否指向回收站视图（辅助函数，供上层拼装缓存键）。
#[must_use]
pub const fn is_trash_view(view: SessionView) -> bool {
    matches!(view, SessionView::Trash)
}

/// 便于上层构造错误。
#[must_use]
pub fn unsupported(capability: &str) -> Error {
    Error::Unsupported {
        provider: ProviderId::Codex.display_name().to_owned(),
        capability: capability.to_owned(),
    }
}

/// 便于上层构造「会话不存在」错误。
#[must_use]
pub fn not_found(id: &str) -> Error {
    SessionError::NotFound { id: id.to_owned() }.into()
}

/// 导出结果的便捷构造。
#[must_use]
pub fn export_result(file_path: impl Into<String>, format: ExportFormat) -> ExportResult {
    ExportResult {
        file_path: file_path.into(),
        format,
    }
}

/// 供 `Branchable` / `Duplicable` / `Exportable` 使用的文件名构造。
pub fn derived_session_path(
    layout: &CodexLayout,
    source: &str,
    id: &str,
    now: &aivc_core::time::Timestamp,
) -> Result<String> {
    let name = build_session_file_name(now, id);
    Ok(layout.join_under(source, &name))
}

/// 测试辅助：判断某目录是否是本机会话目录。
#[must_use]
pub fn is_session_directory(path: &Path) -> bool {
    path.join("sessions").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_codex_home_prefers_the_environment_variable() {
        // 环境变量优先于主目录；这里只验证判据本身，不改动进程环境。
        let home = local_codex_home();
        assert!(
            home.is_none() || home.as_ref().is_some_and(|path| path.is_absolute()),
            "配置目录必须是绝对路径"
        );
    }

    #[test]
    fn provider_reports_codex_identity() {
        let provider = CodexProvider::new();
        assert_eq!(provider.id(), ProviderId::Codex);
        assert_eq!(provider.label(), "Codex");
    }

    #[test]
    fn layout_kind_follows_the_target_kind() {
        let local = codex_target("t", "/home/a/.codex", None);
        let wsl = codex_target("t", "/home/a/.codex", Some("Ubuntu"));
        assert_eq!(CodexProvider::layout(&local).kind(), StoreKind::Local);
        assert_eq!(CodexProvider::layout(&wsl).kind(), StoreKind::Wsl);
    }

    #[test]
    fn cache_keys_separate_views() {
        // 活跃与回收站必须落在不同缓存键上。
        assert_ne!(
            format!("local-{}", SessionView::Active.as_str()),
            format!("local-{}", SessionView::Trash.as_str())
        );
        assert!(is_trash_view(SessionView::Trash));
        assert!(!is_trash_view(SessionView::Active));
    }

    #[test]
    fn derived_paths_keep_the_source_directory() {
        let layout = CodexLayout::new("/home/a/.codex", StoreKind::Wsl);
        let now = aivc_core::time::Timestamp::from_raw("2026-09-16T12:34:56.789Z");
        let path = derived_session_path(
            &layout,
            "/home/a/.codex/sessions/2026/09/16/rollout-x.jsonl",
            "new-id",
            &now,
        )
        .expect("derived path");
        assert_eq!(
            path,
            "/home/a/.codex/sessions/2026/09/16/rollout-2026-09-16T12-34-56-new-id.jsonl"
        );
    }

    #[test]
    fn wsl_target_is_produced_even_when_no_codex_home_resolves() {
        // **回归守门**：这里曾经有「解析不出 Codex 主目录就 `continue`」。
        //
        // 实机后果：`wsl -l -q` 给出 Ubuntu-22.04 与 Ubuntu-24.04，Ubuntu-22.04 没有
        // `~/.codex/sessions`，于是它**从目标列表里消失了**——用户看到的目标比原平台
        // 少一个，而界面上没有任何东西解释为什么。
        //
        // 更严重的是它同时封死了一条恢复路径：WSL 内的 Codex 装在非标准位置时，用户
        // 要靠「手动指定 Codex 目录」把它接进来，而那个入口只对**已经出现在列表里**的
        // 目标开放。目标被跳过 = 永远无法配置。
        let target =
            CodexProvider::wsl_target("Ubuntu-22.04", WslProbe::reachable(String::new(), false));
        assert_eq!(target.id.as_str(), "codex:wsl:Ubuntu-22.04");
        assert_eq!(target.label, "WSL · Ubuntu-22.04");
        assert!(target.available, "进得去的发行版必须可选");
        assert_eq!(target.codex_home, None, "解析不出路径时不得编造一个");
        assert_eq!(
            target.detail.as_deref(),
            Some("可进入 WSL，未找到 Codex 目录")
        );
    }

    #[test]
    fn an_unreachable_distro_is_listed_but_marked_unavailable() {
        // **这是原平台没有的一种状态。**
        //
        // 原平台对**每一个**发行版都写 `available: true`，于是一个注册在册、却因为
        // VHDX 挂载被拒而进不去的发行版（实测返回
        // `Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED`）在界面上与可用的
        // 发行版长得一模一样，用户点下去只会收到一个失败。
        //
        // 正确做法是：**照样列出**（用户要能看见它、要能对它做「手动指定」），
        // 但如实标注不可用，并说明原因。
        let target = CodexProvider::wsl_target(
            "Ubuntu-22.04",
            WslProbe::unreachable("Wsl/Service/CreateInstance/MountDisk/HCS/E_ACCESSDENIED"),
        );
        assert_eq!(target.id.as_str(), "codex:wsl:Ubuntu-22.04");
        assert!(
            !target.available,
            "进不去的发行版不能被标成可用——前端会据此决定是否自动选中它"
        );
        assert_eq!(target.codex_home, None);
        let detail = target.detail.as_deref().unwrap_or_default();
        assert!(
            detail.starts_with("无法进入 WSL："),
            "必须说清「进不去」，而不是含糊成「未找到 Codex 目录」：{detail}"
        );
        assert!(
            detail.contains("E_ACCESSDENIED"),
            "原因要带上，否则排查时又要靠猜：{detail}"
        );
    }

    #[test]
    fn wsl_target_detail_distinguishes_the_four_states() {
        // `detail` 是界面上唯一能解释「为什么这个目标点进去是空的」的依据，因此各状态
        // 必须给出**互不相同**的文案。
        let with_sessions =
            CodexProvider::wsl_target("Ubuntu-24.04", WslProbe::reachable("/home/a/.codex", true));
        let bare_dir =
            CodexProvider::wsl_target("Ubuntu-22.04", WslProbe::reachable("/home/a/.codex", false));
        let nothing =
            CodexProvider::wsl_target("Ubuntu-20.04", WslProbe::reachable(String::new(), false));
        let broken = CodexProvider::wsl_target("Ubuntu-18.04", WslProbe::unreachable("挂载失败"));
        assert_eq!(with_sessions.detail.as_deref(), Some("找到 Codex 会话"));
        assert_eq!(bare_dir.detail.as_deref(), Some("找到 Codex 目录"));
        assert_eq!(
            nothing.detail.as_deref(),
            Some("可进入 WSL，未找到 Codex 目录")
        );
        assert_eq!(broken.detail.as_deref(), Some("无法进入 WSL：挂载失败"));
        // 目录存在就如实记录路径——它是「手动指定」那条路的人工核对依据。
        assert_eq!(with_sessions.codex_home.as_deref(), Some("/home/a/.codex"));
        assert_eq!(bare_dir.codex_home.as_deref(), Some("/home/a/.codex"));
        // 四种状态里只有「进不去」是不可用的。
        for (name, target) in [
            ("with_sessions", &with_sessions),
            ("bare_dir", &bare_dir),
            ("nothing", &nothing),
        ] {
            assert!(target.available, "{name} 应当可用");
        }
    }

    #[test]
    fn every_wsl_distro_yields_a_target_regardless_of_its_state() {
        // 把「循环不会跳过任何发行版」这件事直接钉住：同样数量的输入，同样数量的输出，
        // 且三种可达状态与一种不可达状态各自都留下了目标。
        let probes = [
            ("Ubuntu-24.04", WslProbe::reachable("/home/a/.codex", true)),
            ("Ubuntu-22.04", WslProbe::reachable("/home/a/.codex", false)),
            ("Debian", WslProbe::reachable(String::new(), false)),
            ("Ubuntu-18.04", WslProbe::unreachable("挂载失败")),
        ];
        let targets: Vec<Target> = probes
            .iter()
            .map(|(distro, probe)| CodexProvider::wsl_target(distro, probe.clone()))
            .collect();
        assert_eq!(targets.len(), probes.len(), "逐个发行版都必须产出目标");
        let ids: Vec<&str> = targets.iter().map(|target| target.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "codex:wsl:Ubuntu-24.04",
                "codex:wsl:Ubuntu-22.04",
                "codex:wsl:Debian",
                "codex:wsl:Ubuntu-18.04"
            ],
            "顺序必须与探测顺序一致"
        );
    }

    // -----------------------------------------------------------------------
    // 端到端：真实临时目录 + 真实数据库
    // -----------------------------------------------------------------------

    use crate::testing::{TestContext, session_content, session_file_name};

    const UUID_A: &str = "0191f2a3-0000-7000-8000-00000000000a";
    const UUID_B: &str = "0191f2a3-0000-7000-8000-00000000000b";

    fn active_relative(uuid: &str) -> String {
        format!(
            "sessions/2026/09/16/{}",
            session_file_name("2026-09-16T10-00-00", uuid)
        )
    }

    fn trash_relative(uuid: &str) -> String {
        format!(
            ".visual-console-trash/sessions/2026/09/16/{}",
            session_file_name("2026-09-16T10-00-00", uuid)
        )
    }

    fn mutation(id: &str) -> SessionMutationRef {
        SessionMutationRef {
            id: SessionId::from(id),
            file_path: None,
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn listing_returns_parsed_sessions() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "标题 A"));

        let sessions = CodexProvider::new()
            .list_sessions(&ctx.context, &target)
            .await
            .expect("list");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id.as_str(), UUID_A);
        assert_eq!(sessions[0].title, "标题 A");
        // **列表路径必须把条数标注成下界。**
        //
        // 它只读了头部窗口，数出来的不是总数。四个 provider 各有一条列表路径，漏一个
        // 就会让那一个把下界当成精确值报出去——而其余三个是对的，所以界面上看起来「大
        // 多数会话都对」，最难查。
        assert!(
            sessions[0].message_count_is_lower_bound,
            "列表路径必须标注条数是下界"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_moves_the_file_into_the_trash_tree() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let relative = active_relative(UUID_A);
        let source = ctx.write_session(&relative, &session_content(UUID_A, "将被删除"));

        let result = CodexProvider::new()
            .delete_session(&ctx.context, &target, &mutation(UUID_A))
            .await
            .expect("delete");
        assert!(result.stale.is_none());
        assert!(!source.exists(), "原文件必须已被移走");

        // 回收站里保持相对 codexHome 的结构（含 sessions/ 前缀）。
        let trashed = ctx.write_session(&trash_relative(UUID_A), "");
        assert!(trashed.exists(), "回收站中应当出现对应文件");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_then_restore_round_trips() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let source = ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "往返"));

        let provider = CodexProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(UUID_A))
            .await
            .expect("delete");
        let restored = provider
            .restore_session(&ctx.context, &target, &SessionId::from(UUID_A))
            .await
            .expect("restore");

        assert_eq!(Path::new(&restored), source.as_path());
        assert!(source.exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restore_refuses_to_overwrite_an_existing_file() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "原文件"));

        let provider = CodexProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(UUID_A))
            .await
            .expect("delete");
        // 原位置又出现了一个同名文件（例如用户在别处恢复了副本）。
        ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "新文件"));

        let error = provider
            .restore_session(&ctx.context, &target, &SessionId::from(UUID_A))
            .await
            .expect_err("必须拒绝覆盖");
        assert_eq!(error.to_string(), "原位置已存在同名会话文件，无法恢复。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn purge_removes_the_file_from_the_trash() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(
            &active_relative(UUID_A),
            &session_content(UUID_A, "将被彻底删除"),
        );

        let provider = CodexProvider::new();
        provider
            .delete_session(&ctx.context, &target, &mutation(UUID_A))
            .await
            .expect("delete");
        let trashed = ctx
            .codex_home
            .join(trash_relative(UUID_A).replace('/', std::path::MAIN_SEPARATOR_STR));
        assert!(trashed.exists());

        let deleted = provider
            .purge_session(&ctx.context, &target, &mutation(UUID_A))
            .await
            .expect("purge");
        assert_eq!(Path::new(&deleted), trashed.as_path());
        assert!(!trashed.exists(), "文件必须已被删除");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_rejects_a_forged_path_outside_the_sessions_root() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        // 在 codex_home 下但不在 sessions/ 内：伪造的 filePath 想把它移进回收站。
        let outside = ctx.write_session("config.toml", "不该被移动");

        let error = CodexProvider::new()
            .delete_session(
                &ctx.context,
                &target,
                &SessionMutationRef {
                    id: SessionId::from(UUID_A),
                    file_path: Some(outside.to_string_lossy().into_owned()),
                },
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "拒绝将 sessions 目录之外的文件移动到回收站"
        );
        assert!(outside.exists(), "被拒绝的文件不得被改动");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_rejects_a_path_whose_content_has_a_different_id() {
        // 用 A 的编号去删 B 的文件：路径在 sessions/ 内，但内容编号不匹配。
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let other = ctx.write_session(
            &active_relative(UUID_B),
            &session_content(UUID_B, "B 的会话"),
        );

        let error = CodexProvider::new()
            .delete_session(
                &ctx.context,
                &target,
                &SessionMutationRef {
                    id: SessionId::from(UUID_A),
                    file_path: Some(other.to_string_lossy().into_owned()),
                },
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            format!("会话文件与会话编号不匹配：{UUID_A}")
        );
        assert!(other.exists(), "编号不匹配的文件不得被改动");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn deleting_a_stale_entry_reports_stale_instead_of_failing() {
        // 列表项已过期：文件不存在了。这不是错误，应如实返回 stale。
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let result = CodexProvider::new()
            .delete_session(&ctx.context, &target, &mutation(UUID_A))
            .await
            .expect("不得报错");
        assert_eq!(result.stale, Some(true));
        assert!(result.moved_to.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn batch_delete_moves_every_session() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "A"));
        ctx.write_session(&active_relative(UUID_B), &session_content(UUID_B, "B"));

        let result = CodexProvider::new()
            .delete_sessions(&ctx.context, &target, &[mutation(UUID_A), mutation(UUID_B)])
            .await
            .expect("batch delete");
        assert_eq!(result.processed.len(), 2);
        assert!(
            result
                .processed
                .iter()
                .all(|entry| entry.moved_to.is_some())
        );
        assert!(
            ctx.codex_home
                .join("sessions/2026/09/16")
                .read_dir()
                .is_ok_and(|mut entries| entries.next().is_none()),
            "活跃目录中的会话文件应已全部移走"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn paging_returns_the_tail_with_its_real_offset() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let mut lines = vec![format!(
            r#"{{"timestamp":"2026-09-16T12:00:00.000Z","type":"session_meta","payload":{{"id":"{UUID_A}","cwd":"/w"}}}}"#
        )];
        for index in 0..10 {
            lines.push(format!(
                r#"{{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{{"type":"user_message","message":"消息{index}"}}}}"#
            ));
        }
        ctx.write_session(&active_relative(UUID_A), &lines.join("\n"));

        let page = CodexProvider::new()
            .get_session_messages_page(
                &ctx.context,
                &target,
                &SessionId::from(UUID_A),
                PageCursor::Latest,
                3,
            )
            .await
            .expect("page");

        let texts: Vec<&str> = page.messages.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, vec!["消息7", "消息8", "消息9"]);
        // 首屏必须回填真实绝对序号，否则向上翻页会错位。
        assert_eq!(page.offset, 7);
        assert!(page.has_more);
        // 每行都要带上原始 JSONL 的绝对行号。
        assert_eq!(page.messages[0].source_line, Some(9));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn search_matches_the_summary_without_reading_the_file() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(
            &active_relative(UUID_A),
            &session_content(UUID_A, "重构会话列表"),
        );
        ctx.write_session(
            &active_relative(UUID_B),
            &session_content(UUID_B, "无关内容"),
        );

        let found = CodexProvider::new()
            .search_sessions(&ctx.context, &target, SessionView::Active, "重构")
            .await
            .expect("search");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id.as_str(), UUID_A);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_empty_search_query_lists_everything() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "A"));
        ctx.write_session(&active_relative(UUID_B), &session_content(UUID_B, "B"));

        let found = CodexProvider::new()
            .search_sessions(&ctx.context, &target, SessionView::Active, "   ")
            .await
            .expect("search");
        assert_eq!(found.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn folder_path_points_at_the_session_directory() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let source = ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "A"));

        let folder = CodexProvider::new()
            .get_session_folder_path(&ctx.context, &target, &SessionId::from(UUID_A))
            .await
            .expect("folder");
        assert_eq!(
            Path::new(&folder),
            source.parent().expect("parent"),
            "所在目录必须与文件位置一致"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reading_a_missing_session_reports_not_found() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let error = CodexProvider::new()
            .get_session(&ctx.context, &target, &SessionId::from(UUID_A), None)
            .await
            .expect_err("必须失败");
        assert_eq!(error.to_string(), format!("未找到会话：{UUID_A}"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reading_a_forged_path_outside_the_sessions_root_is_rejected() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let outside = ctx.write_session("config.toml", "{}");

        let error = CodexProvider::new()
            .get_session(
                &ctx.context,
                &target,
                &SessionId::from(UUID_A),
                Some(&outside.to_string_lossy()),
            )
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "拒绝读取 Codex 会话目录之外的文件。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn trash_sessions_are_listed_separately_from_active_ones() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(&active_relative(UUID_A), &session_content(UUID_A, "活跃"));
        ctx.write_session(&trash_relative(UUID_B), &session_content(UUID_B, "回收站"));

        let provider = CodexProvider::new();
        let active = provider
            .list_sessions(&ctx.context, &target)
            .await
            .expect("active");
        let trash = provider
            .list_trash_sessions(&ctx.context, &target)
            .await
            .expect("trash");

        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id.as_str(), UUID_A);
        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id.as_str(), UUID_B);
    }
}
