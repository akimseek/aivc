//! WSL 输出的解码与解析。
//!
//! `wsl.exe` 的输出编码**不固定**：`wsl -l -q` 返回 UTF-16LE（Windows 工具链的默认
//! 宽字符输出），而发行版内命令的输出是 UTF-8。现网的做法是两种都解一遍，取「去掉
//! NUL 后更长」的那个——这个启发式在实践中有效，因此保留。

use aivc_core::session::SessionFile;

/// 解码 `wsl.exe` 的输出。
///
/// # 判定顺序
///
/// 1. **优先 UTF-8**：能无错解码且不含 NUL 时，几乎可以确定就是 UTF-8。
/// 2. 否则按 **UTF-16LE** 解码（`wsl.exe -l -q` 的输出形式）。
///
/// # 相对现网的改进
///
/// 现网的做法是两种都解一遍、取「去掉 NUL 后更长」的那个。该启发式在纯 ASCII 上成立，
/// 但**纯中文的 UTF-8 输出按 UTF-16LE 误读后字符数反而更多**（3 字节 UTF-8 序列被
/// 当成 1.5 个 UTF-16 码元，长度对比失真），于是现网会选错。按「是否合法 UTF-8 且无
/// NUL」判定既简单又不会错。
#[must_use]
pub fn decode_wsl_output(output: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(output)
        && !text.contains('\0')
    {
        return text.to_owned();
    }
    decode_utf16_le(output)
}

/// 按 UTF-16LE 解码字节序列（奇数长度时丢弃末尾单字节）。
fn decode_utf16_le(output: &[u8]) -> String {
    let units: Vec<u16> = output
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    // 去掉 UTF-16 解码后残留的 NUL（`wsl -l -q` 会在每行末尾带一个）。
    String::from_utf16_lossy(&units).replace('\0', "")
}

/// 解析 `find ... -printf '%p\t%T@\t%C@\t%s\n'` 的输出为会话文件列表。
///
/// 同时兼容两种列数：
///
/// - **4 列**（当前）：`路径`、`mtime`、`ctime`、`大小`；
/// - **3 列**（旧版 `find` 缺少 `%C@`）：`路径`、`mtime`、`大小`。
///
/// 残缺或非法行直接丢弃，而不是让整次扫描失败——单个文件的异常不应让会话列表打不开。
#[must_use]
pub fn parse_wsl_session_file_list(stdout: &str) -> Vec<SessionFile> {
    stdout
        .split('\n')
        // 只去掉行尾的 `\r`（CRLF），**不能**整行 trim：
        // 整行 trim 会把「以制表符开头的空路径」这一列的边界信息吃掉，
        // 让 `\t1\t2\t10` 被误读成合法的 3 列记录。
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .filter_map(parse_wsl_session_file_line)
        .collect()
}

/// 解析单行。
fn parse_wsl_session_file_line(line: &str) -> Option<SessionFile> {
    let mut columns: Vec<&str> = line.split('\t').collect();
    // 去掉尾部空列（`find` 输出可能以制表符结尾），但不影响中间的空列。
    while columns.len() > 1 && columns.last().is_some_and(|column| column.is_empty()) {
        columns.pop();
    }

    let (file_path, mtime, changed, size) = match columns.len() {
        // 当前格式：路径、mtime、ctime、大小。
        4 => (columns[0], columns[1], columns[2], columns[3]),
        // 旧版 `find` 缺少 `%C@`：路径、mtime、大小，ctime 回退为 mtime。
        3 => (columns[0], columns[1], columns[1], columns[2]),
        _ => return None,
    };

    let mtime_ms = mtime.trim().parse::<f64>().ok()? * 1000.0;
    let change_ms = changed.trim().parse::<f64>().ok()? * 1000.0;
    let size = size.trim().parse::<f64>().ok()?;

    if file_path.is_empty() || !mtime_ms.is_finite() || !change_ms.is_finite() || !size.is_finite()
    {
        return None;
    }

    Some(SessionFile {
        file_path: file_path.to_owned(),
        mtime_ms,
        change_ms: Some(change_ms),
        // 文件大小为负或超出 u64 时按 0 处理（防御性，正常输入不会发生）。
        size: to_size(size),
    })
}

/// 把 find 报出的大小转为 u64。
///
/// 已确认 size 有限；负值按 0 处理，因此截断转换不会丢符号。
#[allow(clippy::cast_sign_loss)]
fn to_size(size: f64) -> u64 {
    if size < 0.0 { 0 } else { size as u64 }
}

#[cfg(test)]
mod tests {
    // 文件指纹是精确键值，断言必须用精确比较。
    #![allow(clippy::float_cmp)]

    use super::*;

    #[test]
    fn decodes_utf16le_output_from_windows_tools() {
        // `wsl -l -q` 的典型输出：UTF-16LE + 行尾 NUL。
        let mut bytes = Vec::new();
        for unit in "Ubuntu\0\r\0\n\0".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let decoded = decode_wsl_output(&bytes);
        assert!(decoded.contains("Ubuntu"), "解码结果：{decoded:?}");
    }

    #[test]
    fn decodes_utf8_output_from_inside_the_distro() {
        let decoded = decode_wsl_output("/home/akim/.codex/sessions\n".as_bytes());
        assert_eq!(decoded.trim(), "/home/akim/.codex/sessions");
    }

    #[test]
    fn decoding_prefers_utf8_when_it_is_valid_and_nul_free() {
        // 纯中文 UTF-8 按 UTF-16LE 误读后字符数反而更多，现网「取更长者」的启发式
        // 会在这里选错；按「合法 UTF-8 且无 NUL」判定则不会。
        let text = "会话文件";
        assert_eq!(decode_wsl_output(text.as_bytes()), text);
    }

    #[test]
    fn decoding_falls_back_to_utf16_for_non_utf8_input() {
        // UTF-16LE 中文：字节序列不是合法 UTF-8，必须走 UTF-16 分支。
        let mut bytes = Vec::new();
        for unit in "会话文件".encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_wsl_output(&bytes), "会话文件");
    }

    #[test]
    fn decoding_empty_and_odd_length_input_does_not_panic() {
        assert_eq!(decode_wsl_output(b""), "");
        // 奇数长度：末尾单字节被丢弃。
        let _ = decode_wsl_output(&[0x41, 0x00, 0x42]);
    }

    #[test]
    fn parses_four_column_find_output() {
        let stdout = "/home/a/.codex/sessions/x.jsonl\t1700000000.5\t1700000001.25\t1234\n";
        let files = parse_wsl_session_file_list(stdout);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_path, "/home/a/.codex/sessions/x.jsonl");
        assert_eq!(files[0].mtime_ms, 1_700_000_000_500.0);
        assert_eq!(files[0].change_ms, Some(1_700_000_001_250.0));
        assert_eq!(files[0].size, 1234);
    }

    #[test]
    fn parses_legacy_three_column_output_with_mtime_fallback() {
        // 旧版 find 缺少 %C@：第三列是大小，ctime 回退为 mtime。
        let stdout = "/home/a/sessions/y.jsonl\t1700000000.5\t999\n";
        let files = parse_wsl_session_file_list(stdout);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].mtime_ms, 1_700_000_000_500.0);
        assert_eq!(files[0].change_ms, Some(1_700_000_000_500.0));
        assert_eq!(files[0].size, 999);
    }

    #[test]
    fn parses_multiple_lines_and_crlf() {
        let stdout = concat!(
            "/a.jsonl\t1\t2\t10\r\n",
            "/b.jsonl\t3\t4\t20\r\n",
            "\n",
            "   \n",
        );
        let files = parse_wsl_session_file_list(stdout);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].file_path, "/a.jsonl");
        assert_eq!(files[1].file_path, "/b.jsonl");
    }

    #[test]
    fn discards_malformed_lines_without_failing_the_scan() {
        let stdout = concat!(
            "/ok.jsonl\t1\t2\t10\n",
            "no-tabs-at-all\n",
            "/bad-mtime.jsonl\tnotanumber\t2\t10\n",
            "only-one-column\n",
            "/ok2.jsonl\t5\t6\t30\n",
        );
        let files = parse_wsl_session_file_list(stdout);
        let paths: Vec<&str> = files.iter().map(|file| file.file_path.as_str()).collect();
        assert_eq!(paths, vec!["/ok.jsonl", "/ok2.jsonl"]);
    }

    #[test]
    fn discards_lines_with_an_empty_path() {
        // 不能整行 trim：那会把「以制表符开头的空路径」这一列的边界信息吃掉，
        // 让这行被误读成合法的 3 列记录。
        let files = parse_wsl_session_file_list("\t1\t2\t10\n");
        assert!(files.is_empty(), "空路径必须被拒绝：{files:?}");
    }

    #[test]
    fn trailing_tab_columns_are_tolerated() {
        // `find` 输出可能以制表符结尾，尾部空列应被忽略而不是让整行失效。
        let files = parse_wsl_session_file_list("/a.jsonl\t1\t2\t10\t\n");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].size, 10);
    }

    #[test]
    fn empty_output_yields_no_files() {
        assert!(parse_wsl_session_file_list("").is_empty());
        assert!(parse_wsl_session_file_list("\n\n").is_empty());
    }

    #[test]
    fn paths_with_spaces_are_preserved() {
        // `-printf` 用制表符分隔，因此路径里的空格不会被误切。
        let stdout = "/home/a/my sessions/x.jsonl\t1\t2\t10\n";
        let files = parse_wsl_session_file_list(stdout);
        assert_eq!(files[0].file_path, "/home/a/my sessions/x.jsonl");
    }
}
