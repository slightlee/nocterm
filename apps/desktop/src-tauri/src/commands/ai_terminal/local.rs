//! 复用当前可见 PTY 的本地终端执行器。
//!
//! 完成检测有两条等价路径，由统一的 `run_local_command` executor 驱动，差异
//! 通过 `LocalCompletionStrategy` 的四个策略方法体现：
//!
//! - `Osc633Strategy`：shell integration 激活时，完成信号取自 shell 上报的 OSC 633;D；
//! - `TextProbeStrategy`：cmd、注入失败或序列被剥时，回退到文本探针与 marker 匹配。
//!
//! 两条路径共享校验、目标绑定、订阅、取消、deadline、中断恢复和错误归一化。

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
/// 集成路径完成信号先于下一提示符文本到达；短暂排空订阅通道汇总命令输出。
const INTEGRATION_DRAIN_LIMIT: Duration = Duration::from_millis(150);

#[derive(Debug, PartialEq, Eq)]
enum LocalInterruption {
    Recovered,
    RecoveryPending,
    WriteFailed(String),
}

/// 完成检测策略：两条路径各自实现准备、轮询完成、采集输出、中断恢复与清理。
///
/// executor 负责共享生命周期（取消、deadline、输出累积、128 KiB 限制、错误归一化、
/// 正常完成后的输出 settle），策略只回答"命令完成了吗、退出码多少"以及各自的
/// 资源占用与释放。策略构造时即收到用户命令文本，`prepare` 拼接要写入 PTY 的
/// 完整字节串（探针含探针命令，集成只含裸命令）。
trait LocalCompletionStrategy {
    /// 写入命令前准备完成协议并返回要写入 PTY 的完整字节串。占用失败时返回 Err，
    /// executor 会调用 `release` 清理已占用资源后回传错误。
    fn prepare(&mut self) -> Result<String, String>;

    /// 轮询完成状态：Some(code) 表示命令已完成并返回真实退出码，None 表示继续等待。
    /// `accumulated` 是 executor 主循环已收到的订阅输出，探针路径据此做文本匹配；
    /// 集成路径的完成信号在注册表事件里，与该参数无关。
    fn poll_completion(&self, accumulated: &str) -> Option<i64>;

    /// 命令完成后采集输出：探针截断 marker 与回显并 sanitize，集成排空订阅通道。
    fn collect_output(&self, accumulated: &mut String, receiver: &mpsc::Receiver<String>);

    /// 中断后的清理与恢复判定（Ctrl+C + 等待恢复信号）。
    fn interrupt_and_recover(&self, receiver: &mpsc::Receiver<String>) -> LocalInterruption;

    /// 释放策略占用的资源（门控 / filter），保证 prepare 失败与正常完成两条路径都清理。
    fn release(&self);

    /// 正常完成后是否需要 settle（等待安静窗口排空提示符重绘）。
    /// 探针路径的完成信号在订阅文本里，marker 之后可能有提示符重绘需要排空；
    /// 集成路径的完成信号经控制平面，drain 已在 collect_output 内完成，无需额外 settle。
    fn needs_settle(&self) -> bool;
}

/// 把已获授权的命令写入当前可见 PTY，并把同一结果返回 Provider。
///
/// 入口不变：校验命令与目标后，按集成是否激活选择策略，统一交给 `run_local_command`。
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
    // 先取得 PTY 独占权再订阅，避免恢复期间的重试遗留无效订阅者。
    // 订阅仍发生在写入命令之前，因此不会漏掉本轮输出。
    let receiver = local_terminals.subscribe(terminal_id);
    let mut strategy: Box<dyn LocalCompletionStrategy> =
        if local_terminals.integration_active(terminal_id) {
            Box::new(Osc633Strategy {
                service: local_terminal_service,
                registry: local_terminals,
                terminal_id,
                command: &command,
                snapshot: local_terminals.completion_snapshot(terminal_id),
            })
        } else {
            let marker = local_completion_marker()?;
            Box::new(TextProbeStrategy {
                service: local_terminal_service,
                registry: local_terminals,
                terminal_id,
                command: &command,
                marker,
            })
        };
    run_local_command(
        local_terminal_service,
        terminal_id,
        cancellation,
        deadline,
        &receiver,
        strategy.as_mut(),
    )
}

/// 统一执行器：驱动策略的共享生命周期循环。
///
/// prepare → 写入 → 主循环（取消 / deadline / poll_completion / 输出累积 + 128 KiB）→
/// 完成则 collect_output + release + settle；否则 interrupt_and_recover + 归一化错误。
#[allow(clippy::too_many_arguments)]
fn run_local_command(
    local_terminal_service: &nocterm_application::terminal::LocalTerminalService,
    terminal_id: &str,
    cancellation: &AtomicBool,
    deadline: Instant,
    receiver: &mpsc::Receiver<String>,
    strategy: &mut dyn LocalCompletionStrategy,
) -> Result<TerminalCommandResult, String> {
    let to_write = strategy.prepare()?;
    if let Err(error) = local_terminal_service.write(terminal_id, &to_write) {
        strategy.release();
        return Err(error.message.to_string());
    }
    let mut output = String::new();
    let exit_code = loop {
        if cancellation.load(Ordering::Acquire) {
            let interruption = strategy.interrupt_and_recover(receiver);
            return Err(interruption_error("AI 任务已停止", interruption));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let interruption = strategy.interrupt_and_recover(receiver);
            return Err(interruption_error("本地终端命令执行超时", interruption));
        }
        // 集成路径的完成信号不经订阅通道（走注册表的 633 事件），先判完成再收输出；
        // 探针路径的完成信号在订阅文本里，先收输出再判匹配。两者都在同一循环内收敛。
        if let Some(code) = strategy.poll_completion(&output) {
            break code;
        }
        match receiver.recv_timeout(remaining.min(CANCELLATION_POLL_INTERVAL)) {
            Ok(data) => {
                output.push_str(&data);
                if let Some(code) = strategy.poll_completion(&output) {
                    break code;
                }
                if truncate_utf8(&mut output, AI_OUTPUT_LIMIT_BYTES) {
                    let interruption = strategy.interrupt_and_recover(receiver);
                    return Err(interruption_error(
                        "本地终端命令输出超过 128 KiB",
                        interruption,
                    ));
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                strategy.release();
                return Err("本地终端输出通道已关闭".to_string());
            }
        }
    };
    strategy.collect_output(&mut output, receiver);
    strategy.release();
    // 探针路径的完成信号在订阅文本里，marker 之后可能有提示符重绘需要排空；
    // 集成路径经控制平面，drain 已在 collect_output 内完成，跳过 settle。
    if strategy.needs_settle() {
        settle_local_terminal_output(receiver, cancellation, deadline);
    }
    Ok(TerminalCommandResult {
        output: output.trim().to_string(),
        exit_code,
    })
}

/// 集成路径策略：完成信号取自 shell 上报的 OSC 633;D（经注册表 633 事件到达）。
struct Osc633Strategy<'a> {
    service: &'a nocterm_application::terminal::LocalTerminalService,
    registry: &'a LocalTerminalRegistry,
    terminal_id: &'a str,
    command: &'a str,
    snapshot: u64,
}

impl<'a> LocalCompletionStrategy for Osc633Strategy<'a> {
    fn prepare(&mut self) -> Result<String, String> {
        // 门控原子争用：同一终端同时最多一条 AI 命令；失败时 executor 调 release 解除。
        if !self.registry.try_acquire_input_gate(self.terminal_id) {
            return Err("当前本地终端已有 AI 命令正在执行，请等待完成后重试".to_string());
        }
        // 写入裸命令；完成信号由 shell 的 prompt 钩子经 633;D 上报，不经可见输出。
        Ok(format!("{}\r", self.command))
    }

    fn poll_completion(&self, _accumulated: &str) -> Option<i64> {
        // 集成路径的完成信号在注册表的 633 事件计数里，与订阅文本无关。
        self.registry
            .wait_for_completion(self.terminal_id, self.snapshot, Duration::ZERO)
    }

    fn collect_output(&self, accumulated: &mut String, receiver: &mpsc::Receiver<String>) {
        // 完成信号先于下一提示符文本；短暂排空订阅通道汇总命令输出（回显 + 结果）。
        let drain_deadline = Instant::now() + INTEGRATION_DRAIN_LIMIT;
        loop {
            let remaining = drain_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match receiver.recv_timeout(remaining) {
                Ok(data) => accumulated.push_str(&data),
                Err(_) => break,
            }
        }
    }

    fn interrupt_and_recover(&self, _receiver: &mpsc::Receiver<String>) -> LocalInterruption {
        // 发送 Ctrl+C 请求中断；已恢复则 D 到达时读取线程已自动解除门控，
        // 未恢复则保留门控——新 AI 命令被拒绝，用户输入继续被拦，直到迟到的 D 放行。
        if let Err(error) = self.service.write(self.terminal_id, "\u{3}") {
            return LocalInterruption::WriteFailed(error.message.to_string());
        }
        let recovered = self.registry.wait_for_completion(
            self.terminal_id,
            self.snapshot,
            LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT,
        );
        if recovered.is_some() {
            LocalInterruption::Recovered
        } else {
            LocalInterruption::RecoveryPending
        }
    }

    fn release(&self) {
        // 正常完成后解除输入门控；中断恢复后由读取线程在 D 到达时自动解除，这里幂等。
        self.registry.set_input_gate(self.terminal_id, false);
    }

    fn needs_settle(&self) -> bool {
        false
    }
}

/// 文本探针路径策略：随机 marker + Shell 探针命令，完成信号在可见输出文本里匹配。
struct TextProbeStrategy<'a> {
    service: &'a nocterm_application::terminal::LocalTerminalService,
    registry: &'a LocalTerminalRegistry,
    terminal_id: &'a str,
    command: &'a str,
    marker: String,
}

impl<'a> LocalCompletionStrategy for TextProbeStrategy<'a> {
    fn prepare(&mut self) -> Result<String, String> {
        // 生成探针命令：Shell 在执行完用户命令后打印 marker + 退出码，供文本匹配。
        let completion_command = self
            .service
            .completion_command(self.terminal_id, &self.marker)
            .map_err(|error| error.message.to_string())?;
        self.registry
            .begin_output_marker_filter(self.terminal_id, &self.marker)?;
        // 两次回车分别提交用户命令和完成探针，后者读取前一命令在当前 Shell 中的状态。
        Ok(format!("{}\r{completion_command}\r", self.command))
    }

    fn poll_completion(&self, accumulated: &str) -> Option<i64> {
        local_completion_status(accumulated, &self.marker).map(|(_, _, code)| code)
    }

    fn collect_output(&self, accumulated: &mut String, _receiver: &mpsc::Receiver<String>) {
        let Some((result_marker_index, _, _)) = local_completion_status(accumulated, &self.marker)
        else {
            return;
        };
        // marker 首次出现位置之前的回显属于内部命令文本，不能返回给 Provider；
        // 退出码与 marker 同行，截断到 marker 之前即可。
        let internal_output_start = accumulated[..=result_marker_index]
            .find(self.marker.as_str())
            .and_then(|index| {
                accumulated[..index]
                    .rfind(['\r', '\n'])
                    .map(|line| line + 1)
            })
            .unwrap_or(result_marker_index);
        accumulated.truncate(internal_output_start);
        // 取消后的迟到 marker 可能在下一轮订阅建立前后到达；保留前缀属于内部协议，
        // 无论来自当前轮还是上一轮都不能返回给 Provider。
        let sanitized = sanitize_local_command_output(accumulated);
        accumulated.clear();
        accumulated.push_str(&sanitized);
    }

    fn interrupt_and_recover(&self, receiver: &mpsc::Receiver<String>) -> LocalInterruption {
        interrupt_local_execution(
            self.service,
            self.registry,
            self.terminal_id,
            receiver,
            &self.marker,
            Instant::now() + LOCAL_TERMINAL_INTERRUPT_SETTLE_LIMIT,
        )
    }

    fn release(&self) {
        self.registry
            .clear_output_marker_filter(self.terminal_id, &self.marker);
    }

    fn needs_settle(&self) -> bool {
        true
    }
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
    // 不能把"已写入 Ctrl+C"当成"前台命令已经退出"。过滤器继续持有 PTY，
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
