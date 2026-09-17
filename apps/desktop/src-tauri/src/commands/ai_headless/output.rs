//! Headless Provider 输出协议与有界读取。
//! stdout/stderr 共用预算，并在输出进入 UI 前验证能力初始化和真实 Gateway 调用。

use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

use tauri::{AppHandle, Emitter, Runtime};

use crate::{
    commands::{
        ai_provider::HeadlessReadinessRequirement,
        ai_stream::{
            PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE, read_bounded_lines, redact_token,
            reserve_turn_output,
        },
    },
    dto::ai::AiOutputEvent,
    state::AiGatewayState,
};

#[derive(Default)]
/// 单轮输出的共享预算，防止并发读取 stdout/stderr 时分别放大上限。
/// 一旦任意线程触发上限，后续读取只负责结束，不再重复发送错误事件。
pub(super) struct HeadlessOutputBudget {
    bytes: Mutex<usize>,
    limit_reached: AtomicBool,
}

/// 输出读取所需的不可变任务上下文。
/// 引用由两个短生命周期读取线程共享，任务状态仍由父模块统一回收。
pub(super) struct HeadlessOutputContext<'a, R: Runtime = tauri::Wry> {
    pub app: &'a AppHandle<R>,
    pub session_id: &'a str,
    pub connection_id: Option<i64>,
    pub bridge_token: Option<&'a str>,
    pub gateway: &'a AiGatewayState,
    pub budget: &'a HeadlessOutputBudget,
    pub failed: &'a AtomicBool,
}

/// 预算拒绝分为首次和后续两种，使并发线程只向 UI 报告一次超限。
pub(super) enum OutputReservation {
    Accepted,
    FirstRejection,
    Rejected,
}

impl HeadlessOutputBudget {
    /// stdout 与 stderr 共用一个预算；只有首个超限线程负责向界面报告错误。
    pub(super) fn reserve(&self, bytes: usize) -> OutputReservation {
        if self.limit_reached.load(Ordering::Acquire) {
            return OutputReservation::Rejected;
        }
        let Ok(mut total) = self.bytes.lock() else {
            return if !self.limit_reached.swap(true, Ordering::AcqRel) {
                OutputReservation::FirstRejection
            } else {
                OutputReservation::Rejected
            };
        };
        // 另一个读取线程可能在当前线程等待预算锁时先触发超限。
        if self.limit_reached.load(Ordering::Acquire) {
            return OutputReservation::Rejected;
        }
        if reserve_turn_output(&mut total, bytes) {
            OutputReservation::Accepted
        } else if !self.limit_reached.swap(true, Ordering::AcqRel) {
            OutputReservation::FirstRejection
        } else {
            OutputReservation::Rejected
        }
    }
}

/// 读取一条 Provider 输出流，并在发布前完成三层校验：
///
/// 1. 输出总量没有超过单轮预算；
/// 2. Provider 初始化事件声明了预期的 Nocterm 工具；
/// 3. 终端绑定任务至少真实调用过一次当前 Gateway。
///
/// 任一校验失败都会设置共享失败标志，由父模块终止进程并统一发出退出事件。
pub(super) fn emit_lines<T: std::io::Read, R: Runtime>(
    stream: &str,
    reader: T,
    context: &HeadlessOutputContext<'_, R>,
    readiness_requirement: Option<&HeadlessReadinessRequirement>,
) -> bool {
    let mut readiness_satisfied = readiness_requirement.is_none();
    let mut readiness_failed = false;
    let mut gateway_call_satisfied =
        readiness_requirement.is_none_or(|requirement| !requirement.require_gateway_call);
    let mut pending_lines = Vec::new();
    let result = read_bounded_lines(reader, |line| {
        // readiness 事件可能晚于普通启动日志，因此持续检查到命中目标事件。
        if !readiness_satisfied
            && let Some(requirement) = readiness_requirement
            && let Some(result) = validate_readiness_event(&line, requirement)
        {
            match result {
                Ok(()) => readiness_satisfied = true,
                Err(message) => {
                    readiness_failed = true;
                    context.failed.store(true, Ordering::Release);
                    emit_headless_error(context, message);
                    return false;
                }
            }
        }
        // 预算在脱敏前按原始字节计数，避免替换操作改变安全边界。
        match context.budget.reserve(line.len()) {
            OutputReservation::Accepted => {}
            OutputReservation::FirstRejection => {
                context.failed.store(true, Ordering::Release);
                let _ = context.app.emit(
                    "nocterm://ai-output",
                    AiOutputEvent {
                        session_id: context.session_id.into(),
                        connection_id: context.connection_id,
                        stream: "stderr".into(),
                        data: PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE.into(),
                    },
                );
                return false;
            }
            OutputReservation::Rejected => {
                context.failed.store(true, Ordering::Release);
                return false;
            }
        }
        let line = redact_token(line, context.bridge_token);
        if !gateway_call_satisfied {
            // 模型文本在真实工具调用前暂存，防止宿主环境猜测泄露到回答中。
            pending_lines.push(line);
            match has_required_gateway_call(context) {
                Ok(true) => {
                    gateway_call_satisfied = true;
                    for pending in pending_lines.drain(..) {
                        emit_headless_line(stream, context, pending);
                    }
                }
                Ok(false) => {}
                Err(error) => {
                    readiness_failed = true;
                    context.failed.store(true, Ordering::Release);
                    emit_headless_error(context, error);
                    return false;
                }
            }
            return true;
        }
        emit_headless_line(stream, context, line);
        true
    });
    if let Err(error) = result {
        context.failed.store(true, Ordering::Release);
        emit_headless_error(context, redact_token(error, context.bridge_token));
        return false;
    }
    if readiness_failed {
        return false;
    }
    // 正常 EOF 也不能绕过初始化与工具调用契约，缺失状态按失败关闭。
    if !readiness_satisfied {
        context.failed.store(true, Ordering::Release);
        emit_headless_error(
            context,
            "Provider 未返回终端能力初始化状态，已停止本次任务".into(),
        );
        return false;
    }
    if !gateway_call_satisfied {
        match has_required_gateway_call(context) {
            Ok(true) => {
                for pending in pending_lines {
                    emit_headless_line(stream, context, pending);
                }
            }
            Ok(false) => {
                context.failed.store(true, Ordering::Release);
                emit_headless_error(
                    context,
                    readiness_requirement
                        .map(|requirement| requirement.missing_gateway_call_message)
                        .unwrap_or("Provider 未调用当前终端能力")
                        .into(),
                );
                return false;
            }
            Err(error) => {
                context.failed.store(true, Ordering::Release);
                emit_headless_error(context, error);
                return false;
            }
        }
    }
    true
}

fn has_required_gateway_call<R: Runtime>(
    context: &HeadlessOutputContext<'_, R>,
) -> Result<bool, String> {
    let Some(token) = context.bridge_token else {
        return Ok(false);
    };
    context.gateway.has_tool_calls(token)
}

/// 只有通过全部前置校验的行才能进入公共 AI 输出事件。
fn emit_headless_line<R: Runtime>(
    stream: &str,
    context: &HeadlessOutputContext<'_, R>,
    data: String,
) {
    let _ = context.app.emit(
        "nocterm://ai-output",
        AiOutputEvent {
            session_id: context.session_id.into(),
            connection_id: context.connection_id,
            stream: stream.into(),
            data,
        },
    );
}

/// 仅识别 Provider 声明的初始化事件；其他 JSONL 事件继续交给输出状态机。
pub(super) fn validate_readiness_event(
    line: &str,
    requirement: &HeadlessReadinessRequirement,
) -> Option<Result<(), String>> {
    let event = serde_json::from_str::<serde_json::Value>(line).ok()?;
    if event.get("type").and_then(serde_json::Value::as_str) != Some(requirement.event_type)
        || event.get("subtype").and_then(serde_json::Value::as_str)
            != Some(requirement.event_subtype)
    {
        return None;
    }
    let tools = event.get("tools").and_then(serde_json::Value::as_array);
    if tools.is_some_and(|tools| {
        tools.iter().any(|tool| {
            tool.as_str()
                .is_some_and(|name| name.starts_with(requirement.tool_prefix))
        })
    }) {
        Some(Ok(()))
    } else {
        Some(Err(requirement.failure_message.into()))
    }
}

/// 协议和读取错误统一映射到 stderr 流，保持前端事件契约稳定。
fn emit_headless_error<R: Runtime>(context: &HeadlessOutputContext<'_, R>, data: String) {
    let _ = context.app.emit(
        "nocterm://ai-output",
        AiOutputEvent {
            session_id: context.session_id.into(),
            connection_id: context.connection_id,
            stream: "stderr".into(),
            data,
        },
    );
}
