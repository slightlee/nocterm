//! OSC 633 序列扫描器：从本地 PTY 的原始字节流中剥离 shell integration 信号，
//! 并把命令完成/提示符边界解析为结构化事件。
//!
//! 背景（#82）：AI 本地命令的完成检测从"可见文本流里找标记"迁移到 shell 主动
//! 上报的 OSC 633 控制序列。序列不参与终端折行，窄窗口下依旧可靠。扫描发生在
//! UTF-8 解码之前——序列可能被 PTY 分块从任意字节处切开，必须跨分块保持状态。

/// 从 633 序列解析出的集成事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Osc633Event {
    /// 633;D;<code>：上一条命令结束并携带真实退出码。
    CommandFinished(i64),
    /// 633;A：提示符开始（也用作"集成已激活"的检测信号）。
    PromptStarted,
}

/// 633 序列前缀；`\x1b]` 之后的部分用于匹配。
const OSC_PREFIX: &[u8] = b"]633;";
/// 单条 633 序列（含终止符）允许的最大长度，超过即按普通数据放行，防止垃圾字节无限驻留。
const MAX_SEQUENCE_BYTES: usize = 64;

/// 有状态的 633 扫描器：每个终端的读取线程持有一个实例。
#[derive(Default)]
pub(crate) struct Osc633Scanner {
    /// 尚无法判定归属的字节（可能是被分块切断的序列前半段）。
    pending: Vec<u8>,
}

impl Osc633Scanner {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 输入一个 PTY 分片，返回剥离 633 序列后的透传字节与解析出的事件。
    ///
    /// 非 633 的其它 OSC/CPI 序列原样透传；被切断的候选序列暂存到下一分片再判定。
    pub(crate) fn feed(&mut self, chunk: &[u8]) -> (Vec<u8>, Vec<Osc633Event>) {
        self.pending.extend_from_slice(chunk);
        let mut passthrough = Vec::new();
        let mut events = Vec::new();
        loop {
            let Some(start) = find_escape(&self.pending) else {
                // 没有 ESC：全部透传，清空暂存。
                passthrough.append(&mut self.pending);
                break;
            };
            // ESC 之前的字节按数据放行。
            if start > 0 {
                let head: Vec<u8> = self.pending.drain(..start).collect();
                passthrough.extend(head);
            }
            // 判定一次 633 序列需要 ESC + 完整前缀 `]633;` 共 6 个字节。
            if self.pending.len() < 1 + OSC_PREFIX.len() {
                // 剩余字节不足以判定：保留等待下一分片；超过单序列上限仍不足
                // 说明是垃圾流，放行一个字节后继续。
                if self.pending.len() >= MAX_SEQUENCE_BYTES {
                    passthrough.push(self.pending.remove(0));
                }
                break;
            }
            if !self.pending[1..].starts_with(OSC_PREFIX) {
                // ESC 后不是 633：透传 ESC，从下一字节继续扫描。
                passthrough.push(self.pending.remove(0));
                continue;
            }
            // 命中 633 前缀：在序列体内寻找终止符 BEL 或 ST（ESC \）。
            let body_start = 1 + OSC_PREFIX.len();
            let Some((end, terminator_length)) = find_terminator(&self.pending, body_start) else {
                if self.pending.len() >= MAX_SEQUENCE_BYTES {
                    // 超长仍无终止符：按垃圾数据放行整段，避免无限驻留。
                    passthrough.append(&mut self.pending);
                }
                break;
            };
            let body = self.pending[body_start..end].to_vec();
            self.pending.drain(..end + terminator_length);
            if let Some(event) = parse_event(&body) {
                events.push(event);
            }
        }
        (passthrough, events)
    }
}

/// 定位下一个 ESC 字节。
fn find_escape(data: &[u8]) -> Option<usize> {
    data.iter().position(|byte| *byte == 0x1b)
}

/// 从 `from` 起在序列体内寻找终止符：BEL（0x07）或 ST（ESC \）。返回位置与终止符长度。
fn find_terminator(data: &[u8], from: usize) -> Option<(usize, usize)> {
    for (index, byte) in data.iter().enumerate().skip(from) {
        match *byte {
            0x07 => return Some((index, 1)),
            0x1b if data.get(index + 1) == Some(&b'\\') => return Some((index, 2)),
            _ => {}
        }
    }
    None
}

/// 解析 `633;` 之后的事件体：单字母事件名，D 事件附带 `;退出码`。
fn parse_event(body: &[u8]) -> Option<Osc633Event> {
    let text = std::str::from_utf8(body).ok()?;
    match text.as_bytes().first() {
        Some(b'A') if text.len() == 1 => Some(Osc633Event::PromptStarted),
        Some(b'D') => {
            let code = text.get(2..)?.parse::<i64>().ok()?;
            Some(Osc633Event::CommandFinished(code))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Osc633Event, Osc633Scanner};

    fn feed_all(scanner: &mut Osc633Scanner, chunks: &[&[u8]]) -> (Vec<u8>, Vec<Osc633Event>) {
        let mut passthrough = Vec::new();
        let mut events = Vec::new();
        for chunk in chunks {
            let (data, mut chunk_events) = scanner.feed(chunk);
            passthrough.extend(data);
            events.append(&mut chunk_events);
        }
        (passthrough, events)
    }

    #[test]
    fn strips_completion_and_prompt_sequences_with_bel() {
        let mut scanner = Osc633Scanner::new();
        let (data, events) = feed_all(
            &mut scanner,
            &[b"before\x1b]633;D;7\x07after\x1b]633;A\x07tail"],
        );
        assert_eq!(data, b"beforeaftertail");
        assert_eq!(
            events,
            vec![Osc633Event::CommandFinished(7), Osc633Event::PromptStarted]
        );
    }

    #[test]
    fn accepts_st_terminator_and_negative_exit_codes() {
        let mut scanner = Osc633Scanner::new();
        let (data, events) = feed_all(&mut scanner, &[b"\x1b]633;D;-12\x1b\\ok"]);
        assert_eq!(data, b"ok");
        assert_eq!(events, vec![Osc633Event::CommandFinished(-12)]);
    }

    #[test]
    fn reassembles_sequences_split_across_chunks() {
        let mut scanner = Osc633Scanner::new();
        let (data, events) = feed_all(&mut scanner, &[b"out\x1b]6", b"33;D", b";0\x07", b"7rest"]);
        assert_eq!(data, b"out7rest");
        assert_eq!(events, vec![Osc633Event::CommandFinished(0)]);
    }

    #[test]
    fn passes_other_osc_and_csi_sequences_through_untouched() {
        let mut scanner = Osc633Scanner::new();
        let original = b"\x1b]0;title\x07\x1b[2Ktext\x1b]133;D;0\x07end";
        let (data, events) = feed_all(&mut scanner, &[original]);
        assert_eq!(data, original);
        assert!(events.is_empty());
    }

    #[test]
    fn flushes_oversized_garbage_without_losing_following_data() {
        let mut scanner = Osc633Scanner::new();
        // 伪装成 633 开头但永不终止的垃圾流，后跟真实数据。
        let garbage = [b"\x1b]633;", b"X".repeat(80).as_slice()].concat();
        let (data, events) = feed_all(&mut scanner, &[&garbage, b"real"]);
        assert!(data.ends_with(b"real"));
        assert!(data.contains(&0x1b));
        assert!(events.is_empty());
    }
}
