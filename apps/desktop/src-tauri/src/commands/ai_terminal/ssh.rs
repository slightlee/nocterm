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
use nocterm_domain::ssh_visible_protocol::{
    MARKER_RECOVERY_GRACE, completion as marker_completion, visible_result,
    wrapped_command as marker_wrapped_command,
};
use nocterm_infrastructure::ssh::create_completion_marker;

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// 可见执行中断后的 OSC 确认窗口：Ctrl+C 只是请求，观察到 D 才算恢复。
const INTERRUPT_SETTLE: Duration = Duration::from_secs(3);
/// Marker 过滤窗口在执行截止之后的余量：覆盖中断确认窗口与迟到输出的到达
/// 时间。窗口过后注册表转入降级直通，过滤器绝不吞掉执行结束后的输出。
const MARKER_FILTER_GRACE: Duration = Duration::from_secs(1);

/// Marker 生成发生在路由已经预占 gate 之后；失败必须在 dispatch 前归还占用。
fn marker_or_release_gate<E>(
    registry: &crate::state::SshTerminalRegistry,
    terminal_id: &str,
    execution_id: &str,
    generated: Result<String, E>,
) -> Result<String, String> {
    generated.map_err(|_| {
        registry.release_input_gate(terminal_id, execution_id);
        "无法生成 SSH 可见执行完成标记".to_string()
    })
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

/// SSH 可见执行器（Stage 4）：把已授权命令写入用户当前可见 PTY，并以
/// nonce 过滤后的 OSC 633;D 完成信号取回真实退出码。
///
/// 前置条件由路由确认（四元绑定、Prompt Ready、门控已占用）；本函数只负责：
/// 订阅输出 → 写入裸命令 → 等待完成/取消/超时 → 交还输出与退出码。
/// 取消/超时向 PTY 写入 Ctrl+C；观察到完成信号前输入门控保持占用，
/// 未恢复时执行器不释放门控，迟到的 D 由读取线程自动放行。
#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_ssh_visible_sync(
    terminal_service: &nocterm_application::terminal::TerminalService,
    registry: &crate::state::SshTerminalRegistry,
    terminal_id: String,
    _connection_id: i64,
    execution_id: &str,
    command: &str,
    cancellation: &AtomicBool,
    deadline: Instant,
) -> Result<TerminalCommandResult, String> {
    // 订阅先于写入，避免漏掉命令回显（方案 §10.7 顺序约束）。
    let subscriber = registry.subscribe(&terminal_id);
    let snapshot = registry.completion_snapshot(&terminal_id);
    let marker = if registry.marker_recovery_supported(&terminal_id) {
        Some(marker_or_release_gate(
            registry,
            &terminal_id,
            execution_id,
            create_completion_marker(),
        )?)
    } else {
        None
    };
    let command_to_write = if let Some(marker) = &marker {
        // 过滤窗口与执行截止对齐并留出中断确认余量；窗口内 echo/无 echo 的
        // 判别照常进行，窗口过后注册表降级直通，长尾输出不再被吞。
        registry.begin_marker_output_suppression(
            &terminal_id,
            marker,
            execution_id,
            command,
            deadline + INTERRUPT_SETTLE + MARKER_FILTER_GRACE,
        );
        marker_wrapped_command(command, marker)
    } else {
        format!("{command}\n")
    };
    if terminal_service
        .write(&terminal_id, &command_to_write)
        .is_err()
    {
        registry.clear_marker_output_suppression(&terminal_id, execution_id);
        registry.release_input_gate(&terminal_id, execution_id);
        return Err("写入可见终端失败".to_string());
    }

    let mut output = String::new();
    let mut marker_result: Option<(i64, Instant)> = None;
    let recover_after_interrupt =
        |registry: &crate::state::SshTerminalRegistry,
         terminal_service: &nocterm_application::terminal::TerminalService,
         terminal_id: &str,
         execution_id: &str| {
            // Ctrl+C 只是中断请求；观察到完成信号才证明 Shell 已恢复。
            // 快照必须先于写入，否则极快返回的 D 会落在新快照之前而被漏掉。
            let recovery_snapshot = registry.completion_snapshot(terminal_id);
            let _ = terminal_service.write(terminal_id, "\u{3}");
            let recovered =
                registry.wait_for_completion(terminal_id, recovery_snapshot, INTERRUPT_SETTLE);
            if recovered.is_some() {
                registry.release_input_gate(terminal_id, execution_id);
            }
            // 未恢复时保持门控：迟到的 D 到达时由读取线程自动放行。
        };

    loop {
        // 完成证据优先于取消和超时：二者并发到达时 Shell 已经回到提示符，
        // 再写 Ctrl+C 会误伤用户正在输入的下一条命令。
        if let Some(code) = registry.wait_for_completion(&terminal_id, snapshot, Duration::ZERO) {
            let mut exceeded_limit = false;
            while let Ok(data) = subscriber.try_recv() {
                output.push_str(&data);
                exceeded_limit |= output.len() > AI_OUTPUT_LIMIT_BYTES;
            }
            registry.clear_marker_output_suppression(&terminal_id, execution_id);
            registry.release_input_gate(&terminal_id, execution_id);
            if exceeded_limit {
                return Err("可见终端命令输出超过 128 KiB".to_string());
            }
            return Ok(TerminalCommandResult {
                output: visible_result(&output, marker.as_deref()),
                exit_code: code,
            });
        }
        if cancellation.load(Ordering::Acquire) {
            if let Some((code, _)) = marker_result {
                // Marker 已证明包装命令结束；gate 仍按 OSC 优先的 grace 策略恢复。
                return Ok(TerminalCommandResult {
                    output: visible_result(&output, marker.as_deref()),
                    exit_code: code,
                });
            }
            recover_after_interrupt(registry, terminal_service, &terminal_id, execution_id);
            return Err("AI 任务已停止，已中断可见终端命令".to_string());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            if let Some((code, _)) = marker_result {
                // 命令结果已知，但 OSC 优先窗口尚未结束时仍保留 gate；registry
                // 会在 grace 到期后基于 owner-bound Marker 惰性恢复输入。
                return Ok(TerminalCommandResult {
                    output: visible_result(&output, marker.as_deref()),
                    exit_code: code,
                });
            }
            recover_after_interrupt(registry, terminal_service, &terminal_id, execution_id);
            return Err("可见终端命令执行超时，已发送中断请求".to_string());
        }
        if let Some((code, observed_at)) = marker_result
            && observed_at.elapsed() >= MARKER_RECOVERY_GRACE
        {
            // Prompt Ready 最初来自可信 OSC；Marker 只在本次受控包装器已完成、
            // 对应 OSC D 丢失时释放 owner gate。这里绝不恢复 Prompt Ready，下一次
            // AI 命令必须等待可信 A/D，否则由路由器降级到 independent exec。
            registry.finalize_marker_recovery(&terminal_id, execution_id);
            return Ok(TerminalCommandResult {
                output: visible_result(&output, marker.as_deref()),
                exit_code: code,
            });
        }
        match subscriber.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(data) => {
                output.push_str(&data);
                if marker_result.is_none()
                    && let Some(marker) = &marker
                    && let Some((_, _, code)) = marker_completion(&output, marker)
                {
                    marker_result = Some((code, Instant::now()));
                }
                if output.len() > AI_OUTPUT_LIMIT_BYTES {
                    if marker_result.is_some() {
                        return Err("可见终端命令输出超过 128 KiB".to_string());
                    }
                    // 读取线程按“发布末尾数据 -> 登记同块 OSC”排序。给控制面一个
                    // 短确认窗口，避免刚好在完成块越界时向已恢复的提示符写 Ctrl+C。
                    if registry
                        .wait_for_completion(&terminal_id, snapshot, CANCELLATION_POLL_INTERVAL)
                        .is_some()
                    {
                        registry.clear_marker_output_suppression(&terminal_id, execution_id);
                        registry.release_input_gate(&terminal_id, execution_id);
                        return Err("可见终端命令输出超过 128 KiB".to_string());
                    }
                    recover_after_interrupt(registry, terminal_service, &terminal_id, execution_id);
                    return Err("可见终端命令输出超过 128 KiB，已发送中断请求".to_string());
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // 读取线程退出（终端已关闭/断连）：无法证明命令状态。
                return Err("可见终端输出通道已关闭".to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests;
