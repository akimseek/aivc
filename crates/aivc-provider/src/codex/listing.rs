//! 会话列表构建：缓存优先 + 有界并发解析。
//!
//! # 缓存判定
//!
//! 缓存命中要求 **mtime、ctime、size 三者全部相等**。只比 mtime 不够：同一秒内改写
//! 文件可能保持 mtime 不变，而 ctime 与 size 能补上这个缺口。
//!
//! # 缓存是整体替换而不是增量更新
//!
//! 每轮列表都从**当前文件集合**重建缓存，因此已删除文件的陈旧条目会被自然淘汰，
//! 不需要额外的失效逻辑。这也是现网的做法。
//!
//! # 两种读取深度
//!
//! - **轻量**（列表页）：只读文件头部 64 KB。会话正文可能几十 MB，列表页只需要标题、
//!   时间与前几条预览。
//! - **完整**：读整个文件。
//!
//! 两者都产出同一套 [`Session`]，区别只在预览条数与 `updatedAt` 的来源（见
//! [`super::parser`]）。

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::stream::{self, StreamExt};

use aivc_core::error::Result;
use aivc_core::ports::SessionStore;
use aivc_core::session::{Session, SessionFile};
use aivc_core::target::Target;
use aivc_infra::db::SessionCacheEntry;

use super::parser::{ParseMode, SessionFileInfo, parse_content};
use crate::common::sort_sessions;
use crate::context::ProviderContext;

/// 列表解析并发上限。
///
/// 现网取 6：再多会让 WSL 侧的多次 `wsl.exe` 调用互相拖慢，也会让机械硬盘上的
/// 随机读退化。
pub const LIST_PARSE_CONCURRENCY: usize = 6;

/// 轻量列表读取的头部字节上限（64 KB）。
pub const LIST_HEAD_BYTES: usize = 64 * 1024;

/// 头部里找不到标题时，为找标题而补读的字节上限（512 KB）。
///
/// 只对**确实没找到标题**的文件生效，因此正常情况下一个字节都不多读。
/// 它不保证一定能找到——注入块再多也总有个头，但那是数据的问题，不是这里能解决的；
/// 找不到时界面会显示「目录 · 时间」而不是裸 UUID（见 `ui/src/app/session-display.ts`）。
pub const LIST_TITLE_BYTES: usize = 512 * 1024;

/// 列表构建的结果。
#[derive(Debug, Default)]
pub struct ListOutcome {
    /// 排序后的会话。
    pub sessions: Vec<Session>,
    /// 命中缓存的条数。
    pub cache_hits: usize,
    /// 实际解析的条数。
    pub parsed: usize,
    /// 解析失败（损坏或缺编号）的条数。
    pub skipped: usize,
}

/// 由文件列表构建会话列表。
///
/// `cache_key` 由 [`ProviderContext::cache_key`] 生成，必须区分目标与视图
/// （活跃/回收站），否则回收站会话会污染活跃列表。
pub async fn list_sessions_from_files(
    ctx: &ProviderContext,
    target: &Target,
    files: &[SessionFile],
    cache_key: &str,
    lightweight: bool,
) -> Result<ListOutcome> {
    let cached = ctx.session_cache().read(cache_key).await?;
    let store = ctx.store_for(target);
    let store = Arc::new(store);

    // 第一步：先把「缓存命中」的快速判定做完，命中的完全不读盘。
    let mut results: Vec<Option<(SessionCacheEntry, bool)>> = vec![None; files.len()];
    let mut pending_indexes: Vec<usize> = Vec::new();
    let mut pending_paths: Vec<String> = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let change_ms = file.change_ms.unwrap_or(file.mtime_ms);
        if let Some(entry) = cached.get(&file.file_path)
            && entry.is_fresh_for(file.mtime_ms, change_ms, file.size)
        {
            results[index] = Some((entry.clone(), true));
            continue;
        }
        pending_indexes.push(index);
        pending_paths.push(file.file_path.clone());
    }

    // 第二步：未命中的文件**一次批量读**。
    //
    // 这里原本是「每个文件一次 `read_head`，配 6 路并发」。在 WSL 上那意味着 **6 个
    // `wsl.exe` 进程同时启动**，而 WSL2 空闲后会自动关闭发行版——于是它们一起抢同一次
    // 冷启动。实测：冷启动 5.3 秒、空闲 20 秒后再调用 6.9 秒；应用日志里因此出现过
    // `head` 之间 **29.99 秒**的空白，用户看到的是「会话列表打不开」。
    //
    // 改成批量后，14 个文件从 14 次调用（3 波并发）压成 1 次调用。
    let contents: Vec<Option<String>> = if lightweight {
        store.read_heads(&pending_paths, LIST_HEAD_BYTES).await
    } else {
        // 完整解析要读整个文件，没有批量接口，保持有界并发。
        stream::iter(pending_paths.iter().cloned())
            .map(|path| {
                let store = Arc::clone(&store);
                async move { store.read_text(&path).await.ok() }
            })
            .buffered(LIST_PARSE_CONCURRENCY)
            .collect()
            .await
    };

    let mode = if lightweight {
        ParseMode::List
    } else {
        ParseMode::Full
    };
    for (slot, content) in pending_indexes.into_iter().zip(contents) {
        // 单个文件读不到只跳过它：一个损坏文件不应让整个列表打不开。
        let Some(content) = content else {
            continue;
        };
        let file = &files[slot];
        let info = SessionFileInfo::from_session_file(file);
        if let Some(session) = parse_content(info, &content, mode) {
            results[slot] = Some((
                SessionCacheEntry {
                    file_path: file.file_path.clone(),
                    mtime_ms: file.mtime_ms,
                    change_ms: file.change_ms.unwrap_or(file.mtime_ms),
                    size: file.size,
                    session,
                },
                false,
            ));
        }
    }

    // 第三步：头部里找不到标题的，**再读一段**，只为找标题。
    //
    // 标题此前完全依赖这 64 KB 的头部窗口，而 Codex 会话开头常常是成片的注入块
    // （`<multi_agent_mode>` 之类），真正的用户提示词落在窗口之外。结果是标题静默
    // 退化成会话编号——列表里 5 条会话全是 UUID，没有任何区分信息
    // （见 docs/rust-rewrite/19）。
    //
    // 之所以不直接把 `LIST_HEAD_BYTES` 调大：那会让**每一个**文件都付这份代价。
    // 这一遍只对**确实没找到标题**的文件读第二次。
    if lightweight {
        let mut retry_indexes: Vec<usize> = Vec::new();
        let mut retry_paths: Vec<String> = Vec::new();
        for (slot, file) in files.iter().enumerate() {
            let Some((entry, _)) = results[slot].as_ref() else {
                continue;
            };
            // 标题等于编号即「第一遍没找到」——这正是需要补读的信号。
            if entry.session.title == entry.session.id.to_string() {
                retry_indexes.push(slot);
                retry_paths.push(file.file_path.clone());
            }
        }
        if !retry_indexes.is_empty() {
            let deeper = store.read_heads(&retry_paths, LIST_TITLE_BYTES).await;
            for (slot, content) in retry_indexes.into_iter().zip(deeper) {
                let Some(content) = content else {
                    continue;
                };
                let file = &files[slot];
                let info = SessionFileInfo::from_session_file(file);
                let Some(found) = parse_content(info, &content, ParseMode::List) else {
                    continue;
                };
                // **只取标题**：其余字段沿用第一遍的结果。用更长的内容整体覆盖会有
                // 副作用——预览会突然变多，而列表并不需要它。
                if found.title != found.id.to_string()
                    && let Some((entry, _)) = results[slot].as_mut()
                {
                    entry.session.title = found.title;
                }
            }
        }
    }

    let mut outcome = ListOutcome::default();
    let mut entries: Vec<SessionCacheEntry> = Vec::with_capacity(results.len());
    let mut sessions: Vec<Session> = Vec::with_capacity(results.len());
    for result in results.into_iter().flatten() {
        let (mut entry, hit) = result;
        if hit {
            outcome.cache_hits += 1;
        } else {
            outcome.parsed += 1;
        }
        // 列表模式只读了头部窗口，**这一遍数出来的条数是真实的下界**，不是总数。
        //
        // 因此保留数出来的条数（它不撒谎：「≥2 条」是真的），同时**把它标注成下界**。
        //
        // 「它是下界」这件事必须跟着数据走，不能只写在界面的一行注释里——否则线上传的
        // 就是一个光看数字分不出是下界还是总数的值。这里置位、界面据此显示「≥2 条」。
        //
        // 曾经的做法是一律置 `0`（沿用 `paging.rs` 的「不得声称已知总数」）并显示「—」。
        // 那修掉的是一句假话（更早时把 80 条显示成「2 条」），但把信息也一起丢了。
        if lightweight {
            entry.session.message_count_is_lower_bound = true;
        }
        sessions.push(entry.session.clone());
        entries.push(entry);
    }
    outcome.skipped = files.len() - sessions.len();

    // 整体替换：已删除文件的陈旧条目随之淘汰。
    ctx.session_cache().replace(cache_key, &entries).await?;

    sort_sessions(&mut sessions);
    outcome.sessions = sessions;
    Ok(outcome)
}

/// 只读缓存，不触发扫描。
pub async fn list_cached_sessions(ctx: &ProviderContext, cache_key: &str) -> Result<Vec<Session>> {
    let cached: BTreeMap<String, SessionCacheEntry> = ctx.session_cache().read(cache_key).await?;
    let mut sessions: Vec<Session> = cached.into_values().map(|entry| entry.session).collect();
    sort_sessions(&mut sessions);
    Ok(sessions)
}

/// 递归枚举本机目录下的会话文件。
///
/// 只接受 `rollout-*.jsonl` 形状的文件名——目录里可能混有别的文件（Codex 自身也会写
/// 索引），把它们当成会话会让列表出现无法解析的条目。
///
/// 读取失败的目录与文件被跳过而不是让整次扫描失败：单个异常不应让会话列表打不开。
pub async fn walk_local_session_files(root: &std::path::Path) -> Vec<SessionFile> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&directory).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(metadata) = entry.metadata().await else {
                continue;
            };
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_session_file_name(&name) {
                continue;
            }
            let mtime_ms = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0.0, |delta| delta.as_millis() as f64);
            files.push(SessionFile {
                file_path: path.to_string_lossy().into_owned(),
                mtime_ms,
                // 本机读取拿不到独立的 ctime，用 mtime 顶上；缓存判定因此退化为
                // 「mtime + size」，仍能覆盖绝大多数改动。
                change_ms: Some(mtime_ms),
                size: metadata.len(),
            });
        }
    }
    files
}

/// 判断文件名是否是 Codex 会话文件。
#[must_use]
pub fn is_session_file_name(name: &str) -> bool {
    name.starts_with("rollout-") && name.to_ascii_lowercase().ends_with(".jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{TestContext, session_content, session_file_name};

    const UUID_A: &str = "0191f2a3-0000-7000-8000-00000000000a";
    const UUID_B: &str = "0191f2a3-0000-7000-8000-00000000000b";

    /// 扫描一个目标下的会话文件。
    async fn files_for(ctx: &TestContext, relative_root: &str) -> Vec<SessionFile> {
        let root = ctx
            .codex_home
            .join(relative_root.replace('/', std::path::MAIN_SEPARATOR_STR));
        walk_local_session_files(&root).await
    }
    #[tokio::test(flavor = "multi_thread")]
    async fn listing_parses_files_and_sorts_by_recency() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(
            &format!(
                "sessions/2026/09/16/{}",
                session_file_name("2026-09-16T10-00-00", UUID_A)
            ),
            &session_content(UUID_A, "第一条"),
        );
        // 让第二个文件的 mtime 更晚。
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        ctx.write_session(
            &format!(
                "sessions/2026/09/17/{}",
                session_file_name("2026-09-17T10-00-00", UUID_B)
            ),
            &session_content(UUID_B, "第二条"),
        );

        let files = files_for(&ctx, "sessions").await;
        assert_eq!(files.len(), 2);

        let outcome = list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("list");
        assert_eq!(outcome.sessions.len(), 2);
        assert_eq!(outcome.parsed, 2);
        assert_eq!(outcome.cache_hits, 0);
        // 列表按 updatedAt（此处为文件 mtime）倒序。
        assert_eq!(outcome.sessions[0].id.as_str(), UUID_B);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_listing_hits_the_cache() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(
            &format!(
                "sessions/2026/09/16/{}",
                session_file_name("2026-09-16T10-00-00", UUID_A)
            ),
            &session_content(UUID_A, "标题"),
        );

        let files = files_for(&ctx, "sessions").await;
        let first = list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("first");
        assert_eq!(first.cache_hits, 0);

        let second = list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("second");
        assert_eq!(second.cache_hits, 1, "文件未变时应当命中缓存");
        assert_eq!(second.parsed, 0);
        assert_eq!(second.sessions[0].title, "标题");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn changing_a_file_invalidates_its_cache_entry() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let relative = format!(
            "sessions/2026/09/16/{}",
            session_file_name("2026-09-16T10-00-00", UUID_A)
        );
        ctx.write_session(&relative, &session_content(UUID_A, "旧标题"));

        let files = files_for(&ctx, "sessions").await;
        list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("first");

        // 改写内容与大小。
        ctx.write_session(&relative, &session_content(UUID_A, "新标题更长一些"));
        let files = files_for(&ctx, "sessions").await;
        let outcome = list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("second");
        assert_eq!(outcome.cache_hits, 0, "文件已变，必须重新解析");
        assert_eq!(outcome.sessions[0].title, "新标题更长一些");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_cache_is_replaced_so_deleted_files_disappear() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        let relative = format!(
            "sessions/2026/09/16/{}",
            session_file_name("2026-09-16T10-00-00", UUID_A)
        );
        let path = ctx.write_session(&relative, &session_content(UUID_A, "会被删掉"));

        let files = files_for(&ctx, "sessions").await;
        list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("first");
        assert_eq!(
            list_cached_sessions(&ctx.context, "local")
                .await
                .expect("cached")
                .len(),
            1
        );

        // 文件消失后再列一次：缓存是整体替换，陈旧条目必须被淘汰。
        std::fs::remove_file(&path).expect("remove");
        let files = files_for(&ctx, "sessions").await;
        let outcome = list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("second");
        assert!(outcome.sessions.is_empty());
        assert!(
            list_cached_sessions(&ctx.context, "local")
                .await
                .expect("cached")
                .is_empty(),
            "已删除文件的缓存条目必须被淘汰"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unreadable_files_are_skipped_without_failing_the_list() {
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(
            &format!(
                "sessions/2026/09/16/{}",
                session_file_name("2026-09-16T10-00-00", UUID_A)
            ),
            &session_content(UUID_A, "正常"),
        );
        // 一个无法确定编号的文件（文件名不含 uuid）。
        ctx.write_session("sessions/2026/09/16/rollout-broken.jsonl", "{}");

        let files = files_for(&ctx, "sessions").await;
        let outcome = list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("list");
        assert_eq!(outcome.sessions.len(), 1);
        assert_eq!(outcome.skipped, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cache_keys_keep_views_apart() {
        // 活跃与回收站必须用不同的缓存键，否则回收站会话会污染活跃列表。
        let ctx = TestContext::new().await;
        let target = ctx.local_target();
        ctx.write_session(
            &format!(
                "sessions/2026/09/16/{}",
                session_file_name("2026-09-16T10-00-00", UUID_A)
            ),
            &session_content(UUID_A, "活跃"),
        );

        let files = files_for(&ctx, "sessions").await;
        list_sessions_from_files(&ctx.context, &target, &files, "local", true)
            .await
            .expect("active");

        // 回收站视图的缓存键不同，因此读不到活跃列表的内容。
        let trash = list_cached_sessions(&ctx.context, "local-trash")
            .await
            .expect("trash");
        assert!(trash.is_empty());
    }
}
