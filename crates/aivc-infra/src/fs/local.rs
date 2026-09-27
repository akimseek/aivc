//! 本机文件系统 `SessionStore` 适配器。
//!
//! 实现 [`SessionStore`] 端口。**目录布局、文件格式解析与分支截断规则不属于本层**，
//! 仍由各 Provider 决定——Codex/Claude/Gemini/Qoder 的存储布局是真实差异，抽象成统一
//! 基类只会增加条件分支（现网 DEV-GUIDE 已记录过这个结论）。
//!
//! # 为什么用 `block_in_place` 而不是 `spawn_blocking`
//!
//! `spawn_blocking` 要求闭包 `'static`，而逐行读取的回调经常借用调用方的局部状态
//! （累积消息、记录锚点）。`block_in_place` 允许在保持借用的同时执行阻塞 I/O，
//! 代价是占用一个工作线程——对本应用（每标签一个读取任务）完全可接受。
//!
//! 若改用「`spawn_blocking` + 通道」把回调搬到异步侧，反而会让每行的 JSON 解析回到
//! 运行时线程上执行，与本层「把解析移出运行时」的目标相反。
//!
//! # 运行时约束
//!
//! 因此本实现**要求多线程运行时**（`#[tokio::main]` 默认即是）。在单线程运行时上调用
//! 逐行读取会 panic——这是 `block_in_place` 的既有行为，属于「快速失败」而不是静默
//! 降级。单元测试因此显式声明 `flavor = "multi_thread"`。

use std::path::{Path, PathBuf};

use aivc_core::error::Result;
use aivc_core::ports::{
    BoxFuture, ConflictPolicy, LineSink, ReadOptions, ReadStart, SessionStore, StoreKind,
};

use super::line_reader::{read_head_blocking, read_lines_blocking};

/// 本机会话文件存储。
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalSessionStore;

impl LocalSessionStore {
    /// 构造。
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl SessionStore for LocalSessionStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Local
    }

    fn read_text<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move { Ok(tokio::fs::read_to_string(path).await?) })
    }

    fn write_text<'a>(&'a self, path: &'a str, content: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            if let Some(parent) = Path::new(path).parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(path, content).await?;
            Ok(())
        })
    }

    fn read_lines<'a>(
        &'a self,
        path: &'a str,
        start: ReadStart,
        _options: ReadOptions,
        mut sink: LineSink<'a>,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // `options.timeout_ms` 只对 WSL 生效：本机读取没有远程挂起风险，
            // 现网也只对 WSL 施加超时。
            tokio::task::block_in_place(|| {
                read_lines_blocking(Path::new(path), start, sink.as_mut())
            })
        })
    }

    fn read_head<'a>(&'a self, path: &'a str, max_bytes: usize) -> BoxFuture<'a, Result<String>> {
        Box::pin(async move {
            tokio::task::block_in_place(|| read_head_blocking(Path::new(path), max_bytes))
        })
    }

    fn exists<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<bool>> {
        Box::pin(async move { Ok(tokio::fs::try_exists(path).await.unwrap_or(false)) })
    }

    fn size<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<u64>> {
        Box::pin(async move {
            let metadata = tokio::fs::metadata(path).await?;
            Ok(metadata.len())
        })
    }

    fn move_file<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
        policy: ConflictPolicy,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let destination = PathBuf::from(to);
            if policy == ConflictPolicy::FailIfExists
                && tokio::fs::try_exists(&destination).await.unwrap_or(false)
            {
                // 「恢复」场景：原位置已存在同名文件必须拒绝，不能静默覆盖。
                return Err(aivc_core::error::SessionError::RestoreConflict.into());
            }
            if let Some(parent) = destination.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::rename(from, &destination).await?;
            Ok(())
        })
    }

    fn remove<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            tokio::fs::remove_file(path).await?;
            Ok(())
        })
    }

    fn remove_tree<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            // 目标不存在时视为成功：删除是幂等的。
            let Ok(metadata) = tokio::fs::symlink_metadata(path).await else {
                return Ok(());
            };
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(path).await?;
            } else {
                tokio::fs::remove_file(path).await?;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::ports::LineControl;

    async fn write(dir: &tempfile::TempDir, name: &str, content: &str) -> String {
        let path = dir.path().join(name);
        let store = LocalSessionStore::new();
        store
            .write_text(path.to_str().expect("utf8 path"), content)
            .await
            .expect("write");
        path.to_string_lossy().into_owned()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn write_then_read_round_trips_and_creates_parents() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = dir.path().join("a").join("b").join("x.jsonl");
        let path_str = path.to_str().expect("utf8");

        store.write_text(path_str, "hello").await.expect("write");
        assert!(store.exists(path_str).await.expect("exists"));
        assert_eq!(store.read_text(path_str).await.expect("read"), "hello");
        assert_eq!(store.size(path_str).await.expect("size"), 5);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reading_a_missing_file_fails_but_exists_returns_false() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = dir.path().join("missing").to_string_lossy().into_owned();

        assert!(!store.exists(&path).await.expect("exists"));
        assert!(store.read_text(&path).await.is_err());
        assert!(store.size(&path).await.is_err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_lines_streams_with_absolute_line_numbers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = write(&dir, "a.jsonl", "one\ntwo\nthree\n").await;

        let mut seen = Vec::new();
        store
            .read_lines(
                &path,
                ReadStart::Line(2),
                ReadOptions::none(),
                Box::new(|line: &str, number: u64, _offset: u64| {
                    seen.push((number, line.to_owned()));
                    LineControl::Continue
                }),
            )
            .await
            .expect("read lines");

        assert_eq!(seen, vec![(2, "two".to_owned()), (3, "three".to_owned())]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_lines_from_a_byte_offset_reports_absolute_line_numbers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = write(&dir, "a.jsonl", "aaa\nbbb\nccc\n").await;

        let mut seen = Vec::new();
        store
            .read_lines(
                &path,
                ReadStart::Byte {
                    offset: 8,
                    line_number: 3,
                },
                ReadOptions::none(),
                Box::new(|line: &str, number: u64, _offset: u64| {
                    seen.push((number, line.to_owned()));
                    LineControl::Continue
                }),
            )
            .await
            .expect("read lines");

        assert_eq!(seen, vec![(3, "ccc".to_owned())]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_lines_can_stop_early() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = write(&dir, "a.jsonl", "one\ntwo\nthree\n").await;

        let mut count = 0u64;
        store
            .read_lines(
                &path,
                ReadStart::BEGINNING,
                ReadOptions::none(),
                Box::new(|_: &str, _: u64, _: u64| {
                    count += 1;
                    LineControl::Stop
                }),
            )
            .await
            .expect("read lines");
        assert_eq!(count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn read_head_returns_only_the_requested_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = write(&dir, "a.jsonl", &"x".repeat(500)).await;

        let head = store.read_head(&path, 64).await.expect("head");
        assert_eq!(head.len(), 64);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn move_with_conflict_policy_refuses_to_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let source = write(&dir, "source.jsonl", "source").await;
        let target = write(&dir, "target.jsonl", "target").await;

        // 「恢复」语义：目标已存在必须拒绝，且不得改动任何一个文件。
        let outcome = store
            .move_file(&source, &target, ConflictPolicy::FailIfExists)
            .await;
        assert!(outcome.is_err());
        assert_eq!(
            outcome.expect_err("必须失败").to_string(),
            "原位置已存在同名会话文件，无法恢复。"
        );
        assert_eq!(store.read_text(&source).await.expect("source"), "source");
        assert_eq!(store.read_text(&target).await.expect("target"), "target");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn move_creates_parent_directories_and_succeeds_when_free() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let source = write(&dir, "source.jsonl", "content").await;
        let target = dir
            .path()
            .join("trash")
            .join("deep")
            .join("source.jsonl")
            .to_string_lossy()
            .into_owned();

        store
            .move_file(&source, &target, ConflictPolicy::FailIfExists)
            .await
            .expect("move");

        assert!(!store.exists(&source).await.expect("source gone"));
        assert_eq!(store.read_text(&target).await.expect("target"), "content");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn move_with_overwrite_policy_replaces_the_target() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let source = write(&dir, "source.jsonl", "new").await;
        let target = write(&dir, "target.jsonl", "old").await;

        store
            .move_file(&source, &target, ConflictPolicy::Overwrite)
            .await
            .expect("move");
        assert_eq!(store.read_text(&target).await.expect("target"), "new");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn remove_deletes_the_file_and_reports_missing_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = LocalSessionStore::new();
        let path = write(&dir, "a.jsonl", "x").await;

        store.remove(&path).await.expect("remove");
        assert!(!store.exists(&path).await.expect("exists"));
        // 再次删除必须报错，而不是静默成功。
        assert!(store.remove(&path).await.is_err());
    }

    #[test]
    fn store_reports_its_kind() {
        assert_eq!(LocalSessionStore::new().kind(), StoreKind::Local);
        assert!(!LocalSessionStore::new().kind().is_wsl());
    }
}
