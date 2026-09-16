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

use tauri::{AppHandle, Emitter};

use crate::{
    commands::{
        ai_process::AiProcessPoll,
        ai_provider::PreparedHeadlessLaunch,
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

struct HeadlessOutputContext<'a> {
    app: &'a AppHandle,
    session_id: &'a str,
    connection_id: Option<i64>,
    bridge_token: Option<&'a str>,
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
        let output_budget = Arc::clone(&output_budget);
        let output_failed = Arc::clone(&output_failed);
        thread::spawn(move || {
            let context = HeadlessOutputContext {
                app: &app,
                session_id: &session_id,
                connection_id,
                bridge_token: bridge_token.as_deref(),
                budget: &output_budget,
                failed: &output_failed,
            };
            emit_lines("stdout", stdout, &context)
        })
    });
    let stderr_thread = stderr.map(|stderr| {
        let app = app.clone();
        let session_id = session_id.clone();
        let bridge_token = bridge_token.clone();
        let output_budget = Arc::clone(&output_budget);
        let output_failed = Arc::clone(&output_failed);
        thread::spawn(move || {
            let context = HeadlessOutputContext {
                app: &app,
                session_id: &session_id,
                connection_id,
                bridge_token: bridge_token.as_deref(),
                budget: &output_budget,
                failed: &output_failed,
            };
            emit_lines("stderr", stderr, &context)
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
                    if let Some(token) = process.bridge_token.as_deref() {
                        gateway.revoke(token);
                    }
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

fn emit_lines<R: std::io::Read>(
    stream: &str,
    reader: R,
    context: &HeadlessOutputContext<'_>,
) -> bool {
    let result = read_bounded_lines(reader, |line| {
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
        let _ = context.app.emit(
            "nocterm://ai-output",
            AiOutputEvent {
                session_id: context.session_id.into(),
                connection_id: context.connection_id,
                stream: stream.into(),
                data: redact_token(line, context.bridge_token),
            },
        );
        true
    });
    if let Err(error) = result {
        context.failed.store(true, Ordering::Release);
        let _ = context.app.emit(
            "nocterm://ai-output",
            AiOutputEvent {
                session_id: context.session_id.into(),
                connection_id: context.connection_id,
                stream: "stderr".into(),
                data: redact_token(error, context.bridge_token),
            },
        );
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{HeadlessOutputBudget, OutputReservation};
    use crate::commands::ai_stream::{MAX_PROVIDER_TURN_OUTPUT_BYTES, redact_token};

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
}
