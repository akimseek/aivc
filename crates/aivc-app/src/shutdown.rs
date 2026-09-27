//! 退出清理协调器。
//!
//! # 现网的三条语义，逐条保留
//!
//! 1. **顺序固定**：停止全部 PTY → 停止 Gateway → flush 日志。
//!    顺序不能颠倒——先关网关会让在途请求失败，而在途请求的日志还没写完。
//! 2. **幂等**：`before-quit` 可能触发多次（用户重复点关闭、系统广播）。
//!    清理只能跑一遍，重复调用要等第一次的结果而不是再跑一遍。
//! 3. **非关键步骤失败不阻止退出**：flush 日志失败只是丢最后几行诊断，
//!    不该让用户卡在一个关不掉的窗口上。反过来，**终止 Gateway 失败也必须继续**
//!    ——它已经尽力了，剩下的资源由进程退出回收。
//!
//! # 为什么需要 `complete` 标志
//!
//! 现网用 `shutdownComplete` 区分「清理已跑完」与「正在跑」，否则 `quit` 会再次触发
//! `before-quit`，形成「清理 → quit → 清理」的循环。这里用状态机显式表达这件事。

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

use tokio::sync::OnceCell;

/// 一步清理动作。
///
/// 用 boxed future 而不是 `async-trait`：这些动作在 trait 对象里只被调用一次，
/// 引入宏依赖不值得。
pub type CleanupStep = Pin<Box<dyn Future<Output = StepOutcome> + Send>>;

/// 一步清理的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// 完成。
    Done,
    /// 失败但可以继续。
    Failed(String),
    /// 未接线的占位步骤（例如 PTY 尚未接入）。
    Skipped(&'static str),
}

/// 清理进度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownState {
    /// 尚未开始。
    Idle,
    /// 已完成。
    Complete,
}

/// 一步清理的执行记录（供诊断与测试断言）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRecord {
    /// 步骤名。
    pub name: &'static str,
    /// 结果。
    pub outcome: StepOutcome,
}

/// 退出清理协调器。
#[derive(Debug)]
pub struct ShutdownCoordinator {
    records: OnceCell<Vec<StepRecord>>,
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

impl ShutdownCoordinator {
    /// 构造。
    #[must_use]
    pub const fn new() -> Self {
        Self {
            records: OnceCell::const_new(),
        }
    }

    /// 按顺序执行清理步骤，幂等。
    ///
    /// 重复或并发调用都返回**第一次**的执行记录，步骤只跑一遍。
    pub async fn run(&self, steps: Vec<(&'static str, CleanupStep)>) -> Vec<StepRecord> {
        self.records
            .get_or_init(|| async move {
                let mut records = Vec::with_capacity(steps.len());
                for (name, step) in steps {
                    let outcome = step.await;
                    if let StepOutcome::Failed(ref message) = outcome {
                        // 失败不中断：每一步都尽力，剩下的由进程退出回收。
                        tracing::warn!(step = name, %message, "退出清理步骤失败，继续");
                    }
                    records.push(StepRecord { name, outcome });
                }
                records
            })
            .await
            .clone()
    }

    /// 当前状态。
    #[must_use]
    pub fn state(&self) -> ShutdownState {
        if self.records.get().is_some() {
            ShutdownState::Complete
        } else {
            ShutdownState::Idle
        }
    }

    /// 已完成时的执行记录。
    #[must_use]
    pub fn records(&self) -> Option<&Vec<StepRecord>> {
        self.records.get()
    }
    /// 是否已经跑过清理。
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.records.get().is_some()
    }
}

/// 把同步动作包成清理步骤。
pub fn sync_step<F>(action: F) -> CleanupStep
where
    F: FnOnce() -> Result<(), String> + Send + 'static,
{
    Box::pin(async move {
        match action() {
            Ok(()) => StepOutcome::Done,
            Err(message) => StepOutcome::Failed(message),
        }
    })
}

/// 把一个 async 动作包成清理步骤。
pub fn async_step<F, Fut>(action: F) -> CleanupStep
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), String>> + Send + 'static,
{
    Box::pin(async move {
        match action().await {
            Ok(()) => StepOutcome::Done,
            Err(message) => StepOutcome::Failed(message),
        }
    })
}

/// 尚未接线的步骤。
pub fn skipped_step(reason: &'static str) -> CleanupStep {
    Box::pin(async move { StepOutcome::Skipped(reason) })
}

/// 供诊断：把步骤记录压成一行摘要。
#[must_use]
pub fn summarize(records: &[StepRecord]) -> String {
    records
        .iter()
        .map(|record| {
            let status = match &record.outcome {
                StepOutcome::Done => "完成".to_owned(),
                StepOutcome::Failed(message) => format!("失败（{message}）"),
                StepOutcome::Skipped(reason) => format!("跳过（{reason}）"),
            };
            format!("{}: {status}", record.name)
        })
        .collect::<Vec<_>>()
        .join(" → ")
}

/// 一个进程内只应存在一个协调器；用 `OnceLock` 兜住这一点。
static GLOBAL: Mutex<Option<std::sync::Arc<ShutdownCoordinator>>> = Mutex::new(None);

/// 取全局协调器（首次调用时创建）。
///
/// `Tauri` 的退出事件可能来自多个回调路径，共享一个协调器才能保证幂等。
#[must_use]
pub fn global() -> std::sync::Arc<ShutdownCoordinator> {
    let mut guard = GLOBAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .get_or_insert_with(|| std::sync::Arc::new(ShutdownCoordinator::new()))
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counter_step(
        name: &'static str,
        log: Arc<Mutex<Vec<&'static str>>>,
        label: &'static str,
    ) -> (&'static str, CleanupStep) {
        (
            name,
            Box::pin(async move {
                log.lock().expect("lock").push(label);
                StepOutcome::Done
            }),
        )
    }

    #[tokio::test]
    async fn steps_run_in_the_declared_order() {
        // 顺序是契约：先关网关会让在途请求的日志写不完。
        let log = Arc::new(Mutex::new(Vec::new()));
        let coordinator = ShutdownCoordinator::new();
        let records = coordinator
            .run(vec![
                counter_step("停止 PTY", Arc::clone(&log), "pty"),
                counter_step("停止 Gateway", Arc::clone(&log), "gateway"),
                counter_step("flush 日志", Arc::clone(&log), "flush"),
            ])
            .await;
        assert_eq!(*log.lock().expect("lock"), ["pty", "gateway", "flush"]);
        assert_eq!(records.len(), 3);
        assert!(records.iter().all(|item| item.outcome == StepOutcome::Done));
    }

    #[tokio::test]
    async fn a_failed_step_does_not_stop_the_rest() {
        // flush 失败只是丢最后几行诊断，不该让用户卡在关不掉的窗口上。
        let log = Arc::new(Mutex::new(Vec::new()));
        let coordinator = ShutdownCoordinator::new();
        let records = coordinator
            .run(vec![
                counter_step("停止 PTY", Arc::clone(&log), "pty"),
                (
                    "停止 Gateway",
                    Box::pin(async { StepOutcome::Failed("端口已释放".to_owned()) }),
                ),
                counter_step("flush 日志", Arc::clone(&log), "flush"),
            ])
            .await;
        assert_eq!(
            *log.lock().expect("lock"),
            ["pty", "flush"],
            "失败步骤之后的步骤仍要执行"
        );
        assert_eq!(
            records[1].outcome,
            StepOutcome::Failed("端口已释放".to_owned())
        );
    }

    #[tokio::test]
    async fn running_twice_does_not_repeat_the_steps() {
        // `before-quit` 可能触发多次；清理只能跑一遍。
        let runs = Arc::new(AtomicUsize::new(0));
        let coordinator = Arc::new(ShutdownCoordinator::new());
        let make = |runs: Arc<AtomicUsize>| {
            vec![(
                "唯一步骤",
                Box::pin(async move {
                    runs.fetch_add(1, Ordering::SeqCst);
                    StepOutcome::Done
                }) as CleanupStep,
            )]
        };

        let first = coordinator.run(make(Arc::clone(&runs))).await;
        let second = coordinator.run(make(Arc::clone(&runs))).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "步骤被重复执行了");
        assert_eq!(first, second, "重复调用应返回同一份记录");
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_run() {
        // 多个回调路径同时触发退出。
        let runs = Arc::new(AtomicUsize::new(0));
        let coordinator = Arc::new(ShutdownCoordinator::new());
        let mut handles = Vec::new();
        for _ in 0..4 {
            let coordinator = Arc::clone(&coordinator);
            let runs = Arc::clone(&runs);
            handles.push(tokio::spawn(async move {
                coordinator
                    .run(vec![(
                        "唯一步骤",
                        Box::pin(async move {
                            // 留出一点时间，让并发的调用真的重叠。
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                            runs.fetch_add(1, Ordering::SeqCst);
                            StepOutcome::Done
                        }) as CleanupStep,
                    )])
                    .await
            }));
        }
        let mut lengths = Vec::new();
        for handle in handles {
            lengths.push(handle.await.expect("join").len());
        }
        assert_eq!(runs.load(Ordering::SeqCst), 1, "并发调用重复执行了步骤");
        assert!(
            lengths.iter().all(|len| *len == 1),
            "每个调用者都应拿到同一份记录：{lengths:?}"
        );
    }

    #[tokio::test]
    async fn state_moves_from_idle_to_complete() {
        let coordinator = ShutdownCoordinator::new();
        assert_eq!(coordinator.state(), ShutdownState::Idle);
        assert!(!coordinator.is_complete());
        coordinator.run(Vec::new()).await;
        assert_eq!(coordinator.state(), ShutdownState::Complete);
        assert!(coordinator.is_complete());
    }

    #[tokio::test]
    async fn a_placeholder_step_is_recorded_as_skipped() {
        // PTY 尚未接入时不该假装清理过。
        let coordinator = ShutdownCoordinator::new();
        let records = coordinator
            .run(vec![("停止 PTY", skipped_step("终端尚未接入命令面"))])
            .await;
        assert_eq!(
            records[0].outcome,
            StepOutcome::Skipped("终端尚未接入命令面")
        );
    }

    #[tokio::test]
    async fn the_summary_reads_as_a_sequence() {
        let summary = summarize(&[
            StepRecord {
                name: "停止 PTY",
                outcome: StepOutcome::Skipped("未接入"),
            },
            StepRecord {
                name: "停止 Gateway",
                outcome: StepOutcome::Done,
            },
            StepRecord {
                name: "flush 日志",
                outcome: StepOutcome::Failed("磁盘满".to_owned()),
            },
        ]);
        assert_eq!(
            summary,
            "停止 PTY: 跳过（未接入） → 停止 Gateway: 完成 → flush 日志: 失败（磁盘满）"
        );
    }

    #[tokio::test]
    async fn sync_and_async_helpers_wrap_outcomes() {
        let coordinator = ShutdownCoordinator::new();
        let records = coordinator
            .run(vec![
                ("同步成功", sync_step(|| Ok(()))),
                ("同步失败", sync_step(|| Err("坏了".to_owned()))),
                ("异步成功", async_step(|| async { Ok(()) })),
            ])
            .await;
        assert_eq!(records[0].outcome, StepOutcome::Done);
        assert_eq!(records[1].outcome, StepOutcome::Failed("坏了".to_owned()));
        assert_eq!(records[2].outcome, StepOutcome::Done);
    }
}
