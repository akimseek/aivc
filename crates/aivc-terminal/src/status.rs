//! 终端状态检测：从 PTY 输出流中识别「等待确认 / 出错 / 本轮已完成」。
//!
//! # 为什么需要一个状态机而不是几条正则
//!
//! PTY 输出按**任意边界**到达——一行可能被切成三段，一段里也可能含多行。因此不能对
//! 每一批数据单独跑正则：`Ask Codex to do anything` 的提示符完全可能被拆成两批。
//! 检测器保留最近一段可识别文本，在**累积缓冲**上判定。
//!
//! # 「本轮已完成」的起点是提交，而不是打字
//!
//! 打字、粘贴草稿和终端协议应答都不能开始轮次。输入检测器保留草稿状态，只有括号粘贴
//! 之外的回车才提交；确认界面的 Enter / y / n / Escape 只推进当前阶段。
//!
//! 提交后必须见过实际输出，再回到空闲提示符才能判定完成。控制序列、输入回显，以及
//! 被切成多批的提示符本身都不算实际输出。确认和异常独立检测，不依赖是否已提交提示词。
//!
//! # 重复输入为什么不能重置缓冲
//!
//! 同一轮里用户可能再按几次回车（确认授权、补充按键）。若每次都重置缓冲，运行中的界面
//! 重绘会把旧的空闲提示误判成「本轮已完成」。因此只有**新一轮**才清空缓冲。

use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use aivc_core::target::ProviderId;

/// 保留的可识别文本上限（字符数）。
pub const MAX_BUFFER_CHARS: usize = 32 * 1024;
/// 确认提示只检查尾部这么多个字符。
pub const CONFIRMATION_TAIL_CHARS: usize = 2400;
/// 提示符只检查尾部这么多个字符。
pub const PROMPT_TAIL_CHARS: usize = 1200;
/// 错误摘要上限。
pub const ERROR_DETAIL_MAX_CHARS: usize = 240;

/// 终端状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TerminalStatus {
    /// 已提交输入，正在运行。
    Running,
    /// 等待用户确认或授权。
    AwaitingConfirmation,
    /// 本轮已完成。
    Completed,
    /// 出现异常。
    Error,
}

impl TerminalStatus {
    /// 序列化短名。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::AwaitingConfirmation => "awaiting-confirmation",
            Self::Completed => "completed",
            Self::Error => "error",
        }
    }
}

/// 需要提醒用户的事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalAttention {
    /// 提醒类别。
    pub kind: TerminalStatus,
    /// 标题。
    pub title: String,
    /// 详情。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// 对外发出的状态事件。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalStatusEvent {
    /// 终端编号。
    pub terminal_id: String,
    /// 状态。
    pub status: TerminalStatus,
    /// 需要提醒时的事件内容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<TerminalAttention>,
    /// 轮次编号（第几次提交输入）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<u64>,
}

/// 提示符的可见性追踪。
///
/// 三个标志合在一起才能回答「本轮是否完成」，因此收成一个结构而不是散在检测器上——
/// 它们总是一起被重置、一起被读取。
#[derive(Debug, Default, Clone, Copy)]
struct PromptTracking {
    /// 提示符曾经出现过。
    was_visible: bool,
    /// 提示符出现过之后又消失（说明有输出把界面推走了）。
    disappeared: bool,
    /// 见过不含提示符的输出。
    saw_output_without_prompt: bool,
}

impl PromptTracking {
    /// 回到本轮开始时的状态。
    fn reset(&mut self) {
        *self = Self::default();
    }

    /// 判定「本轮已完成」是否成立。
    ///
    /// 三个条件缺一不可：出现过、消失过、又出现。只看「现在可见」会把启动界面上的
    /// 空闲提示立刻判定成「本轮已完成」。
    const fn can_complete(self) -> bool {
        self.saw_output_without_prompt && (!self.was_visible || self.disappeared)
    }
}

/// Track raw PTY input across writes. Newlines inside bracketed paste are draft
/// text; cursor reports, mouse reports and navigation sequences are not submits.
#[derive(Debug, Default)]
struct InputTracking {
    draft: String,
    escape: String,
    bracketed_paste: bool,
    recalled_history: bool,
}

#[derive(Default)]
struct InputAction {
    submission: Option<String>,
    confirmation_key: bool,
}

impl InputTracking {
    fn record(&mut self, data: &str) -> InputAction {
        let mut action = InputAction::default();
        // An isolated Escape cancels a confirmation, unlike an arrow/DSR packet.
        action.confirmation_key = data == "\u{1b}" && !self.bracketed_paste;
        for ch in data.chars() {
            if !self.escape.is_empty() {
                self.escape.push(ch);
                let finished = if self.escape.starts_with("\u{1b}[") {
                    self.escape.len() > 2 && ('@'..='~').contains(&ch)
                } else if self.escape.starts_with("\u{1b}]") {
                    ch == '\u{7}' || self.escape.ends_with("\u{1b}\\")
                } else {
                    self.escape.len() > 1
                };
                if finished || self.escape.len() > 256 {
                    match self.escape.as_str() {
                        "\u{1b}[200~" => self.bracketed_paste = true,
                        "\u{1b}[201~" => self.bracketed_paste = false,
                        "\u{1b}[A" | "\u{1b}[B" => self.recalled_history = true,
                        _ => {}
                    }
                    self.escape.clear();
                }
                continue;
            }
            if ch == '\u{1b}' {
                self.escape.push(ch);
                continue;
            }
            if self.bracketed_paste {
                self.draft.push(ch);
                continue;
            }
            match ch {
                '\r' | '\n' => {
                    action.confirmation_key = true;
                    let draft = std::mem::take(&mut self.draft);
                    if !draft.trim().is_empty() || self.recalled_history {
                        action.submission = Some(draft);
                    }
                    self.recalled_history = false;
                }
                '\u{8}' | '\u{7f}' => {
                    self.draft.pop();
                }
                '\u{3}' | '\u{15}' => {
                    self.draft.clear();
                    self.recalled_history = false;
                }
                ch if !ch.is_control() => {
                    self.draft.push(ch);
                    action.confirmation_key |= matches!(ch, 'y' | 'Y' | 'n' | 'N');
                }
                _ => {}
            }
        }
        truncate_front(&mut self.draft, MAX_BUFFER_CHARS);
        action
    }
}

/// 终端状态检测器。
#[derive(Debug)]
pub struct TerminalStatusDetector {
    provider_id: ProviderId,
    buffer: String,
    turn_active: bool,
    stopped: bool,
    input: InputTracking,
    submitted_text: String,
    turn_id: u64,
    last_status: Option<TerminalStatus>,
    last_attention_key: String,
    prompt: PromptTracking,
}

impl TerminalStatusDetector {
    /// 构造。`provider_id` 决定用哪一套提示符。
    #[must_use]
    pub fn new(provider_id: ProviderId) -> Self {
        Self {
            provider_id,
            buffer: String::new(),
            turn_active: false,
            stopped: false,
            input: InputTracking::default(),
            submitted_text: String::new(),
            turn_id: 0,
            last_status: None,
            last_attention_key: String::new(),
            prompt: PromptTracking::default(),
        }
    }

    /// 当前轮次编号。
    #[must_use]
    pub const fn turn_id(&self) -> u64 {
        self.turn_id
    }

    /// 当前是否处于一轮运行中。
    #[must_use]
    pub const fn is_turn_active(&self) -> bool {
        self.turn_active
    }

    /// 记录用户输入。
    ///
    /// 返回需要发出的事件（可能是「无事件」）。
    pub fn mark_input(&mut self, data: &str) -> Option<TerminalStatusEvent> {
        if self.stopped {
            return None;
        }
        let action = self.input.record(data);
        let answering_confirmation = self.last_status == Some(TerminalStatus::AwaitingConfirmation)
            && action.confirmation_key;
        if action.submission.is_none() && !answering_confirmation {
            return None;
        }
        // 同一轮中的后续输入不应重置检测缓冲：运行中的界面重绘可能把旧的空闲提示
        // 误判成「本轮已完成」。等待确认是例外——响应确认意味着进入下一阶段。
        if self.turn_active && !answering_confirmation {
            return None;
        }
        if !self.turn_active && !answering_confirmation {
            self.turn_id += 1;
        }
        if let Some(text) = action.submission {
            self.submitted_text = text;
        }
        if answering_confirmation {
            self.input.draft.clear();
            if data == "\u{1b}" {
                self.input.escape.clear();
            }
        }
        // Answering a startup/trust prompt does not itself start an assistant
        // turn. Otherwise the first real submission would be ignored as busy.
        if !answering_confirmation {
            self.turn_active = true;
        }
        self.buffer.clear();
        self.prompt.reset();
        self.publish(TerminalStatus::Running, None)
    }

    /// 检查一批 PTY 输出。
    pub fn inspect_output(&mut self, data: &str) -> Option<TerminalStatusEvent> {
        if self.stopped || data.is_empty() {
            return None;
        }
        let cleaned = strip_ansi(data);
        self.buffer.push_str(&cleaned);
        truncate_front(&mut self.buffer, MAX_BUFFER_CHARS);

        let prompt_visible = is_prompt(self.provider_id, &self.buffer);
        if self.turn_active && !prompt_visible && self.has_progress_output() {
            self.prompt.saw_output_without_prompt = true;
            if self.prompt.was_visible {
                self.prompt.disappeared = true;
            }
        }

        let detection = detect_attention(self.provider_id, &self.buffer)?;
        if detection.kind == TerminalStatus::Completed {
            if !self.turn_active || self.turn_id == 0 {
                return None;
            }
            // 提示符此刻不可见，说明还在输出——不能判定完成。
            if !prompt_visible {
                return None;
            }
            // 三条件缺一不可：出现过、消失过、又出现。
            let can_complete = self.prompt.can_complete();
            self.prompt.was_visible = true;
            if !can_complete {
                return None;
            }
        }
        if matches!(
            detection.kind,
            TerminalStatus::Completed | TerminalStatus::Error
        ) {
            self.turn_active = false;
        }
        self.publish(detection.kind, Some(detection))
    }

    /// A split idle prompt or the submitted text being echoed is not evidence
    /// that the assistant has started answering.
    fn has_progress_output(&self) -> bool {
        let prompt = match self.provider_id {
            ProviderId::Codex => "Ask Codex to do anything",
            ProviderId::Gemini => "Ask Gemini to do anything",
            ProviderId::Claude => "Ask Claude to do anything",
            ProviderId::Qoder => "Ask Qoder to do anything",
        };
        self.buffer.lines().any(|line| {
            let text = line.trim().trim_start_matches(['›', '❯', '>']).trim();
            !text.is_empty()
                && !prompt.starts_with(text)
                && !(self.provider_id == ProviderId::Qoder
                    && "Ask anything to do anything".starts_with(text))
                && !self
                    .submitted_text
                    .lines()
                    .any(|input| input.trim().starts_with(text))
        })
    }

    /// 记录进程退出。
    pub fn mark_exit(&mut self, exit_code: i32) -> Option<TerminalStatusEvent> {
        self.turn_active = false;
        // 正常退出只表示终端生命周期结束，不代表助手完成了一轮回答：
        // 历史会话启动、CLI 自行结束或恢复失败都不应伪造「本轮已完成」提醒。
        if self.stopped || exit_code == 0 {
            return None;
        }
        self.publish(
            TerminalStatus::Error,
            Some(TerminalAttention {
                kind: TerminalStatus::Error,
                title: "会话异常停止".to_owned(),
                detail: Some(format!("进程退出码 {exit_code}")),
            }),
        )
    }

    /// 用户主动关闭终端，不应把随后的强制退出当成异常提醒。
    pub fn mark_stopped(&mut self) {
        self.stopped = true;
        self.turn_active = false;
    }

    /// 去重后发出。
    fn publish(
        &mut self,
        status: TerminalStatus,
        attention: Option<TerminalAttention>,
    ) -> Option<TerminalStatusEvent> {
        // Details are diagnostic text and may contain a changing URL, retry
        // count, or upstream body. They must not defeat same-type de-duplication
        // within one submitted turn.
        let key = attention
            .as_ref()
            .map_or_else(String::new, |item| item.kind.as_str().to_owned());
        if self.last_status == Some(status) && key == self.last_attention_key {
            return None;
        }
        self.last_status = Some(status);
        self.last_attention_key = key;
        Some(TerminalStatusEvent {
            terminal_id: String::new(),
            status,
            attention,
            turn_id: (self.turn_id > 0).then_some(self.turn_id),
        })
    }
}

/// 从累积文本判定当前应提醒什么。
///
/// 顺序固定：确认提示 → 错误 → 空闲提示符。「等待确认」优先于「已完成」，因为确认界面
/// 本身就是带提示符的空闲界面，先判完成会让用户看到「本轮已完成」而错过授权请求。
#[must_use]
pub fn detect_attention(provider_id: ProviderId, text: &str) -> Option<TerminalAttention> {
    let normalized = text.replace('\r', "");
    if is_confirmation_prompt(provider_id, &normalized) {
        return Some(TerminalAttention {
            kind: TerminalStatus::AwaitingConfirmation,
            title: "等待确认".to_owned(),
            detail: Some("终端正在等待你的确认或授权。".to_owned()),
        });
    }
    if let Some(detail) = find_error(&normalized) {
        return Some(TerminalAttention {
            kind: TerminalStatus::Error,
            title: "会话出现异常".to_owned(),
            detail: Some(detail),
        });
    }
    if is_prompt(provider_id, &normalized) {
        return Some(TerminalAttention {
            kind: TerminalStatus::Completed,
            title: "本轮已完成".to_owned(),
            detail: None,
        });
    }
    None
}

/// 输入里是否含真正的用户内容（忽略纯控制字符与空白）。
#[must_use]
pub fn has_user_input(data: &str) -> bool {
    strip_ansi(data)
        .chars()
        .filter(|ch| !(ch.is_control() || *ch == '\u{7f}'))
        .collect::<String>()
        .trim()
        .chars()
        .count()
        > 0
}

/// 去掉 ANSI 转义序列。
///
/// 三类序列，顺序不能反：
///
/// 1. **OSC**（`ESC ] … BEL` 或 `ESC ] … ESC \`）——先去掉。它的结束符里含 `ESC \`，
///    若先跑 CSI 规则会把结束符吃掉一半，留下尾巴。
/// 2. **CSI**（`ESC [ 参数 中间字节 结束字节`）——`ESC[31m`、`ESC[A` 等绝大多数序列。
/// 3. **其他 Fe 转义**（`ESC` + 单个字符）——`ESC(B` 之类的字符集切换。
#[must_use]
pub fn strip_ansi(value: &str) -> String {
    let step = remove_all(osc_pattern(), value);
    let step = remove_all(csi_pattern(), &step);
    remove_all(esc_pattern(), &step)
}

/// 从前面截断，保留末尾 `limit` 个字符。
fn truncate_front(buffer: &mut String, limit: usize) {
    let count = buffer.chars().count();
    if count <= limit {
        return;
    }
    let skip = count - limit;
    let start = buffer
        .char_indices()
        .nth(skip)
        .map_or(buffer.len(), |(index, _)| index);
    buffer.drain(..start);
}

/// 取末尾 `limit` 个字符。
fn tail(text: &str, limit: usize) -> &str {
    let count = text.chars().count();
    if count <= limit {
        return text;
    }
    let skip = count - limit;
    let start = text
        .char_indices()
        .nth(skip)
        .map_or(text.len(), |(index, _)| index);
    &text[start..]
}

/// 是否处于确认提示。
fn is_confirmation_prompt(provider_id: ProviderId, text: &str) -> bool {
    let tail = tail(text, CONFIRMATION_TAIL_CHARS);
    let patterns = [
        r"(?i)Press enter to confirm\s+or\s+esc to cancel\s*$",
        r"(?i)(?:Yes, proceed|Yes, and don't ask again|No, and tell Codex what to do differently)[^\n]*\n(?:[^\n]*\n){0,3}\s*Press enter to confirm",
        r"(?i)\b(?:y/n|yes/no|allow|approve|confirm)\b\s*[:?]?\s*$",
        r"(?iu)(?:Would you like to run|Do you want to proceed|Approve this command|Permission required|需要确认|请求授权|是否执行)[^\n]{0,120}[?：:]?\s*$",
        r"(?u)(?:是否允许|是否继续|确认执行|需要授权|请求提权|允许此操作)\s*[：:]?\s*$",
    ];
    if patterns.iter().any(|pattern| matches(pattern, tail)) {
        return true;
    }
    // Codex 特有的授权块。
    provider_id == ProviderId::Codex
        && matches(r"(?i)(?:Allow|Approve|Run command)\s*\[[^\]]+\]", tail)
}

/// 查找错误摘要。
fn find_error(text: &str) -> Option<String> {
    let patterns = [
        r"(?i)stream disconnected before completion[^\n]*",
        r"(?i)transport error[^\n]*",
        r"(?i)conversation interrupted[^\n]*",
        r"(?i)(?:fatal error|unhandled exception)[^\n]*",
        r"(?i)(?:codex|gemini|claude|qoder)\s+(?:已退出|退出)[^\n]*",
    ];
    patterns.iter().find_map(|pattern| {
        find_match(pattern, text)
            .map(|found| found.trim().chars().take(ERROR_DETAIL_MAX_CHARS).collect())
    })
}

/// 是否停在空闲提示符上。
fn is_prompt(provider_id: ProviderId, text: &str) -> bool {
    let tail = tail(text, PROMPT_TAIL_CHARS);
    // 各平台提示符措辞固定，但 `›`/`❯` 之外还接受 `>`（Qoder 用 `>`）。
    let pattern = match provider_id {
        ProviderId::Codex => r"(?u)(?:^|\n)\s*[›❯]\s*Ask Codex to do anything\s*$",
        ProviderId::Gemini => r"(?u)(?:^|\n)\s*[›❯]\s*Ask Gemini to do anything\s*$",
        ProviderId::Claude => r"(?u)(?:^|\n)\s*[›❯]\s*Ask Claude to do anything\s*$",
        ProviderId::Qoder => r"(?iu)(?:^|\n)\s*[›❯>]\s*Ask (?:Qoder|anything) to do anything\s*$",
    };
    matches(pattern, tail)
}

/// 编译正则（带缓存）。
///
/// 模式都是固定的字面量，编译失败只可能来自开发期笔误。此时返回 `None`，由调用方把
/// 这条规则视为「永不匹配」——开发期笔误只该让一条规则失效，不该让整个终端崩掉。
fn compile(pattern: &str) -> Option<Arc<Regex>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<Arc<Regex>>>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(guard) = cache.lock()
        && let Some(found) = guard.get(pattern)
    {
        return found.clone();
    }
    // 编译失败也缓存：否则每次输出批次都要重试一次注定失败的编译。
    let compiled = Regex::new(pattern).ok().map(Arc::new);
    if let Ok(mut guard) = cache.lock() {
        guard.insert(pattern.to_owned(), compiled.clone());
    }
    compiled
}

/// 模式是否在文本中命中（编译失败视为不命中）。
fn matches(pattern: &str, text: &str) -> bool {
    compile(pattern).is_some_and(|regex| regex.is_match(text))
}

/// 模式在文本中的第一处命中。
fn find_match<'a>(pattern: &str, text: &'a str) -> Option<&'a str> {
    compile(pattern)?.find(text).map(|found| found.as_str())
}

/// 去掉所有命中的片段。
fn remove_all(pattern: &str, text: &str) -> String {
    compile(pattern).map_or_else(
        || text.to_owned(),
        |regex| regex.replace_all(text, "").into_owned(),
    )
}

/// OSC 序列：`ESC ] … BEL` 或 `ESC ] … ESC \`。
///
/// 正文用「非 BEL 非 ESC」的否定字符类而不是惰性 `.*?`：惰性量词在长文本上会带来
/// 回溯，而 OSC 正文里本来就不允许出现这两个字符。
fn osc_pattern() -> &'static str {
    "\u{1b}\\][^\u{7}\u{1b}]*(?:\u{7}|\u{1b}\\\\)"
}

/// CSI 序列：`ESC [` + 参数字节 + 中间字节 + 结束字节。
///
/// 这是 ANSI 转义序列的通用形状，覆盖 `ESC[31m`（颜色）、`ESC[A`（光标）、
/// `ESC[2J`（清屏）等绝大多数实际输出。
fn csi_pattern() -> &'static str {
    "\u{1b}\\[[0-9;?]*[ -/]*[@-~]"
}

/// 其他 Fe 转义序列：`ESC` + 单个字符（如 `ESC(B` 的字符集切换）。
fn esc_pattern() -> &'static str {
    "\u{1b}[@-Z\\\\-_]"
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODEX_PROMPT: &str = "\n› Ask Codex to do anything\n";

    fn detector(provider: ProviderId) -> TerminalStatusDetector {
        TerminalStatusDetector::new(provider)
    }

    #[test]
    fn ansi_sequences_are_stripped() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}[1;32mok\u{1b}[m"), "ok");
        // OSC（设置窗口标题）也要去掉。
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}text"), "text");
        // 纯文本不变。
        assert_eq!(strip_ansi("plain"), "plain");
    }

    #[test]
    fn pure_control_input_is_not_user_input() {
        // 方向键、退格等控制序列不算「提交了输入」。
        assert!(!has_user_input("\u{1b}[A"));
        assert!(!has_user_input("\r\n"));
        assert!(!has_user_input("   "));
        // 真实字符算。
        assert!(has_user_input("hi"));
        assert!(has_user_input("  你好  "));
        // 控制字符包里夹带真实字符也算。
        assert!(has_user_input("\u{1b}[Ax"));
    }

    #[test]
    fn submitting_the_first_draft_starts_a_turn() {
        let mut d = detector(ProviderId::Codex);
        assert_eq!(d.turn_id(), 0);
        // 控制输入不开始轮次。
        assert!(d.mark_input("\u{1b}[A").is_none());
        assert_eq!(d.turn_id(), 0);

        assert!(d.mark_input("hello").is_none(), "输入草稿不算提交");
        assert_eq!(d.turn_id(), 0);
        let event = d.mark_input("\r").expect("event");
        assert_eq!(event.status, TerminalStatus::Running);
        assert_eq!(event.turn_id, Some(1));
        assert!(d.is_turn_active());
    }

    #[test]
    fn further_input_in_the_same_turn_does_not_restart_it() {
        // 同一轮里的后续输入（确认授权、补充按键）不该重置检测缓冲，
        // 否则运行中的界面重绘会把旧的空闲提示误判成「本轮已完成」。
        let mut d = detector(ProviderId::Codex);
        d.mark_input("first\r").expect("first");
        assert!(d.mark_input("second").is_none());
        assert_eq!(d.turn_id(), 1);
    }

    #[test]
    fn input_while_awaiting_confirmation_advances_the_turn() {
        // 响应确认意味着进入下一阶段，此时应当重置缓冲。
        let mut d = detector(ProviderId::Codex);
        d.mark_input("first\r").expect("first");
        d.inspect_output("Do you want to proceed? (y/n)")
            .expect("confirmation");
        assert_eq!(d.last_status, Some(TerminalStatus::AwaitingConfirmation));
        let event = d.mark_input("y").expect("event");
        assert_eq!(event.status, TerminalStatus::Running);
        assert_eq!(d.turn_id(), 1, "确认不算新一轮");
    }

    #[test]
    fn output_is_ignored_when_no_turn_is_running() {
        let mut d = detector(ProviderId::Codex);
        // 启动界面上的提示符不该产生任何事件。
        assert!(d.inspect_output(CODEX_PROMPT).is_none());
    }

    #[test]
    fn the_startup_prompt_alone_never_means_completed() {
        // 关键：提示符可见但从未消失过 → 不能判定完成。
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        assert!(d.inspect_output(CODEX_PROMPT).is_none(), "首次出现不算完成");
    }

    #[test]
    fn completion_requires_the_prompt_to_disappear_and_return() {
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        // 输出把提示符推走。
        assert!(d.inspect_output("thinking...\n").is_none());
        // 提示符回来 → 完成。
        let event = d.inspect_output(CODEX_PROMPT).expect("completed");
        assert_eq!(event.status, TerminalStatus::Completed);
        assert!(!d.is_turn_active(), "完成后不再处于运行中");
    }

    #[test]
    fn a_prompt_split_across_chunks_is_still_recognized() {
        // PTY 输出按任意边界到达，因此判定必须基于累积缓冲。
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        d.inspect_output("output\n");
        assert!(d.inspect_output("\n› Ask Codex to do ").is_none());
        let event = d.inspect_output("anything\n").expect("completed");
        assert_eq!(event.status, TerminalStatus::Completed);
    }

    #[test]
    fn errors_are_reported_with_their_detail() {
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        let event = d
            .inspect_output("stream disconnected before completion: broken pipe")
            .expect("error");
        assert_eq!(event.status, TerminalStatus::Error);
        let detail = event.attention.expect("attention").detail.expect("detail");
        assert!(detail.contains("stream disconnected"), "{detail}");
    }

    #[test]
    fn confirmation_takes_priority_over_completion() {
        // 确认界面本身带提示符，先判完成会让用户错过授权请求。
        let text = "Do you want to proceed? (y/n)";
        let detection = detect_attention(ProviderId::Codex, text).expect("detection");
        assert_eq!(detection.kind, TerminalStatus::AwaitingConfirmation);
        assert_eq!(detection.title, "等待确认");
    }

    #[test]
    fn interactive_confirmation_patterns_are_recognized() {
        for text in [
            "Press enter to confirm or esc to cancel",
            "  Press enter to confirm or esc to cancel  ",
            "Allow [y/n]",
            "Approve this command?",
            "Permission required",
            "Would you like to run this?",
            "需要确认吗",
            "是否允许",
            "确认执行：",
            "请求授权",
        ] {
            assert!(
                is_confirmation_prompt(ProviderId::Codex, text),
                "未识别：{text}"
            );
        }
        assert!(!is_confirmation_prompt(ProviderId::Codex, "all done"));
    }

    #[test]
    fn the_codex_approval_block_is_recognized() {
        let text =
            "Yes, proceed\nNo, and tell Codex what to do differently\n\nPress enter to confirm";
        assert!(is_confirmation_prompt(ProviderId::Codex, text));
        assert!(is_confirmation_prompt(
            ProviderId::Codex,
            "Run command [npm test]"
        ));
        // 非 Codex 平台不套用这条 Codex 专有规则。
        assert!(!is_confirmation_prompt(
            ProviderId::Claude,
            "Run command [npm test]"
        ));
    }

    #[test]
    fn every_provider_has_its_own_prompt() {
        let cases = [
            (ProviderId::Codex, "Ask Codex to do anything"),
            (ProviderId::Gemini, "Ask Gemini to do anything"),
            (ProviderId::Claude, "Ask Claude to do anything"),
            (ProviderId::Qoder, "Ask Qoder to do anything"),
        ];
        for (provider, text) in cases {
            assert!(
                is_prompt(provider, &format!("\n› {text}\n")),
                "{provider:?} 的提示符未识别"
            );
            // 别的平台的提示符不该被认出来。
            let other = if provider == ProviderId::Codex {
                ProviderId::Claude
            } else {
                ProviderId::Codex
            };
            assert!(
                !is_prompt(other, &format!("\n› {text}\n")),
                "{other:?} 误识别了别人的提示符"
            );
        }
    }

    #[test]
    fn the_qoder_prompt_also_accepts_a_plain_chevron() {
        assert!(is_prompt(
            ProviderId::Qoder,
            "\n> Ask Qoder to do anything\n"
        ));
        assert!(is_prompt(
            ProviderId::Qoder,
            "\n› Ask anything to do anything\n"
        ));
    }

    #[test]
    fn duplicate_events_are_suppressed() {
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        d.inspect_output("out\n");
        d.inspect_output(CODEX_PROMPT).expect("first");
        // 同样的状态与提醒不再重复发出。
        assert!(d.inspect_output(CODEX_PROMPT).is_none());
    }

    #[test]
    fn mark_exit_with_zero_is_silent() {
        // 正常退出只表示终端生命周期结束，不代表助手完成了一轮回答。
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        assert!(d.mark_exit(0).is_none());
        assert!(!d.is_turn_active());
    }

    #[test]
    fn a_non_zero_exit_after_input_reports_an_error() {
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        let event = d.mark_exit(1).expect("error");
        assert_eq!(event.status, TerminalStatus::Error);
        let attention = event.attention.expect("attention");
        assert_eq!(attention.title, "会话异常停止");
        assert_eq!(attention.detail.as_deref(), Some("进程退出码 1"));
    }

    #[test]
    fn a_non_zero_exit_without_any_input_reports_startup_failure() {
        // 启动阶段的异常也需要提醒，不依赖用户已提交过提示词。
        let mut d = detector(ProviderId::Codex);
        assert_eq!(d.mark_exit(1).expect("error").status, TerminalStatus::Error);
    }

    #[test]
    fn the_buffer_is_bounded_and_keeps_the_tail() {
        let mut buffer = "x".repeat(MAX_BUFFER_CHARS + 100);
        truncate_front(&mut buffer, MAX_BUFFER_CHARS);
        assert_eq!(buffer.chars().count(), MAX_BUFFER_CHARS);
        // 保留的是末尾内容。
        buffer.push_str("END");
        assert!(buffer.ends_with("END"));
    }

    #[test]
    fn truncation_respects_multibyte_boundaries() {
        // 按字符截断：按字节会把多字节字符切成半个，产生无效 UTF-8。
        let mut buffer = "字".repeat(10);
        truncate_front(&mut buffer, 5);
        assert_eq!(buffer.chars().count(), 5);
        assert_eq!(buffer, "字".repeat(5));
    }

    #[test]
    fn the_tail_helper_keeps_multibyte_text_intact() {
        let text = "字".repeat(10);
        assert_eq!(tail(&text, 3).chars().count(), 3);
        assert_eq!(tail(&text, 100), text);
    }

    #[test]
    fn error_details_are_truncated() {
        let long = format!("fatal error {}", "x".repeat(500));
        let detail = find_error(&long).expect("detail");
        assert_eq!(detail.chars().count(), ERROR_DETAIL_MAX_CHARS);
    }

    #[test]
    fn all_the_documented_error_patterns_are_recognized() {
        for text in [
            "stream disconnected before completion: oops",
            "Transport error: connection reset",
            "conversation interrupted",
            "Fatal error: out of memory",
            "Unhandled exception in handler",
            "codex 已退出",
        ] {
            assert!(find_error(text).is_some(), "未识别：{text}");
        }
        assert!(find_error("everything is fine").is_none());
    }

    #[test]
    fn a_carriage_return_does_not_break_prompt_detection() {
        // PTY 用 `\r\n` 换行，判定前必须统一。
        let mut d = detector(ProviderId::Codex);
        d.mark_input("hello\r").expect("input");
        d.inspect_output("out\r\n");
        let event = d
            .inspect_output("\r\n› Ask Codex to do anything\r\n")
            .expect("done");
        assert_eq!(event.status, TerminalStatus::Completed);
    }
}
