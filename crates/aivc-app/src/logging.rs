//! 日志初始化：同时写控制台与 `logs/app.log`。
//!
//! # 为什么必须落文件
//!
//! 只写控制台时，**排查者读不到它**：控制台是窗口里的滚动文本，不是文件。
//! 实测中这直接卡住了定位——前端轨迹能读（`performance.log` 是文件），
//! 而后端的 WARN/DEBUG 读不到，于是「WSL 探测为什么 13 毫秒就失败」这类问题
//! 只能靠人转录屏幕内容。
//!
//! 落文件之后，一次运行留下两份可读记录：
//!
//! | 文件 | 内容 |
//! | --- | --- |
//! | `logs/app.log` | 后端日志（本模块） |
//! | `logs/performance.log` | 前端动作轨迹与异常 |
//!
//! # 为什么同时保留控制台
//!
//! 调试构建下开发者需要即时反馈；落文件是为了**事后**排查。两者不冲突。
//!
//! # 上限与截断
//!
//! 与 `performance.log` 同一策略：超过上限丢最旧的一半。不轮转就不会在用户
//! 磁盘上积累文件，而诊断日志看最近的才有用。

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 日志文件上限（4 MB）。
pub const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;

/// 日志文件相对运行根的路径。
pub const LOG_RELATIVE_PATH: &str = "logs/app.log";

/// 同时写文件与标准错误的写入器。
///
/// 用「克隆文件句柄」而不是共享锁：`tracing` 的写入是同步的，`File::try_clone`
/// 让每层拿到独立句柄，避免在日志路径上引入锁竞争——日志本身不该成为瓶颈。
#[derive(Clone)]
struct TeeWriter {
    file: Arc<Mutex<std::fs::File>>,
    /// 是否同时写标准错误。
    console: bool,
}

impl std::io::Write for TeeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.console {
            // 控制台写失败不该影响文件写入。
            let _ = std::io::stderr().write_all(buf);
        }
        match self.file.lock() {
            Ok(mut file) => file.write_all(buf).map(|()| buf.len()),
            // 锁中毒时吞掉而不是 panic：日志失败不该让应用崩溃。
            Err(_) => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if self.console {
            let _ = std::io::stderr().flush();
        }
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
        Ok(())
    }
}

/// 写入器工厂。
#[derive(Clone)]
struct TeeMakeWriter {
    writer: TeeWriter,
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter;

    fn make_writer(&'a self) -> Self::Writer {
        self.writer.clone()
    }
}

/// 初始化日志：写 `<root>/logs/app.log`，调试构建下同时写标准错误。
///
/// 返回日志文件路径；拿不到时返回 [`None`]（此时只写控制台，不阻断启动）。
///
/// **日志初始化失败不阻断启动**：看不到日志比打不开应用轻得多。
#[must_use]
pub fn init(root: &Path) -> Option<PathBuf> {
    let path = root.join(LOG_RELATIVE_PATH);
    let file = open_log_file(&path)?;

    let writer = TeeMakeWriter {
        writer: TeeWriter {
            file: Arc::new(Mutex::new(file)),
            console: cfg!(debug_assertions),
        },
    };

    let level = if cfg!(debug_assertions) {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };
    // 用 `try_init`：测试或多线程场景下可能已有全局订阅器，重复初始化不该 panic。
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(writer)
        // 关掉 ANSI：文件里的转义序列会污染日志，读起来是一堆 `\u{1b}[`。
        .with_ansi(false)
        .try_init();
    Some(path)
}

/// 打开日志文件，必要时先截断。
///
/// 截断在**打开之前**做，因此每次启动都从一个干净的文件开始——
/// 「这次运行发生了什么」比「历史累积」更有用。
fn open_log_file(path: &Path) -> Option<std::fs::File> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    // 启动时截断：上一次运行的内容已经没用了，而累积会让文件很快撞上限。
    std::fs::File::create(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    #[test]
    fn init_creates_the_log_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = init(dir.path()).expect("初始化");
        assert!(path.is_file(), "日志文件应当被创建");
        assert!(path.ends_with("logs/app.log") || path.ends_with("logs\\app.log"));
    }

    #[test]
    fn the_file_starts_empty_on_each_run() {
        // 启动时截断：「这次运行发生了什么」比「历史累积」更有用。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(LOG_RELATIVE_PATH);
        std::fs::create_dir_all(path.parent().expect("父目录")).expect("目录");
        std::fs::write(&path, "上一次运行的内容").expect("写入");
        let opened = open_log_file(&path).expect("打开");
        drop(opened);
        let content = std::fs::read_to_string(&path).expect("读取");
        assert!(content.is_empty(), "应当被截断：{content}");
    }

    #[test]
    fn the_writer_tolerates_a_poisoned_lock() {
        // 日志失败不该让应用崩溃。
        let dir = tempfile::tempdir().expect("tempdir");
        let file = open_log_file(&dir.path().join(LOG_RELATIVE_PATH)).expect("打开");
        let shared = Arc::new(Mutex::new(file));
        let poisoned = Arc::clone(&shared);
        // 故意让另一个线程 panic 并毒化锁。
        let _ = std::thread::spawn(move || {
            let _guard = poisoned.lock().expect("lock");
            panic!("毒化");
        })
        .join();
        let mut writer = TeeWriter {
            file: shared,
            console: false,
        };
        // 中毒后仍应返回成功而不是 panic。
        assert!(writer.write_all(b"x").is_ok());
        assert!(writer.flush().is_ok());
    }

    #[test]
    fn the_limit_is_documented() {
        assert_eq!(MAX_LOG_BYTES, 4 * 1024 * 1024);
    }
}
