//! 单个 Codex app-server 子进程与 turn 生命周期。

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

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Runtime};

use super::{
    CodexSessionIdentity,
    protocol::{
        codex_config_read_request, codex_feature_list_request, codex_provider_config,
        codex_thread_start_request, configured_mcp_server_names, terminate_child,
        validate_disabled_features, validate_thread_security, wait_for_response, write_message,
    },
};
use crate::{
    commands::{
        ai_persistent::{PersistentProviderSession, SessionTermination},
        ai_process::{ManagedChild, StreamCompletion},
        ai_stream::{
            PROVIDER_TURN_OUTPUT_LIMIT_MESSAGE, ProviderStderrRelay, append_startup_diagnostics,
            provider_event_channel, read_bounded_lines, redact_token, reserve_turn_output,
        },
    },
    dto::ai::{AiExitEvent, AiOutputEvent},
    state::{AiCommandPolicy, AiGatewayState},
};

pub(super) struct CodexAppServer {
    identity: CodexSessionIdentity,
    thread_id: String,
    stdin: Mutex<ChildStdin>,
    child: ManagedChild,
    active_turn: Mutex<Option<ActiveTurn>>,
    next_request_id: AtomicU64,
    alive: AtomicBool,
    bridge_token: Option<String>,
    gateway: Arc<AiGatewayState>,
    termination: SessionTermination,
    stderr_completion: StreamCompletion,
}

/// 创建 app-server 所需的 Provider 专属依赖；公共注册表只传递取消和终止钩子。
pub(super) struct CodexAppServerLaunch {
    pub identity: CodexSessionIdentity,
    pub bridge: Option<(String, String)>,
    pub bridge_executable: String,
    pub provider_executable: PathBuf,
    pub gateway: Arc<AiGatewayState>,
}

#[derive(Clone)]
struct ActiveTurn {
    session_id: String,
    connection_id: Option<i64>,
    start_request_id: u64,
    turn_id: Option<String>,
    cancelled: bool,
    output_bytes: usize,
    output_limit_reached: bool,
}

impl CodexAppServer {
    pub(super) fn spawn(
        app: AppHandle,
        launch: CodexAppServerLaunch,
        startup_cancellation: Arc<AtomicBool>,
        termination: SessionTermination,
    ) -> Result<Arc<Self>, String> {
        let CodexAppServerLaunch {
            identity,
            bridge,
            bridge_executable,
            provider_executable,
            gateway,
        } = launch;
        let mut process = Command::new(&provider_executable);
        process
            .args(["app-server", "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = identity
            .working_directory
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            process.current_dir(directory);
        }
        if let Some((_, token)) = bridge.as_ref() {
            process.env("NOCTERM_MCP_TOKEN", token);
        } else {
            process.env_remove("NOCTERM_MCP_TOKEN");
        }
        let mut child = process
            .spawn()
            .map_err(|error| format!("启动 codex app-server 失败：{error}"))?;
        let Some(mut stdin) = child.stdin.take() else {
            terminate_child(&mut child);
            return Err("Codex app-server 标准输入不可用".to_string());
        };
        let Some(stdout) = child.stdout.take() else {
            terminate_child(&mut child);
            return Err("Codex app-server 标准输出不可用".to_string());
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

        // 初始化和 thread 创建必须在注册长生命周期读线程前完成，失败时同步回收子进程。
        let bootstrap = (|| {
            write_message(
                &mut stdin,
                &json!({
                    "id": 1,
                    "method": "initialize",
                    // 对外标识必须报告产品版本（唯一源 package.json，经 build.rs 注入）；
                    // CARGO_PKG_VERSION 是内部 crate 版本，规范禁止用作产品版本。
                    "params": {"clientInfo": {"name": "nocterm", "version": env!("NOCTERM_PRODUCT_VERSION")}}
                }),
            )?;
            wait_for_response(
                &line_receiver,
                1,
                "初始化 Codex app-server",
                &startup_cancellation,
            )?;
            write_message(&mut stdin, &json!({"method": "initialized"}))?;

            write_message(
                &mut stdin,
                &codex_config_read_request(identity.working_directory.as_deref()),
            )?;
            let config_response = wait_for_response(
                &line_receiver,
                2,
                "读取 Codex 生效配置",
                &startup_cancellation,
            )?;
            let configured_mcp_servers = configured_mcp_server_names(&config_response)?;
            let config = codex_provider_config(
                &bridge_executable,
                bridge.as_ref().map(|(endpoint, _)| endpoint.as_str()),
                &configured_mcp_servers,
            );
            let thread_request = codex_thread_start_request(&identity, config, bridge.is_some());
            write_message(&mut stdin, &thread_request)?;
            let response = wait_for_response(
                &line_receiver,
                3,
                "创建 Codex thread",
                &startup_cancellation,
            )?;
            validate_thread_security(&response)?;
            let thread_id = response
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "Codex thread/start 未返回 thread id".to_string())?;
            write_message(&mut stdin, &codex_feature_list_request(&thread_id))?;
            let feature_response = wait_for_response(
                &line_receiver,
                4,
                "验证 Codex 执行能力隔离",
                &startup_cancellation,
            )?;
            validate_disabled_features(&feature_response)?;
            Ok(thread_id)
        })();
        let thread_id = match bootstrap {
            Ok(thread_id) => thread_id,
            Err(error) => {
                terminate_child(&mut child);
                stderr_relay.wait(Duration::from_millis(250));
                return Err(redact_token(
                    append_startup_diagnostics(error, &stderr_relay),
                    bridge.as_ref().map(|(_, token)| token.as_str()),
                ));
            }
        };

        let server = Arc::new(Self {
            identity,
            thread_id,
            stdin: Mutex::new(stdin),
            child: ManagedChild::new(child),
            active_turn: Mutex::new(None),
            next_request_id: AtomicU64::new(5),
            alive: AtomicBool::new(true),
            bridge_token: bridge.map(|(_, token)| token),
            gateway,
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

    pub(super) fn matches_identity(&self, identity: &CodexSessionIdentity) -> bool {
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
            return Err("Codex app-server 已退出，请重试".to_string());
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        {
            let mut active = self
                .active_turn
                .lock()
                .map_err(|_| "Codex turn 状态不可用".to_string())?;
            if active.is_some() {
                return Err("当前 Codex 会话仍有任务在执行".to_string());
            }
            *active = Some(ActiveTurn {
                session_id: session_id.clone(),
                connection_id: self.identity.connection_id,
                start_request_id: request_id,
                turn_id: None,
                cancelled: false,
                output_bytes: 0,
                output_limit_reached: false,
            });
        }
        // 先原子占用 turn，再激活授权；重复启动不能改写正在运行任务的审计身份。
        if let Some(token) = self.bridge_token.as_deref()
            && let Err(error) =
                self.gateway
                    .activate_session(token, session_id.clone(), command_policy)
        {
            if let Ok(mut active) = self.active_turn.lock()
                && active
                    .as_ref()
                    .is_some_and(|turn| turn.session_id == session_id)
            {
                *active = None;
            }
            return Err(error);
        }
        let request = json!({
            "id": request_id,
            "method": "turn/start",
            "params": {
                "threadId": self.thread_id,
                "input": [{"type": "text", "text": prompt}]
            }
        });
        let result = self
            .stdin
            .lock()
            .map_err(|_| "Codex app-server 标准输入不可用".to_string())
            .and_then(|mut stdin| write_message(&mut *stdin, &request));
        if let Err(error) = result {
            self.alive.store(false, Ordering::Release);
            self.finish_turn(&app, Some(1), false);
            return Err(error);
        }
        Ok(())
    }

    pub(super) fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        let turn = {
            let mut active = self
                .active_turn
                .lock()
                .map_err(|_| "Codex turn 状态不可用".to_string())?;
            let Some(turn) = active.as_mut().filter(|turn| turn.session_id == session_id) else {
                return Ok(false);
            };
            turn.cancelled = true;
            turn.clone()
        };
        let Some(turn_id) = turn.turn_id else {
            // turn/start 尚未应答时没有可中断 ID，关闭服务器并由 stdout EOF 收尾。
            self.shutdown();
            return Ok(true);
        };
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "id": request_id,
            "method": "turn/interrupt",
            "params": {"threadId": self.thread_id, "turnId": turn_id}
        });
        let result = self
            .stdin
            .lock()
            .map_err(|_| "Codex app-server 标准输入不可用".to_string())
            .and_then(|mut stdin| write_message(&mut *stdin, &request));
        if let Err(error) = result {
            // 协议中断无法写入时不保留失控 turn，关闭进程触发统一清理。
            self.shutdown();
            return Err(error);
        }
        Ok(true)
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
        // 持续协议没有发出 turn 完成事件便退出，即使进程代码为 0 也属于任务失败。
        self.finish_turn(
            &app,
            Some(exit_code.filter(|code| *code != 0).unwrap_or(1)),
            false,
        );
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
            self.emit_output(app, "stderr", line.to_string());
            return;
        };
        if message.get("id").is_some() && message.get("method").is_some() {
            self.reject_server_request(app, &message);
            return;
        }
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            self.handle_response(app, id, &message);
            return;
        }
        let method = message.get("method").and_then(Value::as_str).unwrap_or("");
        match method {
            "item/agentMessage/delta"
            | "item/reasoning/summaryTextDelta"
            | "item/started"
            | "item/completed" => self.emit_output(app, "stdout", line.to_string()),
            "turn/completed" => {
                let status = message
                    .pointer("/params/turn/status")
                    .and_then(Value::as_str);
                let error = message
                    .pointer("/params/turn/error/message")
                    .and_then(Value::as_str);
                if let Some(error) = error {
                    self.emit_output(app, "stderr", error.to_string());
                }
                let cancelled = self
                    .active_turn
                    .lock()
                    .ok()
                    .and_then(|active| active.as_ref().map(|turn| turn.cancelled))
                    .unwrap_or(status == Some("interrupted"));
                self.finish_turn(
                    app,
                    Some(if status == Some("completed") { 0 } else { 1 }),
                    cancelled,
                );
            }
            _ => {}
        }
    }

    /// Bridge 承接命令审批；其他 Codex 交互请求没有对应 UI，必须明确拒绝。
    fn reject_server_request(&self, app: &AppHandle, message: &Value) {
        let Some(id) = message.get("id").cloned() else {
            return;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let response = json!({
            "id": id,
            "error": {"code": -32601, "message": "Nocterm 暂不支持该 Codex 交互请求"}
        });
        let write_result = self
            .stdin
            .lock()
            .map_err(|_| "Codex app-server 标准输入不可用".to_string())
            .and_then(|mut stdin| write_message(&mut *stdin, &response));
        self.emit_output(
            app,
            "stderr",
            if let Err(error) = write_result {
                error
            } else {
                format!("Codex 请求 {method} 未被当前 Nocterm 界面支持")
            },
        );
    }

    fn handle_response(&self, app: &AppHandle, id: u64, message: &Value) {
        let mut failed = None;
        if let Ok(mut active) = self.active_turn.lock()
            && let Some(turn) = active.as_mut().filter(|turn| turn.start_request_id == id)
        {
            if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
                failed = Some(error.to_string());
            } else {
                turn.turn_id = message
                    .pointer("/result/turn/id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if turn.turn_id.is_none() {
                    failed = Some("Codex turn/start 未返回 turn id".to_string());
                }
            }
        }
        if let Some(error) = failed {
            self.emit_output(app, "stderr", error);
            self.finish_turn(app, Some(1), false);
        }
    }

    fn emit_output(&self, app: &AppHandle, stream: &str, data: String) {
        let data = redact_token(data, self.bridge_token.as_deref());
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
            // 仅在唯一收尾路径撤销本轮工具授权，文本输出不代表 turn 已完成。
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
        self.child.terminate();
        if let Some(token) = self.bridge_token.as_deref() {
            self.gateway.revoke(token);
        }
        self.termination.notify();
    }

    /// 只回收仍属于被中断 session 的 turn，不能误杀其间启动的新任务。
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

impl PersistentProviderSession for CodexAppServer {
    type TurnContext = AppHandle;

    fn matches_identity(&self, identity: &CodexSessionIdentity) -> bool {
        CodexAppServer::matches_identity(self, identity)
    }

    fn is_alive(&self) -> bool {
        CodexAppServer::is_alive(self)
    }

    fn start_turn(
        &self,
        app: Self::TurnContext,
        session_id: String,
        prompt: String,
        command_policy: AiCommandPolicy,
    ) -> Result<(), String> {
        CodexAppServer::start_turn(self, app, session_id, prompt, command_policy)
    }

    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        CodexAppServer::stop_turn(self, session_id)
    }

    fn shutdown(&self) {
        CodexAppServer::shutdown(self);
    }

    fn shutdown_if_turn_active(&self, session_id: &str) {
        CodexAppServer::shutdown_if_turn_active(self, session_id);
    }
}

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::ai_stream::redact_token;

    #[test]
    fn bridge_tokens_never_leave_the_codex_output_boundary() {
        assert_eq!(
            redact_token("endpoint token=secret-123".into(), Some("secret-123")),
            "endpoint token=[REDACTED]"
        );
        assert_eq!(redact_token("plain".into(), None), "plain");
    }
}
