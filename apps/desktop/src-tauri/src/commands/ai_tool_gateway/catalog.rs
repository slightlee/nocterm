//! 依据终端目标生成 Provider 可见的 MCP 工具目录。

use serde_json::{Value, json};

use crate::{commands::ai_tools::server_tool_definitions, state::AiTarget};

pub(super) fn tools_for_target(target: &AiTarget) -> Value {
    let mut tools = vec![json!({
        "name":"session_context",
        "description":"读取当前任务由 Nocterm 后端绑定的终端目标，不接受调用方覆盖目标",
        "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
    })];
    if matches!(target, AiTarget::Ssh { .. }) {
        tools.extend(server_tool_definitions());
    }
    let execution_description = match target {
        AiTarget::Ssh { .. } => {
            "通过当前已认证的 Nocterm SSH 连接在同一远程服务器执行通用命令；这不是新的连接或独立环境，且不继承可见交互终端中临时的 cd、export 或虚拟环境状态。是否需要确认由当前会话权限决定；仅在结构化服务器检查无法完成任务时使用，不得改到本机执行"
        }
        AiTarget::Local { .. } => {
            "在当前 Nocterm 可见本地终端中执行命令，继承该终端的目录和环境。查询主机名、当前目录、文件、进程、端口、服务、日志、安装或部署时必须使用此能力，不得改用其他本机执行工具；是否需要确认由当前会话权限决定"
        }
    };
    tools.push(json!({
        "name":execution_tool_name(target),
        "description":execution_description,
        "inputSchema":{"type":"object","properties":{"command":{"type":"string","minLength":1,"maxLength":4096}},"required":["command"],"additionalProperties":false},
        "outputSchema":{
            "type":"object",
            "properties":{
                "output":{"type":"string"},
                "exitCode":{"type":"integer"},
                "succeeded":{"type":"boolean"},
                "execution":{
                    "type":"object",
                    "properties":{
                        "mode":{"type":"string","enum":["current_authenticated_ssh_connection","visible_local_terminal"]},
                        "visibleTerminalStateInherited":{"type":"boolean"},
                        "workingDirectoryScope":{"type":"string","enum":["remote_command_process","visible_terminal"]},
                        "description":{"type":"string"}
                    },
                    "required":["mode","visibleTerminalStateInherited","workingDirectoryScope","description"],
                    "additionalProperties":false
                }
            },
            "required":["output","exitCode","succeeded","execution"],
            "additionalProperties":false
        },
        "annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false}
    }));
    json!({"tools":tools})
}

pub(super) fn execution_tool_name(target: &AiTarget) -> &'static str {
    match target {
        AiTarget::Ssh { .. } => "ssh_exec",
        AiTarget::Local { .. } => "local_terminal_exec",
    }
}

#[cfg(test)]
mod tests {
    use crate::state::AiTarget;

    use super::tools_for_target;

    #[test]
    fn target_tools_include_structured_server_capabilities_and_one_escape_hatch() {
        let ssh = tools_for_target(&AiTarget::Ssh { connection_id: 7 });
        assert_eq!(ssh["tools"][0]["name"], "session_context");
        assert_eq!(ssh["tools"][1]["name"], "get_system_info");
        assert_eq!(ssh["tools"][12]["name"], "ssh_exec");
        assert_eq!(ssh["tools"][1]["annotations"]["readOnlyHint"], true);
        assert_eq!(ssh["tools"][12]["annotations"]["destructiveHint"], true);
        assert_eq!(
            ssh["tools"][12]["outputSchema"]["properties"]["exitCode"]["type"],
            "integer"
        );
        assert_eq!(
            ssh["tools"][12]["outputSchema"]["properties"]["execution"]["properties"]["visibleTerminalStateInherited"]
                ["type"],
            "boolean"
        );

        let local = tools_for_target(&AiTarget::Local {
            session_id: "local:one".into(),
            terminal_id: "pty-1".into(),
        });
        assert_eq!(local["tools"][0]["name"], "session_context");
        assert_eq!(local["tools"][1]["name"], "local_terminal_exec");
        assert!(
            local["tools"][1]["description"]
                .as_str()
                .is_some_and(|value| value.contains("主机名") && value.contains("不得改用"))
        );
        assert_eq!(local["tools"].as_array().unwrap().len(), 2);
    }
}
