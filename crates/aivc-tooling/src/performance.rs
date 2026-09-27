//! 启动与前端埋点日志：`performance.log`。
//!
//! # 为什么它比「性能分析」更重要
//!
//! 这个文件的第一价值不是性能数据，而是**前端异常的唯一去处**。
//! 渲染层的 JS 抛错在 Rust 侧完全不可见——没有它，「界面静默失效」就是一个黑盒。
//! 实测中确实遇到过：前端启动后只发了两条命令就停了，日志里一行错误都没有。
//!
//! # 2 MB 上限与「截断而不是轮转」
//!
//! 与现网一致：超过上限后**丢掉最旧的一半**。不做多文件轮转是因为这是诊断日志，
//! 用户不会去翻历史；保留最近的内容才有用。而且不做轮转意味着不需要清理逻辑，
//! 不会在用户磁盘上积累文件。
//!
//! # 一行一个 JSON
//!
//! 追加而不是重写：崩溃时已写入的行不会丢。反过来，若每次都重写整个文件，
//! 一次写入失败就会把历史也毁掉。

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

/// 日志文件上限（2 MB）。
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;

/// 一条埋点记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceEntry {
    /// 时间戳（ISO 8601）。
    pub at: String,
    /// 标签。前端异常会带上 `frontend.error` / `frontend.rejection` 前缀。
    pub label: String,
    /// 耗时（毫秒，已取整）。
    pub duration_ms: i64,
    /// 状态。
    pub status: String,
}

impl PerformanceEntry {
    /// 序列化成一行 JSON。
    ///
    /// 手写而不是用 `serde_json` 的结构体派生：字段只有四个，且**字段顺序固定**
    /// 让日志肉眼可读；序列化结构体会因版本变化调整顺序。
    #[must_use]
    pub fn to_line(&self) -> String {
        format!(
            "{{\"at\":{},\"label\":{},\"durationMs\":{},\"status\":{}}}\n",
            quote(&self.at),
            quote(&self.label),
            self.duration_ms,
            quote(&self.status)
        )
    }
}

/// 把字符串包成 JSON 字面量。
///
/// 标签来自前端，可能含引号、换行、控制字符——不转义就会**破坏整个文件的结构**，
/// 让日志再也解析不了。
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // 其余控制字符用 \u 转义；JSON 不允许裸控制字符。
            other if other.is_control() => {
                // 用 `write!` 直接写入：`push_str(&format!(..))` 会多分配一个临时字符串。
                let _ = write!(out, "\\u{:04x}", other as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// 追加一条记录。
///
/// # 为什么「先截断再追加」
///
/// 先写会导致文件短暂超过上限；但**先检查再写**会产生另一个问题：并发写入时
/// 两条记录都在检查后写入，实际仍会超出。这个函数不做并发控制——调用方是单线程的
/// 命令处理路径；越界几十字节对诊断日志没有影响。
///
/// # Errors
///
/// 目录创建或写入失败时返回错误。
pub fn append_line(path: &Path, line: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建日志目录失败：{error}"))?;
    }
    // 超限时丢掉最旧的一半：保留最近的内容才有诊断价值。
    if let Ok(metadata) = std::fs::metadata(path)
        && metadata.len() > MAX_LOG_BYTES
        && let Ok(content) = std::fs::read_to_string(path)
    {
        let keep_from = content.len() / 2;
        // 从半途开始切会切断一行；往后找到第一个换行，保证留下的是完整的行。
        let tail = content
            .get(keep_from..)
            .and_then(|rest| rest.find('\n').map(|index| &rest[index + 1..]))
            .unwrap_or("");
        let _ = std::fs::write(path, tail);
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("打开日志文件失败：{error}"))?;
    file.write_all(line.as_bytes())
        .map_err(|error| format!("写入日志失败：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(label: &str) -> PerformanceEntry {
        PerformanceEntry {
            at: "2026-09-25T10:00:00.000Z".to_owned(),
            label: label.to_owned(),
            duration_ms: 12,
            status: "ok".to_owned(),
        }
    }

    #[test]
    fn an_entry_serializes_to_one_line() {
        let line = entry("app.mounted").to_line();
        assert_eq!(
            line,
            "{\"at\":\"2026-09-25T10:00:00.000Z\",\"label\":\"app.mounted\",\"durationMs\":12,\"status\":\"ok\"}\n"
        );
        assert_eq!(line.matches('\n').count(), 1, "一行只能有一个换行");
    }

    #[test]
    fn quotes_and_control_characters_are_escaped() {
        // 标签来自前端，不转义就会破坏整个文件的结构，让日志再也解析不了。
        let line = PerformanceEntry {
            at: "t".to_owned(),
            label: "带\"引号\"和\n换行".to_owned(),
            duration_ms: 0,
            status: "e".to_owned(),
        }
        .to_line();
        assert!(line.contains("\\\""), "{line}");
        assert!(line.contains("\\n"), "{line}");
        // 除去结尾那一个，行内不该再有裸换行。
        assert_eq!(
            line.trim_end_matches('\n').matches('\n').count(),
            0,
            "{line}"
        );
        // 反斜杠也要转义，否则 `\n` 与真实换行无法区分。
        let line = PerformanceEntry {
            at: "t".to_owned(),
            label: "反\\斜杠".to_owned(),
            duration_ms: 0,
            status: "e".to_owned(),
        }
        .to_line();
        assert!(line.contains("\\\\"), "{line}");
    }

    #[test]
    fn lines_are_appended_not_rewritten() {
        // 崩溃时已写入的行不会丢。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("performance.log");
        append_line(&path, &entry("first").to_line()).expect("写入");
        append_line(&path, &entry("second").to_line()).expect("写入");
        let content = std::fs::read_to_string(&path).expect("读取");
        assert!(content.contains("first"));
        assert!(content.contains("second"), "追加不该覆盖已有内容");
        assert_eq!(content.lines().count(), 2);
    }

    #[test]
    fn the_directory_is_created() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir
            .path()
            .join("logs")
            .join("nested")
            .join("performance.log");
        append_line(&path, &entry("x").to_line()).expect("写入");
        assert!(path.is_file());
    }

    #[test]
    fn an_oversized_file_keeps_the_recent_half() {
        // 保留最近的内容才有诊断价值。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("performance.log");
        let filler = "x".repeat(1024);
        let mut content = String::new();
        while (content.len() as u64) < MAX_LOG_BYTES + 4096 {
            let _ = writeln!(content, "{{\"label\":\"{filler}\"}}");
        }
        std::fs::write(&path, &content).expect("写入");
        append_line(&path, &entry("after-trim").to_line()).expect("写入");

        let after = std::fs::read_to_string(&path).expect("读取");
        assert!(
            (after.len() as u64) < MAX_LOG_BYTES,
            "截断后应当低于上限：{}",
            after.len()
        );
        assert!(after.contains("after-trim"), "新记录必须在");
        // 截断必须落在行边界上，否则留下半行 JSON。
        for line in after.lines() {
            assert!(line.starts_with('{'), "有半行残留：{line:.40}");
        }
    }

    #[test]
    fn the_limit_matches_the_desktop_reference() {
        assert_eq!(MAX_LOG_BYTES, 2 * 1024 * 1024);
    }
}
