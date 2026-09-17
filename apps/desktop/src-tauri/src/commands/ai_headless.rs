//! Headless Provider 子进程生命周期。
//! 本模块独占 stdin 写入、并发输出读取、进程登记和退出事件顺序。

use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use tauri::{AppHandle, Emitter, Runtime};

use crate::{
    commands::{
        ai_process::AiProcessPoll,
        ai_provider::{HeadlessReadinessRequirement, PreparedHeadlessLaunch},
        ai_stream::{
            PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE, read_bounded_lines, redact_token,
            reserve_turn_output,
        },
    },
    dto::ai::{AiExitEvent, AiOutputEvent},
    state::AppState,
};

pub(super) struct HeadlessSessionLaunch {
    pub session_id: String,
    pub provider_command: &'static str,
    pub provider_executable: PathBuf,
    pub prepared: PreparedHeadlessLaunch,
    pub working_directory: Option<String>,
    pub connection_id: Option<i64>,
    pub bridge_token: Option<String>,
}

#[derive(Default)]
struct HeadlessOutputBudget {
    bytes: Mutex<usize>,
    limit_reached: AtomicBool,
}

struct HeadlessOutputContext<'a, R: Runtime = tauri::Wry> {
    app: &'a AppHandle<R>,
    session_id: &'a str,
    connection_id: Option<i64>,
    bridge_token: Option<&'a str>,
    gateway: &'a crate::state::AiGatewayState,
    budget: &'a HeadlessOutputBudget,
    failed: &'a AtomicBool,
}

enum OutputReservation {
    Accepted,
    FirstRejection,
    Rejected,
}

impl HeadlessOutputBudget {
    /// stdout 与 stderr 共用一个预算；只有首个超限线程负责向界面报告错误。
    fn reserve(&self, bytes: usize) -> OutputReservation {
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

/// 启动并登记进程后异步回收输出线程，退出事件始终在全部输出之后发送。
pub(super) fn start_headless_session(
    app: AppHandle,
    state: &AppState,
    launch: HeadlessSessionLaunch,
) -> Result<(), String> {
    let HeadlessSessionLaunch {
        session_id,
        provider_command,
        provider_executable,
        mut prepared,
        working_directory,
        connection_id,
        bridge_token,
    } = launch;
    let stdin_payload = prepared.take_stdin_payload();
    let readiness_requirement = prepared.take_readiness_requirement();
    let mut process = Command::new(&provider_executable);
    process
        .args(&prepared.args)
        .stdin(if stdin_payload.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process.envs(
        prepared
            .environment
            .iter()
            .map(|(key, value)| (key.as_os_str(), value.as_os_str())),
    );
    if let Some(directory) = prepared.current_directory.as_ref() {
        process.current_dir(directory);
    } else if let Some(directory) = working_directory {
        process.current_dir(directory);
    }
    // Token 只通过子进程环境传递，不能进入 argv 或 Provider 配置文件。
    if let Some(token) = bridge_token.as_deref() {
        process.env("NOCTERM_MCP_TOKEN", token);
    } else {
        process.env_remove("NOCTERM_MCP_TOKEN");
    }
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            revoke_bridge(state, bridge_token.as_deref());
            return Err(format!("启动 {provider_command} 失败：{error}"));
        }
    };
    if let Some(payload) = stdin_payload {
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| format!("{provider_command} 标准输入不可用"))
            .and_then(|mut stdin| {
                stdin
                    .write_all(payload.as_bytes())
                    .and_then(|_| stdin.flush())
                    .map_err(|error| format!("写入 {provider_command} 标准输入失败：{error}"))
            });
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            revoke_bridge(state, bridge_token.as_deref());
            return Err(error);
        }
        // ChildStdin 在此关闭，以 EOF 明确结束 Claude 的 text 输入。
    }
    let cleanup_paths = prepared.take_cleanup_paths();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    if let Err(error) = state.ai_processes().insert(
        session_id.clone(),
        child,
        connection_id,
        bridge_token.clone(),
        cleanup_paths,
    ) {
        revoke_bridge(state, bridge_token.as_deref());
        return Err(error);
    }

    let manager = Arc::clone(state.ai_processes());
    let gateway = Arc::clone(state.ai_gateway());
    let reader_id = session_id.clone();
    let output_budget = Arc::new(HeadlessOutputBudget::default());
    let output_failed = Arc::new(AtomicBool::new(false));
    // 两个管道必须并发消费；任一缓冲区写满都会阻塞 Provider 退出。
    let stdout_thread = stdout.map(|stdout| {
        let app = app.clone();
        let session_id = session_id.clone();
        let bridge_token = bridge_token.clone();
        let gateway = Arc::clone(&gateway);
        let output_budget = Arc::clone(&output_budget);
        let output_failed = Arc::clone(&output_failed);
        thread::spawn(move || {
            let context = HeadlessOutputContext {
                app: &app,
                session_id: &session_id,
                connection_id,
                bridge_token: bridge_token.as_deref(),
                gateway: &gateway,
                budget: &output_budget,
                failed: &output_failed,
            };
            emit_lines("stdout", stdout, &context, readiness_requirement.as_ref())
        })
    });
    let stderr_thread = stderr.map(|stderr| {
        let app = app.clone();
        let session_id = session_id.clone();
        let bridge_token = bridge_token.clone();
        let gateway = Arc::clone(&gateway);
        let output_budget = Arc::clone(&output_budget);
        let output_failed = Arc::clone(&output_failed);
        thread::spawn(move || {
            let context = HeadlessOutputContext {
                app: &app,
                session_id: &session_id,
                connection_id,
                bridge_token: bridge_token.as_deref(),
                gateway: &gateway,
                budget: &output_budget,
                failed: &output_failed,
            };
            emit_lines("stderr", stderr, &context, None)
        })
    });
    let exit_app = app.clone();
    thread::spawn(move || {
        let (code, cancelled) = loop {
            // 协议输出损坏或累计超限后主动终止，不能等待仍在运行的 Provider 自行退出。
            if output_failed.load(Ordering::Acquire) {
                match manager.remove(&reader_id) {
                    Ok(Some(mut process)) => {
                        debug_assert_eq!(process.connection_id, connection_id);
                        if let Some(token) = process.bridge_token.as_deref() {
                            gateway.revoke(token);
                        }
                        let _ = process.child.kill();
                        let _ = process.child.wait();
                        break (Some(1), false);
                    }
                    Ok(None) => break (Some(1), false),
                    Err(error) => {
                        let _ = exit_app.emit(
                            "nocterm://ai-output",
                            AiOutputEvent {
                                session_id: reader_id.clone(),
                                connection_id,
                                stream: "stderr".into(),
                                data: error,
                            },
                        );
                        break (Some(1), false);
                    }
                }
            }
            match manager.poll(&reader_id) {
                AiProcessPoll::Running => thread::sleep(Duration::from_millis(25)),
                AiProcessPoll::Finished(mut process) => {
                    // 进程表和事件闭包中的目标必须一致，避免未来改动时串任务。
                    debug_assert_eq!(process.connection_id, connection_id);
                    break match process.child.wait() {
                        Ok(status) => (status.code(), false),
                        Err(_) => (None, false),
                    };
                }
                AiProcessPoll::Missing => break (None, true),
                AiProcessPoll::Failed(error) => {
                    let _ = exit_app.emit(
                        "nocterm://ai-output",
                        AiOutputEvent {
                            session_id: reader_id.clone(),
                            connection_id,
                            stream: "stderr".into(),
                            data: error,
                        },
                    );
                    break (None, false);
                }
            }
        };
        // 管道 EOF 后先回收读线程，确保退出事件之后不再追加输出。
        let stdout_ok = stdout_thread
            .map(|handle| handle.join().unwrap_or(false))
            .unwrap_or(true);
        let stderr_ok = stderr_thread
            .map(|handle| handle.join().unwrap_or(false))
            .unwrap_or(true);
        // stdout 的完成校验需要读取本轮 Gateway 调用记录，因此必须在回收读线程后撤销。
        if let Some(token) = bridge_token.as_deref() {
            gateway.revoke(token);
        }
        let code = if stdout_ok && stderr_ok {
            code
        } else {
            Some(1)
        };
        let _ = exit_app.emit(
            "nocterm://ai-exit",
            AiExitEvent {
                session_id: reader_id,
                connection_id,
                code,
                cancelled,
            },
        );
    });
    Ok(())
}

fn revoke_bridge(state: &AppState, token: Option<&str>) {
    if let Some(token) = token {
        state.ai_gateway().revoke(token);
    }
}

fn emit_lines<T: std::io::Read, R: Runtime>(
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

fn validate_readiness_event(
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, atomic::AtomicBool};

    use super::{
        HeadlessOutputBudget, HeadlessOutputContext, OutputReservation, emit_lines,
        validate_readiness_event,
    };
    use crate::commands::ai_provider::HeadlessReadinessRequirement;
    use crate::commands::ai_stream::{MAX_PROVIDER_TURN_OUTPUT_BYTES, redact_token};
    use crate::state::{AiCommandPolicy, AiGatewayBinding, AiGatewayState, AiTarget};
    use tauri::Listener;

    #[test]
    fn bridge_tokens_never_leave_headless_output() {
        assert_eq!(
            redact_token("provider token=secret-123".into(), Some("secret-123")),
            "provider token=[REDACTED]"
        );
        assert_eq!(redact_token("plain".into(), None), "plain");
    }

    #[test]
    fn stdout_and_stderr_share_one_headless_output_budget() {
        let budget = HeadlessOutputBudget::default();
        assert!(matches!(
            budget.reserve(MAX_PROVIDER_TURN_OUTPUT_BYTES - 1),
            OutputReservation::Accepted
        ));
        assert!(matches!(budget.reserve(1), OutputReservation::Accepted));
        assert!(matches!(
            budget.reserve(1),
            OutputReservation::FirstRejection
        ));
        assert!(matches!(budget.reserve(1), OutputReservation::Rejected));
    }

    #[test]
    fn readiness_requires_the_expected_provider_tool_prefix() {
        let requirement = HeadlessReadinessRequirement {
            event_type: "system",
            event_subtype: "init",
            tool_prefix: "mcp__nocterm__",
            failure_message: "missing nocterm tools",
            require_gateway_call: true,
            missing_gateway_call_message: "missing nocterm call",
        };

        assert_eq!(
            validate_readiness_event(
                r#"{"type":"system","subtype":"init","tools":["mcp__nocterm__session_context"]}"#,
                &requirement,
            ),
            Some(Ok(()))
        );
        assert_eq!(
            validate_readiness_event(
                r#"{"type":"system","subtype":"init","tools":["Bash"]}"#,
                &requirement,
            ),
            Some(Err("missing nocterm tools".into()))
        );
        assert_eq!(
            validate_readiness_event(r#"{"type":"assistant"}"#, &requirement),
            None
        );
    }

    #[test]
    fn terminal_bound_output_is_not_emitted_without_a_real_gateway_call() {
        let app = tauri::test::mock_app();
        let output_events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&output_events);
        app.listen("nocterm://ai-output", move |event| {
            captured.lock().unwrap().push(event.payload().to_string());
        });
        let gateway = AiGatewayState::default();
        let token = "test-bridge-token";
        gateway
            .bind(
                token.into(),
                AiGatewayBinding {
                    target: AiTarget::Ssh { connection_id: 7 },
                    provider: "claude-code".into(),
                    session_id: "ai-test".into(),
                    command_policy: AiCommandPolicy::AutoSafe,
                },
            )
            .unwrap();
        gateway
            .activate_session(token, "ai-test".into(), AiCommandPolicy::AutoSafe)
            .unwrap();
        let budget = HeadlessOutputBudget::default();
        let failed = AtomicBool::new(false);
        let context = HeadlessOutputContext {
            app: app.handle(),
            session_id: "ai-test",
            connection_id: Some(7),
            bridge_token: Some(token),
            gateway: &gateway,
            budget: &budget,
            failed: &failed,
        };
        let requirement = HeadlessReadinessRequirement {
            event_type: "system",
            event_subtype: "init",
            tool_prefix: "mcp__nocterm__",
            failure_message: "missing nocterm tools",
            require_gateway_call: true,
            missing_gateway_call_message: "missing nocterm call",
        };
        let output = concat!(
            "{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"mcp__nocterm__session_context\"]}\n",
            "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Mings-Mac.local\"}]}}\n"
        );

        assert!(!emit_lines(
            "stdout",
            output.as_bytes(),
            &context,
            Some(&requirement)
        ));
        let events = output_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("missing nocterm call"));
        assert!(!events[0].contains("Mings-Mac.local"));
    }
}
