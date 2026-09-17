//! 依据终端目标生成 Provider 可见的 MCP 工具目录。

use serde_json::{Value, json};

use crate::{
    commands::{ai_tool_contract::execution_context_schema, ai_tools::server_tool_definitions},
    state::AiTarget,
};

pub(super) fn tools_for_target(target: &AiTarget) -> Value {
    let mut tools = vec![json!({
        "name":"session_context",
        "description":"仅在用户询问当前绑定的是本地终端还是 SSH 连接、连接名称或连接身份时，读取 Nocterm 后端绑定的目标；其中 name 和 host 是连接配置元数据，不是操作系统主机名，也不包含当前工作目录",
        "inputSchema":{"type":"object","properties":{},"additionalProperties":false},
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"idempotentHint":true,"openWorldHint":false}
    })];
    if matches!(target, AiTarget::Ssh { .. }) {
        tools.extend(server_tool_definitions());
    }
    let execution_description = match target {
        AiTarget::Ssh { .. } => {
            "通过当前任务绑定的 SSH 连接执行通用命令；仅在结构化服务器工具无法完整取得用户请求的事实时使用。在不扩大信息范围和审批等级的前提下，一个最小命令可以取得全部请求事实时才合并调用；不得仅为减少调用次数而组合命令或叠加重合的结构化检查、会话查询；不附加探索性检查"
        }
        AiTarget::Local { .. } => {
            "在当前任务绑定的可见本地终端中执行请求所需的最小命令，并继承该终端的目录和 Shell 环境；不附加未请求的探索性检查"
        }
    };
    let execution_schema = execution_context_schema();
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
                "execution":execution_schema
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
        assert!(
            ssh["tools"][0]["description"]
                .as_str()
                .is_some_and(|value| value.contains("不是操作系统主机名")
                    && value.contains("不包含当前工作目录"))
        );
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
                .is_some_and(|value| value.contains("可见本地终端")
                    && value.contains("Shell 环境")
                    && value.contains("最小命令"))
        );
        assert!(
            ssh["tools"][12]["description"]
                .as_str()
                .is_some_and(|value| value.contains("结构化服务器工具")
                    && value.contains("审批等级")
                    && value.contains("最小命令")
                    && value.contains("不附加探索性检查"))
        );
        assert_eq!(local["tools"].as_array().unwrap().len(), 2);
    }
}
