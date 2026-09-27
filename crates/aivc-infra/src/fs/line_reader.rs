//! 本机文件的流式逐行读取。
//!
//! 会话 JSONL 可能达到几十 MB，因此必须逐行回调而不是整文件读入；回调返回
//! [`LineControl::Stop`] 可提前中止——分页与分支扫描都依赖该语义。
//!
//! # 两种起点
//!
//! - [`ReadStart::Line`]：从第 N 行开始。需要跳过前 N-1 行，是**线性**的。
//! - [`ReadStart::Byte`]：`seek` 到字节偏移直接定位，**O(1)**。同时携带绝对行号，
//!   因为行号无法从偏移推导，而它是消息的身份。
//!
//! 现网只有行号一种，因此大 JSONL 的分页定位始终是线性的。

use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

use aivc_core::error::Result;
use aivc_core::ports::{LineControl, ReadStart};

/// 同步逐行读取。必须在阻塞上下文（`block_in_place` 或阻塞线程）中调用。
///
/// 回调收到的行**不含**行尾的 `\n` / `\r\n`；空行会照常回调（解析器自行忽略）。
/// 文件提前结束时直接返回，不视为错误。
///
/// 回调同时收到该行的**绝对字节偏移**：分页锚点要能被 `seek` 直接定位，就必须记录
/// 绝对偏移，而只有这里知道每一行的长度。
pub fn read_lines_blocking(
    path: &Path,
    start: ReadStart,
    sink: &mut dyn FnMut(&str, u64, u64) -> LineControl,
) -> Result<()> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);

    let mut line_number = start.line_number().max(1);
    let mut byte_offset;
    if let Some(offset) = start.byte_offset() {
        reader.seek(SeekFrom::Start(offset))?;
        byte_offset = offset;
    } else {
        // 行号起点：只能线性跳过，同时累计跳过的字节数——跳过之后才知道起始行的
        // 绝对字节偏移。
        byte_offset = 0;
        let mut skipped = 1u64;
        let mut discard = Vec::new();
        while skipped < line_number {
            discard.clear();
            let read = reader.read_until(b'\n', &mut discard)?;
            if read == 0 {
                // 文件比请求的起始行还短：没有可读内容。
                return Ok(());
            }
            byte_offset += read as u64;
            skipped += 1;
        }
    }

    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        let read = reader.read_until(b'\n', &mut buffer)?;
        if read == 0 {
            break;
        }
        // 合法 UTF-8 时零拷贝；非法序列按替换字符处理（与 Node 的 utf8 解码一致）。
        let text = String::from_utf8_lossy(&buffer);
        let line = text.trim_end_matches(['\n', '\r']);
        if sink(line, line_number, byte_offset) == LineControl::Stop {
            break;
        }
        byte_offset += read as u64;
        line_number += 1;
    }
    Ok(())
}

/// 读取文件头部至多 `max_bytes` 字节。
///
/// 用于列表页的轻量解析：只读前 64 KB 就能拿到 `session_meta` 与前几条消息。
pub fn read_head_blocking(path: &Path, max_bytes: usize) -> Result<String> {
    use std::io::Read;
    let mut file = File::open(path)?;
    let mut buffer = vec![0u8; max_bytes];
    let mut filled = 0usize;
    loop {
        if filled >= max_bytes {
            break;
        }
        let read = file.read(&mut buffer[filled..])?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    buffer.truncate(filled);
    // 头部末尾可能截断在多字节字符中间，`from_utf8_lossy` 会安全地替换。
    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_file(dir: &tempfile::TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        let mut file = File::create(&path).expect("create");
        file.write_all(content.as_bytes()).expect("write");
        path
    }

    fn collect(path: &Path, start: ReadStart) -> Vec<(u64, String)> {
        let mut lines = Vec::new();
        read_lines_blocking(path, start, &mut |line, number, _offset| {
            lines.push((number, line.to_owned()));
            LineControl::Continue
        })
        .expect("read");
        lines
    }

    #[test]
    fn reads_every_line_with_one_based_numbers() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\ntwo\nthree\n");
        assert_eq!(
            collect(&path, ReadStart::BEGINNING),
            vec![
                (1, "one".to_owned()),
                (2, "two".to_owned()),
                (3, "three".to_owned())
            ]
        );
    }

    #[test]
    fn handles_crlf_and_a_missing_final_newline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\r\ntwo\r\nthree");
        assert_eq!(
            collect(&path, ReadStart::BEGINNING),
            vec![
                (1, "one".to_owned()),
                (2, "two".to_owned()),
                (3, "three".to_owned())
            ]
        );
    }

    #[test]
    fn empty_lines_are_reported_so_the_parser_can_skip_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\n\ntwo\n");
        assert_eq!(
            collect(&path, ReadStart::BEGINNING),
            vec![
                (1, "one".to_owned()),
                (2, String::new()),
                (3, "two".to_owned())
            ]
        );
    }

    #[test]
    fn line_start_skips_the_requested_prefix() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\ntwo\nthree\nfour\n");
        assert_eq!(
            collect(&path, ReadStart::Line(3)),
            vec![(3, "three".to_owned()), (4, "four".to_owned())]
        );
        // 从第 1 行开始等价于文件开头。
        assert_eq!(collect(&path, ReadStart::Line(1)).len(), 4);
    }

    #[test]
    fn line_start_beyond_the_end_yields_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\ntwo\n");
        assert!(collect(&path, ReadStart::Line(99)).is_empty());
    }

    #[test]
    fn byte_start_seeks_directly_and_reports_absolute_line_numbers() {
        let dir = tempfile::tempdir().expect("tempdir");
        // 每行 4 字节（"aaa\n"），第 3 行从偏移 8 开始。
        let path = write_file(&dir, "a.jsonl", "aaa\nbbb\nccc\nddd\n");
        let lines = collect(
            &path,
            ReadStart::Byte {
                offset: 8,
                line_number: 3,
            },
        );
        assert_eq!(
            lines,
            vec![(3, "ccc".to_owned()), (4, "ddd".to_owned())],
            "字节定位后行号必须是绝对行号，不能从 1 重新计数"
        );
    }

    #[test]
    fn byte_start_at_the_file_start_reads_everything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\ntwo\n");
        assert_eq!(
            collect(
                &path,
                ReadStart::Byte {
                    offset: 0,
                    line_number: 1
                }
            )
            .len(),
            2
        );
    }

    #[test]
    fn stopping_early_does_not_read_the_rest_of_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "one\ntwo\nthree\nfour\n");

        let mut seen = Vec::new();
        read_lines_blocking(&path, ReadStart::BEGINNING, &mut |line, number, _offset| {
            seen.push((number, line.to_owned()));
            if number >= 2 {
                LineControl::Stop
            } else {
                LineControl::Continue
            }
        })
        .expect("read");
        assert_eq!(
            seen,
            vec![(1, "one".to_owned()), (2, "two".to_owned())],
            "提前停止后不得继续读取"
        );
    }

    #[test]
    fn byte_offsets_are_absolute_and_match_the_source() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content = "aaa\nbbbb\ncc\n";
        let path = write_file(&dir, "a.jsonl", content);

        let mut seen = Vec::new();
        read_lines_blocking(&path, ReadStart::BEGINNING, &mut |line, number, offset| {
            seen.push((number, offset, line.to_owned()));
            LineControl::Continue
        })
        .expect("read");

        assert_eq!(
            seen,
            vec![
                (1, 0, "aaa".to_owned()),
                (2, 4, "bbbb".to_owned()),
                (3, 9, "cc".to_owned()),
            ]
        );
        // 每个偏移都必须真的指向该行行首。
        for (_, offset, line) in &seen {
            assert!(
                content[*offset as usize..].starts_with(line.as_str()),
                "偏移 {offset} 未对齐行首"
            );
        }
    }

    #[test]
    fn byte_offsets_from_a_line_start_are_still_absolute() {
        // 按行号起读时要线性跳过前缀，但回调收到的偏移仍必须是绝对偏移，
        // 否则基于它建立的锚点会 seek 到错误位置。
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "aaa\nbbbb\ncc\n");

        let mut seen = Vec::new();
        read_lines_blocking(&path, ReadStart::Line(2), &mut |line, number, offset| {
            seen.push((number, offset, line.to_owned()));
            LineControl::Continue
        })
        .expect("read");
        assert_eq!(
            seen,
            vec![(2, 4, "bbbb".to_owned()), (3, 9, "cc".to_owned())]
        );
    }

    #[test]
    fn byte_offsets_from_a_byte_start_continue_from_that_offset() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "aaa\nbbbb\ncc\n");

        let mut seen = Vec::new();
        read_lines_blocking(
            &path,
            ReadStart::Byte {
                offset: 4,
                line_number: 2,
            },
            &mut |line, number, offset| {
                seen.push((number, offset, line.to_owned()));
                LineControl::Continue
            },
        )
        .expect("read");
        assert_eq!(
            seen,
            vec![(2, 4, "bbbb".to_owned()), (3, 9, "cc".to_owned())]
        );
    }

    #[test]
    fn multibyte_content_is_decoded_correctly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", "中文一\n中文二\n");
        assert_eq!(
            collect(&path, ReadStart::BEGINNING),
            vec![(1, "中文一".to_owned()), (2, "中文二".to_owned())]
        );
    }

    #[test]
    fn reading_a_missing_file_reports_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outcome = read_lines_blocking(
            &dir.path().join("missing.jsonl"),
            ReadStart::BEGINNING,
            &mut |_, _, _| LineControl::Continue,
        );
        assert!(outcome.is_err());
    }

    #[test]
    fn read_head_stops_at_the_requested_size() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", &"x".repeat(1_000));
        assert_eq!(read_head_blocking(&path, 64).expect("head").len(), 64);
        // 请求超过文件大小时返回全部内容。
        assert_eq!(read_head_blocking(&path, 4_096).expect("head").len(), 1_000);
    }

    #[test]
    fn read_head_truncating_multibyte_text_does_not_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_file(&dir, "a.jsonl", &"中".repeat(100));
        // 3 字节字符被切成 4 字节边界时不得 panic。
        let head = read_head_blocking(&path, 4).expect("head");
        assert!(!head.is_empty());
    }
}
