//! Grok Agent Client Protocol 的消息构造、边界校验与初始化响应读取。

use std::{
    collections::VecDeque,
    io::Write,
    process::Child,
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::commands::{ai_persistent::receive_startup_line, ai_provider::ProviderSessionIdentity};

const GROK_LOCAL_TERMINAL_RULES: &str = concat!(
    "This session is bound to a visible Nocterm local terminal. For every terminal-related ",
    "request, use only the connected Nocterm MCP server. Its complete tool catalog is ",
    "session_context and local_terminal_exec. Make exactly one search_tool call covering every ",
    "required exact tool name, then call the discovered tools through use_tool. Do not repeat ",
    "discovery. Use session_context only when the user asks about target identity; otherwise use ",
    "only local_terminal_exec with the smallest command needed. Never use another tool or server ",
    "to inspect or operate the machine. State only requested facts returned by tools."
);
const GROK_SSH_TERMINAL_RULES: &str = concat!(
    "This session is bound to a Nocterm SSH connection. For every server or terminal-related ",
    "request, use only the connected Nocterm MCP server. Its complete tool catalog is ",
    "session_context, get_system_info, list_processes, list_listening_ports, get_service_status, ",
    "read_service_logs, get_disk_usage, get_memory_usage, list_docker_containers, get_docker_info, ",
    "get_docker_container_status, read_docker_logs, and ssh_exec. Make exactly one search_tool ",
    "call covering every required exact tool name, then call the discovered tools through ",
    "use_tool. Do not repeat discovery. Prefer the narrow structured tool; use ssh_exec only for ",
    "a missing fact, and use session_context only when the user asks about target identity. Never ",
    "inspect unrelated state. The SSH command channel reuses the current authenticated connection ",
    "to the same server, is not a new environment, and does not inherit temporary interactive-shell ",
    "state. State only requested facts returned by tools."
);
const MAX_BOOTSTRAP_PENDING_MESSAGES: usize = 128;
const MAX_BOOTSTRAP_PENDING_BYTES: usize = 1024 * 1024;
pub(super) const NOCTERM_MCP_SERVER_ID: &str = "nocterm";
// ACP 在线路上为扩展方法增加 `_` 前缀；官方内部扩展名仍是 `x.ai/mcp/sdk_call`。
pub(super) const GROK_MCP_SDK_CALL_METHOD: &str = "_x.ai/mcp/sdk_call";
const MIN_SUPPORTED_GROK_VERSION: [u64; 3] = [1, 0, 34];
const MIN_SUPPORTED_GROK_VERSION_TEXT: &str = "1.0.34";

#[derive(Default)]
pub(super) struct BootstrapMessages {
    messages: VecDeque<(Value, usize)>,
    bytes: usize,
}

impl BootstrapMessages {
    /// 握手响应前到达的通知必须暂存，但 Provider 不能借此制造无界内存增长。
    pub(super) fn push(&mut self, message: Value, encoded_bytes: usize) -> Result<(), String> {
        let next_bytes = self
            .bytes
            .checked_add(encoded_bytes)
            .ok_or_else(|| "Grok ACP 启动事件累计大小溢出".to_string())?;
        if self.messages.len() >= MAX_BOOTSTRAP_PENDING_MESSAGES
            || next_bytes > MAX_BOOTSTRAP_PENDING_BYTES
        {
            return Err("Grok ACP 启动阶段返回了过多待处理事件".to_string());
        }
        self.messages.push_back((message, encoded_bytes));
        self.bytes = next_bytes;
        Ok(())
    }

    fn pop_front(&mut self) -> Option<Value> {
        let (message, encoded_bytes) = self.messages.pop_front()?;
        self.bytes = self.bytes.saturating_sub(encoded_bytes);
        Some(message)
    }
}

pub(super) fn initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": 1,
            "clientCapabilities": {
                "fs": {"readTextFile": false, "writeTextFile": false},
                "terminal": false
            },
            "clientInfo": {
                "name": "nocterm",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

/// Nocterm 是会话唯一 MCP；Grok 通过官方 ACP 反向请求调用进程内公共 Gateway。
pub(super) fn session_new_request(cwd: &str, identity: &ProviderSessionIdentity) -> Value {
    let rules = if identity.connection_id.is_some() {
        Some(GROK_SSH_TERMINAL_RULES)
    } else if identity.target_session_id.is_some() {
        Some(GROK_LOCAL_TERMINAL_RULES)
    } else {
        None
    };
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "session/new",
        "params": {
            "cwd": cwd,
            "mcpServers": [],
            "_meta": {
                "yoloMode": true,
                "rules": rules,
                "x.ai/mcp/servers": [{
                    "name": "nocterm",
                    "serverId": NOCTERM_MCP_SERVER_ID
                }]
            }
        }
    })
}

/// 当前产品只支持经过真实握手验证的 Grok MCP-over-ACP，旧版必须显式升级。
pub(super) fn require_supported_sdk_mcp(initialize_response: &Value) -> Result<(), String> {
    let version_text = initialize_response
        .pointer("/result/_meta/agentVersion")
        .and_then(Value::as_str)
        .filter(|version| !version.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "无法识别当前 Grok 版本，请升级到 Grok {MIN_SUPPORTED_GROK_VERSION_TEXT} 或更高版本后重试"
            )
        })?;
    let version = parse_core_version(version_text).ok_or_else(|| {
        format!(
            "无法识别当前 Grok 版本 {version_text}，请升级到 Grok {MIN_SUPPORTED_GROK_VERSION_TEXT} 或更高版本后重试"
        )
    })?;
    if version < MIN_SUPPORTED_GROK_VERSION {
        return Err(format!(
            "当前 Grok 版本 {version_text} 不受支持，请升级到 Grok {MIN_SUPPORTED_GROK_VERSION_TEXT} 或更高版本后重试"
        ));
    }
    let supports_sdk_mcp = initialize_response
        .pointer("/result/_meta/x.ai~1mcp~1sdk")
        .and_then(Value::as_bool)
        == Some(true);
    if !supports_sdk_mcp {
        return Err(format!(
            "当前 Grok {version_text} 不支持 Nocterm 所需的 MCP-over-ACP，请升级到 Grok {MIN_SUPPORTED_GROK_VERSION_TEXT} 或更高版本后重试"
        ));
    }
    Ok(())
}

fn parse_core_version(version: &str) -> Option<[u64; 3]> {
    let core = version
        .split_once(['-', '+', ' '])
        .map_or(version, |(core, _)| core);
    let mut parts = core.split('.');
    let parsed = [
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ];
    parts.next().is_none().then_some(parsed)
}

/// 把 Grok 的 ACP 扩展请求转换为嵌套 MCP JSON-RPC 响应；工具执行仍由公共分发器完成。
pub(super) fn sdk_call_response(
    request: &Value,
    dispatch: impl FnOnce(&Value) -> Option<Value>,
) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("method").and_then(Value::as_str) != Some(GROK_MCP_SDK_CALL_METHOD) {
        return json_rpc_error(id, -32601, "ACP client method not found");
    }
    if request.pointer("/params/serverId").and_then(Value::as_str) != Some(NOCTERM_MCP_SERVER_ID) {
        return json_rpc_error(id, -32602, "unknown MCP serverId");
    }
    let Some(message) = request
        .pointer("/params/message")
        .filter(|value| value.is_object())
    else {
        return json_rpc_error(id, -32602, "missing MCP message");
    };
    if message.get("id").is_none_or(Value::is_null) {
        return json_rpc_error(id, -32602, "MCP-over-ACP requires a request id");
    }
    let Some(response) = dispatch(message) else {
        return json_rpc_error(id, -32602, "MCP-over-ACP does not support notifications");
    };
    json!({"jsonrpc":"2.0","id":id,"result":response})
}

fn json_rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

/// Grok 的 session/new 不等待 MCP 握手；目标绑定时必须额外等待完成通知并检查工具数。
pub(super) fn wait_for_mcp_ready_with_requests(
    lines: &mpsc::Receiver<Result<String, String>>,
    pending: &mut BootstrapMessages,
    session_id: &str,
    cancellation: &AtomicBool,
    mut handle_request: impl FnMut(&Value) -> Result<bool, String>,
) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let message = if let Some(message) = pending.pop_front() {
            message
        } else {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("初始化 Nocterm MCP 超时".to_string());
            }
            let line = receive_startup_line(
                lines,
                deadline,
                cancellation,
                "初始化 Nocterm MCP",
                "初始化 Nocterm MCP 失败：Grok ACP 进程提前退出",
            )?;
            serde_json::from_str::<Value>(&line)
                .map_err(|_| "初始化 Nocterm MCP 失败：Grok ACP 返回了无效 JSON".to_string())?
        };
        if message.get("id").is_some()
            && message.get("method").is_some()
            && handle_request(&message)?
        {
            continue;
        }
        if message.get("method").and_then(Value::as_str) != Some("_x.ai/mcp_initialized")
            || message.pointer("/params/sessionId").and_then(Value::as_str) != Some(session_id)
        {
            continue;
        }
        let tool_count = message
            .pointer("/params/mcpToolCount")
            .and_then(Value::as_u64)
            .ok_or_else(|| "初始化 Nocterm MCP 失败：Grok 未返回工具数量".to_string())?;
        return if tool_count > 0 {
            Ok(())
        } else {
            Err("初始化 Nocterm MCP 失败：未发现可用终端工具".to_string())
        };
    }
}

pub(super) fn prompt_request(request_id: u64, session_id: &str, prompt: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "session/prompt",
        "params": {
            "sessionId": session_id,
            "prompt": [{"type": "text", "text": prompt}]
        }
    })
}

pub(super) fn cancel_notification(session_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "session/cancel",
        "params": {"sessionId": session_id}
    })
}

pub(super) fn close_request(request_id: u64, session_id: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "session/close",
        "params": {"sessionId": session_id}
    })
}

pub(super) fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|error| format!("编码 Grok ACP 请求失败：{error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("写入 Grok ACP 失败：{error}"))
}

pub(super) fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
pub(super) fn wait_for_response(
    lines: &mpsc::Receiver<Result<String, String>>,
    expected_id: u64,
    action: &str,
) -> Result<Value, String> {
    wait_for_response_collect(
        lines,
        expected_id,
        action,
        &mut BootstrapMessages::default(),
        &AtomicBool::new(false),
    )
}

pub(super) fn wait_for_response_collect(
    lines: &mpsc::Receiver<Result<String, String>>,
    expected_id: u64,
    action: &str,
    pending: &mut BootstrapMessages,
    cancellation: &AtomicBool,
) -> Result<Value, String> {
    wait_for_response_collect_with_requests(
        lines,
        expected_id,
        action,
        pending,
        cancellation,
        |_| Ok(false),
    )
}

pub(super) fn wait_for_response_collect_with_requests(
    lines: &mpsc::Receiver<Result<String, String>>,
    expected_id: u64,
    action: &str,
    pending: &mut BootstrapMessages,
    cancellation: &AtomicBool,
    mut handle_request: impl FnMut(&Value) -> Result<bool, String>,
) -> Result<Value, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!("{action}超时"));
        }
        let line = receive_startup_line(
            lines,
            deadline,
            cancellation,
            action,
            &format!("{action}失败：Grok ACP 进程提前退出"),
        )?;
        let message = serde_json::from_str::<Value>(&line)
            .map_err(|_| format!("{action}失败：Grok ACP 返回了无效 JSON"))?;
        if message.get("id").is_some()
            && message.get("method").is_some()
            && handle_request(&message)?
        {
            continue;
        }
        if message.get("id").and_then(Value::as_u64) != Some(expected_id) {
            pending.push(message, line.len())?;
            continue;
        }
        if let Some(error) = response_error(&message) {
            return Err(format!("{action}失败：{error}"));
        }
        return Ok(message);
    }
}

pub(super) fn response_error(message: &Value) -> Option<&str> {
    message.pointer("/error/message").and_then(Value::as_str)
}

/// ACP prompt 必须携带明确结束原因；未知值按失败处理，避免把协议漂移误报为成功。
pub(super) fn prompt_completion(
    message: &Value,
    cancellation_requested: bool,
) -> Result<(i32, bool), String> {
    let stop_reason = message
        .pointer("/result/stopReason")
        .and_then(Value::as_str)
        .ok_or_else(|| "Grok ACP 完成响应缺少 stopReason".to_string())?;
    let cancelled = cancellation_requested || matches!(stop_reason, "cancelled" | "user_cancelled");
    let successful = matches!(stop_reason, "end_turn" | "stop_sequence" | "max_tokens");
    Ok((if successful || cancelled { 0 } else { 1 }, cancelled))
}

pub(super) fn is_renderable_update(message: &Value, session_id: &str) -> bool {
    if message.get("method").and_then(Value::as_str) != Some("session/update")
        || message.pointer("/params/sessionId").and_then(Value::as_str) != Some(session_id)
    {
        return false;
    }
    matches!(
        message
            .pointer("/params/update/sessionUpdate")
            .and_then(Value::as_str),
        Some("agent_message_chunk" | "agent_thought_chunk" | "tool_call" | "tool_call_update")
    )
}

#[cfg(test)]
mod tests;
