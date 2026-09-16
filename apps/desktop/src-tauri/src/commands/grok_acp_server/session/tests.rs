use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    sync::{atomic::AtomicBool, mpsc},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::Value;

use super::super::protocol::{
    BootstrapMessages, close_request, initialize_request, session_new_request, terminate_child,
    wait_for_mcp_ready, wait_for_response, write_message,
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
    let identity = ProviderSessionIdentity {
        connection_id: None,
        target_session_id: None,
        working_directory: None,
    };
    write_message(
        &mut stdin,
        &session_new_request(
            prepared.current_directory.to_str().expect("utf8 runtime"),
            &identity,
            None,
        ),
    )
    .expect("session request");
    let session = wait_for_response(&receiver, 2, "创建 Grok ACP 会话").expect("session response");
    let session_id = session
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .expect("Grok session id");
    assert!(!session_id.is_empty());

    let mut failed_mcp = session_new_request(
        prepared.current_directory.to_str().expect("utf8 runtime"),
        &ProviderSessionIdentity {
            connection_id: Some(7),
            target_session_id: None,
            working_directory: None,
        },
        Some(("/nocterm/missing-mcp-executable", "127.0.0.1:1")),
    );
    failed_mcp["id"] = Value::from(3);
    write_message(&mut stdin, &failed_mcp).expect("failed MCP session request");
    let failed_response =
        wait_for_response(&receiver, 3, "验证 Grok MCP 失败路径").expect("session response");
    let failed_session_id = failed_response
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .expect("failed MCP session id");
    assert!(
        wait_for_mcp_ready(
            &receiver,
            &mut BootstrapMessages::default(),
            failed_session_id,
            &AtomicBool::new(false),
        )
        .is_err(),
        "Grok MCP readiness must fail when no tools were discovered"
    );

    let mock_mcp = prepared.current_directory.join("mock-mcp");
    let mut mock_mcp_file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&mock_mcp)
        .expect("create mock MCP");
    mock_mcp_file
        .write_all(
            br#"#!/usr/bin/env node
const readline = require('node:readline');
const input = readline.createInterface({ input: process.stdin });
input.on('line', (line) => {
  const request = JSON.parse(line);
  if (request.method === 'initialize') {
    console.log(JSON.stringify({jsonrpc:'2.0',id:request.id,result:{protocolVersion:'2024-11-05',capabilities:{tools:{}},serverInfo:{name:'nocterm-test',version:'1'}}}));
  } else if (request.method === 'tools/list') {
    console.log(JSON.stringify({jsonrpc:'2.0',id:request.id,result:{tools:[{name:'session_context',description:'test',inputSchema:{type:'object',properties:{},additionalProperties:false}}]}}));
  }
});
"#,
        )
        .expect("write mock MCP");
    drop(mock_mcp_file);
    fs::set_permissions(&mock_mcp, fs::Permissions::from_mode(0o700))
        .expect("make mock MCP executable");
    let mut ready_mcp = session_new_request(
        prepared.current_directory.to_str().expect("utf8 runtime"),
        &ProviderSessionIdentity {
            connection_id: Some(7),
            target_session_id: None,
            working_directory: None,
        },
        Some((mock_mcp.to_str().expect("utf8 mock MCP"), "127.0.0.1:1")),
    );
    ready_mcp["id"] = Value::from(4);
    write_message(&mut stdin, &ready_mcp).expect("ready MCP session request");
    let ready_response =
        wait_for_response(&receiver, 4, "验证 Grok MCP 成功路径").expect("session response");
    let ready_session_id = ready_response
        .pointer("/result/sessionId")
        .and_then(Value::as_str)
        .expect("ready MCP session id");
    wait_for_mcp_ready(
        &receiver,
        &mut BootstrapMessages::default(),
        ready_session_id,
        &AtomicBool::new(false),
    )
    .expect("Grok should discover the mock MCP tool");

    let _ = write_message(&mut stdin, &close_request(5, session_id));
    terminate_child(&mut child);
}
