//! 端口（ports）：领域层对外部能力的抽象。
//!
//! # 为什么用 `BoxFuture` 而不是 `async-trait`
//!
//! 这些 trait 必须对象安全（`Box<dyn SessionStore>`），而原生 `async fn in trait`
//! 目前不是 dyn 兼容的。可选方案是用 `async-trait` 过程宏，但那会给「零依赖」的
//! `aivc-core` 引入一个宏依赖，且它生成的代码同样是每次调用一次装箱。
//! 这里直接写 `BoxFuture`：等价开销、无额外依赖、且签名显式可见。
//!
//! 调用频率低（全是 I/O 边界），装箱成本可忽略。
//!
//! # 范围
//!
//! 本模块只定义**与存储结构无关**的稳定端口。仓储（Repository）端口依赖数据库
//! schema，在 P1 设计 schema 时一并定义，避免凭空猜测接口。

use std::future::Future;
use std::pin::Pin;

use crate::error::Result;

/// 装箱的 Send future，用于对象安全的异步 trait 方法。
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 逐行读取回调的返回值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineControl {
    /// 继续读取下一行。
    Continue,
    /// 已取到需要的内容，提前停止（不读完整个文件）。
    Stop,
}

/// 逐行读取的回调。参数为（行内容，1 起的行号，该行在文件中的起始字节偏移）。
///
/// # 为什么回调必须收到字节偏移
///
/// 分页锚点要能被 `seek` 直接定位，就必须记录**绝对**字节偏移。行内容与行号都无法
/// 推导出它：行内容不含自身位置，而行号与字节数的换算需要知道每一行的长度——那正是
/// 逐行读取时才知道的信息。因此只有读取方能在回调里给出它。
///
/// 返回 [`LineControl::Stop`] 可提前停止读取。
pub type LineSink<'a> = Box<dyn FnMut(&str, u64, u64) -> LineControl + Send + 'a>;

/// 逐行读取的起点。
///
/// 现网只有「从第 N 行开始」这一种，而 `readLines` 即使已知目标行号也必须从文件头
/// 逐行读过去，因此大 JSONL 的分页定位实际是线性的。消息索引记录**字节偏移**后，
/// 本地文件可 `seek` 直接定位（真 O(1)）；WSL 侧与首次解析仍走行号。
///
/// # 为什么字节起点还要带行号
///
/// 行号**无法**从字节偏移推导，而详情返回的 `source_line` 必须是原始 JSONL 的绝对
/// 行号——「从此处分支」依赖它定位轮次。若从字节偏移起读后把行号按相对值计数，
/// 分页越靠后行号偏移越大，分支位置就会漂移。因此两者必须同时给出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStart {
    /// 从第 N 行开始（1 起）。行号未知时的默认起点，也是 WSL 后端唯一支持的方式。
    Line(u64),
    /// 从第 N 字节开始，且该处是绝对第 M 行的起点。
    Byte {
        /// 字节偏移。
        offset: u64,
        /// 该偏移处的绝对行号（1 起）。
        line_number: u64,
    },
}

impl ReadStart {
    /// 文件起始位置。
    pub const BEGINNING: Self = Self::Line(1);

    /// 起点对应的字节偏移（仅字节起点有值）。
    #[must_use]
    pub const fn byte_offset(self) -> Option<u64> {
        match self {
            Self::Byte { offset, .. } => Some(offset),
            Self::Line(_) => None,
        }
    }

    /// 起点对应的绝对行号。
    #[must_use]
    pub const fn line_number(self) -> u64 {
        match self {
            Self::Line(line) => line,
            Self::Byte { line_number, .. } => line_number,
        }
    }
}

/// 逐行读取的可选参数。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadOptions {
    /// 读取超时（毫秒）。WSL 目标的详情读取按文件体积动态放宽，因此需要可覆盖。
    pub timeout_ms: Option<u64>,
}

impl ReadOptions {
    /// 无超时限制（本机读取）。
    #[must_use]
    pub const fn none() -> Self {
        Self { timeout_ms: None }
    }

    /// 指定超时。
    #[must_use]
    pub const fn with_timeout(timeout_ms: u64) -> Self {
        Self {
            timeout_ms: Some(timeout_ms),
        }
    }
}

/// 移动文件时的冲突策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictPolicy {
    /// 目标已存在时直接失败（用于「恢复」：原位置已有同名文件必须拒绝）。
    FailIfExists,
    /// 覆盖目标（用于「删除到回收站」等由调用方保证安全的场景）。
    Overwrite,
}

/// 会话文件存储抽象。
///
/// 对应现网 `createSessionStorage({ kind, distro })`：本机走 `std::fs` / `tokio::fs`，
/// WSL 走持久 worker。**目录布局、文件格式解析与分支截断规则不属于本层**，仍由各
/// Provider 决定——现网刻意不做统一基类，因为 Codex/Claude/Gemini/Qoder 的存储布局
/// 是真实差异，抽象只会增加条件分支。
pub trait SessionStore: Send + Sync {
    /// 该存储面向本机还是 WSL 发行版（决定路径语义：`/` 与 `\`、大小写敏感性）。
    fn kind(&self) -> StoreKind;

    /// 读取整个文本文件。
    fn read_text<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<String>>;

    /// 写入整个文本文件，自动创建父目录。
    fn write_text<'a>(&'a self, path: &'a str, content: &'a str) -> BoxFuture<'a, Result<()>>;

    /// 从 `start` 起逐行读取，回调返回 [`LineControl::Stop`] 时提前结束。
    ///
    /// 这是流式分页与分支行扫描的公共底座：必须支持**不读完整个文件**，
    /// 否则大 JSONL 会被整体载入内存。
    fn read_lines<'a>(
        &'a self,
        path: &'a str,
        start: ReadStart,
        options: ReadOptions,
        sink: LineSink<'a>,
    ) -> BoxFuture<'a, Result<()>>;

    /// 读取文件头部至多 `max_bytes` 字节（列表轻量解析用）。
    fn read_head<'a>(&'a self, path: &'a str, max_bytes: usize) -> BoxFuture<'a, Result<String>>;

    /// 批量读取多个文件的头部。
    ///
    /// 返回值与 `paths` **一一对应**：读不到的位置是 [`None`]，向量长度绝不缩短。
    /// 调用方按下标把内容与文件元数据（mtime / size）对齐，一旦错位就会让会话张冠李戴，
    /// 因此这里宁可返回 `None` 也不跳过。
    ///
    /// 默认实现逐个调用 [`Self::read_head`]——对本机存储这就足够了。
    ///
    /// # WSL 存储**必须**覆盖它
    ///
    /// 默认实现在 WSL 上会**为每个文件启动一个 `wsl.exe`**，而列表是 6 路并发拉起的。
    /// WSL2 在空闲后会自动关闭发行版，于是这 6 个进程**同时抢同一次冷启动**。实测数据：
    ///
    /// | 场景 | 耗时 |
    /// | --- | --- |
    /// | 发行版 Stopped 时首次调用 | **5.3 s** |
    /// | 紧接着调用 | 0.09 s |
    /// | 空闲 20 秒后 | **6.9 s**（又被关掉了） |
    ///
    /// 应用日志里因此出现过 `head` 之间 **29.99 秒**的空白——30 个文件、6 路并发、
    /// 反复抢冷启动。覆盖成**一次调用**能把它压成「一次冷启动 + 一次脚本执行」。
    fn read_heads<'a>(
        &'a self,
        paths: &'a [String],
        max_bytes: usize,
    ) -> BoxFuture<'a, Vec<Option<String>>> {
        Box::pin(async move {
            let mut heads = Vec::with_capacity(paths.len());
            for path in paths {
                heads.push(self.read_head(path, max_bytes).await.ok());
            }
            heads
        })
    }

    /// 文件是否存在。
    fn exists<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<bool>>;

    /// 文件大小（字节）。
    fn size<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<u64>>;

    /// 移动文件。
    fn move_file<'a>(
        &'a self,
        from: &'a str,
        to: &'a str,
        policy: ConflictPolicy,
    ) -> BoxFuture<'a, Result<()>>;

    /// 删除文件。
    fn remove<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>>;

    /// 递归删除文件或目录树。
    ///
    /// 与 [`SessionStore::remove`] 的区别：某些平台的一份会话由**多个产物**组成，
    /// 其中包含目录（如 Qoder 的 `tasks/<编号>/`）。只删文件会留下孤儿目录，
    /// 而 `rm -f` 对目录无效。
    ///
    /// 目标不存在时视为成功——删除是幂等的。
    fn remove_tree<'a>(&'a self, path: &'a str) -> BoxFuture<'a, Result<()>>;
}

/// 存储后端类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// 本机文件系统。
    Local,
    /// WSL 发行版内。
    Wsl,
}

impl StoreKind {
    /// 是否为 WSL 后端。
    #[must_use]
    pub const fn is_wsl(self) -> bool {
        matches!(self, Self::Wsl)
    }
}

/// 时钟端口。抽出来是为了让「时间相关」的逻辑可在测试中确定性验证。
pub trait Clock: Send + Sync {
    /// 当前时间。
    fn now(&self) -> crate::time::Timestamp;

    /// 当前 Unix 毫秒。
    fn now_unix_millis(&self) -> i64;
}

/// 使用系统时钟的默认实现。
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    /// 由 [`Clock::now_unix_millis`] 派生，保证两个方法使用**同一个原语**。
    ///
    /// 反过来实现（`now_unix_millis` 调用 `now`）会让两者各自读一次系统时钟，
    /// 在毫秒边界上产生不一致；虽然调用方通常只用一个，但保持单一原语能消除这类
    /// 隐蔽的时间倒流。
    fn now(&self) -> crate::time::Timestamp {
        crate::time::Timestamp::from_unix_millis(self.now_unix_millis())
    }

    fn now_unix_millis(&self) -> i64 {
        crate::time::Timestamp::now().to_unix_millis().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_options_default_is_unbounded() {
        assert_eq!(ReadOptions::default(), ReadOptions::none());
        assert_eq!(ReadOptions::with_timeout(60_000).timeout_ms, Some(60_000));
    }

    #[test]
    fn store_kind_reports_wsl() {
        assert!(StoreKind::Wsl.is_wsl());
        assert!(!StoreKind::Local.is_wsl());
    }

    #[test]
    fn system_clock_returns_a_parseable_recent_timestamp() {
        let clock = SystemClock;
        let now = clock.now();
        let millis = now
            .to_unix_millis()
            .expect("system clock must be parseable");
        // `now()` 与 `now_unix_millis()` 是两次独立的系统时钟读取，可能落在不同毫秒，
        // 因此只断言量级一致（并允许极小的时间差）。
        let direct = clock.now_unix_millis();
        assert!(
            (direct - millis).abs() < 1_000,
            "两个时钟原语差异过大：{millis} vs {direct}"
        );
        // 2020-01-01 之后、2100 年之前，确保没有把秒当成毫秒之类的量纲错误。
        assert!(
            millis > 1_577_836_800_000,
            "unexpected clock value: {millis}"
        );
        assert!(
            millis < 4_102_444_800_000,
            "unexpected clock value: {millis}"
        );
    }

    #[test]
    fn clock_now_is_derived_from_the_same_primitive() {
        // `now()` 必须由 `now_unix_millis()` 派生，避免两个方法各自读表导致时间倒流。
        let clock = SystemClock;
        let millis = clock.now_unix_millis();
        let round_tripped = clock
            .now()
            .to_unix_millis()
            .expect("system clock must be parseable");
        // 两者都在同一毫秒或相邻毫秒内。
        assert!(
            (round_tripped - millis).abs() <= 1,
            "now() 与 now_unix_millis() 不一致：{millis} vs {round_tripped}"
        );
    }

    #[test]
    fn read_start_defaults_to_the_beginning_of_the_file() {
        assert_eq!(ReadStart::BEGINNING, ReadStart::Line(1));
        assert_eq!(ReadStart::BEGINNING.line_number(), 1);
        assert_eq!(ReadStart::BEGINNING.byte_offset(), None);
    }

    #[test]
    fn read_start_always_reports_an_absolute_line_number() {
        // 行号是消息身份，两种起点都必须能给出。
        let line = ReadStart::Line(42);
        assert_eq!(line.line_number(), 42);
        assert_eq!(line.byte_offset(), None);

        let byte = ReadStart::Byte {
            offset: 4096,
            line_number: 42,
        };
        assert_eq!(byte.byte_offset(), Some(4096));
        assert_eq!(byte.line_number(), 42, "字节起点也必须携带绝对行号");

        // 同一个字节偏移配不同行号是不同起点——这正是「不能只存偏移」的原因。
        assert_ne!(
            ReadStart::Byte {
                offset: 100,
                line_number: 1
            },
            ReadStart::Byte {
                offset: 100,
                line_number: 2
            }
        );
    }
}
