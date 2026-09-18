//! Provider JSONL 输出的统一有界读取器。
//! 子进程输出是不可信输入：读取失败、非法 UTF-8 或超长单行都必须显式失败，不能伪装成 EOF。

use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    sync::{Condvar, Mutex, mpsc},
    time::Duration,
};

/// 单条 Provider 协议消息的硬上限。正常流式事件远小于该值，同时允许较大的工具结果事件。
pub const MAX_PROVIDER_LINE_BYTES: usize = 1024 * 1024;
/// 单轮累计输出上限同时覆盖 stdout 与 stderr，防止 Provider 淹没 Tauri 事件队列和前端内存。
pub const MAX_PROVIDER_TURN_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE: &str =
    "AI Provider 单轮输出超过 4096 KiB 上限，已停止当前任务";
const MAX_STARTUP_DIAGNOSTIC_BYTES: usize = 16 * 1024;
const PROVIDER_EVENT_QUEUE_CAPACITY: usize = 128;
const MAX_STARTUP_DIAGNOSTIC_LINES: usize = PROVIDER_EVENT_QUEUE_CAPACITY;

/// Provider 读取线程通过固定容量队列向协议线程施加背压。
pub fn provider_event_channel<T>() -> (mpsc::SyncSender<T>, mpsc::Receiver<T>) {
    mpsc::sync_channel(PROVIDER_EVENT_QUEUE_CAPACITY)
}

/// 为单轮输出预留预算；失败时保持累计值不变，便于调用方只报告一次超限错误。
pub fn reserve_turn_output(total: &mut usize, bytes: usize) -> bool {
    // 空行仍会形成一次 IPC/前端事件，至少计一个字节，避免零长度事件绕过累计上限。
    let Some(next) = total.checked_add(bytes.max(1)) else {
        return false;
    };
    if next > MAX_PROVIDER_TURN_OUTPUT_BYTES {
        return false;
    }
    *total = next;
    true
}

/// 从进程创建开始排空 stderr，避免握手期间管道写满；成功后可无缝转交给会话读线程。
#[derive(Default)]
pub struct ProviderStderrRelay {
    state: Mutex<StderrRelayState>,
    completion: Condvar,
}

/// 在保留主错误的同时附加有限的 Provider 启动诊断。
pub fn append_startup_diagnostics(error: String, relay: &ProviderStderrRelay) -> String {
    match relay.startup_diagnostics() {
        Some(diagnostics) => format!("{error}\nProvider 诊断：\n{diagnostics}"),
        None => error,
    }
}

/// Bridge Token 只允许短暂存在于进程环境中；进入 IPC 输出前统一替换为占位符。
pub fn redact_token(mut text: String, token: Option<&str>) -> String {
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        text = text.replace(token, "[REDACTED]");
    }
    text
}

#[derive(Default)]
struct StderrRelayState {
    buffered: VecDeque<Result<String, String>>,
    buffered_bytes: usize,
    truncated: bool,
    consumer: Option<mpsc::SyncSender<Result<String, String>>>,
    completed: bool,
}

impl ProviderStderrRelay {
    /// 本方法在专用线程中运行；读取结束会关闭后续 attach 返回的 channel。
    pub fn capture(&self, reader: impl Read) {
        let result = read_bounded_lines(reader, |line| self.push(Ok(line)));
        if let Err(error) = result {
            self.push(Err(error));
        }
        if let Ok(mut state) = self.state.lock() {
            state.completed = true;
            state.consumer.take();
            self.completion.notify_all();
        }
    }

    /// 握手成功后转交已缓存及未来的诊断；调用方必须持续消费到 channel 关闭。
    pub fn attach(&self) -> mpsc::Receiver<Result<String, String>> {
        let (sender, receiver) = provider_event_channel();
        let Ok(mut state) = self.state.lock() else {
            return receiver;
        };
        while let Some(line) = state.buffered.pop_front() {
            let _ = sender.send(line);
        }
        state.buffered_bytes = 0;
        if !state.completed {
            state.consumer = Some(sender);
        }
        receiver
    }

    /// 启动失败时返回最近的有限诊断，防止把无界 Provider 输出拼入 IPC 错误。
    pub fn startup_diagnostics(&self) -> Option<String> {
        let state = self.state.lock().ok()?;
        if state.buffered.is_empty() {
            return None;
        }
        let mut lines = state
            .buffered
            .iter()
            .map(|line| match line {
                Ok(line) | Err(line) => line.as_str(),
            })
            .collect::<Vec<_>>();
        if state.truncated {
            lines.insert(0, "[较早的 Provider 诊断已截断]");
        }
        Some(lines.join("\n"))
    }

    pub fn wait(&self, timeout: Duration) {
        let Ok(state) = self.state.lock() else {
            return;
        };
        if !state.completed {
            let _ = self.completion.wait_timeout(state, timeout);
        }
    }

    fn push(&self, line: Result<String, String>) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if let Some(consumer) = state.consumer.clone() {
            drop(state);
            return consumer.send(line).is_ok();
        }
        let line = bound_diagnostic_line(line);
        let bytes = diagnostic_len(&line);
        while state.buffered_bytes + bytes > MAX_STARTUP_DIAGNOSTIC_BYTES
            || state.buffered.len() >= MAX_STARTUP_DIAGNOSTIC_LINES
        {
            let Some(removed) = state.buffered.pop_front() else {
                break;
            };
            state.buffered_bytes = state
                .buffered_bytes
                .saturating_sub(diagnostic_len(&removed));
            state.truncated = true;
        }
        state.buffered_bytes += bytes;
        state.buffered.push_back(line);
        true
    }
}

fn diagnostic_len(line: &Result<String, String>) -> usize {
    match line {
        Ok(line) | Err(line) => line.len(),
    }
}

fn bound_diagnostic_line(line: Result<String, String>) -> Result<String, String> {
    line.map(bounded_diagnostic_suffix)
        .map_err(bounded_diagnostic_suffix)
}

fn bounded_diagnostic_suffix(line: String) -> String {
    if line.len() <= MAX_STARTUP_DIAGNOSTIC_BYTES {
        return line;
    }
    let mut start = line.len() - MAX_STARTUP_DIAGNOSTIC_BYTES;
    while !line.is_char_boundary(start) {
        start += 1;
    }
    line[start..].to_string()
}

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

    use super::{
        MAX_PROVIDER_LINE_BYTES, MAX_PROVIDER_TURN_OUTPUT_BYTES, ProviderStderrRelay,
        read_bounded_lines, redact_token, reserve_turn_output,
    };

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

    #[test]
    fn stderr_relay_buffers_startup_lines_and_streams_follow_up_lines() {
        let relay = ProviderStderrRelay::default();
        relay.push(Ok("startup diagnostic".into()));

        let lines = relay.attach();
        relay.push(Ok("runtime diagnostic".into()));

        assert_eq!(lines.recv().unwrap().unwrap(), "startup diagnostic");
        assert_eq!(lines.recv().unwrap().unwrap(), "runtime diagnostic");
        drop(relay);
    }

    #[test]
    fn redacts_bridge_tokens_without_mutating_plain_output() {
        assert_eq!(
            redact_token("endpoint token=secret-123".into(), Some("secret-123")),
            "endpoint token=[REDACTED]"
        );
        assert_eq!(redact_token("plain".into(), None), "plain");
    }

    #[test]
    fn turn_output_budget_is_bounded_without_mutating_on_rejection() {
        let mut total = MAX_PROVIDER_TURN_OUTPUT_BYTES - 1;
        assert!(reserve_turn_output(&mut total, 1));
        assert_eq!(total, MAX_PROVIDER_TURN_OUTPUT_BYTES);
        assert!(!reserve_turn_output(&mut total, 1));
        assert_eq!(total, MAX_PROVIDER_TURN_OUTPUT_BYTES);
        assert!(!reserve_turn_output(&mut total, usize::MAX));
    }

    #[test]
    fn empty_output_events_still_consume_the_turn_budget() {
        let mut total = 0;

        assert!(reserve_turn_output(&mut total, 0));
        assert_eq!(total, 1);
    }
}
