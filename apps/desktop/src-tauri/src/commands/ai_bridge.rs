//! Provider 进程使用的最小 MCP stdio Bridge。
//! stdio 子进程只负责协议适配，真正的 SSH 操作仍在 Tauri 进程内完成。
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use tauri::AppHandle;

use crate::{
    commands::{ai_policy::AI_BRIDGE_RESPONSE_TIMEOUT, ai_tool_gateway::GatewayServices},
    state::{AiGatewayState, AppState},
};

const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_GATEWAY_CONNECTIONS: usize = 16;
const GATEWAY_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// 本地 Gateway 连接也必须有界；Drop 保证处理线程正常返回或 panic 时都归还额度。
struct GatewayConnectionPermit(Arc<AtomicUsize>);

impl Drop for GatewayConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

pub fn start_gateway(app: AppHandle, state: &AppState) {
    let gateway = Arc::clone(state.ai_gateway());
    let services = Arc::new(GatewayServices::new(app, state));
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(_) => return,
    };
    let Ok(address) = listener.local_addr() else {
        return;
    };
    let endpoint = address.to_string();
    gateway.set_endpoint(endpoint);
    let active_connections = Arc::new(AtomicUsize::new(0));
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if active_connections
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                    (active < MAX_GATEWAY_CONNECTIONS).then_some(active + 1)
                })
                .is_err()
            {
                continue;
            }
            let permit = GatewayConnectionPermit(Arc::clone(&active_connections));
            let gateway = Arc::clone(&gateway);
            let services = Arc::clone(&services);
            thread::spawn(move || {
                let _permit = permit;
                handle_gateway(stream, gateway, services);
            });
        }
    });
}

fn handle_gateway(
    mut stream: TcpStream,
    gateway: Arc<AiGatewayState>,
    services: Arc<GatewayServices>,
) {
    // 每次连接只承载一个短期请求；未认证客户端不能长期占用 Gateway 线程。
    let _ = stream.set_read_timeout(Some(GATEWAY_HANDSHAKE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(clone);
    while let Ok(Some(line)) = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES) {
        let request: Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => break,
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let token = request
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let access = match gateway.access_for(token) {
            Ok(Some(access)) => access,
            Ok(None) => {
                let _ = write_json(
                    &mut stream,
                    &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":"invalid task token"}}),
                );
                continue;
            }
            Err(error) => {
                let _ = write_json(
                    &mut stream,
                    &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32002,"message":error}}),
                );
                continue;
            }
        };
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let result = match method {
            "initialize" => {
                json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"nocterm","version":"0.1"}})
            }
            "notifications/initialized" => Value::Null,
            "tools/list" => services.tools_for_target(&access.binding.target),
            "tools/call" => services.call(&gateway, token, &access, &request),
            _ => {
                if method.starts_with("notifications/") {
                    continue;
                }
                if write_json(
                    &mut stream,
                    &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}),
                )
                .is_err()
                {
                    break;
                }
                continue;
            }
        };
        if method.starts_with("notifications/") {
            continue;
        }
        if write_json(
            &mut stream,
            &json!({"jsonrpc":"2.0","id":id,"result":result}),
        )
        .is_err()
        {
            break;
        }
    }
}

fn write_json(stream: &mut TcpStream, value: &Value) -> std::io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    stream.write_all(&payload)?;
    stream.write_all(b"\n")
}

/// Provider CLI 参数使用的 stdio MCP 服务器描述；不触碰用户全局配置。
///
/// 任务 token 由父进程通过 `NOCTERM_MCP_TOKEN` 环境变量传递，避免出现在
/// Provider 的命令行参数、进程列表和 MCP 配置 JSON 中。
pub fn mcp_config(executable: &str, endpoint: &str) -> String {
    json!({"mcpServers":{"nocterm":{"command":executable,"args":["mcp-stdio","--endpoint",endpoint]}}}).to_string()
}

pub fn run_stdio(endpoint: &str) -> i32 {
    let token = match std::env::var("NOCTERM_MCP_TOKEN") {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            eprintln!("Nocterm MCP Bridge: NOCTERM_MCP_TOKEN is missing");
            return 1;
        }
    };
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    loop {
        let request_bytes = match read_bounded_line(&mut reader, MAX_MESSAGE_BYTES) {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(error) => {
                eprintln!("Nocterm MCP Bridge: invalid MCP request: {error}");
                return 1;
            }
        };
        let Ok(mut request) = serde_json::from_slice::<Value>(&request_bytes) else {
            continue;
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let notification = request
            .get("method")
            .and_then(Value::as_str)
            .is_some_and(|method| method.starts_with("notifications/"));
        if let Some(object) = request.as_object_mut() {
            object.insert("token".into(), Value::String(token.clone()));
        }
        match forward_gateway_request(endpoint, &request, notification) {
            Ok(Some(response)) => {
                if write_stdout_line(response.as_bytes()).is_err() {
                    return 1;
                }
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("Nocterm MCP Bridge: {}", error.diagnostic());
                if notification {
                    continue;
                }
                if write_stdout_json(&error.response(id)).is_err() {
                    return 1;
                }
            }
        }
    }
    0
}

/// stdio 子进程可以长期存在，但回环 TCP 只承载一个请求，避免空闲连接过期后
/// 让整个 MCP transport 失效。请求发送后绝不自动重放，防止命令重复执行。
fn forward_gateway_request(
    endpoint: &str,
    request: &Value,
    notification: bool,
) -> Result<Option<String>, BridgeForwardError> {
    let mut stream = TcpStream::connect(endpoint).map_err(|_| BridgeForwardError::Unavailable)?;
    stream
        .set_read_timeout(Some(AI_BRIDGE_RESPONSE_TIMEOUT))
        .map_err(|_| BridgeForwardError::Unavailable)?;
    stream
        .set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| BridgeForwardError::Unavailable)?;
    write_json(&mut stream, request).map_err(|_| BridgeForwardError::SendFailed)?;
    if notification {
        return Ok(None);
    }
    let mut response_reader = BufReader::new(stream);
    let response = read_bounded_line(&mut response_reader, MAX_RESPONSE_BYTES)
        .map_err(|_| BridgeForwardError::ResponseFailed)?
        .ok_or(BridgeForwardError::ResponseFailed)?;
    String::from_utf8(response)
        .map(Some)
        .map_err(|_| BridgeForwardError::ResponseFailed)
}

fn write_stdout_line(payload: &[u8]) -> io::Result<()> {
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(payload)?;
    stdout.write_all(b"\n")?;
    stdout.flush()
}

fn write_stdout_json(value: &Value) -> io::Result<()> {
    let payload = serde_json::to_vec(value).map_err(io::Error::other)?;
    write_stdout_line(&payload)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BridgeForwardError {
    Unavailable,
    SendFailed,
    ResponseFailed,
}

impl BridgeForwardError {
    fn response(self, id: Value) -> Value {
        let (rpc_code, stable_code, message, retryable) = match self {
            Self::Unavailable => (
                -32010,
                "AI_BRIDGE_UNAVAILABLE",
                "Nocterm Bridge 暂时不可用，本次操作尚未发送，请稍后重试",
                true,
            ),
            Self::SendFailed => (
                -32011,
                "AI_BRIDGE_SEND_FAILED",
                "Nocterm Bridge 发送请求失败，操作结果未知，请勿自动重试",
                false,
            ),
            Self::ResponseFailed => (
                -32012,
                "AI_BRIDGE_RESPONSE_FAILED",
                "Nocterm Bridge 未返回完整结果，操作可能已经执行，请勿自动重试",
                false,
            ),
        };
        json!({
            "jsonrpc":"2.0",
            "id":id,
            "error":{
                "code":rpc_code,
                "message":message,
                "data":{"code":stable_code,"retryable":retryable}
            }
        })
    }

    fn diagnostic(self) -> &'static str {
        match self {
            Self::Unavailable => "gateway unavailable before request send",
            Self::SendFailed => "gateway request send failed; outcome unknown",
            Self::ResponseFailed => "gateway response failed; outcome unknown",
        }
    }
}

/// MCP stdio 传输使用换行分隔 JSON-RPC 消息，单条消息设上限避免异常输入占满内存。
fn read_bounded_line(reader: &mut impl BufRead, limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    // 额外读取 CRLF 两个分隔字节，限制只计算 JSON 负载本身。
    let mut bounded = reader.take(limit.saturating_add(2) as u64);
    if bounded.read_until(b'\n', &mut line)? == 0 {
        return Ok(None);
    }
    while matches!(line.last(), Some(b'\r' | b'\n')) {
        line.pop();
    }
    if line.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "message exceeds size limit",
        ));
    }
    Ok((!line.is_empty()).then_some(line))
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufReader, Cursor},
        net::TcpListener,
        thread,
    };

    use serde_json::{Value, json};

    use crate::commands::ai_policy::{AI_BRIDGE_RESPONSE_TIMEOUT, AI_TOOL_CALL_TIMEOUT};

    use super::{
        BridgeForwardError, MAX_MESSAGE_BYTES, forward_gateway_request, mcp_config,
        read_bounded_line, write_json,
    };

    #[test]
    fn task_config_contains_ephemeral_bridge_credentials() {
        let config = mcp_config("/tmp/nocterm", "127.0.0.1:4123");
        assert!(config.contains("mcp-stdio"));
        assert!(!config.contains("task-token"));
    }

    #[test]
    fn bridge_response_budget_exceeds_the_complete_tool_call_budget() {
        assert!(AI_BRIDGE_RESPONSE_TIMEOUT > AI_TOOL_CALL_TIMEOUT);
    }

    #[test]
    fn stdio_reads_newline_delimited_json() {
        let mut reader = std::io::BufReader::new(Cursor::new(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}
"#,
        ));
        let line = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
            .expect("read message")
            .expect("mcp message");
        let value: serde_json::Value = serde_json::from_slice(&line).expect("json message");
        assert_eq!(value["method"], "tools/list");
    }

    #[test]
    fn stdio_rejects_a_message_larger_than_the_limit() {
        let input = vec![b'a'; MAX_MESSAGE_BYTES + 1];
        let mut reader = std::io::BufReader::new(Cursor::new(input));

        let error = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
            .expect_err("oversized message must fail");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn stdio_accepts_a_message_at_the_exact_limit_with_crlf() {
        let mut input = vec![b'a'; MAX_MESSAGE_BYTES];
        input.extend_from_slice(b"\r\n");
        let mut reader = std::io::BufReader::new(Cursor::new(input));

        let line = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
            .expect("read bounded message")
            .expect("message");

        assert_eq!(line.len(), MAX_MESSAGE_BYTES);
    }

    #[test]
    fn each_gateway_request_uses_a_fresh_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test gateway");
        let endpoint = listener.local_addr().expect("gateway address").to_string();
        let server = thread::spawn(move || {
            for expected_id in [1, 2] {
                let (mut stream, _) = listener.accept().expect("accept request connection");
                let mut reader = BufReader::new(stream.try_clone().expect("clone request stream"));
                let bytes = read_bounded_line(&mut reader, MAX_MESSAGE_BYTES)
                    .expect("read request")
                    .expect("request payload");
                let request: Value = serde_json::from_slice(&bytes).expect("request json");
                assert_eq!(request["id"], expected_id);
                write_json(
                    &mut stream,
                    &json!({"jsonrpc":"2.0","id":expected_id,"result":{"ok":true}}),
                )
                .expect("write response");
            }
        });

        for id in [1, 2] {
            let response = forward_gateway_request(
                &endpoint,
                &json!({"jsonrpc":"2.0","id":id,"method":"tools/list","token":"test"}),
                false,
            )
            .expect("forward request")
            .expect("request response");
            assert_eq!(
                serde_json::from_str::<Value>(&response).expect("response json")["id"],
                id
            );
        }
        server.join().expect("gateway thread");
    }

    #[test]
    fn response_failure_is_non_retryable_and_preserves_the_request_id() {
        let response = BridgeForwardError::ResponseFailed.response(json!(42));

        assert_eq!(response["id"], 42);
        assert_eq!(
            response["error"]["data"]["code"],
            "AI_BRIDGE_RESPONSE_FAILED"
        );
        assert_eq!(response["error"]["data"]["retryable"], false);
    }
}
