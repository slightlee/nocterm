//! Provider JSONL 输出的统一有界读取器。
//! 子进程输出是不可信输入：读取失败、非法 UTF-8 或超长单行都必须显式失败，不能伪装成 EOF。

use std::io::{BufRead, BufReader, Read};

/// 单条 Provider 协议消息的硬上限。正常流式事件远小于该值，同时允许较大的工具结果事件。
pub const MAX_PROVIDER_LINE_BYTES: usize = 1024 * 1024;

/// 逐行读取 Provider 输出，并在分配超过上限前停止。
pub fn read_bounded_lines(
    reader: impl Read,
    mut on_line: impl FnMut(String) -> bool,
) -> Result<(), String> {
    let mut reader = BufReader::new(reader);
    loop {
        let mut bytes = Vec::new();
        // `take` 限制 read_until 的本次分配；额外两个字节容纳 CRLF。
        let read = reader
            .by_ref()
            .take((MAX_PROVIDER_LINE_BYTES + 2) as u64)
            .read_until(b'\n', &mut bytes)
            .map_err(|error| format!("读取 AI Provider 输出失败：{error}"))?;
        if read == 0 {
            return Ok(());
        }

        let terminated = bytes.last() == Some(&b'\n');
        if terminated {
            bytes.pop();
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        if bytes.len() > MAX_PROVIDER_LINE_BYTES || (!terminated && read > MAX_PROVIDER_LINE_BYTES)
        {
            return Err(format!(
                "AI Provider 单条输出超过 {} KiB 上限",
                MAX_PROVIDER_LINE_BYTES / 1024
            ));
        }
        let line =
            String::from_utf8(bytes).map_err(|_| "AI Provider 输出不是有效 UTF-8".to_string())?;
        if !on_line(line) {
            return Ok(());
        }
        if !terminated {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read};

    use super::{MAX_PROVIDER_LINE_BYTES, read_bounded_lines};

    #[test]
    fn reads_crlf_and_final_unterminated_line() {
        let mut lines = Vec::new();
        read_bounded_lines(&b"one\r\ntwo"[..], |line| {
            lines.push(line);
            true
        })
        .unwrap();
        assert_eq!(lines, ["one", "two"]);
    }

    #[test]
    fn rejects_overlong_and_invalid_utf8_lines() {
        let overlong = vec![b'x'; MAX_PROVIDER_LINE_BYTES + 1];
        assert!(read_bounded_lines(&overlong[..], |_| true).is_err());
        assert!(read_bounded_lines(&[0xff, b'\n'][..], |_| true).is_err());
    }

    #[test]
    fn surfaces_reader_errors() {
        struct FailedReader;
        impl Read for FailedReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("broken pipe"))
            }
        }

        let error = read_bounded_lines(FailedReader, |_| true).expect_err("reader error");
        assert!(error.contains("broken pipe"));
    }
}
