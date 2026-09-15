//! 单个 Codex app-server 子进程与 turn 生命周期。

use std::{
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use super::{
    CodexSessionIdentity,
    protocol::{
        codex_provider_config, codex_thread_start_request, terminate_child, wait_for_response,
        write_message,
    },
};
use crate::{
    commands::ai_stream::read_bounded_lines,
    dto::ai::{AiExitEvent, AiOutputEvent},
    state::{AiCommandPolicy, AiGatewayState},
};

pub(super) struct CodexAppServer {
    identity: CodexSessionIdentity,
    thread_id: String,
    stdin: Mutex<ChildStdin>,
    child: Mutex<Child>,
    active_turn: Mutex<Option<ActiveTurn>>,
    next_request_id: AtomicU64,
    alive: AtomicBool,
    bridge_token: Option<String>,
    gateway: Arc<AiGatewayState>,
}

#[derive(Clone)]
struct ActiveTurn {
    session_id: String,
    connection_id: Option<i64>,
    start_request_id: u64,
    turn_id: Option<String>,
    cancelled: bool,
}

impl CodexAppServer {
    pub(super) fn spawn(
        app: AppHandle,
        identity: CodexSessionIdentity,
        bridge: Option<(String, String)>,
        executable: String,
        provider_executable: &PathBuf,
        gateway: Arc<AiGatewayState>,
    ) -> Result<Arc<Self>, String> {
        let mut process = Command::new(provider_executable);
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
        let stderr = child.stderr.take();
        let (line_sender, line_receiver) = mpsc::channel();
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
                    "params": {"clientInfo": {"name": "nocterm", "version": env!("CARGO_PKG_VERSION")}}
                }),
            )?;
            wait_for_response(&line_receiver, 1, "初始化 Codex app-server")?;
            write_message(&mut stdin, &json!({"method": "initialized"}))?;

            let config = codex_provider_config(
                &executable,
                bridge.as_ref().map(|(endpoint, _)| endpoint.as_str()),
            );
            let thread_request = codex_thread_start_request(&identity, config, bridge.is_some());
            write_message(&mut stdin, &thread_request)?;
            let response = wait_for_response(&line_receiver, 2, "创建 Codex thread")?;
            response
                .pointer("/result/thread/id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "Codex thread/start 未返回 thread id".to_string())
        })();
        let thread_id = match bootstrap {
            Ok(thread_id) => thread_id,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };

        let server = Arc::new(Self {
            identity,
            thread_id,
            stdin: Mutex::new(stdin),
            child: Mutex::new(child),
            active_turn: Mutex::new(None),
            next_request_id: AtomicU64::new(3),
            alive: AtomicBool::new(true),
            bridge_token: bridge.map(|(_, token)| token),
            gateway,
        });
        let reader_server = Arc::clone(&server);
        let reader_app = app.clone();
        thread::spawn(move || reader_server.read_stdout(reader_app, line_receiver));
        if let Some(stderr) = stderr {
            let stderr_server = Arc::clone(&server);
            thread::spawn(move || stderr_server.read_stderr(app, stderr));
        }
        Ok(server)
    }

    pub(super) fn matches_identity(&self, identity: &CodexSessionIdentity) -> bool {
        self.identity == *identity
    }

    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub(super) fn start_turn(
        &self,
        app: AppHandle,
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
        if let Some(error) = read_error {
            self.emit_output(&app, "stderr", error);
            self.finish_turn(&app, Some(1), false);
            self.shutdown();
        } else {
            self.finish_turn(&app, None, false);
        }
    }

    fn read_stderr(&self, app: AppHandle, stderr: impl std::io::Read) {
        let result = read_bounded_lines(stderr, |line| {
            self.emit_output(&app, "stderr", line);
            true
        });
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
        let turn = self
            .active_turn
            .lock()
            .ok()
            .and_then(|active| active.clone());
        if let Some(turn) = turn {
            let _ = app.emit(
                "nocterm://ai-output",
                AiOutputEvent {
                    session_id: turn.session_id,
                    connection_id: turn.connection_id,
                    stream: stream.to_string(),
                    data,
                },
            );
        }
    }

    fn finish_turn(&self, app: &AppHandle, code: Option<i32>, cancelled_on_eof: bool) {
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
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(token) = self.bridge_token.as_deref() {
            self.gateway.revoke(token);
        }
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

impl Drop for CodexAppServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}
