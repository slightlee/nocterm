//! Headless Provider 子进程生命周期。
//! 本模块独占 stdin 写入、并发输出读取、进程登记和退出事件顺序。

use std::{
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use nocterm_application::ai_native_session::{AiNativeSessionLease, AiNativeSessionService};
use nocterm_domain::ai_native_session::{AiNativeSessionIdentity, AiNativeSessionRecord};
use tauri::{AppHandle, Emitter};

use crate::{
    commands::{
        ai_persistent::receive_startup_line,
        ai_process::{AiProcessPoll, AiProcessStartup},
        ai_provider::PreparedHeadlessLaunch,
    },
    dto::ai::{AiExitEvent, AiOutputEvent},
    state::AppState,
};

mod output;

use output::{HeadlessOutputBudget, HeadlessOutputContext, emit_lines};

pub(super) struct HeadlessSessionLaunch {
    pub startup: AiProcessStartup,
    pub session_id: String,
    pub provider_command: &'static str,
    pub provider_executable: PathBuf,
    pub prepared: PreparedHeadlessLaunch,
    pub working_directory: Option<String>,
    pub connection_id: Option<i64>,
    pub bridge_token: Option<String>,
    pub conversation_id: String,
    pub native_session_identity: AiNativeSessionIdentity,
    pub native_session_lease: AiNativeSessionLease,
    pub native_session_service: AiNativeSessionService,
}

/// 启动并登记进程后异步回收输出线程，退出事件始终在全部输出之后发送。
pub(super) fn start_headless_session(
    app: AppHandle,
    state: &AppState,
    launch: HeadlessSessionLaunch,
) -> Result<(), String> {
    let HeadlessSessionLaunch {
        startup,
        session_id,
        provider_command,
        provider_executable,
        mut prepared,
        working_directory,
        connection_id,
        bridge_token,
        conversation_id,
        native_session_identity,
        native_session_lease,
        native_session_service,
    } = launch;
    startup.check()?;
    native_session_service
        .ensure_current(&native_session_lease)
        .map_err(|error| error.message)?;
    if prepared
        .native_session()
        .is_some_and(|session| session.resumed)
    {
        match probe_claude_resume(
            &provider_executable,
            &prepared,
            working_directory.as_deref(),
            startup.cancellation(),
        )? {
            ClaudeResumeProbe::Available => {}
            ClaudeResumeProbe::Missing => {
                native_session_service
                    .clear_missing_binding(&native_session_lease)
                    .map_err(|error| error.message)?;
                prepared
                    .apply_resume_fallback()
                    .ok_or_else(|| "Claude Code 文本降级状态不可用".to_string())?;
            }
        }
    }
    // Claude 的 UUID 由 Nocterm 预分配，但只有 Provider 的初始化事件能证明会话已建立。
    // 启动前写库会在 spawn、stdin 或 MCP 初始化失败时留下无法恢复的悬空绑定。
    let native_session_record =
        prepared
            .native_session()
            .map(|native_session| AiNativeSessionRecord {
                conversation_id: conversation_id.clone(),
                native_session_id: native_session.id.clone(),
                identity: native_session_identity,
                updated_at_ms: current_unix_ms(),
            });
    let stdin_payload = prepared.take_stdin_payload();
    let readiness_requirement = prepared.take_readiness_requirement();
    let mut process = build_headless_command(
        &provider_executable,
        &prepared.args,
        &prepared,
        working_directory.as_deref(),
        bridge_token.as_deref(),
        stdin_payload.is_some(),
    );
    startup.check()?;
    native_session_service
        .ensure_current(&native_session_lease)
        .map_err(|error| error.message)?;
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            revoke_bridge(state, bridge_token.as_deref());
            return Err(format!("启动 {provider_command} 失败：{error}"));
        }
    };
    let stdin = child.stdin.take();
    let cleanup_paths = prepared.take_cleanup_paths();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    // 在写入 Prompt 前交接 Child，停止操作可以终止阻塞的 stdin 写入。
    // insert 和 cancel_startup 共用锁，拒绝取消之后迟到的 spawn 结果。
    if let Err(error) = state.ai_processes().insert(
        &startup,
        child,
        connection_id,
        bridge_token.clone(),
        cleanup_paths,
    ) {
        revoke_bridge(state, bridge_token.as_deref());
        return Err(error);
    }
    let write_result = (|| {
        startup.check()?;
        native_session_service
            .ensure_current(&native_session_lease)
            .map_err(|error| error.message)?;
        if let Some(payload) = stdin_payload {
            stdin
                .ok_or_else(|| format!("{provider_command} 标准输入不可用"))
                .and_then(|mut stdin| {
                    stdin
                        .write_all(payload.as_bytes())
                        .and_then(|_| stdin.flush())
                        .map_err(|error| format!("写入 {provider_command} 标准输入失败：{error}"))
                })?;
        }
        // ChildStdin 在此关闭，以 EOF 明确结束 Claude 的 text 输入。
        startup.check()
    })();
    if let Err(error) = write_result {
        // AiProcess::drop 负责 kill/wait 和目录清理，不能遗留已登记但无人轮询的进程。
        let removed = state.ai_processes().remove(&session_id);
        revoke_bridge(state, bridge_token.as_deref());
        drop(removed?);
        return Err(error);
    }

    let manager = Arc::clone(state.ai_processes());
    let gateway = Arc::clone(state.ai_gateway());
    let reader_id = session_id.clone();
    let output_budget = Arc::new(HeadlessOutputBudget::default());
    let output_failed = Arc::new(AtomicBool::new(false));
    // 读线程完成信号：Provider 退出后其派生子进程可能仍持有管道写端，
    // 读线程感知不到 EOF——exit 事件不能无界等待它们，只能限时等待。
    let (stdout_done_tx, stdout_done_rx) = mpsc::channel::<bool>();
    let (stderr_done_tx, stderr_done_rx) = mpsc::channel::<bool>();
    // stdout 提前关闭不代表进程已结束；启动所有权必须覆盖进程回收与退出通知。
    let process_lease = native_session_lease.clone();
    // 两个管道必须并发消费；任一缓冲区写满都会阻塞 Provider 退出。
    let stdout_thread = stdout.map(|stdout| {
        let app = app.clone();
        let session_id = session_id.clone();
        let bridge_token = bridge_token.clone();
        let gateway = Arc::clone(&gateway);
        let output_budget = Arc::clone(&output_budget);
        let output_failed = Arc::clone(&output_failed);
        let native_session_service = native_session_service.clone();
        thread::spawn(move || {
            let context = HeadlessOutputContext {
                emitter: &app,
                session_id: &session_id,
                connection_id,
                bridge_token: bridge_token.as_deref(),
                gateway: &gateway,
                budget: &output_budget,
                failed: &output_failed,
            };
            let persist_native_session = || {
                let Some(record) = native_session_record.as_ref() else {
                    return Ok(());
                };
                native_session_service
                    .save(&native_session_lease, record)
                    .map_err(|error| error.message)
            };
            let ok = emit_lines(
                "stdout",
                stdout,
                &context,
                readiness_requirement.as_ref(),
                Some(&persist_native_session),
            );
            drop(native_session_lease);
            let _ = stdout_done_tx.send(ok);
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
                emitter: &app,
                session_id: &session_id,
                connection_id,
                bridge_token: bridge_token.as_deref(),
                gateway: &gateway,
                budget: &output_budget,
                failed: &output_failed,
            };
            let ok = emit_lines("stderr", stderr, &context, None, None);
            let _ = stderr_done_tx.send(ok);
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
        // 限时等待读线程收尾：Provider 退出后其派生的子进程（MCP 等）可能仍
        // 持有管道写端，EOF 不会到来——无界等待会让 exit 事件永不发出，前端
        // 任务永远停在"输出中"。超时视为管道异常（ok=false），照常发 exit；
        // 未退出的读线程就地 detach，随应用退出回收。
        let pipe_teardown_deadline = Instant::now() + Duration::from_secs(3);
        let stdout_ok = stdout_thread
            .map(|_| {
                stdout_done_rx
                    .recv_timeout(pipe_teardown_deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(false)
            })
            .unwrap_or(true);
        let stderr_ok = stderr_thread
            .map(|_| {
                stderr_done_rx
                    .recv_timeout(pipe_teardown_deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or(false)
            })
            .unwrap_or(true);
        // stdout 的完成校验需要读取本轮 Gateway 调用记录，因此必须在回收读线程后撤销。
        if let Some(token) = bridge_token.as_deref() {
            gateway.revoke(token);
        }
        let mut code = if stdout_ok && stderr_ok {
            code
        } else {
            Some(1)
        };
        // 读线程可能仍持有租约；显式结束本代际，拒绝迟到写回且不阻塞下一轮。
        if let Err(error) = native_session_service.finish(&process_lease) {
            code = Some(1);
            let _ = exit_app.emit(
                "nocterm://ai-output",
                AiOutputEvent {
                    session_id: reader_id.clone(),
                    connection_id,
                    stream: "stderr".into(),
                    data: error.message,
                },
            );
        }
        drop(process_lease);
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

#[derive(Debug, PartialEq, Eq)]
enum ClaudeResumeProbe {
    Available,
    Missing,
}

/// Claude 没有独立的 session inspect 命令。stream-json 在读取 Prompt 前就会完成
/// session lookup；safe-mode 禁用用户 Hook/Plugin，使探测不会执行用户自动化。
fn probe_claude_resume(
    provider_executable: &std::path::Path,
    prepared: &PreparedHeadlessLaunch,
    working_directory: Option<&str>,
    cancellation: &AtomicBool,
) -> Result<ClaudeResumeProbe, String> {
    let session_id = prepared
        .native_session()
        .filter(|session| session.resumed)
        .map(|session| session.id.as_str())
        .ok_or_else(|| "Claude Code 恢复候选无效".to_string())?;
    let mut args = prepared.args.clone();
    if let Some(index) = args
        .iter()
        .position(|argument| argument == "--input-format")
        && let Some(format) = args.get_mut(index + 1)
    {
        *format = "stream-json".into();
    }
    if let Some(index) = args.iter().position(|argument| argument == "--mcp-config")
        && let Some(config) = args.get_mut(index + 1)
    {
        // 探测只读取本地 session 元数据，不能启动 MCP 或携带一次性 Gateway token。
        *config = r#"{"mcpServers":{}}"#.into();
    }
    args.push("--safe-mode".into());
    let mut command = build_headless_command(
        provider_executable,
        &args,
        prepared,
        working_directory,
        None,
        false,
    );
    let (status, stdout, stderr) =
        run_resume_probe(&mut command, cancellation, Duration::from_secs(10))?;
    classify_claude_probe(&stdout, &stderr, session_id, status.success())
}

/// 进程退出和两条输出流共用截止时间；后代占有写端时不阻塞 join，取消也覆盖收尾阶段。
fn run_resume_probe(
    command: &mut Command,
    cancellation: &AtomicBool,
    timeout: Duration,
) -> Result<(std::process::ExitStatus, String, String), String> {
    if cancellation.load(Ordering::Acquire) {
        return Err("探测 Claude Code 原生会话已取消".into());
    }
    let deadline = Instant::now() + timeout;
    let mut child = command
        .spawn()
        .map_err(|error| format!("探测 Claude Code 原生会话失败：{error}"))?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = spawn_probe_reader(stdout);
    let stderr_reader = spawn_probe_reader(stderr);
    let result = (|| {
        let status = loop {
            if cancellation.load(Ordering::Acquire) {
                return Err("探测 Claude Code 原生会话已取消".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) => {
                    return Err("探测 Claude Code 原生会话超时".into());
                }
                Err(error) => {
                    return Err(format!("读取 Claude Code 原生会话状态失败：{error}"));
                }
            }
        };
        let stdout = receive_startup_line(
            &stdout_reader,
            deadline,
            cancellation,
            "读取 Claude Code 探测输出",
            "Claude Code 探测输出线程意外退出",
        )?;
        let stderr = receive_startup_line(
            &stderr_reader,
            deadline,
            cancellation,
            "读取 Claude Code 探测诊断",
            "Claude Code 探测诊断线程意外退出",
        )?;
        Ok((status, stdout, stderr))
    })();
    // Child 没有自动回收语义，所有失败和取消路径均经过同一收尾。
    if result.is_err() {
        let _ = child.kill();
    }
    let _ = child.wait();
    result
}

fn spawn_probe_reader<T: Read + Send + 'static>(
    stream: Option<T>,
) -> mpsc::Receiver<Result<String, String>> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let _ = sender.send(read_probe_output(stream));
    });
    receiver
}

fn classify_claude_probe(
    stdout: &str,
    stderr: &str,
    session_id: &str,
    exit_success: bool,
) -> Result<ClaudeResumeProbe, String> {
    let reports_missing_session = stdout.lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line).is_ok_and(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("result")
                && event.get("subtype").and_then(serde_json::Value::as_str)
                    == Some("error_during_execution")
                && event.get("num_turns").and_then(serde_json::Value::as_u64) == Some(0)
                && event.get("session_id").and_then(serde_json::Value::as_str) == Some(session_id)
                && event
                    .get("errors")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|errors| {
                        errors.iter().any(|error| {
                            error.as_str()
                                == Some(&format!(
                                    "No conversation found with session ID: {session_id}"
                                ))
                        })
                    })
        })
    });
    // 只有 CLI 同时以失败状态报告精确的零轮 missing-session 结果时才允许文本降级。
    // 认证、协议或其他错误即使包含相似文案，也不能清除原生 session 绑定。
    if !exit_success && reports_missing_session {
        return Ok(ClaudeResumeProbe::Missing);
    }
    if !exit_success {
        return Err("Claude Code 原生会话探测未成功完成，已停止以避免重复执行".into());
    }

    // Claude Code 2.1.330 会在 session lookup 成功、stdin 立即 EOF 时以 0 退出且
    // 不产生任何输出。失败 session 则以非零退出并返回上面的结构化结果。
    let reports_silent_success = stdout.trim().is_empty() && stderr.trim().is_empty();
    let reports_loaded_session = stdout.lines().any(|line| {
        serde_json::from_str::<serde_json::Value>(line).is_ok_and(|event| {
            event.get("type").and_then(serde_json::Value::as_str) == Some("system")
                && event.get("session_id").and_then(serde_json::Value::as_str) == Some(session_id)
        })
    }) || stderr
        .contains("Provide a prompt to continue the conversation")
        || stderr.contains("No deferred tool marker found in the resumed session");
    if reports_silent_success || reports_loaded_session {
        Ok(ClaudeResumeProbe::Available)
    } else {
        Err("Claude Code 未返回可判定的原生会话状态，已停止以避免重复执行".into())
    }
}

const MAX_CLAUDE_PROBE_BYTES: u64 = 1024 * 1024;

fn read_probe_output<T: Read>(stream: Option<T>) -> Result<String, String> {
    let Some(stream) = stream else {
        return Ok(String::new());
    };
    let mut bytes = Vec::new();
    stream
        .take(MAX_CLAUDE_PROBE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("读取 Claude Code 探测输出失败：{error}"))?;
    if bytes.len() as u64 > MAX_CLAUDE_PROBE_BYTES {
        return Err("Claude Code 原生会话探测输出过大".into());
    }
    String::from_utf8(bytes).map_err(|_| "Claude Code 原生会话探测输出不是有效 UTF-8".into())
}

fn build_headless_command(
    provider_executable: &std::path::Path,
    args: &[String],
    prepared: &PreparedHeadlessLaunch,
    working_directory: Option<&str>,
    bridge_token: Option<&str>,
    stdin_piped: bool,
) -> Command {
    let mut process = Command::new(provider_executable);
    crate::commands::ai_process::hide_console_window(&mut process);
    process
        .args(args)
        .stdin(if stdin_piped {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .envs(
            prepared
                .environment
                .iter()
                .map(|(key, value)| (key.as_os_str(), value.as_os_str())),
        );
    if let Some(path) = crate::commands::ai_provider::provider_environment_path(provider_executable)
    {
        process.env("PATH", path);
    }
    crate::commands::ai_provider::apply_local_network_bypass(&mut process);
    if let Some(directory) = prepared.current_directory.as_ref() {
        process.current_dir(directory);
    } else if let Some(directory) = working_directory {
        process.current_dir(directory);
    }
    if let Some(token) = bridge_token {
        process.env("NOCTERM_MCP_TOKEN", token);
    } else {
        process.env_remove("NOCTERM_MCP_TOKEN");
    }
    process
}

fn current_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .clamp(1, i64::MAX as u128) as i64
}

fn revoke_bridge(state: &AppState, token: Option<&str>) {
    if let Some(token) = token {
        state.ai_gateway().revoke(token);
    }
}

#[cfg(test)]
mod tests;
