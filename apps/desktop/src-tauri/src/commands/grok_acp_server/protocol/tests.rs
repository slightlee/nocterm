use std::sync::{atomic::AtomicBool, mpsc};

use serde_json::json;

use super::{
    BootstrapMessages, GROK_MCP_SDK_CALL_METHOD, MAX_BOOTSTRAP_PENDING_BYTES,
    NOCTERM_MCP_SERVER_ID, cancel_notification, initialize_request, is_renderable_update,
    prompt_completion, prompt_request, require_supported_sdk_mcp, sdk_call_response,
    session_new_request, wait_for_mcp_ready_with_requests, wait_for_response,
    wait_for_response_collect_with_requests, write_message,
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
fn session_registers_only_the_in_process_nocterm_mcp() {
    let request = session_new_request("/private/runtime", &identity());
    let encoded = request.to_string();
    assert!(
        request["params"]["mcpServers"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        request["params"]["_meta"]["x.ai/mcp/servers"][0],
        json!({"name":"nocterm","serverId":NOCTERM_MCP_SERVER_ID})
    );
    assert_eq!(request["params"]["_meta"]["yoloMode"], true);
    let rules = request["params"]["_meta"]["rules"].as_str().unwrap();
    assert!(rules.contains("Make exactly one search_tool call"));
    assert!(rules.contains("get_system_info"));
    assert!(rules.contains("ssh_exec"));
    assert!(rules.contains("Do not repeat discovery"));
    assert!(!encoded.contains("mcp-stdio"));
    assert!(!encoded.contains("127.0.0.1"));
    assert!(!encoded.contains("NOCTERM_MCP_TOKEN"));
}

#[test]
fn local_session_rules_expose_only_the_local_mcp_catalog() {
    let request = session_new_request(
        "/private/runtime",
        &ProviderSessionIdentity {
            connection_id: None,
            target_session_id: Some("local-one".into()),
            working_directory: None,
        },
    );
    let rules = request["params"]["_meta"]["rules"].as_str().unwrap();

    assert!(rules.contains("session_context and local_terminal_exec"));
    assert!(rules.contains("Make exactly one search_tool call"));
    assert!(!rules.contains("get_system_info"));
    assert!(!rules.contains("ssh_exec"));
}

#[test]
fn sdk_mcp_support_requires_the_minimum_version_and_capability() {
    assert!(
        require_supported_sdk_mcp(&json!({
            "result":{"_meta":{"x.ai/mcp/sdk":true,"agentVersion":"1.0.34"}}
        }))
        .is_ok()
    );
    assert!(
        require_supported_sdk_mcp(&json!({
            "result":{"_meta":{"x.ai/mcp/sdk":true,"agentVersion":"1.1.0-beta.1"}}
        }))
        .is_ok()
    );

    let old_version = require_supported_sdk_mcp(&json!({
        "result":{"_meta":{"x.ai/mcp/sdk":true,"agentVersion":"1.0.33"}}
    }))
    .expect_err("old Grok versions must fail closed");
    assert!(old_version.contains("1.0.33"));
    assert!(old_version.contains("1.0.34 或更高版本"));

    let missing_capability = require_supported_sdk_mcp(&json!({
        "result":{"_meta":{"x.ai/mcp/sdk":false,"agentVersion":"1.0.34"}}
    }))
    .expect_err("MCP-over-ACP is required");
    assert!(missing_capability.contains("MCP-over-ACP"));
    assert!(require_supported_sdk_mcp(&json!({"result":{"_meta":{}}})).is_err());
}

#[test]
fn sdk_call_wraps_the_shared_mcp_response_and_preserves_both_ids() {
    let request = json!({
        "jsonrpc":"2.0",
        "id":41,
        "method":GROK_MCP_SDK_CALL_METHOD,
        "params":{
            "serverId":NOCTERM_MCP_SERVER_ID,
            "message":{"jsonrpc":"2.0","id":7,"method":"tools/list"}
        }
    });
    let response = sdk_call_response(&request, |message| {
        assert_eq!(message["id"], 7);
        Some(json!({"jsonrpc":"2.0","id":7,"result":{"tools":[]}}))
    });

    assert_eq!(response["id"], 41);
    assert_eq!(response["result"]["id"], 7);
    assert_eq!(response["result"]["result"]["tools"], json!([]));
}

#[test]
fn sdk_call_rejects_unknown_servers_and_mcp_notifications_without_dispatching() {
    let unknown_server = json!({
        "id":1,
        "method":GROK_MCP_SDK_CALL_METHOD,
        "params":{"serverId":"other","message":{"id":2,"method":"tools/list"}}
    });
    let unknown_response = sdk_call_response(&unknown_server, |_| {
        panic!("unknown server must not reach the Gateway")
    });
    assert_eq!(unknown_response["error"]["code"], -32602);

    let notification = json!({
        "id":1,
        "method":GROK_MCP_SDK_CALL_METHOD,
        "params":{
            "serverId":NOCTERM_MCP_SERVER_ID,
            "message":{"method":"notifications/initialized"}
        }
    });
    let notification_response = sdk_call_response(&notification, |_| {
        panic!("notifications must not reach the half-duplex Gateway")
    });
    assert_eq!(notification_response["error"]["code"], -32602);
}

#[test]
fn bootstrap_wait_services_reverse_requests_before_the_expected_response() {
    let (sender, receiver) = mpsc::channel();
    sender
        .send(Ok(json!({
            "id":40,
            "method":GROK_MCP_SDK_CALL_METHOD,
            "params":{
                "serverId":NOCTERM_MCP_SERVER_ID,
                "message":{"id":6,"method":"tools/list"}
            }
        })
        .to_string()))
        .unwrap();
    sender
        .send(Ok(
            json!({"id":2,"result":{"sessionId":"grok-one"}}).to_string()
        ))
        .unwrap();
    let mut handled = 0;

    let response = wait_for_response_collect_with_requests(
        &receiver,
        2,
        "session/new",
        &mut BootstrapMessages::default(),
        &AtomicBool::new(false),
        |_| {
            handled += 1;
            Ok(true)
        },
    )
    .unwrap();

    assert_eq!(handled, 1);
    assert_eq!(response["result"]["sessionId"], "grok-one");
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
        wait_for_mcp_ready_with_requests(
            &receiver,
            &mut BootstrapMessages::default(),
            "grok-one",
            &AtomicBool::new(false),
            |_| Ok(false),
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
        wait_for_mcp_ready_with_requests(
            &receiver,
            &mut pending,
            "grok-one",
            &AtomicBool::new(false),
            |_| Ok(false),
        )
        .is_ok()
    );
}
