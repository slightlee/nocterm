//! Tauri 进程内的回环 Gateway 监听器与 MCP 工具路由。

use std::{
    io::BufReader,
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
    commands::ai_tool_gateway::GatewayServices,
    state::{AiGatewayState, AppState},
};

use super::{
    MAX_MESSAGE_BYTES,
    transport::{read_bounded_line, write_json},
};

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
    let dispatcher = Arc::new(McpRequestDispatcher::new(app, state));
    let gateway = Arc::clone(state.ai_gateway());
    let listener = match TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener,
        Err(_) => return,
    };
    let Ok(address) = listener.local_addr() else {
        return;
    };
    gateway.set_endpoint(address.to_string());
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
            let dispatcher = Arc::clone(&dispatcher);
            thread::spawn(move || {
                let _permit = permit;
                handle_gateway(stream, dispatcher);
            });
        }
    });
}

fn handle_gateway(mut stream: TcpStream, dispatcher: Arc<McpRequestDispatcher>) {
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
        let token = request
            .get("token")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(response) = dispatcher.dispatch(token, &request) else {
            continue;
        };
        if write_json(&mut stream, &response).is_err() {
            break;
        }
    }
}

/// MCP 传输只负责交付 JSON-RPC；stdio、TCP 和 ACP 必须复用同一分发与安全边界。
pub struct McpRequestDispatcher {
    gateway: Arc<AiGatewayState>,
    services: GatewayServices,
}

impl McpRequestDispatcher {
    pub(crate) fn new(app: AppHandle, state: &AppState) -> Self {
        Self {
            gateway: Arc::clone(state.ai_gateway()),
            services: GatewayServices::new(app, state),
        }
    }

    /// 返回完整 MCP JSON-RPC 响应；notification 没有响应，调用方必须只转发副作用。
    pub(crate) fn dispatch(&self, token: &str, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let access = match self.gateway.access_for(token) {
            Ok(Some(access)) => access,
            Ok(None) => {
                return Some(json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "error":{"code":-32001,"message":"invalid task token"}
                }));
            }
            Err(error) => {
                return Some(json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "error":{"code":-32002,"message":error}
                }));
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
            "notifications/initialized" => return None,
            "tools/list" => self.services.tools_for_target(&access.binding.target),
            "tools/call" => self.services.call(&self.gateway, token, &access, request),
            _ if method.starts_with("notifications/") => return None,
            _ => {
                return Some(json!({
                    "jsonrpc":"2.0",
                    "id":id,
                    "error":{"code":-32601,"message":"method not found"}
                }));
            }
        };
        Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
    }
}
