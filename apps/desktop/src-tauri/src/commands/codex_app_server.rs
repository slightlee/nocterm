use std::{
    collections::HashMap,
    io::Write,
    path::PathBuf,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

use crate::{
    commands::ai_stream::read_bounded_lines,
    dto::ai::{AiExitEvent, AiOutputEvent},
    state::{AiCommandPolicy, AiGatewayState},
};

const CODEX_LOCAL_TERMINAL_INSTRUCTIONS: &str = concat!(
    "This thread is bound to a visible Nocterm local terminal. For every terminal-related ",
    "request, use only the nocterm MCP server: use session_context when target identity matters ",
    "and local_terminal_exec for commands. Never use Codex command execution, browser or ",
    "computer-use tools, node/cua REPL tools, or another MCP server to inspect or operate the ",
    "machine. Do not claim the terminal capability is unavailable unless the relevant nocterm ",
    "tool was actually called and returned an error. Keep internal server and tool names out of ",
    "user-facing responses; describe actions in natural language."
);
const CODEX_SSH_TERMINAL_INSTRUCTIONS: &str = concat!(
    "This thread is bound to a Nocterm SSH connection. For every server or terminal-related ",
    "request, use only the nocterm MCP server: prefer its structured inspection tools and use ",
    "ssh_exec only when they cannot complete the task. Never use Codex command execution, ",
    "browser or computer-use tools, node/cua REPL tools, or another MCP server to inspect or ",
    "operate the machine. Do not claim the connection capability is unavailable unless the ",
    "relevant nocterm tool was actually called and returned an error. Keep internal server and ",
    "tool names out of user-facing responses; describe actions in natural language."
);
/// app-server 正常应快速确认 interrupt；超过宽限期仍未收尾时必须终止进程，避免失控 turn
/// 永久占用对话并让前端只能等待空闲超时。
const CODEX_INTERRUPT_GRACE_TIMEOUT: Duration = Duration::from_secs(5);

/// Codex app-server 与一个 Nocterm 对话一一对应。目标变化时必须重建，防止旧 MCP token
/// 被用于另一个终端；同一对话的连续 turn 则复用进程、配置和 Codex thread。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexSessionIdentity {
    pub connection_id: Option<i64>,
    pub target_session_id: Option<String>,
    pub working_directory: Option<String>,
}

/// 启动一次 Codex turn 所需的完整上下文；调用方无需依赖长参数列表的位置约定。
pub struct CodexTurnRequest {
    pub conversation_id: String,
    pub session_id: String,
    pub identity: CodexSessionIdentity,
    pub initial_prompt: String,
    pub continuation_prompt: String,
    pub bridge: Option<(String, String)>,
    pub bridge_executable: String,
    pub provider_executable: PathBuf,
    pub command_policy: AiCommandPolicy,
}

#[derive(Default)]
pub struct CodexAppServerManager {
    sessions: Mutex<HashMap<String, Arc<CodexAppServer>>>,
}

struct CodexAppServer {
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

impl CodexAppServerManager {
    pub fn start_turn(
        &self,
        app: AppHandle,
        request: CodexTurnRequest,
        gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String> {
        let CodexTurnRequest {
            conversation_id,
            session_id,
            identity,
            initial_prompt,
            continuation_prompt,
            bridge,
            bridge_executable,
            provider_executable,
            command_policy,
        } = request;
        let (server, is_new) = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "Codex 会话状态不可用".to_string())?;
            let reusable = sessions
                .get(&conversation_id)
                .filter(|server| server.alive.load(Ordering::Acquire))
                .filter(|server| server.identity == identity)
                .cloned();
            if let Some(server) = reusable {
                (server, false)
            } else {
                if let Some(previous) = sessions.remove(&conversation_id) {
                    previous.shutdown();
                }
                let server = CodexAppServer::spawn(
                    app.clone(),
                    identity,
                    bridge,
                    bridge_executable,
                    &provider_executable,
                    gateway,
                )?;
                sessions.insert(conversation_id.clone(), Arc::clone(&server));
                (server, true)
            }
        };
        let prompt = if is_new {
            initial_prompt
        } else {
            continuation_prompt
        };
        if let Err(error) = server.start_turn(app, session_id, prompt, command_policy) {
            if is_new || !server.alive.load(Ordering::Acquire) {
                // 首轮失败或传输已损坏的复用进程都不能继续留在会话表中。
                let removed = {
                    let mut sessions = self
                        .sessions
                        .lock()
                        .map_err(|_| "Codex 会话状态不可用".to_string())?;
                    sessions
                        .get(&conversation_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &server))
                        .then(|| sessions.remove(&conversation_id))
                        .flatten()
                };
                if let Some(removed) = removed {
                    removed.shutdown();
                }
            }
            return Err(error);
        }
        Ok(is_new)
    }

    /// 停止当前 turn 时优先使用协议中断，保留已加载的 app-server 供下一轮复用。
    pub fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        let servers = self
            .sessions
            .lock()
            .map_err(|_| "Codex 会话状态不可用".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for server in servers {
            if server.stop_turn(session_id)? {
                let interrupted_server = Arc::clone(&server);
                let interrupted_session_id = session_id.to_string();
                thread::spawn(move || {
                    thread::sleep(CODEX_INTERRUPT_GRACE_TIMEOUT);
                    interrupted_server.shutdown_if_turn_active(&interrupted_session_id);
                });
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 清空、删除或切换 Provider 时同步销毁对应后台会话和会话级 Bridge token。
    pub fn reset_conversation(&self, conversation_id: &str) -> bool {
        let server = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(conversation_id));
        if let Some(server) = server {
            server.shutdown();
            true
        } else {
            false
        }
    }
}

impl Drop for CodexAppServerManager {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, server) in sessions.drain() {
                server.shutdown();
            }
        }
    }
}

impl CodexAppServer {
    fn spawn(
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

    fn start_turn(
        &self,
        app: AppHandle,
        session_id: String,
        prompt: String,
        command_policy: AiCommandPolicy,
    ) -> Result<(), String> {
        if !self.alive.load(Ordering::Acquire) {
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
        // 先原子占用 turn，再激活工具授权；重复启动不能改写正在运行任务的审计身份。
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

    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
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
            // turn/start 尚未应答时没有可中断 ID；关闭服务器，stdout EOF 会完成前端生命周期。
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
            // 协议中断无法写入时不保留失控的活动 turn，关闭进程触发统一退出清理。
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

    /// Nocterm 的命令审批由 MCP Bridge 自己承接。其他 Codex 交互请求首版没有对应 UI，
    /// 明确拒绝比悬挂整个 turn 更安全，也不会误把 Provider 请求当作客户端响应。
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
            // 文本输出不代表 turn 完成；仅在唯一收尾路径撤销本轮工具授权。
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

    fn shutdown(&self) {
        self.alive.store(false, Ordering::Release);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(token) = self.bridge_token.as_deref() {
            self.gateway.revoke(token);
        }
    }

    /// 只回收仍属于被中断 session 的 turn；其间若旧 turn 已结束且新 turn 已启动，不能误杀新任务。
    fn shutdown_if_turn_active(&self, session_id: &str) {
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

/// 终端路由属于宿主安全约束，必须进入 Codex developer instructions，而不是仅作为
/// 普通用户消息前缀。无目标 thread 不附加约束，仍保留 Provider 的常规 Agent 能力。
fn terminal_developer_instructions(identity: &CodexSessionIdentity) -> Option<&'static str> {
    if identity.connection_id.is_some() {
        Some(CODEX_SSH_TERMINAL_INSTRUCTIONS)
    } else if identity.target_session_id.is_some() {
        Some(CODEX_LOCAL_TERMINAL_INSTRUCTIONS)
    } else {
        None
    }
}

/// 目标已绑定时 Nocterm MCP 是执行能力而非可选增强；启动失败必须阻止 thread 创建，
/// 避免 Codex 在工具缺失时静默改用用户配置中的其他本机工具。
fn codex_provider_config(executable: &str, endpoint: Option<&str>) -> Value {
    let mut config = json!({"features": {"shell_tool": false, "unified_exec": false}});
    if let Some(endpoint) = endpoint {
        config["mcp_servers"] = json!({
            "nocterm": {
                "command": executable,
                "args": ["mcp-stdio", "--endpoint", endpoint],
                "env_vars": ["NOCTERM_MCP_TOKEN"],
                "default_tools_approval_mode": "approve",
                "required": true,
                "startup_timeout_sec": 5
            }
        });
    }
    config
}

/// 把目标约束和 Bridge 配置一次性装入创建请求，避免后续协议修改只更新其中一边。
fn codex_thread_start_request(
    identity: &CodexSessionIdentity,
    config: Value,
    bridge_enabled: bool,
) -> Value {
    let developer_instructions = bridge_enabled
        .then(|| terminal_developer_instructions(identity))
        .flatten();
    json!({
        "id": 2,
        "method": "thread/start",
        "params": {
            "approvalPolicy": "never",
            "cwd": identity.working_directory,
            "developerInstructions": developer_instructions,
            "ephemeral": true,
            "config": config
        }
    })
}

fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|error| format!("编码 Codex app-server 请求失败：{error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("写入 Codex app-server 失败：{error}"))
}

/// `Child` 的 Drop 不会回收进程，启动阶段任一步失败都必须显式终止并等待。
fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn wait_for_response(
    lines: &mpsc::Receiver<Result<String, String>>,
    expected_id: u64,
    action: &str,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("{action}超时"));
        }
        let line = lines
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => format!("{action}超时"),
                mpsc::RecvTimeoutError::Disconnected => {
                    format!("{action}失败：Codex app-server 提前退出")
                }
            })??;
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if message.get("id").and_then(Value::as_u64) != Some(expected_id) {
            continue;
        }
        if let Some(error) = message.pointer("/error/message").and_then(Value::as_str) {
            return Err(format!("{action}失败：{error}"));
        }
        return Ok(message);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use serde_json::json;

    use super::{
        CodexSessionIdentity, codex_provider_config, codex_thread_start_request,
        terminal_developer_instructions, wait_for_response, write_message,
    };

    #[test]
    fn writes_one_json_message_per_line() {
        let mut output = Vec::new();
        write_message(&mut output, &json!({"id": 3, "method": "turn/start"}))
            .expect("write request");
        assert_eq!(
            String::from_utf8(output).expect("utf8"),
            "{\"id\":3,\"method\":\"turn/start\"}\n"
        );
    }

    #[test]
    fn initialization_skips_notifications_before_matching_response() {
        let input = b"{\"method\":\"thread/started\",\"params\":{}}\n{\"id\":2,\"result\":{\"thread\":{\"id\":\"thread-1\"}}}\n";
        let (sender, receiver) = mpsc::channel();
        for line in String::from_utf8_lossy(input).lines() {
            sender.send(Ok(line.to_string())).expect("send line");
        }
        let response = wait_for_response(&receiver, 2, "test").expect("matching response");
        assert_eq!(
            response.pointer("/result/thread/id"),
            Some(&json!("thread-1"))
        );
    }

    #[test]
    fn initialization_surfaces_protocol_errors() {
        let input = b"{\"id\":1,\"error\":{\"message\":\"unsupported\"}}\n";
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(String::from_utf8_lossy(input).trim().to_string()))
            .expect("send line");
        let error = wait_for_response(&receiver, 1, "initialize").expect_err("protocol error");
        assert!(error.contains("unsupported"));
    }

    #[test]
    fn initialization_surfaces_output_reader_errors() {
        let (sender, receiver) = mpsc::channel();
        sender.send(Err("provider stream failed".into())).unwrap();
        let error = wait_for_response(&receiver, 1, "initialize").expect_err("reader error");
        assert!(error.contains("provider stream failed"));
    }

    #[test]
    fn target_bound_threads_require_nocterm_and_route_terminal_tools() {
        let config = codex_provider_config("/Applications/Nocterm", Some("127.0.0.1:4567"));
        assert_eq!(config["mcp_servers"]["nocterm"]["required"], true);
        assert_eq!(config["features"]["shell_tool"], false);
        assert_eq!(config["features"]["unified_exec"], false);

        let local = CodexSessionIdentity {
            connection_id: None,
            target_session_id: Some("local:one".into()),
            working_directory: None,
        };
        let instructions = terminal_developer_instructions(&local).unwrap();
        assert!(instructions.contains("local_terminal_exec"));
        assert!(instructions.contains("Do not claim"));
        let request = codex_thread_start_request(&local, config, true);
        assert_eq!(request["method"], "thread/start");
        assert!(
            request["params"]["developerInstructions"]
                .as_str()
                .is_some_and(|value| value.contains("local_terminal_exec"))
        );
        assert_eq!(
            request["params"]["config"]["mcp_servers"]["nocterm"]["required"],
            true
        );

        let untargeted = CodexSessionIdentity {
            connection_id: None,
            target_session_id: None,
            working_directory: None,
        };
        assert!(terminal_developer_instructions(&untargeted).is_none());
        let untargeted_config = codex_provider_config("/Applications/Nocterm", None);
        assert_eq!(untargeted_config["features"]["shell_tool"], false);
        assert!(untargeted_config.get("mcp_servers").is_none());
        assert!(codex_thread_start_request(&untargeted, untargeted_config, false)["params"]
            ["developerInstructions"]
            .is_null());
    }
}
