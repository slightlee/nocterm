//! 复用当前认证连接的 SSH 独立 exec 执行器。

use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use super::TerminalCommandResult;
use crate::commands::{
    ai_policy::{AI_OUTPUT_LIMIT_BYTES, truncate_utf8, validate_approved_command},
    terminal_text::Utf8Stream,
};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug)]
struct CollectedSshOutput {
    output: String,
    exit_code: Option<u32>,
}

enum SshReadEvent {
    Data(Vec<u8>),
    Eof,
    Failed(String),
}

pub(crate) fn execute_ssh_sync(
    connection_service: &nocterm_application::connection::ConnectionService,
    terminal_service: &nocterm_application::terminal::TerminalService,
    connection_id: i64,
    raw_command: &str,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Result<TerminalCommandResult, String> {
    if cancellation.load(Ordering::Acquire) {
        return Err("AI 任务已停止，远程命令未执行".to_string());
    }
    let command = validate_approved_command(raw_command)?;
    // 删除连接后不能凭旧 terminal 句柄继续执行任何 AI 命令。
    connection_service
        .get(connection_id)
        .map_err(|error| error.message.to_string())?;
    // 只复用当前已认证连接；断开时不能读取凭据建立用户不可见的新连接。
    let opened = terminal_service
        .exec_existing(connection_id, &command, cancellation)
        .map_err(|error| error.message.to_string())?
        .ok_or_else(|| "当前 SSH 会话已断开，请重新连接后再执行".to_string())?;
    let collected = collect_ssh_output(terminal_service, opened, cancellation, deadline)?;
    let exit_code = collected
        .exit_code
        .ok_or_else(|| "SSH 命令结束，但服务器未返回退出状态，不能判断是否成功".to_string())?;
    Ok(TerminalCommandResult {
        output: collected.output,
        exit_code: i64::from(exit_code),
    })
}

/// 结构化检查验证 SSH 协议返回的真实退出码，避免依赖 Shell 标记或把失败当成功。
pub(crate) fn execute_ssh_inspection_sync(
    connection_service: &nocterm_application::connection::ConnectionService,
    terminal_service: &nocterm_application::terminal::TerminalService,
    connection_id: i64,
    command: &str,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Result<String, String> {
    let result = execute_ssh_sync(
        connection_service,
        terminal_service,
        connection_id,
        command,
        cancellation,
        deadline,
    )?;
    require_successful_ssh_result(result)
}

fn require_successful_ssh_result(result: TerminalCommandResult) -> Result<String, String> {
    let exit_code = result.exit_code;
    if exit_code == 0 {
        return Ok(result.output);
    }
    let detail = if result.output.is_empty() {
        String::new()
    } else {
        format!("：{}", result.output)
    };
    Err(format!("服务器检查失败（退出码 {exit_code}）{detail}"))
}

/// 远端关闭独立 exec channel 即表示输出结束，退出状态由协议通道单独确认。
fn collect_ssh_output(
    terminal_service: &nocterm_application::terminal::TerminalService,
    opened: nocterm_domain::terminal::OpenedSshExecution,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Result<CollectedSshOutput, String> {
    let terminal_id = opened.id;
    let mut reader = opened.reader;
    let completion = opened.completion;
    let (sender, receiver) = mpsc::sync_channel(16);
    // 阻塞 reader 放入独立线程，调用方才能实施总超时并主动关闭通道。
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let _ = sender.send(SshReadEvent::Eof);
                    break;
                }
                Ok(length) => {
                    if sender
                        .send(SshReadEvent::Data(buffer[..length].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.send(SshReadEvent::Failed(error.to_string()));
                    break;
                }
            }
        }
    });
    let mut decoder = Utf8Stream::default();
    let mut output = String::new();
    let mut exceeded_limit = false;
    loop {
        if cancellation.load(Ordering::Acquire) {
            let _ = terminal_service.close(&terminal_id);
            return Err("AI 任务已停止，已关闭独立执行通道".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = terminal_service.close(&terminal_id);
            return Err("SSH 命令执行超时，已关闭独立执行通道".to_string());
        }
        match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(SshReadEvent::Data(data)) => {
                output.push_str(&decoder.push(&data));
                if truncate_utf8(&mut output, AI_OUTPUT_LIMIT_BYTES) {
                    exceeded_limit = true;
                    break;
                }
            }
            Ok(SshReadEvent::Eof) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(SshReadEvent::Failed(error)) => {
                let _ = terminal_service.close(&terminal_id);
                return Err(format!("读取 SSH 命令输出失败：{error}"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
        }
    }
    let _ = terminal_service.close(&terminal_id);
    if exceeded_limit {
        return Err("SSH 命令输出超过 128 KiB，已关闭独立执行通道".to_string());
    }
    let exit_code = loop {
        if cancellation.load(Ordering::Acquire) {
            return Err("AI 任务已停止，已关闭独立执行通道".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("SSH 命令输出已结束，但等待退出状态超时".to_string());
        }
        match completion.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(completion) => break completion.exit_code,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("SSH 命令输出已结束，但退出状态通道意外关闭".to_string());
            }
        }
    };
    Ok(CollectedSshOutput {
        output: output.trim().to_string(),
        exit_code,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Cursor, Read},
        sync::{Arc, atomic::AtomicBool},
        time::{Duration, Instant},
    };

    use nocterm_application::terminal::TerminalService;
    use nocterm_infrastructure::ssh::SshTerminalManager;

    use super::{collect_ssh_output, require_successful_ssh_result};
    use crate::commands::ai_terminal::TerminalCommandResult;

    struct FailingReader;

    impl Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::ConnectionReset, "test reset"))
        }
    }

    fn opened_ssh_execution(
        id: &str,
        output: &[u8],
        exit_code: Option<u32>,
    ) -> nocterm_domain::terminal::OpenedSshExecution {
        let (completion_tx, completion) = std::sync::mpsc::channel();
        completion_tx
            .send(nocterm_domain::terminal::SshExecutionCompletion { exit_code })
            .expect("send test completion");
        nocterm_domain::terminal::OpenedSshExecution {
            id: id.into(),
            reader: Box::new(Cursor::new(output.to_vec())),
            completion,
        }
    }

    #[test]
    fn reused_exec_output_completes_on_channel_eof_without_a_shell_marker() {
        let service = TerminalService::new(Arc::new(SshTerminalManager::default()));
        let opened = opened_ssh_execution("ssh-exec-test", b"container-a\n", Some(0));
        let output = collect_ssh_output(
            &service,
            opened,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(1),
        )
        .expect("read exec output");
        assert_eq!(output.output, "container-a");
        assert_eq!(output.exit_code, Some(0));
    }

    #[test]
    fn cancelled_or_expired_exec_never_returns_buffered_output_as_success() {
        let service = TerminalService::new(Arc::new(SshTerminalManager::default()));
        let opened = || opened_ssh_execution("ssh-exec-cancelled", b"late output\n", Some(0));
        let cancelled = AtomicBool::new(true);

        assert!(
            collect_ssh_output(
                &service,
                opened(),
                &cancelled,
                Instant::now() + Duration::from_secs(1),
            )
            .unwrap_err()
            .contains("已停止")
        );
        assert!(
            collect_ssh_output(&service, opened(), &AtomicBool::new(false), Instant::now())
                .unwrap_err()
                .contains("超时")
        );
    }

    #[test]
    fn ssh_reader_failure_is_not_treated_as_a_successful_eof() {
        let service = TerminalService::new(Arc::new(SshTerminalManager::default()));
        let (completion_sender, completion) = std::sync::mpsc::channel();
        completion_sender
            .send(nocterm_domain::terminal::SshExecutionCompletion { exit_code: Some(0) })
            .unwrap();
        let opened = nocterm_domain::terminal::OpenedSshExecution {
            id: "ssh-exec-failed-read".into(),
            reader: Box::new(FailingReader),
            completion,
        };

        let error = collect_ssh_output(
            &service,
            opened,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(1),
        )
        .expect_err("read errors must fail the command");

        assert!(error.contains("读取 SSH 命令输出失败"));
        assert!(error.contains("test reset"));
    }

    #[test]
    fn inspection_output_requires_a_valid_success_status() {
        assert_eq!(
            require_successful_ssh_result(TerminalCommandResult {
                output: "server-a".into(),
                exit_code: 0,
            })
            .unwrap(),
            "server-a"
        );
        assert!(
            require_successful_ssh_result(TerminalCommandResult {
                output: "permission denied".into(),
                exit_code: 1,
            })
            .unwrap_err()
            .contains("退出码 1")
        );
    }
}
