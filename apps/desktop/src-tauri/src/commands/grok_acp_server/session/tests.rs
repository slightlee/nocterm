use std::{
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{Value, json};

use super::super::protocol::{
    BootstrapMessages, close_request, initialize_request, require_supported_sdk_mcp,
    sdk_call_response, session_new_request, terminate_child, wait_for_mcp_ready_with_requests,
    wait_for_response, wait_for_response_collect_with_requests, write_message,
};
use crate::commands::{
    ai_provider::{ProviderSessionIdentity, prepare_grok_acp_launch, provider_adapter},
    ai_stream::read_bounded_lines,
};

/// 该测试只验证本机 CLI 握手，不发送模型 Prompt；默认 CI 不要求安装 Grok。
#[test]
#[ignore = "requires an installed and configured Grok CLI"]
fn installed_grok_accepts_the_isolated_acp_profile() {
    let executable = provider_adapter("grok")
        .and_then(|adapter| adapter.executable())
        .expect("installed Grok executable");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let prepared = prepare_grok_acp_launch(timestamp, 1).expect("isolated Grok launch");
    let mut child = Command::new(executable)
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
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn Grok ACP");
    let mut stdin = child.stdin.take().expect("Grok stdin");
    let stdout = child.stdout.take().expect("Grok stdout");
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let result = read_bounded_lines(stdout, |line| sender.send(Ok(line)).is_ok());
        if let Err(error) = result {
            let _ = sender.send(Err(error));
        }
    });

    write_message(&mut stdin, &initialize_request()).expect("initialize request");
    let initialized =
        wait_for_response(&receiver, 1, "初始化 Grok ACP").expect("initialize response");
    assert_eq!(
        initialized
            .pointer("/result/protocolVersion")
            .and_then(Value::as_u64),
        Some(1)
    );
    require_supported_sdk_mcp(&initialized).expect("installed Grok supports MCP-over-ACP");
    let identity = ProviderSessionIdentity {
        connection_id: Some(7),
        target_session_id: None,
        working_directory: None,
    };
    write_message(
        &mut stdin,
        &session_new_request(
            prepared.current_directory.to_str().expect("utf8 runtime"),
            &identity,
        ),
    )
    .expect("session request");
    let mut pending = BootstrapMessages::default();
    let cancellation = AtomicBool::new(false);
    let mut handle_request = |request: &Value| {
        let response = sdk_call_response(request, |message| {
            let id = message.get("id").cloned().unwrap_or(Value::Null);
            let result = match message.get("method").and_then(Value::as_str) {
                Some("initialize") => json!({
                    "protocolVersion":"2024-11-05",
                    "capabilities":{"tools":{}},
                    "serverInfo":{"name":"nocterm-test","version":"1"}
                }),
                Some("tools/list") => json!({"tools":[{
                    "name":"session_context",
                    "description":"test",
                    "inputSchema":{"type":"object","properties":{},"additionalProperties":false}
                }]}),
                _ => {
                    return Some(json!({
                        "jsonrpc":"2.0",
                        "id":id,
                        "error":{"code":-32601,"message":"method not found"}
                    }));
                }
            };
            Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
        });
        write_message(&mut stdin, &response)?;
        Ok(true)
    };
    let session = wait_for_response_collect_with_requests(
        &receiver,
        2,
        "创建 Grok ACP 会话",
        &mut pending,
        &cancellation,
        &mut handle_request,
    )
    .expect("session response");
    let session_id = session
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .expect("Grok session id");
    assert!(!session_id.is_empty());
    wait_for_mcp_ready_with_requests(
        &receiver,
        &mut pending,
        session_id,
        &cancellation,
        &mut handle_request,
    )
    .expect("Grok should discover the in-process Nocterm MCP tool");

    let _ = write_message(&mut stdin, &close_request(5, session_id));
    terminate_child(&mut child);
}
