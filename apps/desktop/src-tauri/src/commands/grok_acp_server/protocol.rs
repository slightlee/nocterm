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
    "request, use only the connected Nocterm MCP server. Use its session context capability ",
    "when target identity matters and its local terminal execution capability for commands. ",
    "Never use another tool or server to inspect or operate the machine. State only facts ",
    "returned by tools and omit unrelated diagnostics."
);
const GROK_SSH_TERMINAL_RULES: &str = concat!(
    "This session is bound to a Nocterm SSH connection. For every server or terminal-related ",
    "request, use only the connected Nocterm MCP server. Prefer its structured inspection ",
    "capabilities and use its general SSH execution capability only when needed. Never use ",
    "another tool or server to inspect or operate the machine. The SSH command channel reuses ",
    "the current authenticated connection to the same server; never describe it as a new ",
    "connection, separate server or independent environment. It does not inherit temporary ",
    "interactive-shell state. State only facts returned by tools and omit unrelated diagnostics."
);
const MAX_BOOTSTRAP_PENDING_MESSAGES: usize = 128;
const MAX_BOOTSTRAP_PENDING_BYTES: usize = 1024 * 1024;

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

/// Nocterm Bridge 是会话唯一 MCP。Token 从 Grok 子进程环境继承，不进入协议消息。
pub(super) fn session_new_request(
    cwd: &str,
    identity: &ProviderSessionIdentity,
    bridge: Option<(&str, &str)>,
) -> Value {
    let mcp_servers = bridge.map_or_else(Vec::new, |(executable, endpoint)| {
        vec![json!({
            "name": "nocterm",
            "command": executable,
            "args": ["mcp-stdio", "--endpoint", endpoint],
            "env": []
        })]
    });
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
            "mcpServers": mcp_servers,
            "_meta": {
                "yoloMode": true,
                "rules": rules
            }
        }
    })
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

/// Grok 的 session/new 不等待 MCP 握手；目标绑定时必须额外等待完成通知并检查工具数。
pub(super) fn wait_for_mcp_ready(
    lines: &mpsc::Receiver<Result<String, String>>,
    pending: &mut BootstrapMessages,
    session_id: &str,
    cancellation: &AtomicBool,
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
mod tests {
    use std::sync::{atomic::AtomicBool, mpsc};

    use serde_json::json;

    use super::{
        BootstrapMessages, MAX_BOOTSTRAP_PENDING_BYTES, cancel_notification, initialize_request,
        is_renderable_update, prompt_completion, prompt_request, session_new_request,
        wait_for_mcp_ready, wait_for_response, write_message,
    };
    use crate::commands::ai_provider::ProviderSessionIdentity;

    fn identity() -> ProviderSessionIdentity {
        ProviderSessionIdentity {
            connection_id: Some(7),
            target_session_id: None,
            working_directory: Some("/srv/app".into()),
        }
    }

    #[test]
    fn initialization_denies_client_filesystem_and_terminal_capabilities() {
        let request = initialize_request();
        assert_eq!(request["params"]["clientCapabilities"]["terminal"], false);
        assert_eq!(
            request["params"]["clientCapabilities"]["fs"]["readTextFile"],
            false
        );
        assert_eq!(request["params"]["protocolVersion"], 1);
    }

    #[test]
    fn session_injects_only_nocterm_without_serializing_the_token() {
        let request = session_new_request(
            "/private/runtime",
            &identity(),
            Some(("/Applications/Nocterm", "127.0.0.1:4567")),
        );
        let encoded = request.to_string();
        assert_eq!(request["params"]["mcpServers"].as_array().unwrap().len(), 1);
        assert_eq!(request["params"]["mcpServers"][0]["name"], "nocterm");
        assert_eq!(request["params"]["_meta"]["yoloMode"], true);
        assert!(encoded.contains("127.0.0.1:4567"));
        assert!(!encoded.contains("NOCTERM_MCP_TOKEN"));
    }

    #[test]
    fn prompt_cancel_and_jsonl_wire_format_follow_acp() {
        assert_eq!(
            prompt_request(3, "grok-one", "inspect")["method"],
            "session/prompt"
        );
        assert_eq!(cancel_notification("grok-one")["method"], "session/cancel");
        let mut output = Vec::new();
        write_message(&mut output, &prompt_request(3, "grok-one", "inspect")).unwrap();
        assert_eq!(output.last(), Some(&b'\n'));
    }

    #[test]
    fn renders_only_supported_updates_for_the_active_grok_session() {
        let message = json!({
            "method": "session/update",
            "params": {
                "sessionId": "grok-one",
                "update": {"sessionUpdate": "agent_message_chunk", "content": {"text": "ok"}}
            }
        });
        assert!(is_renderable_update(&message, "grok-one"));
        assert!(!is_renderable_update(&message, "grok-two"));
        let ignored = json!({
            "method": "session/update",
            "params": {"sessionId": "grok-one", "update": {"sessionUpdate": "user_message_chunk"}}
        });
        assert!(!is_renderable_update(&ignored, "grok-one"));
    }

    #[test]
    fn bootstrap_wait_surfaces_protocol_errors() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(
                json!({"id": 1, "error": {"message": "unsupported"}}).to_string()
            ))
            .unwrap();
        assert!(wait_for_response(&receiver, 1, "initialize").is_err());
    }

    #[test]
    fn prompt_completion_requires_a_stop_reason_and_fails_unknown_values() {
        assert!(prompt_completion(&json!({"result": {}}), false).is_err());
        assert_eq!(
            prompt_completion(&json!({"result": {"stopReason": "end_turn"}}), false),
            Ok((0, false))
        );
        assert_eq!(
            prompt_completion(&json!({"result": {"stopReason": "future_value"}}), false),
            Ok((1, false))
        );
        assert_eq!(
            prompt_completion(&json!({"result": {"stopReason": "cancelled"}}), false),
            Ok((0, true))
        );
    }

    #[test]
    fn mcp_readiness_requires_a_positive_tool_count_for_the_expected_session() {
        let (sender, receiver) = mpsc::channel();
        sender
            .send(Ok(json!({
                "method": "_x.ai/mcp_initialized",
                "params": {"sessionId": "other", "mcpToolCount": 9}
            })
            .to_string()))
            .unwrap();
        sender
            .send(Ok(json!({
                "method": "_x.ai/mcp_initialized",
                "params": {"sessionId": "grok-one", "mcpToolCount": 0}
            })
            .to_string()))
            .unwrap();
        assert!(
            wait_for_mcp_ready(
                &receiver,
                &mut BootstrapMessages::default(),
                "grok-one",
                &AtomicBool::new(false),
            )
            .is_err()
        );

        let ready = json!({
            "method": "_x.ai/mcp_initialized",
            "params": {"sessionId": "grok-one", "mcpToolCount": 12}
        });
        let mut pending = BootstrapMessages::default();
        pending
            .push(ready.clone(), ready.to_string().len())
            .expect("queue readiness event");
        assert!(
            wait_for_mcp_ready(&receiver, &mut pending, "grok-one", &AtomicBool::new(false),)
                .is_ok()
        );
    }

    #[test]
    fn bootstrap_pending_messages_have_a_cumulative_size_limit() {
        let mut pending = BootstrapMessages::default();
        pending
            .push(json!({"method": "small"}), MAX_BOOTSTRAP_PENDING_BYTES)
            .expect("first message fits exactly");
        assert!(
            pending
                .push(json!({"method": "overflow"}), 1)
                .expect_err("pending events must be bounded")
                .contains("过多待处理事件")
        );
    }
}
