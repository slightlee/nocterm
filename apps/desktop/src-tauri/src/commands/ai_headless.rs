//! Headless Provider 子进程生命周期。
//! 本模块独占 stdin 写入、并发输出读取、进程登记和退出事件顺序。

use std::{
    io::Write,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::Duration,
};

use tauri::{AppHandle, Emitter};

use crate::{
    commands::{
        ai_process::AiProcessPoll, ai_provider::PreparedHeadlessLaunch,
        ai_stream::read_bounded_lines,
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
    // 两个管道必须并发消费；任一缓冲区写满都会阻塞 Provider 退出。
    let stdout_thread = stdout.map(|stdout| {
        let app = app.clone();
        let session_id = session_id.clone();
        thread::spawn(move || emit_lines(&app, &session_id, connection_id, "stdout", stdout))
    });
    let stderr_thread = stderr.map(|stderr| {
        let app = app.clone();
        let session_id = session_id.clone();
        thread::spawn(move || emit_lines(&app, &session_id, connection_id, "stderr", stderr))
    });
    let exit_app = app.clone();
    thread::spawn(move || {
        let (code, cancelled) = loop {
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
    app: &AppHandle,
    session_id: &str,
    connection_id: Option<i64>,
    stream: &str,
    reader: R,
) -> bool {
    let result = read_bounded_lines(reader, |line| {
        let _ = app.emit(
            "nocterm://ai-output",
            AiOutputEvent {
                session_id: session_id.into(),
                connection_id,
                stream: stream.into(),
                data: line,
            },
        );
        true
    });
    if let Err(error) = result {
        let _ = app.emit(
            "nocterm://ai-output",
            AiOutputEvent {
                session_id: session_id.into(),
                connection_id,
                stream: "stderr".into(),
                data: error,
            },
        );
        return false;
    }
    true
}
