//! 单个 Grok ACP 子进程、协议会话与 turn 生命周期。

use std::{
    path::PathBuf,
    process::{ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use serde_json::Value;
use tauri::{AppHandle, Emitter, Runtime};

use super::protocol::{
    BootstrapMessages, GROK_MCP_SDK_CALL_METHOD, cancel_notification, close_request,
    initialize_request, is_renderable_update, prompt_completion, prompt_request,
    require_supported_sdk_mcp, response_error, sdk_call_response, session_new_request,
    terminate_child, wait_for_mcp_ready_with_requests, wait_for_response_collect,
    wait_for_response_collect_with_requests, write_message,
};
use crate::{
    commands::{
        ai_bridge::McpRequestDispatcher,
        ai_persistent::{PersistentProviderSession, SessionTermination},
        ai_process::{ManagedChild, StreamCompletion},
        ai_provider::{PreparedGrokAcpLaunch, ProviderSessionIdentity, prepare_grok_acp_launch},
        ai_stream::{
            PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE, ProviderStderrRelay, append_startup_diagnostics,
            provider_event_channel, read_bounded_lines, reserve_turn_output,
        },
    },
    dto::ai::{AiExitEvent, AiOutputEvent},
    state::{AiCommandPolicy, AiGatewayState},
};

pub(super) struct GrokAcpServer {
    identity: ProviderSessionIdentity,
    acp_session_id: String,
    stdin: Mutex<ChildStdin>,
    child: ManagedChild,
    active_turn: Mutex<Option<ActiveTurn>>,
    next_request_id: AtomicU64,
    alive: AtomicBool,
    bridge_token: Option<String>,
    gateway: Arc<AiGatewayState>,
    mcp_dispatcher: Arc<McpRequestDispatcher>,
    prepared: PreparedGrokAcpLaunch,
    termination: SessionTermination,
    stderr_completion: StreamCompletion,
}

/// ACP 进程创建参数属于 Grok Adapter，不进入跨 Provider 生命周期接口。
pub(super) struct GrokAcpServerLaunch {
    pub identity: ProviderSessionIdentity,
    pub bridge_token: Option<String>,
    pub provider_executable: PathBuf,
    pub gateway: Arc<AiGatewayState>,
    pub mcp_dispatcher: Arc<McpRequestDispatcher>,
    pub timestamp: u128,
    pub sequence: u64,
}

#[derive(Clone)]
struct ActiveTurn {
    session_id: String,
    connection_id: Option<i64>,
    request_id: u64,
    cancelled: bool,
    output_bytes: usize,
    output_limit_reached: bool,
}

impl GrokAcpServer {
    pub(super) fn spawn(
        app: AppHandle,
        launch: GrokAcpServerLaunch,
        startup_cancellation: Arc<AtomicBool>,
        termination: SessionTermination,
    ) -> Result<Arc<Self>, String> {
        let GrokAcpServerLaunch {
            identity,
            bridge_token,
            provider_executable,
            gateway,
            mcp_dispatcher,
            timestamp,
            sequence,
        } = launch;
        let prepared = prepare_grok_acp_launch(timestamp, sequence)?;
        let mut process = Command::new(&provider_executable);
        process
            .args(&prepared.args)
            .env_clear()
            .envs(
                prepared
                    .environment
                    .iter()
                    .map(|(key, value)| (key.as_os_str(), value.as_os_str())),
            )
            .current_dir(&prepared.current_directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = process
            .spawn()
            .map_err(|error| format!("启动 Grok ACP 失败：{error}"))?;
        let Some(mut stdin) = child.stdin.take() else {
            terminate_child(&mut child);
            return Err("Grok ACP 标准输入不可用".to_string());
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_child(&mut child);
            return Err("Grok ACP 标准输出不可用".to_string());
        };
        let stderr_relay = Arc::new(ProviderStderrRelay::default());
        if let Some(stderr) = child.stderr.take() {
            let relay = Arc::clone(&stderr_relay);
            thread::spawn(move || relay.capture(stderr));
        }
        let (line_sender, line_receiver) = provider_event_channel();
        thread::spawn(move || {
            let sender = line_sender.clone();
            let result = read_bounded_lines(stdout, |line| line_sender.send(Ok(line)).is_ok());
            if let Err(error) = result {
                let _ = sender.send(Err(error));
            }
        });

        // 初始化期间同步等待固定响应，避免把尚未就绪的进程注册成可复用会话。
        let bootstrap = (|| {
            let mut pending = BootstrapMessages::default();
            write_message(&mut stdin, &initialize_request())?;
            let initialized = wait_for_response_collect(
                &line_receiver,
                1,
                "初始化 Grok ACP",
                &mut pending,
                &startup_cancellation,
            )?;
            if initialized
                .pointer("/result/protocolVersion")
                .and_then(Value::as_u64)
                != Some(1)
            {
                return Err("Grok ACP 返回了不兼容的协议版本".to_string());
            }
            require_supported_sdk_mcp(&initialized)?;
            if bridge_token.is_none() {
                return Err("Nocterm MCP 授权未建立，请重新打开终端后重试".to_string());
            }
            let cwd = prepared
                .current_directory
                .to_str()
                .ok_or_else(|| "Grok 隔离目录不是有效 UTF-8".to_string())?;
            let session_request = session_new_request(cwd, &identity);
            write_message(&mut stdin, &session_request)?;
            let mut handle_request = |request: &Value| {
                handle_server_request(
                    &mut stdin,
                    request,
                    bridge_token.as_deref(),
                    &mcp_dispatcher,
                )
            };
            let response = wait_for_response_collect_with_requests(
                &line_receiver,
                2,
                "创建 Grok ACP 会话",
                &mut pending,
                &startup_cancellation,
                &mut handle_request,
            )?;
            let session_id = response
                .pointer("/result/sessionId")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "Grok session/new 未返回 session id".to_string())?;
            wait_for_mcp_ready_with_requests(
                &line_receiver,
                &mut pending,
                &session_id,
                &startup_cancellation,
                &mut handle_request,
            )?;
            Ok(session_id)
        })();
        let acp_session_id = match bootstrap {
            Ok(session_id) => session_id,
            Err(error) => {
                terminate_child(&mut child);
                stderr_relay.wait(Duration::from_millis(250));
                return Err(redact_bridge_token(
                    prepared.redact(append_startup_diagnostics(error, &stderr_relay)),
                    bridge_token.as_deref(),
                ));
            }
        };

        let server = Arc::new(Self {
            identity,
            acp_session_id,
            stdin: Mutex::new(stdin),
            child: ManagedChild::new(child),
            active_turn: Mutex::new(None),
            next_request_id: AtomicU64::new(3),
            alive: AtomicBool::new(true),
            bridge_token,
            gateway,
            mcp_dispatcher,
            prepared,
            termination,
            stderr_completion: StreamCompletion::default(),
        });
        let reader_server = Arc::clone(&server);
        let reader_app = app.clone();
        thread::spawn(move || reader_server.read_stdout(reader_app, line_receiver));
        let stderr_server = Arc::clone(&server);
        let stderr_lines = stderr_relay.attach();
        thread::spawn(move || stderr_server.read_stderr(app, stderr_lines));
        Ok(server)
    }

    pub(super) fn matches_identity(&self, identity: &ProviderSessionIdentity) -> bool {
        self.identity == *identity
    }

    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(super) fn start_turn<R: Runtime>(
        &self,
        app: AppHandle<R>,
        session_id: String,
        prompt: String,
        command_policy: AiCommandPolicy,
    ) -> Result<(), String> {
        if !self.is_alive() {
            return Err("Grok ACP 已退出，请重试".to_string());
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut active = self
                .active_turn
                .lock()
                .map_err(|_| "Grok turn 状态不可用".to_string())?;
            if active.is_some() {
                return Err("当前 Grok 会话仍有任务在执行".to_string());
            }
            *active = Some(ActiveTurn {
                session_id: session_id.clone(),
                connection_id: self.identity.connection_id,
                request_id,
                cancelled: false,
                output_bytes: 0,
                output_limit_reached: false,
            });
        }
        if let Some(token) = self.bridge_token.as_deref()
            && let Err(error) =
                self.gateway
                    .activate_session(token, session_id.clone(), command_policy)
        {
            self.clear_turn(&session_id);
            return Err(error);
        }
        let request = prompt_request(request_id, &self.acp_session_id, &prompt);
        let result = self
            .stdin
            .lock()
            .map_err(|_| "Grok ACP 标准输入不可用".to_string())
            .and_then(|mut stdin| write_message(&mut *stdin, &request));
        if let Err(error) = result {
            self.alive.store(false, Ordering::Release);
            self.finish_turn(&app, Some(1), false);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        {
            let mut active = self
                .active_turn
                .lock()
                .map_err(|_| "Grok turn 状态不可用".to_string())?;
            let Some(turn) = active.as_mut().filter(|turn| turn.session_id == session_id) else {
                return Ok(false);
            };
            turn.cancelled = true;
        }
        let _ = self.gateway.deactivate_session(session_id);
        let request = cancel_notification(&self.acp_session_id);
        let result = self
            .stdin
            .lock()
            .map_err(|_| "Grok ACP 标准输入不可用".to_string())
            .and_then(|mut stdin| write_message(&mut *stdin, &request));
        if let Err(error) = result {
            self.shutdown();
            return Err(error);
        }
        Ok(true)
    }

    fn clear_turn(&self, session_id: &str) {
        if let Ok(mut active) = self.active_turn.lock()
            && active
                .as_ref()
                .is_some_and(|turn| turn.session_id == session_id)
        {
            *active = None;
        }
    }

    fn read_stdout(self: Arc<Self>, app: AppHandle, lines: mpsc::Receiver<Result<String, String>>) {
        let mut read_error = None;
        for result in lines {
            match result {
                Ok(line) => self.handle_message(&app, &line),
                Err(error) => {
                    read_error = Some(error);
                    break;
                }
            }
        }
        self.alive.store(false, Ordering::Release);
        if let Some(token) = self.bridge_token.as_deref() {
            self.gateway.revoke(token);
        }
        let exit_code = self.child.wait_after_output_closed(Duration::from_secs(1));
        self.stderr_completion.wait(Duration::from_millis(500));
        if let Some(error) = read_error {
            self.emit_output(&app, "stderr", error);
        }
        self.finish_turn(
            &app,
            Some(exit_code.filter(|code| *code != 0).unwrap_or(1)),
            false,
        );
        self.prepared.cleanup();
        self.termination.notify();
    }

    fn read_stderr(&self, app: AppHandle, lines: mpsc::Receiver<Result<String, String>>) {
        let result = lines.into_iter().try_for_each(|line| match line {
            Ok(line) => {
                self.emit_output(&app, "stderr", line);
                Ok(())
            }
            Err(error) => Err(error),
        });
        self.stderr_completion.complete();
        if let Err(error) = result {
            self.emit_output(&app, "stderr", error);
            self.finish_turn(&app, Some(1), false);
            self.shutdown();
        }
    }

    fn handle_message(&self, app: &AppHandle, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            self.emit_output(app, "stderr", "Grok ACP 返回了无效 JSON".to_string());
            self.finish_turn(app, Some(1), false);
            self.shutdown();
            return;
        };
        if message.get("id").is_some() && message.get("method").is_some() {
            self.handle_server_request(app, &message);
            return;
        }
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            self.handle_response(app, id, &message);
            return;
        }
        if is_renderable_update(&message, &self.acp_session_id) {
            self.emit_output(app, "stdout", line.to_string());
        }
    }

    fn handle_server_request(&self, app: &AppHandle, message: &Value) {
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let write_result = self
            .stdin
            .lock()
            .map_err(|_| "Grok ACP 标准输入不可用".to_string())
            .and_then(|mut stdin| {
                handle_server_request(
                    &mut stdin,
                    message,
                    self.bridge_token.as_deref(),
                    &self.mcp_dispatcher,
                )
                .map(|_| ())
            });
        if let Err(error) = write_result {
            self.emit_output(app, "stderr", error);
            self.shutdown();
            return;
        }
        if method != GROK_MCP_SDK_CALL_METHOD {
            self.emit_output(
                app,
                "stderr",
                format!("Grok 请求 {method} 未被当前 Nocterm 界面支持"),
            );
        }
    }

    fn handle_response(&self, app: &AppHandle, id: u64, message: &Value) {
        let active = self
            .active_turn
            .lock()
            .ok()
            .and_then(|active| active.clone());
        let Some(turn) = active.filter(|turn| turn.request_id == id) else {
            return;
        };
        if message.get("error").is_some() {
            let error = response_error(message).unwrap_or("Grok ACP 返回了未说明原因的协议错误");
            self.emit_output(app, "stderr", error.to_string());
            self.finish_turn(app, Some(1), false);
            return;
        }
        match prompt_completion(message, turn.cancelled) {
            Ok((code, cancelled)) => self.finish_turn(app, Some(code), cancelled),
            Err(error) => {
                self.emit_output(app, "stderr", error);
                self.finish_turn(app, Some(1), false);
            }
        }
    }

    fn emit_output(&self, app: &AppHandle, stream: &str, data: String) {
        let data = redact_bridge_token(self.prepared.redact(data), self.bridge_token.as_deref());
        let Some((session_id, connection_id, limit_reached)) =
            self.active_turn.lock().ok().and_then(|mut active| {
                let turn = active.as_mut()?;
                if turn.output_limit_reached {
                    return None;
                }
                let within_budget = reserve_turn_output(&mut turn.output_bytes, data.len());
                if !within_budget {
                    turn.output_limit_reached = true;
                }
                Some((turn.session_id.clone(), turn.connection_id, !within_budget))
            })
        else {
            return;
        };
        let _ = app.emit(
            "nocterm://ai-output",
            AiOutputEvent {
                session_id,
                connection_id,
                stream: if limit_reached { "stderr" } else { stream }.to_string(),
                data: if limit_reached {
                    PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE.to_string()
                } else {
                    data
                },
            },
        );
        if limit_reached {
            self.finish_turn(app, Some(1), false);
            self.shutdown();
        }
    }

    fn finish_turn<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        code: Option<i32>,
        cancelled_on_eof: bool,
    ) {
        let turn = self
            .active_turn
            .lock()
            .ok()
            .and_then(|mut active| active.take());
        if let Some(turn) = turn {
            let _ = self.gateway.deactivate_session(&turn.session_id);
            let _ = app.emit(
                "nocterm://ai-exit",
                AiExitEvent {
                    session_id: turn.session_id,
                    connection_id: turn.connection_id,
                    code,
                    cancelled: turn.cancelled || cancelled_on_eof,
                },
            );
        }
    }

    pub(super) fn shutdown(&self) {
        self.alive.store(false, Ordering::Release);
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = write_message(
                &mut *stdin,
                &close_request(request_id, &self.acp_session_id),
            );
        }
        self.child.terminate();
        self.prepared.cleanup();
        if let Some(token) = self.bridge_token.as_deref() {
            self.gateway.revoke(token);
        }
        self.termination.notify();
    }

    pub(super) fn shutdown_if_turn_active(&self, session_id: &str) {
        let still_active = self
            .active_turn
            .lock()
            .ok()
            .and_then(|active| active.as_ref().map(|turn| turn.session_id == session_id))
            .unwrap_or(false);
        if still_active {
            self.shutdown();
        }
    }
}

/// 握手和运行阶段共用同一反向请求处理，避免 session/new 期间 MCP 初始化死锁。
fn handle_server_request(
    writer: &mut ChildStdin,
    request: &Value,
    bridge_token: Option<&str>,
    dispatcher: &McpRequestDispatcher,
) -> Result<bool, String> {
    let response = if let Some(token) = bridge_token {
        sdk_call_response(request, |message| dispatcher.dispatch(token, message))
    } else {
        sdk_call_response(request, |_| None)
    };
    write_message(writer, &response)?;
    Ok(true)
}

impl PersistentProviderSession for GrokAcpServer {
    type TurnContext = AppHandle;

    fn matches_identity(&self, identity: &ProviderSessionIdentity) -> bool {
        GrokAcpServer::matches_identity(self, identity)
    }

    fn is_alive(&self) -> bool {
        GrokAcpServer::is_alive(self)
    }

    fn start_turn(
        &self,
        app: Self::TurnContext,
        session_id: String,
        prompt: String,
        command_policy: AiCommandPolicy,
    ) -> Result<(), String> {
        GrokAcpServer::start_turn(self, app, session_id, prompt, command_policy)
    }

    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        GrokAcpServer::stop_turn(self, session_id)
    }

    fn shutdown(&self) {
        GrokAcpServer::shutdown(self);
    }

    fn shutdown_if_turn_active(&self, session_id: &str) {
        GrokAcpServer::shutdown_if_turn_active(self, session_id);
    }
}

fn redact_bridge_token(mut text: String, token: Option<&str>) -> String {
    if let Some(token) = token.filter(|token| !token.is_empty()) {
        text = text.replace(token, "[REDACTED]");
    }
    text
}

impl Drop for GrokAcpServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(all(test, unix))]
#[path = "session/tests.rs"]
mod tests;
