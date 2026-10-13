//! Codex 分支与复制的纯逻辑。
//!
//! # 分支（`thread/fork`）
//!
//! 官方 `app-server` 的 `thread/fork` 需要两个参数：
//!
//! - `threadId`：父会话编号；
//! - `lastTurnId`：**轮次**编号，不是消息行号。
//!
//! 官方注释写明「分页线程拒绝 `thread/rollback`，`lastTurnId` 在创建原生分支的同时
//! 原子地完成截断」，因此分支点必须由轮次表达。而 UI 选中的是**消息行号**，两者之间
//! 需要一次映射：从文件头顺序扫描，记住最近一个 `task_started.turn_id`，当扫到目标
//! 行号时，那个 `turn_id` 就是该消息所属轮次。
//!
//! # 复制
//!
//! 复制不调用 app-server：它只是把原文重写一份，并把 `session_meta` 的编号换成新
//! 编号。这样复制出的会话与原会话完全独立，且不需要 Codex 在本地可用。
//!
//! # 一处关键取舍
//!
//! `canonicalize_forked_session` **以 app-server 返回的 `thread.id` 为权威身份**，
//! 而不是 JSONL 里的 `session_meta.id`。原因是部分 Codex 版本在 fork 文件刚创建的
//! 短窗口内仍会写入父线程 ID；若以文件内容为准，刚建好的分支会立刻被误判成父会话。
//! 现网对此有专门的契约测试，这里保持一致。

use serde_json::{Value, json};

use aivc_core::error::{Result, SessionError};
use aivc_core::id::SessionId;
use aivc_core::ports::{LineControl, ReadOptions, ReadStart, SessionStore};
use aivc_core::session::Session;
use aivc_core::text::should_keep_message;
use aivc_core::time::Timestamp;

use super::parser::extract_message;
use super::paths::CodexLayout;

/// `app-server` 返回的分支结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkResult {
    /// 新线程编号（权威身份）。
    pub thread_id: String,
    /// 新线程的会话文件路径。
    pub file_path: String,
}

/// 找出消息行号所属的轮次编号。
///
/// `message_line` 是**原始 JSONL 的绝对行号**（1 起），必须指向一条可见会话消息；
/// 否则返回 [`SessionError::BranchAnchorNotMessage`]。
///
/// 找不到所属轮次（该行之前没有 `task_started`）时返回
/// [`SessionError::BranchAnchorStale`]——这通常意味着详情页是旧版本打开的，
/// 让用户重新打开比给一个错误的轮次更安全。
pub async fn find_branch_turn_id(
    store: &dyn SessionStore,
    file_path: &str,
    message_line: u64,
) -> Result<String> {
    if message_line == 0 {
        return Err(SessionError::BranchNeedsContext.into());
    }

    let mut active_turn_id = String::new();
    let mut selected_turn_id: Option<String> = None;
    let mut selected_message = false;
    let mut anchor_error: Option<SessionError> = None;

    store
        .read_lines(
            file_path,
            ReadStart::BEGINNING,
            ReadOptions::none(),
            Box::new(|line: &str, line_number: u64, _offset: u64| {
                let Ok(value) = serde_json::from_str::<Value>(line) else {
                    return LineControl::Continue;
                };
                let Some(object) = value.as_object() else {
                    return LineControl::Continue;
                };
                let kind = object
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let payload = object.get("payload").cloned().unwrap_or(Value::Null);

                // 记住最近一个轮次起点。
                if let Some(turn_id) = record_turn_id(&value) {
                    turn_id.clone_into(&mut active_turn_id);
                }

                if line_number != message_line {
                    return LineControl::Continue;
                }

                // 到达目标行：必须是一条可见消息，且它所属轮次已知。
                match extract_message(&payload, kind) {
                    Some(message) if should_keep_message(&message.text) => {
                        selected_message = true;
                        selected_turn_id = Some(active_turn_id.clone());
                    }
                    _ => anchor_error = Some(SessionError::BranchAnchorNotMessage),
                }
                LineControl::Stop
            }),
        )
        .await?;

    if let Some(error) = anchor_error {
        return Err(error.into());
    }
    if !selected_message || selected_turn_id.as_deref().unwrap_or_default().is_empty() {
        return Err(SessionError::BranchAnchorStale.into());
    }
    Ok(selected_turn_id.unwrap_or_default())
}

/// 校验并归一化 `app-server` 返回的分支。
///
/// 三项校验缺一不可：
///
/// 1. 新线程编号必须存在且**不等于父会话**（否则会在原地打开自己）；
/// 2. 分支文件必须位于当前 `sessions` 目录内（防止 Codex 返回任意路径）；
/// 3. 分支文件不能与父会话文件相同（否则分支会覆盖父会话）。
pub fn canonicalize_forked_session(
    layout: &CodexLayout,
    parent: &Session,
    parsed_branch: Session,
    fork: &ForkResult,
) -> Result<Session> {
    if fork.thread_id.is_empty() || fork.thread_id == parent.id.as_str() {
        return Err(SessionError::ForkIdInvalid.into());
    }
    if !layout.is_strictly_inside(&fork.file_path, &layout.sessions_root()) {
        return Err(SessionError::ForkPathOutsideRoot.into());
    }
    if same_path(&fork.file_path, &parent.file_path, layout) {
        return Err(SessionError::ForkSameAsParent.into());
    }

    Ok(Session {
        // 以 app-server 返回的线程编号为权威身份，而不是 JSONL 里的 session_meta.id。
        id: SessionId::from(fork.thread_id.clone()),
        file_path: fork.file_path.clone(),
        ..parsed_branch
    })
}

/// 判断两个路径是否指向同一位置。
///
/// 本机按分隔符归一化并在 Windows 上忽略大小写（`path.resolve` 的语义近似）；
/// WSL 侧用 POSIX 归一化。
pub(super) fn same_path(left: &str, right: &str, layout: &CodexLayout) -> bool {
    match layout.kind() {
        aivc_core::ports::StoreKind::Local => {
            let normalize = |value: &str| {
                let unified = value.replace('\\', "/");
                let trimmed = unified.trim_end_matches('/').to_owned();
                if cfg!(windows) {
                    trimmed.to_lowercase()
                } else {
                    trimmed
                }
            };
            normalize(left) == normalize(right)
        }
        aivc_core::ports::StoreKind::Wsl => {
            aivc_core::path::posix_normalize(left) == aivc_core::path::posix_normalize(right)
        }
    }
}

/// 构造复制后的会话文本。
///
/// 逐行处理：**只**重写 `session_meta` 那一行，其余行原样保留（包括无法解析的行——
/// 复制应该尽可能忠实，而不是顺手丢弃解析不了的内容）。
///
/// 重写时同时更新 `payload.id` 与 `payload.session_id`：不同 Codex 版本读的是不同的
/// 字段，只改一个会让复制出的会话在部分版本里仍被认成原会话。
pub fn build_duplicate_session_text(
    source_text: &str,
    duplicate_id: &str,
    now: &Timestamp,
) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut rewritten_meta = false;

    for raw_line in source_text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            lines.push(line.to_owned());
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            lines.push(line.to_owned());
            continue;
        };
        if value.get("type").and_then(Value::as_str) != Some("session_meta") {
            lines.push(line.to_owned());
            continue;
        }

        let mut object = value;
        if let Some(map) = object.as_object_mut() {
            map.insert("timestamp".to_owned(), json!(now.as_str()));
            let payload = map.entry("payload".to_owned()).or_insert_with(|| json!({}));
            if let Some(payload) = payload.as_object_mut() {
                payload.insert("id".to_owned(), json!(duplicate_id));
                payload.insert("session_id".to_owned(), json!(duplicate_id));
                payload.insert("timestamp".to_owned(), json!(now.as_str()));
            }
        }
        lines.push(serde_json::to_string(&object).unwrap_or_else(|_| line.to_owned()));
        rewritten_meta = true;
    }

    if !rewritten_meta {
        return Err(SessionError::DuplicateMissingMeta.into());
    }
    // 现网在末尾补一个换行，保持一致。
    Ok(format!("{}\n", lines.join("\n")))
}

/// 构造分支后的会话文本：保留选中消息所属整轮并重写编号。
///
/// 这是**不使用 app-server 时的回退路径**：官方服务不可用（未安装、版本过旧、
/// 启动超时）时，用文件截断得到一份语义等价的分支。官方路径仍优先，因为它能原子地
/// 完成截断并生成 Codex 原生可识别的分支。
pub fn build_branch_session_text(
    source_text: &str,
    branch_id: &str,
    now: &Timestamp,
    message_line: u64,
) -> Result<String> {
    let cutoff = branch_cutoff_line(source_text, message_line)?;
    let mut kept: Vec<&str> = Vec::new();
    for (index, raw_line) in source_text.split('\n').enumerate() {
        let line_number = index as u64 + 1;
        if line_number > cutoff {
            break;
        }
        kept.push(raw_line.trim_end_matches('\r'));
    }
    // 截断后的文本必然以换行结尾，否则 Codex 可能读不到最后一行。
    let truncated = format!("{}\n", kept.join("\n"));
    build_duplicate_session_text(&truncated, branch_id, now)
}

fn record_turn_id(value: &Value) -> Option<&str> {
    let kind = value.get("type")?.as_str()?;
    let payload = value.get("payload")?;
    if kind == "turn_context"
        || (kind == "event_msg" && payload.get("type")?.as_str()? == "task_started")
    {
        payload.get("turn_id")?.as_str().filter(|id| !id.is_empty())
    } else {
        None
    }
}

/// 老日志没有轮次编号时退回消息截断；有轮次编号时与原生 fork 保持一致。
fn branch_cutoff_line(source: &str, message_line: u64) -> Result<u64> {
    if message_line == 0 {
        return Err(SessionError::BranchNeedsContext.into());
    }
    let mut active_turn = None;
    let mut selected_turn = None;
    let mut selected = false;
    let mut cutoff = 0;
    for (index, line) in source.lines().enumerate() {
        let line_number = index as u64 + 1;
        let value = serde_json::from_str::<Value>(line).ok();
        if let Some(id) = value.as_ref().and_then(record_turn_id) {
            if selected && selected_turn.as_deref() != Some(id) {
                break;
            }
            active_turn = Some(id.to_owned());
        }
        if line_number == message_line {
            let message = value.as_ref().and_then(|value| {
                extract_message(
                    &value["payload"],
                    value["type"].as_str().unwrap_or_default(),
                )
            });
            if !message.is_some_and(|message| should_keep_message(&message.text)) {
                return Err(SessionError::BranchAnchorNotMessage.into());
            }
            selected = true;
            selected_turn.clone_from(&active_turn);
        }
        cutoff = line_number;
        if selected && selected_turn.is_none() {
            break;
        }
    }
    if !selected {
        return Err(SessionError::BranchAnchorStale.into());
    }
    Ok(cutoff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivc_core::ports::{BoxFuture, ConflictPolicy, LineSink, ReadStart, StoreKind};
    use aivc_core::session::{Message, PageCursor, Role};

    /// 一个只服务 `read_lines` 的假存储：内容按行提供，忽略起点参数。
    struct FakeStore {
        lines: Vec<String>,
    }

    impl SessionStore for FakeStore {
        fn kind(&self) -> StoreKind {
            StoreKind::Local
        }

        fn read_text<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<String>> {
            let text = self.lines.join("\n");
            Box::pin(async move { Ok(text) })
        }

        fn write_text<'a>(
            &'a self,
            _path: &'a str,
            _content: &'a str,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move { Ok(()) })
        }

        fn read_lines<'a>(
            &'a self,
            _path: &'a str,
            _start: ReadStart,
            _options: ReadOptions,
            mut sink: LineSink<'a>,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                for (index, line) in self.lines.iter().enumerate() {
                    if sink(line, index as u64 + 1, 0) == LineControl::Stop {
                        break;
                    }
                }
                Ok(())
            })
        }

        fn read_head<'a>(&'a self, _path: &'a str, _max: usize) -> BoxFuture<'a, Result<String>> {
            let text = self.lines.join("\n");
            Box::pin(async move { Ok(text) })
        }

        fn exists<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<bool>> {
            Box::pin(async move { Ok(true) })
        }

        fn size<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<u64>> {
            Box::pin(async move { Ok(0) })
        }

        fn move_file<'a>(
            &'a self,
            _from: &'a str,
            _to: &'a str,
            _policy: ConflictPolicy,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move { Ok(()) })
        }

        fn remove<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move { Ok(()) })
        }

        fn remove_tree<'a>(&'a self, _path: &'a str) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn store(lines: Vec<String>) -> FakeStore {
        FakeStore { lines }
    }

    fn task_started(turn_id: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:00.000Z","type":"event_msg","payload":{{"type":"task_started","turn_id":"{turn_id}"}}}}"#
        )
    }

    fn user(text: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:01.000Z","type":"event_msg","payload":{{"type":"user_message","message":"{text}"}}}}"#
        )
    }

    fn meta(id: &str) -> String {
        format!(
            r#"{{"timestamp":"2026-09-16T12:00:00.000Z","type":"session_meta","payload":{{"id":"{id}","session_id":"{id}","cwd":"/w"}}}}"#
        )
    }

    fn stamp() -> Timestamp {
        Timestamp::from_raw("2026-09-16T12:34:56.789Z")
    }

    fn session(id: &str, file_path: &str) -> Session {
        Session {
            id: SessionId::from(id),
            title: "t".to_owned(),
            source_title: None,
            cwd: None,
            created_at: None,
            updated_at: None,
            model: None,
            model_status: None,
            cli_version: None,
            file_path: file_path.to_owned(),
            file_mtime_ms: None,
            file_change_ms: None,
            file_size: None,
            message_count: 1,
            message_count_is_lower_bound: false,
            preview: vec![Message {
                role: Role::User,
                text: "hi".to_owned(),
                timestamp: None,
                source_line: Some(1),
            }],
            preview_offset: None,
            usage: None,
            metadata: None,
        }
    }

    fn wsl_layout() -> CodexLayout {
        CodexLayout::new("/home/a/.codex", StoreKind::Wsl)
    }

    // --- find_branch_turn_id ---

    #[tokio::test(flavor = "multi_thread")]
    async fn the_turn_id_is_the_last_task_started_before_the_message() {
        let store = store(vec![
            meta("s1"),
            task_started("turn-1"),
            user("第一条"),
            task_started("turn-2"),
            user("第二条"),
        ]);
        // 第 3 行属于 turn-1，第 5 行属于 turn-2。
        assert_eq!(
            find_branch_turn_id(&store, "/a.jsonl", 3)
                .await
                .expect("turn"),
            "turn-1"
        );
        assert_eq!(
            find_branch_turn_id(&store, "/a.jsonl", 5)
                .await
                .expect("turn"),
            "turn-2"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_message_before_any_task_started_is_stale() {
        // 该行之前没有轮次起点：拿不到轮次，必须拒绝而不是猜一个。
        let store = store(vec![meta("s1"), user("孤立消息")]);
        let error = find_branch_turn_id(&store, "/a.jsonl", 2)
            .await
            .expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "分支位置已失效，请重新打开会话详情后再试。"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_non_message_line_is_rejected() {
        let store = store(vec![meta("s1"), task_started("turn-1"), user("内容")]);
        // 第 1 行是 session_meta，不是可见消息。
        let error = find_branch_turn_id(&store, "/a.jsonl", 1)
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "分支位置不是可见会话消息。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_synthetic_block_is_not_a_valid_anchor() {
        let store = store(vec![
            meta("s1"),
            task_started("turn-1"),
            user("<environment_context>"),
        ]);
        let error = find_branch_turn_id(&store, "/a.jsonl", 3)
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "分支位置不是可见会话消息。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn line_zero_means_no_context_was_selected() {
        let store = store(vec![meta("s1")]);
        let error = find_branch_turn_id(&store, "/a.jsonl", 0)
            .await
            .expect_err("必须拒绝");
        assert_eq!(error.to_string(), "请选择至少一条前置上下文后再创建分支。");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn scanning_stops_at_the_target_line() {
        // 目标行之后即使还有内容也不应影响结果（提前停止）。
        let store = store(vec![
            meta("s1"),
            task_started("turn-1"),
            user("目标"),
            task_started("turn-2"),
            user("之后"),
        ]);
        assert_eq!(
            find_branch_turn_id(&store, "/a.jsonl", 3)
                .await
                .expect("turn"),
            "turn-1"
        );
    }

    // --- canonicalize_forked_session ---

    #[test]
    fn canonicalize_takes_the_thread_id_from_the_app_server() {
        // 关键：JSONL 里可能仍写着父线程 ID，必须以 app-server 的返回为准。
        let layout = wsl_layout();
        let parent = session(
            "parent",
            "/home/a/.codex/sessions/2026/09/16/rollout-p.jsonl",
        );
        let parsed = session(
            "parent",
            "/home/a/.codex/sessions/2026/09/16/rollout-c.jsonl",
        );
        let fork = ForkResult {
            thread_id: "child".to_owned(),
            file_path: "/home/a/.codex/sessions/2026/09/16/rollout-c.jsonl".to_owned(),
        };
        let result = canonicalize_forked_session(&layout, &parent, parsed, &fork).expect("fork");
        assert_eq!(
            result.id.as_str(),
            "child",
            "必须以 app-server 返回的编号为准"
        );
        assert_eq!(
            result.file_path,
            "/home/a/.codex/sessions/2026/09/16/rollout-c.jsonl"
        );
    }

    #[test]
    fn canonicalize_rejects_an_empty_or_self_thread_id() {
        let layout = wsl_layout();
        let parent = session(
            "parent",
            "/home/a/.codex/sessions/2026/09/16/rollout-p.jsonl",
        );
        let parsed = session("x", "/home/a/.codex/sessions/2026/09/16/rollout-c.jsonl");

        for thread_id in ["", "parent"] {
            let fork = ForkResult {
                thread_id: thread_id.to_owned(),
                file_path: "/home/a/.codex/sessions/2026/09/16/rollout-c.jsonl".to_owned(),
            };
            let error = canonicalize_forked_session(&layout, &parent, parsed.clone(), &fork)
                .expect_err("必须拒绝");
            assert_eq!(
                error.to_string(),
                "Codex 返回的分支会话编号无效，已停止打开分支。"
            );
        }
    }

    #[test]
    fn canonicalize_rejects_a_path_outside_the_sessions_root() {
        let layout = wsl_layout();
        let parent = session(
            "parent",
            "/home/a/.codex/sessions/2026/09/16/rollout-p.jsonl",
        );
        let parsed = session("x", "/etc/passwd");
        let fork = ForkResult {
            thread_id: "child".to_owned(),
            file_path: "/etc/passwd".to_owned(),
        };
        let error =
            canonicalize_forked_session(&layout, &parent, parsed, &fork).expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "Codex 返回的分支文件不在当前会话目录内，已停止打开分支。"
        );
    }

    #[test]
    fn canonicalize_rejects_the_parent_file_itself() {
        let layout = wsl_layout();
        let path = "/home/a/.codex/sessions/2026/09/16/rollout-p.jsonl";
        let parent = session("parent", path);
        let parsed = session("x", path);
        let fork = ForkResult {
            thread_id: "child".to_owned(),
            file_path: path.to_owned(),
        };
        let error =
            canonicalize_forked_session(&layout, &parent, parsed, &fork).expect_err("必须拒绝");
        assert_eq!(
            error.to_string(),
            "Codex 返回的分支文件与原会话相同，已停止打开分支。"
        );
    }

    #[test]
    fn same_path_normalizes_separators_and_dot_segments() {
        let wsl = wsl_layout();
        assert!(same_path("/a/b/c.jsonl", "/a/b/./c.jsonl", &wsl));
        assert!(!same_path("/a/b/c.jsonl", "/a/b/d.jsonl", &wsl));

        let local = CodexLayout::new("C:\\home\\.codex", StoreKind::Local);
        assert!(same_path(
            "C:\\home\\.codex\\sessions\\a.jsonl",
            "C:/home/.codex/sessions/a.jsonl",
            &local
        ));
    }

    // --- build_duplicate_session_text ---

    #[test]
    fn duplication_rewrites_only_the_session_meta_line() {
        let source = [meta("old"), user("内容一"), user("内容二")].join("\n");
        let text = build_duplicate_session_text(&source, "new-id", &stamp()).expect("duplicate");
        let lines: Vec<&str> = text.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 3);

        let first: Value = serde_json::from_str(lines[0]).expect("meta");
        assert_eq!(first["payload"]["id"], "new-id");
        // 不同 Codex 版本读不同字段，两个都要改。
        assert_eq!(first["payload"]["session_id"], "new-id");
        assert_eq!(first["timestamp"], "2026-09-16T12:34:56.789Z");
        assert_eq!(first["payload"]["timestamp"], "2026-09-16T12:34:56.789Z");
        // 其他字段保留。
        assert_eq!(first["payload"]["cwd"], "/w");

        // 其余行原样保留。
        assert_eq!(lines[1], user("内容一"));
        assert_eq!(lines[2], user("内容二"));
    }

    #[test]
    fn duplication_preserves_unparseable_lines() {
        // 复制应当尽可能忠实，而不是顺手丢掉解析不了的行。
        let source = [meta("old"), "not json".to_owned(), user("内容")].join("\n");
        let text = build_duplicate_session_text(&source, "new-id", &stamp()).expect("duplicate");
        assert!(text.contains("not json"));
    }

    #[test]
    fn duplication_without_session_meta_fails() {
        let source = [user("内容一"), user("内容二")].join("\n");
        let error =
            build_duplicate_session_text(&source, "new-id", &stamp()).expect_err("必须失败");
        assert_eq!(error.to_string(), "复制会话失败：缺少 session_meta 记录。");
    }

    #[test]
    fn duplication_ends_with_a_newline() {
        let source = meta("old");
        let text = build_duplicate_session_text(&source, "new-id", &stamp()).expect("duplicate");
        assert!(
            text.ends_with('\n'),
            "缺少结尾换行会让 Codex 读不到最后一行"
        );
    }

    // --- build_branch_session_text ---

    #[test]
    fn branching_keeps_content_up_to_the_anchor_line() {
        let source = [meta("old"), user("第一条"), user("第二条"), user("第三条")].join("\n");
        let text = build_branch_session_text(&source, "branch-id", &stamp(), 3).expect("branch");
        let lines: Vec<&str> = text.trim_end().split('\n').collect();
        assert_eq!(lines.len(), 3, "第 3 行之后的内容必须被截掉");
        assert!(lines[1].contains("第一条"));
        assert!(lines[2].contains("第二条"));
        assert!(!text.contains("第三条"));

        let first: Value = serde_json::from_str(lines[0]).expect("meta");
        assert_eq!(first["payload"]["id"], "branch-id");
    }

    #[test]
    fn branching_needs_at_least_one_context_line() {
        let source = meta("old");
        let error = build_branch_session_text(&source, "b", &stamp(), 0).expect_err("必须拒绝");
        assert_eq!(error.to_string(), "请选择至少一条前置上下文后再创建分支。");
    }

    #[test]
    fn branching_beyond_the_file_rejects_a_stale_anchor() {
        let source = [meta("old"), user("唯一一条")].join("\n");
        assert!(build_branch_session_text(&source, "b", &stamp(), 99).is_err());
    }

    #[test]
    fn file_fallback_keeps_the_selected_turn_and_stops_before_the_next_context() {
        let source = [
            meta("old"),
            task_started("t1"),
            user("选中"),
            r#"{"type":"event_msg","payload":{"type":"agent_message","message":"本轮后续"}}"#
                .into(),
            r#"{"type":"event_msg","payload":{"type":"task_complete","turn_id":"t1"}}"#.into(),
            r#"{"type":"turn_context","payload":{"turn_id":"t2"}}"#.into(),
            task_started("t2"),
            user("下一轮"),
        ]
        .join("\n");
        let text = build_branch_session_text(&source, "b", &stamp(), 3).unwrap();
        assert!(text.contains("本轮后续"));
        assert!(text.contains("task_complete"));
        assert!(!text.contains("t2"));
        assert!(!text.contains("下一轮"));
    }

    #[test]
    fn file_fallback_rejects_metadata_as_a_branch_anchor() {
        let source = [meta("old"), user("消息")].join("\n");
        assert!(build_branch_session_text(&source, "b", &stamp(), 1).is_err());
    }

    #[test]
    fn cursor_type_is_reexported_for_callers() {
        // 分页游标类型在本模块的测试里被间接使用；这里显式引用一次，
        // 避免 `PageCursor` 的导入在无分支构建下被误判为未使用。
        assert!(PageCursor::Latest.is_latest());
    }
}
