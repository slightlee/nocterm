//! 终端字节流到 UTF-8 文本的增量解码。
//!
//! PTY 与 SSH 通道的读取边界由内核缓冲和网络分片决定，与字符边界无关：一个 3 字节的
//! 汉字完全可能被 8 KiB 的读缓冲切成 2 + 1 两次返回。若每次读取都直接
//! `String::from_utf8_lossy`，被切断的那半个字符会**永久**变成 U+FFFD（`�`），
//! 中文输出、CJK 文件名与带框线的 TUI 界面都会在读边界处出现乱码。
//!
//! 因此把尾部"尚未读完的多字节序列"留到下一块再拼接，只对确实非法的字节做替换。
//! 这也是 xterm.js 自带的 UTF-8 解码器与 OpenSSH 客户端的做法：解码器跨读取保持状态。

/// 跨多次读取保持解码状态的 UTF-8 拼接器。
#[derive(Default)]
pub struct Utf8Stream {
    /// 上一块末尾被截断的多字节序列前缀；UTF-8 最长 4 字节，故至多驻留 3 字节。
    carry: Vec<u8>,
}

impl Utf8Stream {
    /// 拼接上次残留后解码本块字节，返回可直接发往前端的文本。
    ///
    /// 尾部不完整的序列不会被解码，而是留到下一次调用；确定非法的字节按
    /// U+FFFD 替换后继续解码其后内容，保证单个坏字节不会吞掉整块输出。
    pub fn push(&mut self, chunk: &[u8]) -> String {
        let bytes = if self.carry.is_empty() {
            // 常态路径：没有残留时不额外拷贝一次输入。
            std::borrow::Cow::Borrowed(chunk)
        } else {
            let mut joined = std::mem::take(&mut self.carry);
            joined.extend_from_slice(chunk);
            std::borrow::Cow::Owned(joined)
        };

        let mut text = String::with_capacity(bytes.len());
        let mut rest: &[u8] = &bytes;
        loop {
            match std::str::from_utf8(rest) {
                Ok(valid) => {
                    text.push_str(valid);
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    // 错误位置之前的部分已被校验为合法 UTF-8，可以安全取用。
                    text.push_str(std::str::from_utf8(&rest[..valid_up_to]).unwrap_or_default());
                    match error.error_len() {
                        // None 表示剩余字节只是某个多字节字符的合法前缀，被读边界截断了，
                        // 留到下一块拼接——这正是本模块存在的理由。
                        None => {
                            self.carry = rest[valid_up_to..].to_vec();
                            break;
                        }
                        // Some 表示这几个字节无论后续来什么都不可能合法（如 Latin-1 文本），
                        // 就地替换并继续解码，避免一个坏字节让整块输出丢失。
                        Some(length) => {
                            text.push(char::REPLACEMENT_CHARACTER);
                            rest = &rest[valid_up_to + length..];
                        }
                    }
                }
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::Utf8Stream;

    #[test]
    fn stitches_a_multibyte_character_split_across_reads() {
        let mut stream = Utf8Stream::default();
        let bytes = "中".as_bytes();
        // 读边界落在汉字中间：第一块不得产出任何字符，第二块补齐后整字出现。
        assert_eq!(stream.push(&bytes[..2]), "");
        assert_eq!(stream.push(&bytes[2..]), "中");
    }

    #[test]
    fn decodes_plain_ascii_without_buffering() {
        let mut stream = Utf8Stream::default();
        assert_eq!(stream.push(b"ls -al\r\n"), "ls -al\r\n");
    }

    #[test]
    fn replaces_bytes_that_can_never_be_valid_and_keeps_going() {
        let mut stream = Utf8Stream::default();
        // 0xFF 不是任何 UTF-8 序列的合法起始字节，只替换它本身，其后内容照常解码。
        let decoded = stream.push(b"a\xffb");
        assert_eq!(decoded, "a\u{fffd}b");
    }

    #[test]
    fn keeps_at_most_a_partial_sequence_pending() {
        let mut stream = Utf8Stream::default();
        // 连续两块都以不完整序列结尾时，残留必须逐块结转而不是累积或丢弃。
        let first = "早上好".as_bytes();
        assert_eq!(stream.push(&first[..4]), "早");
        assert_eq!(stream.push(&first[4..]), "上好");
    }
}
