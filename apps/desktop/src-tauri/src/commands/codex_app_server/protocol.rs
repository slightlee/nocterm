//! Codex app-server JSON-RPC 协议构造与初始化响应读取。

use std::{
    io::Write,
    process::Child,
    sync::{atomic::AtomicBool, mpsc},
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};

use super::CodexSessionIdentity;
use crate::commands::{
    ai_persistent::receive_startup_line, ai_provider::terminal_session_instructions,
};
const CODEX_DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "unified_exec",
    "unified_exec_tty",
    "view_image",
    "code_mode",
    "code_mode_host",
    "multi_agent",
    "apps",
    "plugins",
    "hooks",
    "request_permissions_tool",
    "tool_suggest",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "computer_use",
    "remote_plugin",
    "image_generation",
    "skill_mcp_dependency_install",
    "skill_search",
    "goals",
    "artifact",
    "web_search_request",
    "web_search_cached",
    "standalone_web_search",
];
// `unified_exec`/`unified_exec_tty` 只选择 Shell 实现；`shell_tool=false` 时 Codex 不注册执行工具。
const CODEX_EXECUTION_GATE_FEATURES: &[&str] = &[
    "shell_tool",
    "view_image",
    "code_mode",
    "code_mode_host",
    "multi_agent",
    "apps",
    "plugins",
    "hooks",
    "request_permissions_tool",
    "tool_suggest",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "computer_use",
    "remote_plugin",
    "image_generation",
    "skill_mcp_dependency_install",
    "skill_search",
    "goals",
    "artifact",
    "web_search_request",
    "web_search_cached",
    "standalone_web_search",
];
const CODEX_REQUIRED_FEATURE_STATES: &[(&str, bool)] =
    &[("shell_tool", false), ("skip_host_skill_discovery", true)];

/// 读取最终生效配置是兼容性门禁，也用于枚举并关闭用户已有的 MCP Server。
pub(super) fn codex_config_read_request(cwd: Option<&str>) -> Value {
    json!({
        "id": 2,
        "method": "config/read",
        "params": {"cwd": cwd, "includeLayers": false}
    })
}

pub(super) fn configured_mcp_server_names(response: &Value) -> Result<Vec<String>, String> {
    let config = response
        .pointer("/result/config")
        .and_then(Value::as_object)
        .ok_or_else(|| "Codex config/read 未返回有效配置".to_string())?;
    let Some(servers) = config.get("mcp_servers") else {
        return Ok(Vec::new());
    };
    let servers = servers
        .as_object()
        .ok_or_else(|| "Codex config/read 返回了无效的 mcp_servers".to_string())?;
    Ok(servers.keys().cloned().collect())
}

/// 目标已绑定时 Nocterm MCP 是唯一执行能力；启动失败必须阻止 thread 创建。
pub(super) fn codex_provider_config(
    executable: &str,
    endpoint: Option<&str>,
    configured_mcp_servers: &[String],
) -> Value {
    // Provider 仍沿用用户的登录与模型设置，但不能继承可绕过 Nocterm 审批和终端路由的工具。
    let mut features = CODEX_DISABLED_FEATURES
        .iter()
        .map(|name| ((*name).to_string(), Value::Bool(false)))
        .collect::<Map<_, _>>();
    features.insert("skip_host_skill_discovery".to_string(), Value::Bool(true));
    let mut config = json!({
        "features": Value::Object(features),
        "sandbox_mode": "read-only",
        "web_search": "disabled",
        "tools": {"web_search": false, "view_image": false},
        "project_doc_max_bytes": 0
    });
    let mut servers = configured_mcp_servers
        .iter()
        .map(|name| (name.clone(), json!({"enabled": false})))
        .collect::<Map<_, _>>();
    if let Some(endpoint) = endpoint {
        servers.insert(
            "nocterm".to_string(),
            json!({
                "command": executable,
                "args": ["mcp-stdio", "--endpoint", endpoint],
                "env_vars": ["NOCTERM_MCP_TOKEN"],
                "default_tools_approval_mode": "approve",
                "required": true,
                "startup_timeout_sec": 5
            }),
        );
    }
    if !servers.is_empty() {
        config["mcp_servers"] = Value::Object(servers);
    }
    config
}

pub(super) fn codex_feature_list_request(thread_id: &str) -> Value {
    json!({
        "id": 4,
        "method": "experimentalFeature/list",
        "params": {"limit": 1000, "threadId": thread_id}
    })
}

/// thread 创建结果与能力列表都是 Provider 返回值，必须验证最终策略而不能只相信请求覆盖。
pub(super) fn validate_thread_security(response: &Value) -> Result<(), String> {
    if response
        .pointer("/result/approvalPolicy")
        .and_then(Value::as_str)
        != Some("never")
    {
        return Err("Codex thread 未应用禁止 Provider 自行审批的策略".to_string());
    }
    if response
        .pointer("/result/sandbox/type")
        .and_then(Value::as_str)
        != Some("readOnly")
    {
        return Err("Codex thread 未应用只读沙箱".to_string());
    }
    Ok(())
}

pub(super) fn validate_disabled_features(response: &Value) -> Result<(), String> {
    let features = response
        .pointer("/result/data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Codex experimentalFeature/list 未返回有效能力列表".to_string())?;
    if response
        .pointer("/result/nextCursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        return Err("Codex 能力列表超过安全检查范围".to_string());
    }
    let mut effective_states = Map::new();
    for feature in features {
        let name = feature
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| "Codex 能力列表包含无效名称".to_string())?;
        let enabled = feature
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("Codex 能力 {name} 缺少有效状态"))?;
        effective_states.insert(name.to_string(), Value::Bool(enabled));
        if enabled && CODEX_EXECUTION_GATE_FEATURES.contains(&name) {
            return Err(format!("Codex 安全能力覆盖未生效：{name} 仍处于启用状态"));
        }
    }
    for (name, expected) in CODEX_REQUIRED_FEATURE_STATES {
        let actual = effective_states.get(*name).and_then(Value::as_bool);
        if actual != Some(*expected) {
            return Err(format!(
                "Codex 版本缺少必需的安全能力状态：{name} 应为 {expected}"
            ));
        }
    }
    Ok(())
}

/// 把目标约束和 Bridge 配置一次性装入 thread 创建请求。
pub(super) fn codex_thread_start_request(
    identity: &CodexSessionIdentity,
    config: Value,
    bridge_enabled: bool,
) -> Value {
    let developer_instructions = bridge_enabled
        .then(|| terminal_session_instructions(identity))
        .flatten();
    json!({
        "id": 3,
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
            &format!("{action}失败：Codex app-server 提前退出"),
        )?;
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
}
