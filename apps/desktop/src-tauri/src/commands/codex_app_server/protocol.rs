//! Codex app-server JSON-RPC 协议构造与初始化响应读取。

use std::{
    io::Write,
    process::Child,
    sync::mpsc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::CodexSessionIdentity;

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

/// 终端路由属于宿主安全约束，必须进入 developer instructions，而不是普通消息前缀。
fn terminal_developer_instructions(identity: &CodexSessionIdentity) -> Option<&'static str> {
    if identity.connection_id.is_some() {
        Some(CODEX_SSH_TERMINAL_INSTRUCTIONS)
    } else if identity.target_session_id.is_some() {
        Some(CODEX_LOCAL_TERMINAL_INSTRUCTIONS)
    } else {
        None
    }
}

/// 目标已绑定时 Nocterm MCP 是必需执行能力；启动失败必须阻止 thread 创建。
pub(super) fn codex_provider_config(executable: &str, endpoint: Option<&str>) -> Value {
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

/// 把目标约束和 Bridge 配置一次性装入 thread 创建请求。
pub(super) fn codex_thread_start_request(
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

pub(super) fn write_message(writer: &mut impl Write, message: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *writer, message)
        .map_err(|error| format!("编码 Codex app-server 请求失败：{error}"))?;
    writer
        .write_all(b"\n")
        .and_then(|_| writer.flush())
        .map_err(|error| format!("写入 Codex app-server 失败：{error}"))
}

/// `Child` 的 Drop 不会回收进程，启动阶段任一步失败都必须显式终止并等待。
pub(super) fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

pub(super) fn wait_for_response(
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
        codex_provider_config, codex_thread_start_request, terminal_developer_instructions,
        wait_for_response, write_message,
    };
    use crate::commands::codex_app_server::CodexSessionIdentity;

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
