//! 复用当前可见 PTY 的本地终端执行器。

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use super::TerminalCommandResult;
use crate::{
    commands::ai_policy::{AI_OUTPUT_LIMIT_BYTES, truncate_utf8, validate_approved_command},
    state::{LocalTerminalRegistry, local_completion_status, sanitize_local_command_output},
};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOCAL_TERMINAL_QUIET_PERIOD: Duration = Duration::from_millis(100);
const LOCAL_TERMINAL_SETTLE_LIMIT: Duration = Duration::from_millis(500);
const LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT: Duration = Duration::from_secs(2);

/// 把已获授权的命令写入当前可见 PTY，并把同一结果返回 Provider。
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
    let command_input = local_terminal_service
        .command_input(terminal_id, &command, &marker)
        .map_err(|error| error.message.to_string())?;
    local_terminals.begin_output_marker_filter(terminal_id, &marker)?;
    if let Err(error) = local_terminal_service.write(terminal_id, &command_input.execution) {
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
                &command_input.interrupt,
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
                &command_input.interrupt,
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
                        &command_input.interrupt,
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
    let completion_line_start = output[..result_marker_index]
        .rfind(['\r', '\n'])
        .map_or(0, |index| index + 1);
    output.truncate(completion_line_start);
    let output = sanitize_local_command_output(&output);
    settle_local_terminal_output(&receiver, cancellation, deadline);
    local_terminals.clear_output_marker_filter(terminal_id, &marker);
    Ok(TerminalCommandResult {
        output: output.trim().to_string(),
        exit_code,
    })
}

/// Ctrl+C 后完成探针可能仍在 PTY 输入队列中；短暂等待 Shell 消费后再释放 UI 过滤器。
fn interrupt_local_execution(
    local_terminal_service: &nocterm_application::terminal::LocalTerminalService,
    local_terminals: &LocalTerminalRegistry,
    terminal_id: &str,
    receiver: &mpsc::Receiver<String>,
    marker: &str,
    interrupt_input: &str,
) {
    let _ = local_terminal_service.write(terminal_id, interrupt_input);
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
                // 取消收尾只识别 marker；限制缓冲避免异常终端持续输出占用内存。
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

/// 退出状态先于提示符重绘到达；等待有绝对上限的安静窗口，避免下一命令插入尾部输出。
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
mod tests;
