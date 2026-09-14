//! AI 工具调用共享的 SSH 与本地终端执行内核。
//! 该模块只负责目标复用、输出边界和完成检测，不处理 Provider 协议或 IPC 事件。

use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    commands::{
        ai_policy::{AI_OUTPUT_LIMIT_BYTES, truncate_utf8, validate_approved_command},
        terminal_text::Utf8Stream,
    },
    state::{LocalTerminalRegistry, local_completion_status},
};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOCAL_TERMINAL_QUIET_PERIOD: Duration = Duration::from_millis(100);
const LOCAL_TERMINAL_SETTLE_LIMIT: Duration = Duration::from_millis(500);
const LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) struct TerminalCommandResult {
    pub output: String,
    /// SSH 取协议级状态，本地终端取当前 Shell 紧随命令返回的状态。
    pub exit_code: i64,
}

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
    // Bridge token 表示“当前已连接会话”，因此只允许复用其认证连接。会话已经
    // 断开时明确失败，不能静默读取凭据再建立一条用户不可见的新连接。
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

/// 收集独立 SSH exec channel 的输出；远端关闭 channel 即表示本次执行结束。
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

/// 把已获用户确认的命令写入当前可见本地 PTY，并把同一结果返回 Provider。
pub(crate) fn execute_local_sync(
    local_terminal_service: &nocterm_application::terminal::LocalTerminalService,
    local_terminals: &LocalTerminalRegistry,
    session_id: &str,
    terminal_id: &str,
    raw_command: &str,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Result<TerminalCommandResult, String> {
    if cancellation.load(Ordering::Acquire) {
        return Err("AI 任务已停止，本地命令未执行".to_string());
    }
    let command = validate_approved_command(raw_command)?;
    if local_terminals
        .terminal_for(session_id)
        .is_none_or(|current| current != terminal_id)
    {
        return Err("目标本地终端不存在或已重新连接".to_string());
    }
    let receiver = local_terminals.subscribe(terminal_id);
    let marker = local_completion_marker()?;
    let completion_command = local_terminal_service
        .completion_command(terminal_id, &marker)
        .map_err(|error| error.message.to_string())?;
    local_terminals.begin_output_marker_filter(terminal_id, &marker)?;
    // 两次回车分别提交用户命令和完成探针，后者读取前一命令在当前 Shell 中的状态。
    if let Err(error) =
        local_terminal_service.write(terminal_id, &format!("{command}\r{completion_command}\r"))
    {
        local_terminals.clear_output_marker_filter(terminal_id, &marker);
        return Err(error.message.to_string());
    }
    let mut output = String::new();
    loop {
        if cancellation.load(Ordering::Acquire) {
            interrupt_local_execution(
                local_terminal_service,
                local_terminals,
                terminal_id,
                &receiver,
                &marker,
            );
            return Err("AI 任务已停止，已中断本地终端命令".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            interrupt_local_execution(
                local_terminal_service,
                local_terminals,
                terminal_id,
                &receiver,
                &marker,
            );
            return Err("本地终端命令执行超时，已发送中断信号".to_string());
        }
        match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(data) => {
                output.push_str(&data);
                if local_completion_status(&output, &marker).is_some() {
                    break;
                }
                if truncate_utf8(&mut output, AI_OUTPUT_LIMIT_BYTES) {
                    interrupt_local_execution(
                        local_terminal_service,
                        local_terminals,
                        terminal_id,
                        &receiver,
                        &marker,
                    );
                    return Err("本地终端命令输出超过 128 KiB，已发送中断信号".to_string());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                local_terminals.clear_output_marker_filter(terminal_id, &marker);
                return Err("本地终端输出通道已关闭".to_string());
            }
        }
    }
    let (result_marker_index, _, exit_code) = local_completion_status(&output, &marker)
        .ok_or_else(|| "本地终端命令结束，但未返回可验证的退出状态".to_string())?;
    let internal_output_start = output[..=result_marker_index]
        .find(&marker)
        .and_then(|index| output[..index].rfind(['\r', '\n']).map(|line| line + 1))
        .unwrap_or(result_marker_index);
    output.truncate(internal_output_start);
    settle_local_terminal_output(&receiver, cancellation, deadline);
    local_terminals.clear_output_marker_filter(terminal_id, &marker);
    Ok(TerminalCommandResult {
        output: output.trim().to_string(),
        exit_code,
    })
}

/// Ctrl+C 后完成探针可能仍在 PTY 输入队列中。短暂等待它被 Shell 消费后再释放
/// UI 过滤器，避免停止或超时路径重新暴露内部 printf/marker。
fn interrupt_local_execution(
    local_terminal_service: &nocterm_application::terminal::LocalTerminalService,
    local_terminals: &LocalTerminalRegistry,
    terminal_id: &str,
    receiver: &mpsc::Receiver<String>,
    marker: &str,
) {
    let _ = local_terminal_service.write(terminal_id, "\u{3}");
    let cleanup_deadline = Instant::now() + LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT;
    if drain_until_local_completion(receiver, marker, cleanup_deadline) {
        let not_cancelled = AtomicBool::new(false);
        settle_local_terminal_output(receiver, &not_cancelled, cleanup_deadline);
    }
    local_terminals.clear_output_marker_filter(terminal_id, marker);
}

fn drain_until_local_completion(
    receiver: &mpsc::Receiver<String>,
    marker: &str,
    deadline: Instant,
) -> bool {
    let mut output = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(data) => {
                output.push_str(&data);
                if local_completion_status(&output, marker).is_some() {
                    return true;
                }
                // 取消收尾只需要识别 marker；限制缓冲避免异常终端持续输出占用内存。
                if output.len() > marker.len() * 4 {
                    let mut keep_from = output.len().saturating_sub(marker.len() * 2);
                    while keep_from < output.len() && !output.is_char_boundary(keep_from) {
                        keep_from += 1;
                    }
                    output.drain(..keep_from);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// 退出状态先于交互式提示符重绘到达。等待一个短暂且有绝对上限的安静窗口，
/// 让同一终端的下一条 AI 命令不会插入上一条完成探针的尾部输出。
fn settle_local_terminal_output(
    receiver: &mpsc::Receiver<String>,
    cancellation: &AtomicBool,
    deadline: Instant,
) {
    let settle_deadline = (Instant::now() + LOCAL_TERMINAL_SETTLE_LIMIT).min(deadline);
    let mut quiet_deadline = (Instant::now() + LOCAL_TERMINAL_QUIET_PERIOD).min(settle_deadline);
    loop {
        if cancellation.load(Ordering::Acquire) {
            break;
        }
        let wait_deadline = quiet_deadline.min(settle_deadline);
        let remaining = wait_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(_) => {
                quiet_deadline =
                    (Instant::now() + LOCAL_TERMINAL_QUIET_PERIOD).min(settle_deadline);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if Instant::now() >= quiet_deadline {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn local_completion_marker() -> Result<String, String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法生成本地终端命令完成标记".to_string())?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("__NOCTERM_LOCAL_AI_DONE_{suffix}__"))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Cursor, Read},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use nocterm_application::terminal::{LocalTerminalService, TerminalService};
    use nocterm_infrastructure::{ssh::SshTerminalManager, terminal::LocalTerminalManager};

    use crate::state::{LocalTerminalRegistry, local_completion_status};

    use super::{
        TerminalCommandResult, collect_ssh_output, drain_until_local_completion,
        execute_local_sync, require_successful_ssh_result,
    };

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
            collect_ssh_output(&service, opened(), &AtomicBool::new(false), Instant::now(),)
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
    fn interrupted_local_execution_waits_for_a_chunked_completion_marker() {
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send("output\r\n__NOCTERM_LOCAL_AI_".into()).unwrap();
        sender.send("DONE_cancel__130\r\n➜ ~ ".into()).unwrap();

        assert!(drain_until_local_completion(
            &receiver,
            "__NOCTERM_LOCAL_AI_DONE_cancel__",
            Instant::now() + Duration::from_secs(1),
        ));
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

    #[test]
    fn local_completion_status_distinguishes_command_echo_from_result() {
        let marker = "__NOCTERM_LOCAL_AI_DONE_1__";
        assert_eq!(
            local_completion_status(&format!("➜ ~ echo {marker}%ERRORLEVEL%\r"), marker),
            None
        );
        assert_eq!(
            local_completion_status(
                &format!("➜ ~ echo {marker}%ERRORLEVEL%\r{marker}17\r➜ ~ "),
                marker
            )
            .map(|(_, _, status)| status),
            Some(17)
        );
        assert_eq!(
            local_completion_status(&format!("{marker}0"), marker),
            Some((0, 1, 0))
        );
        assert_eq!(
            local_completion_status(&format!("{marker}1x"), marker),
            None
        );
        assert_eq!(
            local_completion_status(&format!("{marker}-1073741510\r"), marker),
            Some((0, 11, -1_073_741_510))
        );
    }

    #[test]
    fn local_approved_execution_uses_the_existing_visible_pty() {
        let service = LocalTerminalService::new(Arc::new(LocalTerminalManager::default()));
        let opened = service.open(40, 24).expect("open local terminal");
        let terminal_id = opened.id;
        let registry = Arc::new(LocalTerminalRegistry::default());
        registry.bind("local:test".into(), terminal_id.clone());
        let reader_registry = Arc::clone(&registry);
        let reader_terminal_id = terminal_id.clone();
        let visible_output = Arc::new(Mutex::new(String::new()));
        let reader_visible_output = Arc::clone(&visible_output);
        let mut reader = opened.reader;
        thread::spawn(move || {
            let mut decoder = crate::commands::terminal_text::Utf8Stream::default();
            let mut buffer = [0_u8; 4096];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => {
                        let data = decoder.push(&buffer[..length]);
                        if !data.is_empty() {
                            let visible =
                                reader_registry.visible_output(&reader_terminal_id, &data);
                            reader_registry.publish(&reader_terminal_id, &data);
                            reader_visible_output.lock().unwrap().push_str(&visible);
                        }
                    }
                }
            }
        });
        let output = execute_local_sync(
            &service,
            &registry,
            "local:test",
            &terminal_id,
            "pwd",
            &AtomicBool::new(false),
            // 默认 Shell 会加载用户配置；并行门禁下保留足够的冷启动余量。
            Instant::now() + Duration::from_secs(15),
        )
        .expect("execute approved local command");
        assert!(!output.output.is_empty());
        assert_eq!(output.exit_code, 0);

        #[cfg(not(windows))]
        let failing_command = "sh -c 'exit 7'";
        #[cfg(windows)]
        let failing_command = "cmd /c exit 7";
        let failed = execute_local_sync(
            &service,
            &registry,
            "local:test",
            &terminal_id,
            failing_command,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .expect("collect failed local command status");
        assert_eq!(failed.exit_code, 7);
        let terminal_output = visible_output.lock().unwrap();
        assert!(!terminal_output.contains("__NOCTERM_"));
        assert!(!terminal_output.contains("printf '\\n"));
        drop(terminal_output);

        #[cfg(not(windows))]
        let long_command = "sleep 10";
        #[cfg(windows)]
        let long_command = "Start-Sleep -Seconds 10";
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::clone(&cancellation);
        let execution_service = service.clone();
        let execution_registry = Arc::clone(&registry);
        let execution_terminal_id = terminal_id.clone();
        let execution = thread::spawn(move || {
            execute_local_sync(
                &execution_service,
                &execution_registry,
                "local:test",
                &execution_terminal_id,
                long_command,
                &cancelled,
                Instant::now() + Duration::from_secs(15),
            )
        });
        thread::sleep(Duration::from_millis(250));
        cancellation.store(true, Ordering::Release);
        assert!(
            execution
                .join()
                .expect("join cancelled local command")
                .expect_err("cancel local command")
                .contains("已停止")
        );
        // 取消收尾完成后再执行一条命令，证明 PTY 没有被遗留过滤器永久占用。
        let after_cancel = execute_local_sync(
            &service,
            &registry,
            "local:test",
            &terminal_id,
            "pwd",
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(5),
        )
        .expect("execute local command after cancellation");
        assert_eq!(after_cancel.exit_code, 0);
        let terminal_output = visible_output.lock().unwrap();
        assert!(!terminal_output.contains("__NOCTERM_"));
        assert!(!terminal_output.contains("printf '\\n"));
        drop(terminal_output);
        service.close(&terminal_id).expect("close local terminal");
    }
}
