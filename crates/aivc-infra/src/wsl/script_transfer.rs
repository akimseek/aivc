//! 把脚本安全地送进 `wsl.exe`。
//!
//! # 为什么需要编码而不是直接传
//!
//! 脚本从进程到发行版内的 bash，要穿过**至少三层解析**：
//!
//! ```text
//! Rust 构造命令行
//!   → Windows 命令行解析（引号、反斜杠、`^`、`%` 都有特殊含义）
//!   → wsl.exe 转发
//!   → 发行版内 bash 再解析一次
//! ```
//!
//! 任意一层把脚本里的引号、`$`、换行、反引号吃掉或改写，结果**从「命令失败」
//! 到「bash 落进读 stdin 而永不返回」都可能**。
//!
//! 实测踩过：`列出 /home/akim/.codex/sessions` 花了 **30 秒**后以退出码 `-1` 失败，
//! 而同一条 `wsl.exe` 调用单独跑只要 0.1 秒。**卡住的就是这个。**
//!
//! # 做法：base64
//!
//! 编码后脚本变成**纯字母数字（加 `+` `/` `=`）的串**，三层解析都无从破坏它。
//! 发行版内先解回原文再交给 `bash`。
//!
//! 这与现网 `src/electron/providers/wsl.ts:102` 的做法一致：
//!
//! ```js
//! const encodedScript = Buffer.from(script, "utf8").toString("base64");
//! const shellScript = `printf %s ${shellQuote(encodedScript)} | base64 -d | bash`;
//! ```
//!
//! # 为什么自己写编码器
//!
//! 只用得到「编码」这一个方向，而它是不足二十行的纯计算。
//! 为它引入一个依赖，在当前环境（离线）反而拿不到。

/// Base64 字母表（RFC 4648 标准表）。
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// 把字节编码成 base64。
///
/// 标准表 + `=` 填充，与 `base64 -d` 期望的格式一致。
#[must_use]
pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = chunk.get(1).copied().map_or(0, u32::from);
        let b2 = chunk.get(2).copied().map_or(0, u32::from);
        let triple = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[((triple >> 18) & 0x3f) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3f) as usize] as char);
        // 最后一组不足 3 字节时补 `=`。
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 把 base64 载荷包成一条可在发行版内执行的单行脚本。
///
/// `printf %s <载荷> | base64 -d | bash`：
///
/// - 用 `printf %s` 而不是 `echo`：`echo` 在各 shell 对转义的处理不一致，
///   而 base64 字母表含 `+`（`echo` 通常无害，但没必要冒险）；
/// - 载荷**仍要加单引号**：字母表里的 `+` `/` `=` 在部分 shell 下是特殊字符；
/// - 载荷只含字母数字与 `+` `/` `=`，**不含单引号**，因此单引号包裹是安全的
///   ——这一点由 [`encode`] 的字母表保证，不依赖调用方。
#[must_use]
pub fn wrap_script(script: &str) -> String {
    format!(
        "printf %s '{}' | base64 -d | bash",
        encode(script.as_bytes())
    )
}

/// 把 base64 载荷包成一条**不占用 stdin** 的单行命令。
///
/// ```text
/// source <(printf %s '<载荷>' | base64 -d)
/// ```
///
/// # 为什么常驻进程必须用这一种
///
/// [`wrap_script`] 的形状是 `printf %s '<载荷>' | base64 -d | bash`——脚本是**从
/// stdin 读进去**的。对一次性执行这没问题（反正也没有别人再用那条 stdin），
/// 但对**常驻 worker 是致命的**：worker 的 stdin 就是帧协议通道，bash 会先把它读干
/// 当作脚本来源，于是 worker 的 `while read` 第一轮就撞上 EOF、**进程立刻退出**。
///
/// 实测症状正是 `worker 已退出（Ubuntu-24.04）`——每一次调用都退回一次性执行，
/// 比不开 worker 还慢（多付一次进程创建）。
///
/// # 为什么不是 `eval "$(...)"`
///
/// 那是最自然的写法，我实测过它**在生产路径上整个失效**（输出为空）：参数里的
/// **双引号**穿不过 `Windows 命令行 → wsl.exe → bash` 这三层解析。
/// 去掉双引号改成 `eval $(...)` 能用，但未加引号的命令替换会做**词分割**，脚本里的
/// `#` 注释会把后续全部内容一起注释掉——实测确认过这两种失败形状。
///
/// `source <(...)` 同时满足两个要求：参数里**没有双引号**，脚本正文以**文件**形式
/// 交给 `source`，因此既不做词分割、也不碰 stdin。
#[must_use]
pub fn wrap_script_preserving_stdin(script: &str) -> String {
    format!(
        "source <(printf %s '{}' | base64 -d)",
        encode(script.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoding_matches_the_rfc_examples() {
        // RFC 4648 的标准测试向量。
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn non_ascii_round_trips() {
        // 中文脚本必须逐字节还原——这是「命令在等 stdin」那类故障的根源。
        let source = "列出 /home/akim/.codex/sessions 失败";
        assert_eq!(
            encode(source.as_bytes()),
            "5YiX5Ye6IC9ob21lL2FraW0vLmNvZGV4L3Nlc3Npb25zIOWksei0pQ=="
        );
    }

    #[test]
    fn the_wrapper_never_contains_a_quote_from_the_payload() {
        // 包装用单引号包住载荷；载荷若含单引号会**提前闭合引号**，
        // 于是后面的一切都成了 shell 语法。字母表保证这不会发生——但要有断言钉住。
        for case in [
            "echo 'hi'",
            "printf \"a\\nb\"",
            "x=$HOME; rm -rf $x",
            "line1\nline2\n`id`",
            "\\backslash\\",
        ] {
            let wrapped = wrap_script(case);
            let payload = wrapped
                .strip_prefix("printf %s '")
                .and_then(|rest| rest.strip_suffix("' | base64 -d | bash"))
                .expect("包装形状");
            assert!(!payload.contains('\''), "载荷不该含单引号：{payload}");
            // 载荷必须是纯 base64 字符集。
            assert!(
                payload
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=')),
                "载荷含非 base64 字符：{payload}"
            );
        }
    }

    #[test]
    fn a_multiline_script_survives_as_a_single_line() {
        // 换行是三层解析里最先被吃掉的东西——编码后必须不再有换行。
        let wrapped = wrap_script("line1\nline2\n");
        assert!(!wrapped.contains('\n'), "包装结果必须是单行");
    }

    #[test]
    fn the_stdin_preserving_wrapper_never_feeds_bash_through_stdin() {
        // **这条测试守着一个真实故障。**
        //
        // 管道形状（`printf ... | base64 -d | bash`）把 bash 的 **stdin 当脚本读干**。
        // 对常驻 worker 这是致命的：它的 stdin 就是帧协议通道，于是 `while read`
        // 第一轮撞上 EOF，进程立刻退出——实测症状是
        // `worker 已退出（Ubuntu-24.04）`，每次调用都退回一次性执行。
        //
        // 因此这个形状里**不能出现 `| bash`**：脚本必须交给 `source` 从进程替换读。
        let source = "while read -r line; do echo \"$line\"; done";
        let wrapped = wrap_script_preserving_stdin(source);
        assert!(
            !wrapped.contains("| bash"),
            "不能把脚本喂给 bash 的 stdin（那会吃掉协议通道）：{wrapped}"
        );
        // 双引号穿不过三层解析——实测过它会整个失效。
        assert!(
            !wrapped.contains('"'),
            "参数里不能有双引号（实测会失效）：{wrapped}"
        );
        let payload = wrapped
            .strip_prefix("source <(printf %s '")
            .and_then(|rest| rest.strip_suffix("' | base64 -d)"))
            .expect("包装形状");
        assert_eq!(payload, encode(source.as_bytes()));
        assert!(
            payload
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '/' | '=')),
            "载荷含非 base64 字符：{payload}"
        );
        assert_eq!(
            wrapped.matches('\'').count(),
            2,
            "只应有包裹载荷的一对单引号"
        );
    }

    #[test]
    fn a_payload_containing_plus_and_slash_is_still_quoted() {
        // `+` 与 `/` 出现在字母表里；单引号包裹让它们在 shell 下无特殊含义。
        let wrapped = wrap_script("\u{ff}\u{ff}\u{ff}");
        // 断言**性质**而不是具体串：字母表含 `+` 与 `/`，它们必须在单引号内；
        // 具体编码值取决于 UTF-8 字节，钉死它只会让测试因无关原因失败。
        let payload = wrapped
            .strip_prefix("printf %s '")
            .and_then(|rest| rest.strip_suffix("' | base64 -d | bash"))
            .expect("包装形状");
        assert!(
            payload.chars().any(|ch| matches!(ch, '+' | '/')),
            "应当出现 + 或 /：{payload}"
        );
        assert_eq!(payload, encode("\u{ff}\u{ff}\u{ff}".as_bytes()));
    }
}
