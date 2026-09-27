//! Qoder CLI 的模型清单解析。
//!
//! # 为什么单独成一个模块
//!
//! `models:list` 的**全部逻辑风险都在解析上**：命令是固定的 `qodercn --list-models`，
//! 而它的输出是人读的表格——带 ANSI 颜色、带表头、可能带空行。
//! 把它抽成纯函数，就能在不依赖 `qodercn` 是否安装的前提下把它测透。
//!
//! # 与 `vendor:list-models` 的区别
//!
//! 那个查**供应商的 HTTP 接口**；这个查 **Qoder CLI 自身的清单**。
//! 两者数据来源完全不同，不能合并——现网也是分开的两条命令。

/// 一个模型条目。
///
/// 只有一个字段：现网返回的就是 `{ id }`，没有显示名与描述。
/// **不臆造字段**——前端按 `{ id }` 解构，多出来的键没有用处。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QoderModel {
    /// 模型编号。
    pub id: String,
}

/// 表头。输出形如：
///
/// ```text
/// MODEL
/// qwen3-coder-plus
/// qwen3-coder-flash
/// ```
const HEADER: &str = "MODEL";

/// 解析 `qodercn --list-models` 的输出。
///
/// 逐行处理：
///
/// 1. **去掉 ANSI 转义序列** —— 命令在有颜色的终端下会输出它们，
///    不剥离就会得到 `\u{1b}[32mqwen3-coder\u{1b}[0m` 这样的「模型名」；
/// 2. `trim` 掉两侧空白（含 `\r`，Windows 输出会有）；
/// 3. **丢掉空行**
/// 4. **丢掉表头**（大小写不敏感——不能假设它一定是大写）
///
/// 其余每一行原样作为模型编号，**不做去重与排序**：那是显示层的选择，
/// 而这里篡改顺序会让「与 CLI 输出一致」这条性质失效。
#[must_use]
pub fn parse_model_list(output: &str) -> Vec<QoderModel> {
    output
        .lines()
        .map(strip_ansi)
        .map(|line| line.trim().to_owned())
        .filter(|line| !line.is_empty())
        .filter(|line| !line.eq_ignore_ascii_case(HEADER))
        .map(|id| QoderModel { id })
        .collect()
}

/// 子命令的固定参数。
const LIST_MODELS_ARG: &str = "--list-models";

/// 执行超时（现网 20 秒）。
///
/// 20 秒是有道理的：CLI 首次启动要拉起运行时，而 5 秒会把它误判成失败。
pub const TIMEOUT_MS: u64 = 20_000;

/// 构造「列出 Qoder 模型」的命令。
///
/// 返回 `(程序, 参数)`。
///
/// # 为什么把 WSL 也表达成普通命令
///
/// [`CommandRunner`] 只接受程序名与参数，没有「发行版」这个维度。
/// 而 `wsl.exe -d <distro> -- <程序> <参数>` 恰好就是这个维度——
/// 因此不需要为 WSL 单独引入一套执行抽象。
///
/// # Windows 上是 `qodercn.cmd`
///
/// 现网如此：npm 在 Windows 上装出的是 `.cmd` 包装脚本，直接调 `qodercn`
/// 会找不到可执行文件。这与平台有关，与目标是不是 WSL 无关——
/// **WSL 里跑的是 Linux 的 `qodercn`，不加 `.cmd`**。
#[must_use]
pub fn command_for(distro: Option<&str>, is_windows_local: bool) -> (String, Vec<String>) {
    match distro {
        Some(distro) => (
            "wsl.exe".to_owned(),
            vec![
                "-d".to_owned(),
                distro.to_owned(),
                "--".to_owned(),
                "qodercn".to_owned(),
                LIST_MODELS_ARG.to_owned(),
            ],
        ),
        None if is_windows_local => ("qodercn.cmd".to_owned(), vec![LIST_MODELS_ARG.to_owned()]),
        None => ("qodercn".to_owned(), vec![LIST_MODELS_ARG.to_owned()]),
    }
}

/// 执行并解析模型清单。
///
/// # 失败要给出原因
///
/// 命令不存在、非零退出、超时**都必须**返回可读的原因，而不是空列表——
/// 否则界面上「没有模型」与「命令没装」长得一模一样。这正是本工程反复踩到的
/// 「静默失败」形态。
///
/// # Errors
///
/// 命令不存在、超时、非零退出、或输出为空时返回原因。
pub fn list_models(
    runner: &dyn crate::cli_env::probe::CommandRunner,
    distro: Option<&str>,
    is_windows_local: bool,
) -> Result<Vec<QoderModel>, String> {
    let (program, args) = command_for(distro, is_windows_local);
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let run = runner.run(&program, &borrowed, TIMEOUT_MS);

    if run.exit_code.is_none() {
        return Err(format!("未找到 {program}。"));
    }
    if run.timed_out {
        return Err(format!("{program} 执行超时。"));
    }
    if run.exit_code != Some(0) {
        let detail = run.stderr.trim();
        return Err(if detail.is_empty() {
            format!("{program} 执行失败（退出码 {:?}）。", run.exit_code)
        } else {
            format!("{program} 执行失败：{detail}")
        });
    }

    let models = parse_model_list(&run.stdout);
    if models.is_empty() {
        // 退出码为 0 却没有任何模型：这是「事实」，不是错误——但要有别于
        // 「命令没装」，所以单独给一句话。
        return Err(format!("{program} 没有返回任何模型。"));
    }
    Ok(models)
}

/// 去掉 ANSI 转义序列。
///
/// 覆盖 CSI（`ESC [ ... 字母`）这一族——它是颜色、光标移动、清屏的共同编码。
/// 不处理 OSC 等更冷门的序列：`--list-models` 的输出里不会出现。
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            out.push(ch);
            continue;
        }
        // `ESC [` 开头：跳过参数字节，直到遇到终止字母（0x40..=0x7E）。
        if chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if ('\u{40}'..='\u{7e}').contains(&next) {
                    break;
                }
            }
        }
        // 单独的 `ESC`：丢掉即可。
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(output: &str) -> Vec<String> {
        parse_model_list(output).into_iter().map(|m| m.id).collect()
    }

    #[test]
    fn the_command_differs_by_target_and_platform() {
        // WSL 里跑 Linux 的 `qodercn`，不加 `.cmd`；Windows 本机才用 `.cmd`。
        let (program, args) = command_for(None, true);
        assert_eq!(program, "qodercn.cmd");
        assert_eq!(args, ["--list-models"]);

        let (program, _args) = command_for(None, false);
        assert_eq!(program, "qodercn");

        let (program, args) = command_for(Some("Ubuntu-24.04"), true);
        assert_eq!(program, "wsl.exe");
        assert_eq!(
            args,
            ["-d", "Ubuntu-24.04", "--", "qodercn", "--list-models"]
        );
    }

    /// 假执行器：记录调用并回放预设结果。
    struct FakeRunner {
        result: crate::cli_env::probe::CommandRun,
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl FakeRunner {
        fn new(result: crate::cli_env::probe::CommandRun) -> Self {
            Self {
                result,
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl crate::cli_env::probe::CommandRunner for FakeRunner {
        fn run(
            &self,
            program: &str,
            args: &[&str],
            _timeout_ms: u64,
        ) -> crate::cli_env::probe::CommandRun {
            self.seen
                .lock()
                .expect("lock")
                .push(format!("{program} {}", args.join(" ")));
            self.result.clone()
        }
    }

    fn run_with(stdout: &str, exit_code: Option<i32>) -> FakeRunner {
        FakeRunner::new(crate::cli_env::probe::CommandRun {
            stdout: stdout.to_owned(),
            stderr: String::new(),
            timed_out: false,
            exit_code,
        })
    }

    #[test]
    fn a_successful_run_returns_the_models() {
        let runner = run_with("MODEL\nqwen3-coder\n", Some(0));
        let models = list_models(&runner, None, false).expect("应当成功");
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "qwen3-coder");
    }

    #[test]
    fn a_missing_command_reports_which_program_was_missing() {
        // 「没有模型」与「命令没装」必须能区分。
        let runner = run_with("", None);
        let error = list_models(&runner, None, false).expect_err("必须失败");
        assert!(error.contains("未找到 qodercn"), "{error}");
    }

    #[test]
    fn a_nonzero_exit_surfaces_stderr() {
        // 诊断信息全在 stderr 上。
        let mut runner = run_with("", Some(1));
        runner.result.stderr = "not logged in".to_owned();
        let error = list_models(&runner, None, false).expect_err("必须失败");
        assert!(error.contains("not logged in"), "{error}");
    }

    #[test]
    fn a_timeout_is_reported_as_such() {
        let mut runner = run_with("", Some(0));
        runner.result.timed_out = true;
        let error = list_models(&runner, None, false).expect_err("必须失败");
        assert!(error.contains("超时"), "{error}");
    }

    #[test]
    fn a_successful_run_without_models_is_not_silent() {
        let runner = run_with("MODEL\n", Some(0));
        let error = list_models(&runner, None, false).expect_err("必须失败");
        assert!(error.contains("没有返回任何模型"), "{error}");
    }
    #[test]
    fn the_header_and_blank_lines_are_dropped() {
        let output = "MODEL\nqwen3-coder-plus\n\nqwen3-coder-flash\n";
        assert_eq!(ids(output), ["qwen3-coder-plus", "qwen3-coder-flash"]);
    }

    #[test]
    fn the_header_match_is_case_insensitive() {
        // 不能假设表头一定是大写。
        assert_eq!(ids("model\nqwen\n"), ["qwen"]);
        assert_eq!(ids("Model\nqwen\n"), ["qwen"]);
    }

    #[test]
    fn ansi_escape_sequences_are_stripped() {
        // 有颜色的终端会输出这些；不剥离就会得到带转义码的「模型名」。
        let output = "\u{1b}[1mMODEL\u{1b}[0m\n\u{1b}[32mqwen3-coder\u{1b}[0m\n";
        assert_eq!(ids(output), ["qwen3-coder"]);
    }

    #[test]
    fn carriage_returns_are_trimmed() {
        // Windows 输出会带 `\r`。
        assert_eq!(ids("MODEL\r\nqwen\r\n"), ["qwen"]);
    }

    #[test]
    fn a_model_merely_containing_the_header_word_is_kept() {
        // 只丢掉**整行等于**表头的情况；名字里含 MODEL 的模型必须保留。
        assert_eq!(ids("MODEL\nmodel-x\nmy-model\n"), ["model-x", "my-model"]);
    }

    #[test]
    fn order_is_preserved_and_duplicates_are_not_removed() {
        // 篡改顺序会让「与 CLI 输出一致」这条性质失效；去重是显示层的选择。
        assert_eq!(ids("b\na\nb\n"), ["b", "a", "b"]);
    }

    #[test]
    fn an_empty_output_yields_nothing() {
        assert!(parse_model_list("").is_empty());
        assert!(parse_model_list("\n\n\n").is_empty());
        assert!(parse_model_list("MODEL\n").is_empty());
    }

    #[test]
    fn a_lone_escape_character_does_not_eat_the_rest() {
        // 单独的 `ESC`（不成序列）只该丢掉自己。
        assert_eq!(ids("qwen\u{1b}\n"), ["qwen"]);
    }
}
