use std::{
    io::Write,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Emitter, State};

use crate::{
    commands::{
        ai_policy::parse_command_policy,
        ai_process::AiProcessPoll,
        ai_provider::{
            ProviderBridge, ProviderLaunch, ProviderLaunchPlan, provider_adapter, provider_adapters,
        },
        ai_stream::read_bounded_lines,
        codex_app_server::{CodexSessionIdentity, CodexTurnRequest},
    },
    dto::ai::{
        AiExitEvent, AiOutputEvent, AiProviderStatus, AiSessionStartRequest, AiSessionStartResponse,
    },
    state::{AiGatewayBinding, AiTarget, AppState},
};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);
const MAX_AI_PROMPT_BYTES: usize = 64 * 1024;
const MAX_WORKING_DIRECTORY_BYTES: usize = 4096;

#[tauri::command]
pub fn ai_provider_status() -> Vec<AiProviderStatus> {
    provider_adapters()
        .into_iter()
        .map(|adapter| AiProviderStatus {
            id: adapter.id().into(),
            command: adapter.command().into(),
            available: adapter.executable().is_some(),
        })
        .collect()
}

#[tauri::command(async)]
pub fn ai_session_start(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AiSessionStartRequest,
) -> Result<AiSessionStartResponse, String> {
    let session_id = validate_client_session_id(&request.client_session_id)?;
    let conversation_id = validate_conversation_id(&request.conversation_id)?;
    validate_prompt(&request.prompt, "AI 请求")?;
    validate_prompt(&request.continuation_prompt, "AI 本轮请求")?;
    let working_directory = validate_working_directory(request.working_directory.as_deref())?;
    let command_policy = parse_command_policy(request.command_policy.as_deref())?;
    let requested_local_session_id =
        validate_target_selectors(request.connection_id, request.target_session_id.as_deref())?;
    // 没有 SSH 目标时，Provider 仍可作为本机 Agent 使用；只有远程工具调用才需要连接。
    let target_connection_id = request.connection_id;
    if let Some(connection_id) = target_connection_id {
        state
            .connection_service()
            .get(connection_id)
            .map_err(|_| "目标 SSH 连接不存在或已被删除".to_string())?;
    }
    let adapter =
        provider_adapter(&request.provider).ok_or_else(|| "不支持的 AI Provider".to_string())?;
    let provider_id = adapter.id();
    let command = adapter.command();
    let provider_executable = adapter
        .executable()
        .ok_or_else(|| format!("未找到本机 AI Provider：{command}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    let target = if let Some(connection_id) = target_connection_id {
        Some(AiTarget::Ssh { connection_id })
    } else if let Some(session_id) = requested_local_session_id.as_deref() {
        let terminal_id = state
            .local_terminals()
            .terminal_for(session_id)
            .ok_or_else(|| "当前本地终端尚未就绪或已关闭".to_string())?;
        Some(AiTarget::Local {
            session_id: session_id.to_string(),
            terminal_id,
        })
    } else {
        None
    };
    // 所有可能失败的无副作用准备都放在 token 绑定之前，避免启动前留下无主授权。
    let executable = std::env::current_exe()
        .map_err(|error| format!("无法定位 Nocterm AI Bridge：{error}"))?
        .to_str()
        .ok_or_else(|| "Nocterm 可执行文件路径不是有效 UTF-8".to_string())?
        .to_string();
    let bridge_token = if let Some(target) = target.as_ref() {
        let endpoint = state
            .ai_gateway()
            .endpoint()
            .ok_or_else(|| "Nocterm AI Bridge 尚未启动，请重启应用后重试".to_string())?;
        let token = generate_bridge_token()?;
        state.ai_gateway().bind(
            token.clone(),
            AiGatewayBinding {
                target: target.clone(),
                provider: provider_id.into(),
                session_id: session_id.clone(),
                command_policy,
            },
        )?;
        Some((endpoint, token))
    } else {
        None
    };
    let bridge = bridge_token.as_ref().map(|(endpoint, _)| ProviderBridge {
        executable: &executable,
        endpoint,
    });
    // Adapter 直接返回互斥的生命周期计划，调用方不会组合出无效模式与参数。
    let launch_plan = match adapter.prepare_launch(ProviderLaunch {
        prompt: &request.prompt,
        bridge,
        timestamp,
        sequence,
    }) {
        Ok(plan) => plan,
        Err(error) => {
            if let Some((_, token)) = bridge_token.as_ref() {
                state.ai_gateway().revoke(token);
            }
            return Err(error);
        }
    };
    if matches!(launch_plan, ProviderLaunchPlan::CodexAppServer) {
        let identity = CodexSessionIdentity {
            connection_id: target_connection_id,
            target_session_id: requested_local_session_id,
            working_directory: working_directory.clone(),
        };
        let started = state.ai_codex_servers().start_turn(
            app,
            CodexTurnRequest {
                conversation_id,
                session_id: session_id.clone(),
                identity,
                initial_prompt: request.prompt,
                continuation_prompt: request.continuation_prompt,
                bridge: bridge_token.clone(),
                bridge_executable: executable,
                provider_executable,
                command_policy,
            },
            Arc::clone(state.ai_gateway()),
        );
        match started {
            Ok(is_new) => {
                // 复用已有服务器时，新候选 token 没有进入子进程，立即撤销其目标绑定。
                if !is_new && let Some((_, token)) = bridge_token.as_ref() {
                    state.ai_gateway().revoke(token);
                }
                return Ok(AiSessionStartResponse { session_id });
            }
            Err(error) => {
                if let Some((_, token)) = bridge_token.as_ref() {
                    state.ai_gateway().revoke(token);
                }
                return Err(error);
            }
        }
    }
    let ProviderLaunchPlan::Headless(mut prepared) = launch_plan else {
        unreachable!("Codex launch plan returned after its dedicated branch");
    };
    if let Some((_, token)) = bridge_token.as_ref()
        && let Err(error) =
            state
                .ai_gateway()
                .activate_session(token, session_id.clone(), command_policy)
    {
        state.ai_gateway().revoke(token);
        return Err(error);
    }
    let stdin_payload = prepared.take_stdin_payload();
    let args = &prepared.args;
    let mut process = Command::new(&provider_executable);
    process
        .args(args)
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
    // 任务 token 只通过子进程环境传递，避免命令行和 Provider 配置文件泄露凭据。
    if let Some((_, token)) = bridge_token.as_ref() {
        process.env("NOCTERM_MCP_TOKEN", token);
    } else {
        process.env_remove("NOCTERM_MCP_TOKEN");
    }
    // 连接/本地终端身份作为受控会话元数据保留；目标通过任务级 MCP token 绑定 Bridge。
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some((_, token)) = bridge_token.as_ref() {
                state.ai_gateway().revoke(token);
            }
            return Err(format!("启动 {command} 失败：{error}"));
        }
    };
    if let Some(payload) = stdin_payload {
        let write_result = child
            .stdin
            .take()
            .ok_or_else(|| format!("{command} 标准输入不可用"))
            .and_then(|mut stdin| {
                stdin
                    .write_all(payload.as_bytes())
                    .and_then(|_| stdin.flush())
                    .map_err(|error| format!("写入 {command} 标准输入失败：{error}"))
            });
        if let Err(error) = write_result {
            let _ = child.kill();
            let _ = child.wait();
            if let Some((_, token)) = bridge_token.as_ref() {
                state.ai_gateway().revoke(token);
            }
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
        target_connection_id,
        bridge_token.as_ref().map(|(_, token)| token.clone()),
        cleanup_paths,
    ) {
        if let Some((_, token)) = bridge_token.as_ref() {
            state.ai_gateway().revoke(token);
        }
        return Err(error);
    }
    let manager = Arc::clone(state.ai_processes());
    let gateway = Arc::clone(state.ai_gateway());
    let reader_id = session_id.clone();
    // 两个管道必须并发消费；任一管道缓冲区写满都会阻塞 Provider 退出。
    let stdout_thread = stdout.map(|stdout| {
        let app = app.clone();
        let session_id = session_id.clone();
        thread::spawn(move || emit_lines(&app, &session_id, target_connection_id, "stdout", stdout))
    });
    let stderr_thread = stderr.map(|stderr| {
        let app = app.clone();
        let session_id = session_id.clone();
        thread::spawn(move || emit_lines(&app, &session_id, target_connection_id, "stderr", stderr))
    });
    let exit_app = app.clone();
    thread::spawn(move || {
        let (code, cancelled) = loop {
            match manager.poll(&reader_id) {
                AiProcessPoll::Running => thread::sleep(Duration::from_millis(25)),
                AiProcessPoll::Finished(mut process) => {
                    // 进程表中的目标身份与事件闭包中的身份必须一致，避免未来改动时串任务。
                    debug_assert_eq!(process.connection_id, target_connection_id);
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
                            connection_id: target_connection_id,
                            stream: "stderr".into(),
                            data: error,
                        },
                    );
                    break (None, false);
                }
            }
        };
        // wait 完成后管道应已到 EOF；先回收读线程，保证退出事件之前不会再追加输出。
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
                connection_id: target_connection_id,
                code,
                cancelled,
            },
        );
    });
    Ok(AiSessionStartResponse { session_id })
}

fn validate_client_session_id(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if !(value.starts_with("ai-")
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-'))
    {
        return Err("AI 会话标识格式无效".to_string());
    }
    Ok(value.to_string())
}

fn validate_conversation_id(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("AI 对话标识格式无效".to_string());
    }
    Ok(value.to_string())
}

fn validate_prompt(raw: &str, label: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err(format!("{label}不能为空"));
    }
    if raw.len() > MAX_AI_PROMPT_BYTES {
        return Err(format!("{label}不能超过 64 KiB"));
    }
    if raw.contains('\0') {
        return Err(format!("{label}包含不允许的空字符"));
    }
    Ok(())
}

fn validate_working_directory(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() > MAX_WORKING_DIRECTORY_BYTES || value.contains('\0') {
        return Err("AI 工作目录无效或超过 4096 字节".to_string());
    }
    Ok(Some(value.to_string()))
}

/// 终端目标必须是 SSH、本地或无目标三者之一；歧义和无效连接 ID 在 IPC 边界直接拒绝。
fn validate_target_selectors(
    connection_id: Option<i64>,
    target_session_id: Option<&str>,
) -> Result<Option<String>, String> {
    if connection_id.is_some_and(|value| value <= 0) {
        return Err("目标 SSH 连接标识无效".to_string());
    }
    let local_session_id = target_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if local_session_id.as_deref().is_some_and(|value| {
        !value.starts_with("local:")
            || value.len() > 128
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_')
            })
    }) {
        return Err("目标本地终端会话标识无效".to_string());
    }
    if connection_id.is_some() && local_session_id.is_some() {
        return Err("AI 会话不能同时绑定本地终端和 SSH 连接".to_string());
    }
    Ok(local_session_id)
}

/// Bridge token 直接使用操作系统 CSPRNG；随机源不可用时禁止启动带工具的会话。
fn generate_bridge_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成安全的 AI Bridge token".to_string())?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push(HEX[usize::from(byte >> 4)] as char);
        token.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    Ok(token)
}

#[tauri::command]
pub fn ai_session_stop(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    // 先关闭 turn 级授权，确保停止请求与 Provider 中断并发时不会再启动新工具调用。
    state.ai_gateway().deactivate_session(&session_id)?;
    if state.ai_codex_servers().stop_turn(&session_id)? {
        return Ok(());
    }
    let mut process = state
        .ai_processes()
        .remove(&session_id)
        .map_err(|error| format!("停止 AI 会话失败：{error}"))?
        .ok_or_else(|| "AI 会话不存在".to_string())?;
    // 先取得 kill 结果，再无条件撤销 Bridge；即使进程刚好自然退出，token 也不能继续有效。
    let kill_result = process.child.kill();
    if let Some(token) = process.bridge_token.as_deref() {
        state.ai_gateway().revoke(token);
    }
    kill_result.map_err(|error| format!("停止 AI 会话失败：{error}"))
}

#[tauri::command]
pub fn ai_conversation_reset(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let conversation_id = validate_conversation_id(&conversation_id)?;
    state
        .ai_codex_servers()
        .reset_conversation(&conversation_id);
    Ok(())
}

#[tauri::command]
pub fn ai_tool_approval_resolve(
    state: State<'_, AppState>,
    approval_id: String,
    session_id: String,
    approved: bool,
) -> Result<(), String> {
    if state
        .ai_gateway()
        .resolve_approval(&approval_id, &session_id, approved)?
    {
        Ok(())
    } else {
        Err("AI 命令确认不存在或已过期".to_string())
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

#[cfg(test)]
mod tests {
    use super::{
        generate_bridge_token, validate_client_session_id, validate_conversation_id,
        validate_prompt, validate_target_selectors, validate_working_directory,
    };

    #[test]
    fn accepts_only_bounded_client_session_ids() {
        assert!(validate_client_session_id("ai-123e4567-e89b-12d3-a456-426614174000").is_ok());
        assert!(validate_client_session_id("other-session").is_err());
        assert!(validate_client_session_id("ai-bad/session").is_err());
        assert!(validate_client_session_id(&format!("ai-{}", "x".repeat(70))).is_err());
    }

    #[test]
    fn accepts_only_bounded_conversation_ids() {
        assert!(validate_conversation_id("123e4567-e89b-12d3-a456-426614174000").is_ok());
        assert!(validate_conversation_id("").is_err());
        assert!(validate_conversation_id("bad/session").is_err());
        assert!(validate_conversation_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn bridge_tokens_are_independent_256_bit_lowercase_hex_values() {
        let first = generate_bridge_token().expect("generate first token");
        let second = generate_bridge_token().expect("generate second token");

        assert_eq!(first.len(), 64);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_ne!(first, second);
    }

    #[test]
    fn terminal_target_selectors_are_unambiguous_and_positive() {
        assert_eq!(validate_target_selectors(None, None).unwrap(), None);
        assert_eq!(
            validate_target_selectors(None, Some(" local:one ")).unwrap(),
            Some("local:one".to_string())
        );
        assert_eq!(validate_target_selectors(Some(7), None).unwrap(), None);
        assert!(validate_target_selectors(Some(0), None).is_err());
        assert!(validate_target_selectors(Some(-1), None).is_err());
        assert!(validate_target_selectors(Some(7), Some("local:one")).is_err());
        assert!(validate_target_selectors(None, Some("remote:one")).is_err());
        assert!(validate_target_selectors(None, Some("local:bad/path")).is_err());
        assert!(
            validate_target_selectors(None, Some(&format!("local:{}", "x".repeat(130)))).is_err()
        );
    }

    #[test]
    fn prompt_and_working_directory_inputs_are_bounded() {
        assert!(validate_prompt("inspect", "request").is_ok());
        assert!(validate_prompt("", "request").is_err());
        assert!(validate_prompt("bad\0prompt", "request").is_err());
        assert!(validate_prompt(&"x".repeat(64 * 1024 + 1), "request").is_err());
        assert_eq!(
            validate_working_directory(Some(" /tmp/project ")).unwrap(),
            Some("/tmp/project".into())
        );
        assert!(validate_working_directory(Some("bad\0path")).is_err());
    }
}
