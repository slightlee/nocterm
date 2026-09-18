use std::sync::{atomic::AtomicBool, mpsc};

use serde_json::json;

use super::{
    codex_config_read_request, codex_feature_list_request, codex_provider_config,
    codex_thread_start_request, configured_mcp_server_names, validate_disabled_features,
    validate_thread_security, wait_for_response, write_message,
};
use crate::commands::codex_app_server::CodexSessionIdentity;

#[test]
fn writes_one_json_message_per_line() {
    let mut output = Vec::new();
    write_message(&mut output, &json!({"id": 3, "method": "turn/start"})).expect("write request");
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
    let response = wait_for_response(&receiver, 2, "test", &AtomicBool::new(false))
        .expect("matching response");
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
    let error = wait_for_response(&receiver, 1, "initialize", &AtomicBool::new(false))
        .expect_err("protocol error");
    assert!(error.contains("unsupported"));
}

#[test]
fn initialization_surfaces_output_reader_errors() {
    let (sender, receiver) = mpsc::channel();
    sender.send(Err("provider stream failed".into())).unwrap();
    let error = wait_for_response(&receiver, 1, "initialize", &AtomicBool::new(false))
        .expect_err("reader error");
    assert!(error.contains("provider stream failed"));
}

#[test]
fn target_bound_threads_require_nocterm_and_route_terminal_tools() {
    let configured_servers = vec!["user-server".to_string(), "nocterm".to_string()];
    let config = codex_provider_config(
        "/Applications/Nocterm",
        Some("127.0.0.1:4567"),
        &configured_servers,
    );
    assert_eq!(config["mcp_servers"]["nocterm"]["required"], true);
    assert_eq!(config["mcp_servers"]["user-server"]["enabled"], false);
    assert_eq!(config["features"]["shell_tool"], false);
    assert_eq!(config["features"]["unified_exec"], false);
    assert_eq!(config["features"]["code_mode_host"], false);
    assert_eq!(config["features"]["browser_use"], false);
    assert_eq!(config["features"]["plugins"], false);
    assert_eq!(config["features"]["skip_host_skill_discovery"], true);
    assert_eq!(config["sandbox_mode"], "read-only");
    assert_eq!(config["web_search"], "disabled");

    let local = CodexSessionIdentity {
        connection_id: None,
        target_session_id: Some("local:one".into()),
        working_directory: None,
    };
    let request = codex_thread_start_request(&local, config, true);
    assert_eq!(request["method"], "thread/start");
    assert_eq!(request["id"], 3);
    assert!(request["params"].get("sandbox").is_none());
    assert!(
        request["params"]["developerInstructions"]
            .as_str()
            .is_some_and(|value| value.contains("directly and concisely"))
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
    let untargeted_config = codex_provider_config("/Applications/Nocterm", None, &[]);
    assert_eq!(untargeted_config["features"]["shell_tool"], false);
    assert!(untargeted_config.get("mcp_servers").is_none());
    assert!(codex_thread_start_request(&untargeted, untargeted_config, false)["params"]
        ["developerInstructions"]
        .is_null());
}

#[test]
fn config_read_discovers_existing_mcp_servers_before_thread_start() {
    let config_request = codex_config_read_request(Some("/workspace"));
    assert_eq!(config_request["method"], "config/read");
    assert_eq!(config_request["params"]["cwd"], "/workspace");
    let names = configured_mcp_server_names(&json!({
        "id": 2,
        "result": {"config": {"mcp_servers": {"alpha": {}, "beta": {}}}}
    }))
    .expect("valid config response");
    assert_eq!(names, ["alpha", "beta"]);

    assert!(
        configured_mcp_server_names(&json!({"id": 2, "result": {}}))
            .expect_err("missing config must fail closed")
            .contains("有效配置")
    );
    assert!(
        configured_mcp_server_names(&json!({
            "id": 2,
            "result": {"config": {"mcp_servers": []}}
        }))
        .expect_err("invalid server map must fail closed")
        .contains("mcp_servers")
    );
}

#[test]
fn thread_security_is_verified_from_provider_responses() {
    let thread_id = "thread-1";
    let feature_request = codex_feature_list_request(thread_id);
    assert_eq!(feature_request["params"]["threadId"], thread_id);

    validate_thread_security(&json!({
        "result": {
            "approvalPolicy": "never",
            "sandbox": {"type": "readOnly"}
        }
    }))
    .expect("secure thread response");
    assert!(
        validate_thread_security(&json!({
            "result": {
                "approvalPolicy": "never",
                "sandbox": {"type": "workspaceWrite"}
            }
        }))
        .expect_err("writable sandbox must fail")
        .contains("只读沙箱")
    );

    validate_disabled_features(&json!({
        "result": {
            "data": [
                {"name": "shell_tool", "enabled": false},
                {"name": "skip_host_skill_discovery", "enabled": true},
                {"name": "unified_exec", "enabled": true},
                {"name": "unrelated_feature", "enabled": true}
            ],
            "nextCursor": null
        }
    }))
    .expect("disabled dangerous features");
    assert!(
        validate_disabled_features(&json!({
            "result": {
                "data": [
                    {"name": "shell_tool", "enabled": false},
                    {"name": "skip_host_skill_discovery", "enabled": true},
                    {"name": "code_mode_host", "enabled": true}
                ],
                "nextCursor": null
            }
        }))
        .expect_err("enabled bypass must fail")
        .contains("code_mode_host")
    );
    assert!(
        validate_disabled_features(&json!({
            "result": {
                "data": [{"name": "shell_tool", "enabled": false}],
                "nextCursor": null
            }
        }))
        .expect_err("missing host-skill isolation must fail")
        .contains("skip_host_skill_discovery")
    );
}
