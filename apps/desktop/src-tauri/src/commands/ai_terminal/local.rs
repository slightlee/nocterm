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

#[derive(Debug, PartialEq, Eq)]
enum LocalInterruption {
    Recovered,
    RecoveryPending,
    WriteFailed(String),
}

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
    let marker = local_completion_marker()?;
    let completion_command = local_terminal_service
        .completion_command(terminal_id, &marker)
        .map_err(|error| error.message.to_string())?;
    local_terminals.begin_output_marker_filter(terminal_id, &marker)?;
    // 先取得 PTY 独占权再订阅，避免恢复期间的重试遗留无效订阅者。
    // 订阅仍发生在写入命令之前，因此不会漏掉本轮输出。
    let receiver = local_terminals.subscribe(terminal_id);
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
            let interruption = interrupt_local_execution(
                local_terminal_service,
                local_terminals,
                terminal_id,
                &receiver,
                &marker,
                Instant::now() + LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT,
            );
            return Err(interruption_error("AI 任务已停止", interruption));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let interruption = interrupt_local_execution(
                local_terminal_service,
                local_terminals,
                terminal_id,
                &receiver,
                &marker,
                Instant::now() + LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT,
            );
            return Err(interruption_error("本地终端命令执行超时", interruption));
        }
        match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(data) => {
                output.push_str(&data);
                if local_completion_status(&output, &marker).is_some() {
                    break;
                }
                if truncate_utf8(&mut output, AI_OUTPUT_LIMIT_BYTES) {
                    let interruption = interrupt_local_execution(
                        local_terminal_service,
                        local_terminals,
                        terminal_id,
                        &receiver,
                        &marker,
                        Instant::now() + LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT,
                    );
                    return Err(interruption_error(
                        "本地终端命令输出超过 128 KiB",
                        interruption,
                    ));
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
    // 取消后的迟到 marker 可能在下一轮订阅建立前后到达；保留前缀属于内部协议，
    // 无论来自当前轮还是上一轮都不能返回给 Provider。
    let output = sanitize_local_command_output(&output);
    settle_local_terminal_output(&receiver, cancellation, deadline);
    local_terminals.clear_output_marker_filter(terminal_id, &marker);
    Ok(TerminalCommandResult {
        output: output.trim().to_string(),
        exit_code,
    })
}

/// Ctrl+C 只是中断请求；只有观察到当前 marker 才能证明 Shell 已恢复并释放 PTY。
fn interrupt_local_execution(
    local_terminal_service: &nocterm_application::terminal::LocalTerminalService,
    local_terminals: &LocalTerminalRegistry,
    terminal_id: &str,
    receiver: &mpsc::Receiver<String>,
    marker: &str,
    cleanup_deadline: Instant,
) -> LocalInterruption {
    if let Err(error) = local_terminal_service.write(terminal_id, "\u{3}") {
        return LocalInterruption::WriteFailed(error.message.to_string());
    }
    if drain_until_local_completion(receiver, marker, cleanup_deadline) {
        let not_cancelled = AtomicBool::new(false);
        settle_local_terminal_output(receiver, &not_cancelled, cleanup_deadline);
        local_terminals.clear_output_marker_filter(terminal_id, marker);
        return LocalInterruption::Recovered;
    }
    // 不能把“已写入 Ctrl+C”当成“前台命令已经退出”。过滤器继续持有 PTY，
    // 直到读取线程收到迟到 marker；否则下一条 AI 命令会写进仍忙碌的 Shell。
    LocalInterruption::RecoveryPending
}

fn interruption_error(reason: &str, interruption: LocalInterruption) -> String {
    match interruption {
        LocalInterruption::Recovered => format!("{reason}，已中断本地终端命令"),
        LocalInterruption::RecoveryPending => {
            format!("{reason}，已发送中断请求；本地终端仍在恢复，恢复完成前不会执行新的 AI 命令")
        }
        LocalInterruption::WriteFailed(error) => {
            format!("{reason}，但中断请求发送失败：{error}；本地终端恢复完成前不会执行新的 AI 命令")
        }
    }
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
