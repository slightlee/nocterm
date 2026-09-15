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
    let gateway = Arc::clone(state.ai_gateway());
    let services = Arc::new(GatewayServices::new(app, state));
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
