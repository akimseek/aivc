//! 实验：Windows 上 `cmd.exe /c <批处理>` 的引号到底怎么走。
//!
//! # 为什么要有这个实验
//!
//! 本机 Windows 的 Codex 终端起不来，报 `unexpected argument '"model_provider=^"akim_gateway""'`。
//! 三层引号叠在一起：我们的 `cmd_quote` → `portable_pty` 的 MSVC 引号 → `cmd.exe` 解析，
//! 而 `codex.cmd` 是个 npm shim，会把 `%*` **原样转发**给 node，再由 **node 的 CRT** 解析一次。
//! 共四层。推理到第三层就已经不可靠了，所以这里**实测**。
//!
//! # 为什么这样测才算数
//!
//! `probe.cmd` 与 `codex.cmd` **同构**（`node "%~dp0probe.js" %*`），`probe.js` 把
//! `process.argv` 原样写进文件。因此读到的 JSON **就是** codex.js 会收到的东西——
//! 不是我们对 cmd 行为的猜测。
//!
//! 结果写进文件而不是读 PTY 输出：ConPTY 的输出要应答光标查询才会推进，
//! 而这里要看的只是「子进程收到了什么」，不必绕那一圈。
//!
//! # 这是实验，不是断言
//!
//! 它只打印各种形式的实际结果，不断言——因为「正确形式」正是它要回答的问题。
//! 确定之后再把它变成 `command.rs` 里的正式断言。
//!
//! 运行：`cargo test -p aivc-ipc --test cmd_quoting_probe -- --nocapture`

// 集成测试是独立 crate，`clippy.toml` 的测试豁免不覆盖 `tests/` 目录。
#![allow(clippy::panic, clippy::expect_used)]

use std::path::Path;

use aivc_core::shell::build_cmd_command;
use aivc_terminal::command::TerminalCommand;
use aivc_terminal::session::PtySession;

/// 命令行构造形式。
#[derive(Debug, Clone, Copy)]
enum Form {
    /// 现状：整条命令行（已按 cmd 规则引号化）作为**一个**参数交给 portable-pty。
    WholeString,
    /// 备选：程序与每个参数**分开**给，由 portable-pty 自己引号化。
    SeparateArgs,
}

/// 按 MSVC/CRT argv 规则引号化一个参数。
///
/// 这是 `portable_pty` 的 `append_quoted`（`cmdbuilder.rs:702`）的**正向**实现，
/// 也就是 `node.exe` 的 CRT 会如何解析。用它来算「正确的 `%*` 应当长什么样」：
/// 该算法设计成可往返，所以只要 `%*` 等于这里算出的结果，CRT 就会还原出原参数。
fn msvc_quote(argument: &str) -> String {
    let needs_quotes = argument.is_empty()
        || argument
            .chars()
            .any(|c| matches!(c, ' ' | '\t' | '\n' | '\u{b}' | '"'));
    if !needs_quotes {
        return argument.to_owned();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for character in argument.chars() {
        match character {
            '\\' => {
                backslashes += 1;
                out.push('\\');
            }
            '"' => {
                // 引号前的反斜杠要加倍，引号自身也要转义。
                for _ in 0..=backslashes {
                    out.push('\\');
                }
                backslashes = 0;
                out.push('"');
            }
            other => {
                backslashes = 0;
                out.push(other);
            }
        }
    }
    for _ in 0..backslashes {
        out.push('\\');
    }
    out.push('"');
    out
}

/// 期望的 `%*`：每个参数按 CRT 规则引号化后以空格相连。
fn expected_raw(args: &[String]) -> String {
    args.iter()
        .map(|argument| msvc_quote(argument))
        .collect::<Vec<_>>()
        .join(" ")
}

/// 跑一种形式，返回批处理实际收到的 `%*`。
async fn run(dir: &Path, probe: &str, args: &[String], form: Form, label: &str) -> String {
    let out = dir.join(format!("argv-{label}.txt"));
    let _ = std::fs::remove_file(&out);

    let (file, args) = match form {
        Form::WholeString => (
            "cmd.exe".to_owned(),
            vec![
                "/d".to_owned(),
                "/s".to_owned(),
                "/c".to_owned(),
                build_cmd_command(probe, args),
            ],
        ),
        Form::SeparateArgs => {
            let mut all = vec![
                "/d".to_owned(),
                "/s".to_owned(),
                "/c".to_owned(),
                probe.to_owned(),
            ];
            all.extend_from_slice(args);
            ("cmd.exe".to_owned(), all)
        }
    };

    let mut env = std::collections::BTreeMap::new();
    env.insert("PROBE_OUT".to_owned(), out.to_string_lossy().into_owned());

    let command = TerminalCommand {
        file,
        args,
        cwd: dir.to_string_lossy().into_owned(),
        env,
        remove_env: std::collections::BTreeSet::new(),
    };

    // 保住会话：`PtySession` 一被丢弃，PTY 就关了，子进程可能来不及写文件。
    let (mut session, mut output) = PtySession::spawn(&command, 120, 30).expect("spawn");

    // **必须替 xterm.js 应答 ConPTY 的光标查询（DSR）。**
    // 不应答时 ConPTY 不推进会话，子进程连第一步都走不完——实验会一直空手而归。
    let mut seen: Vec<u8> = Vec::new();
    for _ in 0..80 {
        while let Ok(chunk) = output.try_recv() {
            seen.extend_from_slice(&chunk);
        }
        if seen.windows(4).any(|window| window == b"\x1b[6n") {
            let _ = session.write("\u{1b}[1;1R");
            seen.clear();
        }
        if let Ok(text) = std::fs::read_to_string(&out) {
            return text.trim_end().to_owned();
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    "<没有写出结果，可能子进程没跑起来>".to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn what_does_the_target_actually_receive() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();

    // 与 codex.cmd 同构的关键点只有一个：**批处理把 %* 原样往下传**。
    // 这里把 %* 写进文件，于是看到的就是 codex.cmd 会转给 node 的那串文本。
    std::fs::write(
        root.join("probe.cmd"),
        "@ECHO off\r\n>\"%PROBE_OUT%\" echo %*\r\n",
    )
    .expect("probe.cmd");

    let probe = root.join("probe.cmd").to_string_lossy().into_owned();
    // 与真实路由配置同形：值里带内嵌双引号，且含 `:` 与 `/`。
    let args = vec![
        "model_provider=\"akim_gateway\"".to_owned(),
        "model_providers.akim_gateway.base_url=\"http://127.0.0.1:1234/gateway/codex/r1\""
            .to_owned(),
        "--version".to_owned(),
    ];

    let whole = run(root, &probe, &args, Form::WholeString, "whole").await;
    let separate = run(root, &probe, &args, Form::SeparateArgs, "separate").await;

    // 用断言而不是打印：`--nocapture` 会被 PowerShell 吞掉 `--` 分隔符，
    // 而断言失败时两侧的值都会出现在失败信息里。
    assert_eq!(
        separate,
        expected_raw(&args),
        "C 形式（程序与参数分开交给 PTY）没有给出可供 CRT 还原的文本。\n\
         A 形式（现状：整条命令作为一个参数）实际得到 = {whole}"
    );
}
