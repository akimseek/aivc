//! 退出确认流程。
//!
//! # 为什么要有它
//!
//! 关闭窗口**不能**直接关掉：应用里可能跑着终端与本地网关。因此流程是
//! 「拦住关闭 → 问前端 → 按回答决定」。中间有**一个窗口被拦住**的状态，
//! 必须显式记着——否则第二次关闭请求会被当成第一次，流程永远出不来。
//!
//! 三种回答：
//!
//! | 回答 | 动作 |
//! | --- | --- |
//! | `minimize` | 隐藏窗口，应用继续在托盘里跑 |
//! | `quit` | 跑完清理再退出 |
//! | `cancel` | 什么都不做，窗口留着 |
//!
//! # 失败安全：窗口**永远关得掉**
//!
//! 这是本模块最要紧的一条。拦住关闭之后如果前端不响应（页面崩了、卡死、事件没接上），
//! 用户就会被**锁在应用里**——点关闭没有任何反应。因此 [`ExitFlow::close_requested`]
//! 有两条放行规则：
//!
//! 1. **已经在问了还再点一次关闭** → 放行。用户坚持要退，就退。
//! 2. 前端**不可达**（页面已经没了）→ 放行。见 [`frontend_reachable`]。
//!
//! ## 为什么不是「超时自动退出」
//!
//! 超时和「用户在读对话框」是同一个信号：对话框弹出来之后，用户可能花三十秒才决定。
//! 一个「三十秒没回答就退出」的定时器会在用户还在看的时候**直接杀掉应用和所有终端**——
//! 拿一个数据丢失的风险去换一个不存在的锁定风险。
//!
//! 上面两条规则能达到同样的「关得掉」保证，而且**不可能在用户正常操作时误触发**：
//! 一条需要用户再点一次关闭，另一条需要前端真的已经不可达。

use std::sync::{Mutex, MutexGuard, PoisonError};

use tauri::{AppHandle, Emitter as _, Manager as _, Runtime};

/// 前端收到退出请求时用的通道。
pub const CHANNEL_REQUEST_EXIT: &str = "app:request-exit";

/// 用户在确认框里的三种选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// 最小化到托盘。
    Minimize,
    /// 直接退出。
    Quit,
    /// 取消（把对话框关掉，什么也不做）。
    Cancel,
}

impl Choice {
    /// 解析线上取值。
    ///
    /// 严格解析：`app:exit-choice` 是**能决定应用生死**的命令，不认识的取值必须
    /// 报错，而不是兜底成某一个动作。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "minimize" => Some(Self::Minimize),
            "quit" => Some(Self::Quit),
            "cancel" => Some(Self::Cancel),
            _ => None,
        }
    }
}

/// 关闭请求的处置。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// 放行：窗口真的关掉。
    Allow,
    /// 拦住，去问前端。
    Prompt,
}

/// 一个选择带来的动作。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChoiceAction {
    /// 隐藏窗口。
    Hide,
    /// 退出。
    Quit,
    /// 什么都不做。
    Dismiss,
}

/// 退出流程的状态。
#[derive(Debug, Default)]
pub struct ExitFlow {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// 已经问过前端，对话框应当正在显示。
    prompt_open: bool,
    /// 用户已经决定退出：后续的关闭请求一律放行。
    quitting: bool,
}

impl ExitFlow {
    /// 空状态。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 窗口收到了关闭请求。
    ///
    /// 返回 [`CloseAction::Prompt`] 时调用方**必须**拦住这次关闭并去问前端；
    /// 返回 [`CloseAction::Allow`] 时什么都不做，让窗口正常关掉。
    pub fn close_requested(&self) -> CloseAction {
        let mut inner = self.lock();
        // 正在退出的收尾途中：放行，别拦自己。
        if inner.quitting {
            return CloseAction::Allow;
        }
        // **失败安全之一**：已经在问了，用户又点了一次关闭——那就是坚持要退。
        //
        // 没有这一条，前端一旦不响应，窗口就永远关不掉：第一次关闭被拦住、事件没人接、
        // 再点还是被拦住。
        if inner.prompt_open {
            return CloseAction::Allow;
        }
        inner.prompt_open = true;
        CloseAction::Prompt
    }

    /// 由托盘「退出」或菜单「退出」发起的请求。
    ///
    /// 返回 `true` 表示**这次真的要去问前端**（调用方负责恢复窗口并推事件）。
    /// 已经在问、或已经在退时返回 `false`——重复请求只该被忽略，不该再弹一次。
    pub fn request(&self) -> bool {
        let mut inner = self.lock();
        if inner.quitting || inner.prompt_open {
            return false;
        }
        inner.prompt_open = true;
        true
    }

    /// 用户做了选择。
    pub fn choose(&self, choice: Choice) -> ChoiceAction {
        let mut inner = self.lock();
        match choice {
            Choice::Cancel => {
                inner.prompt_open = false;
                ChoiceAction::Dismiss
            }
            Choice::Minimize => {
                inner.prompt_open = false;
                ChoiceAction::Hide
            }
            Choice::Quit => {
                inner.quitting = true;
                inner.prompt_open = false;
                ChoiceAction::Quit
            }
        }
    }

    /// 当前是否正在等前端回答（供诊断与测试）。
    #[must_use]
    pub fn is_prompting(&self) -> bool {
        self.lock().prompt_open
    }

    /// 用户是否已经决定退出（供诊断与测试）。
    #[must_use]
    pub fn is_quitting(&self) -> bool {
        self.lock().quitting
    }

    /// 取锁。
    ///
    /// 中毒（有线程在持锁时 panic）时**恢复数据**而不是 panic：这里的数据没有「坏掉」
    /// 的概念，而 panic 只会让窗口更关不掉——那正是本模块要防的事。
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// 前端是否还够得着。
///
/// `eval` 在 webview 被销毁之后返回 `Err`，因此它是一条**便宜且精确**的存活信号。
///
/// # 它测不出什么
///
/// 它测不出「前端卡死但页面还在」——那时 `eval` 照样成功。那种情况由「再点一次关闭」
/// 兜住（见模块文档）。
pub fn frontend_reachable<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window(crate::tray::MAIN_WINDOW) else {
        // 窗口还没建好或已经没了：没有前端可以问。
        return false;
    };
    window.eval("void 0").is_ok()
}

/// 推一条「请弹退出确认框」给前端。
///
/// 送不到只记 debug：这条事件送不到不该影响已经发生的关闭请求——那种情况由
/// [`ExitFlow::close_requested`] 的放行规则兜住。
pub fn emit_request<R: Runtime>(app: &AppHandle<R>) {
    if let Err(error) = app.emit(CHANNEL_REQUEST_EXIT, ()) {
        tracing::debug!(%error, "退出请求事件发送失败");
    }
}

/// 发起一次退出请求：恢复窗口并问前端。
///
/// 返回 `true` 表示这次真的问了。
pub fn request_exit<R: Runtime>(app: &AppHandle<R>, flow: &ExitFlow) -> bool {
    if !flow.request() {
        tracing::debug!("退出请求被忽略：已经在问前端或已经在退出");
        return false;
    }
    // 窗口可能被最小化或藏在托盘后面：不恢复，用户根本看不到那个对话框，
    // 而这是一次**必须被看到**的询问。
    crate::tray::restore_window(app);
    emit_request(app);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_parsed_strictly() {
        assert_eq!(Choice::parse("minimize"), Some(Choice::Minimize));
        assert_eq!(Choice::parse("quit"), Some(Choice::Quit));
        assert_eq!(Choice::parse("cancel"), Some(Choice::Cancel));
        // 不认识的取值必须报错，而不是兜底成某一个动作——这条命令能决定应用生死。
        assert_eq!(Choice::parse(""), None);
        assert_eq!(Choice::parse("Quit"), None);
        assert_eq!(Choice::parse("exit"), None);
    }

    #[test]
    fn the_first_close_request_asks_the_frontend() {
        let flow = ExitFlow::new();
        assert_eq!(flow.close_requested(), CloseAction::Prompt);
        assert!(flow.is_prompting());
        assert!(!flow.is_quitting(), "只是问了，还没决定退");
    }

    #[test]
    fn a_second_close_request_while_prompting_is_allowed() {
        // **失败安全**。没有这一条，前端一旦不响应，窗口就永远关不掉：
        // 第一次被拦住、事件没人接、再点还是被拦住。用户就被锁在应用里了。
        let flow = ExitFlow::new();
        assert_eq!(flow.close_requested(), CloseAction::Prompt);
        assert_eq!(flow.close_requested(), CloseAction::Allow);
        // 而且不该因此进入「正在退出」——放行由关闭流程自己收尾。
        assert!(!flow.is_quitting());
    }

    #[test]
    fn cancelling_reopens_the_prompt() {
        // 取消之后必须能**再次**被问到，否则用户取消一次就再也关不掉了。
        let flow = ExitFlow::new();
        flow.close_requested();
        assert_eq!(flow.choose(Choice::Cancel), ChoiceAction::Dismiss);
        assert!(!flow.is_prompting());
        assert!(!flow.is_quitting());
        assert_eq!(flow.close_requested(), CloseAction::Prompt);
    }

    #[test]
    fn minimizing_does_not_quit() {
        // 最小化到托盘：应用继续跑（终端与网关都还在），所以绝不能进「正在退出」。
        let flow = ExitFlow::new();
        flow.close_requested();
        assert_eq!(flow.choose(Choice::Minimize), ChoiceAction::Hide);
        assert!(!flow.is_quitting());
        assert_eq!(flow.close_requested(), CloseAction::Prompt, "还能再问一次");
    }

    #[test]
    fn choosing_quit_releases_every_later_close_request() {
        // 收尾途中窗口可能被再关一次（清理里的某些步骤会关窗口），
        // 那一次必须放行，否则会拦下自己的收尾。
        let flow = ExitFlow::new();
        flow.close_requested();
        assert_eq!(flow.choose(Choice::Quit), ChoiceAction::Quit);
        assert!(flow.is_quitting());
        assert_eq!(flow.close_requested(), CloseAction::Allow);
    }

    #[test]
    fn a_repeated_request_from_the_tray_is_ignored() {
        // 托盘「退出」点两下不该弹两次对话框。
        let flow = ExitFlow::new();
        assert!(flow.request());
        assert!(!flow.request());
    }

    #[test]
    fn a_request_while_quitting_is_ignored() {
        let flow = ExitFlow::new();
        flow.close_requested();
        flow.choose(Choice::Quit);
        assert!(!flow.request());
    }

    #[test]
    fn a_request_and_a_close_request_share_the_same_prompt_state() {
        // 两条入口（托盘退出 / 关闭窗口）必须共用同一份状态。各记一份的话，
        // 会出现「对话框已经开着，点关闭又弹一个」——或者更糟，谁都以为自己没问过。
        let flow = ExitFlow::new();
        assert!(flow.request());
        assert_eq!(
            flow.close_requested(),
            CloseAction::Allow,
            "已经在问了，再点关闭应当放行"
        );
    }

    #[test]
    fn a_poisoned_lock_still_answers() {
        // 中毒时**恢复数据**而不是 panic：panic 只会让窗口更关不掉。
        let flow = std::sync::Arc::new(ExitFlow::new());
        let poisoner = std::sync::Arc::clone(&flow);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.inner.lock().expect("首次加锁成功");
            panic!("故意毒化锁");
        })
        .join();

        // 不 panic 即通过。
        assert_eq!(flow.close_requested(), CloseAction::Prompt);
    }
}
