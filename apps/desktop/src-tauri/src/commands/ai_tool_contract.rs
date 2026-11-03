//! Provider 可见的终端执行结果契约。
//! 所有命令型工具都返回相同的成功状态和执行环境，避免模型从工具名猜测语义。

use nocterm_domain::ai_execution::AiExecutionRecord;
use serde_json::{Value, json};

use crate::state::AiTarget;

/// 执行通道事实由任务绑定生成，Provider 不能通过参数覆盖。
pub(crate) fn execution_context(target: &AiTarget, record: Option<&AiExecutionRecord>) -> Value {
    match target {
        AiTarget::Ssh { .. }
            if record.is_some_and(|record| {
                matches!(
                    record.mode,
                    nocterm_domain::ai_execution::AiExecutionMode::SshVisibleOsc
                        | nocterm_domain::ai_execution::AiExecutionMode::SshVisibleMarker
                )
            }) =>
        {
            json!({
                "mode":"visible_ssh_terminal",
                "visibleTerminalStateInherited":true,
                "workingDirectoryScope":"visible_terminal",
                "description":"命令直接写入当前可见 SSH 终端，并继承该终端当前目录、环境变量、alias 与虚拟环境状态"
            })
        }
        AiTarget::Ssh { .. } => json!({
            "mode":"current_authenticated_ssh_connection",
            "visibleTerminalStateInherited":false,
            "workingDirectoryScope":"remote_command_process",
            "description":"命令通过当前已认证的 Nocterm SSH 连接在同一远程服务器执行；这不是新的连接、服务器或独立环境，也不继承可见交互终端中临时的 cd、export 或虚拟环境状态"
        }),
        AiTarget::Local { .. } => json!({
            "mode":"visible_local_terminal",
            "visibleTerminalStateInherited":true,
            "workingDirectoryScope":"visible_terminal",
            "description":"命令直接写入当前可见本地终端，并继承该终端当前的目录和 Shell 环境"
        }),
    }
}

/// Schema 与运行时结果共用一个来源，防止新增字段后 Provider 看到过期契约。
pub(crate) fn execution_context_schema() -> Value {
    json!({
        "type":"object",
        "properties":{
            "mode":{"type":"string","enum":["current_authenticated_ssh_connection","visible_ssh_terminal","visible_local_terminal"]},
            "visibleTerminalStateInherited":{"type":"boolean"},
            "workingDirectoryScope":{"type":"string","enum":["remote_command_process","visible_terminal"]},
            "description":{"type":"string"}
        },
        "required":["mode","visibleTerminalStateInherited","workingDirectoryScope","description"],
        "additionalProperties":false
    })
}

/// 构造统一的命令结果；非零退出码仍保留真实输出，并通过 MCP `isError` 标记失败。
pub(crate) fn command_result(
    target: &AiTarget,
    output: String,
    exit_code: i64,
    record: Option<&AiExecutionRecord>,
) -> Value {
    let succeeded = exit_code == 0;
    let structured = json!({
        "output":output,
        "exitCode":exit_code,
        "succeeded":succeeded,
        "execution":execution_context(target, record),
        "executionId":record.map(|value| value.execution_id.clone()),
        "resultState":record.map(|value| value.result.as_str()),
        "executionMode":record.map(|value| value.mode.as_str()),
    });
    json!({
        "isError":!succeeded,
        "content":[{"type":"text","text":structured.to_string()}],
        "structuredContent":structured
    })
}

pub(crate) fn execution_error(record: &AiExecutionRecord, message: &str) -> Value {
    let structured = json!({
        "executionId":record.execution_id,
        "resultState":record.result.as_str(),
        "executionMode":record.mode.as_str(),
        "phase":record.phase.as_str(),
        "errorCode":record.error_code,
        "message":message,
    });
    json!({
        "isError":true,
        "content":[{"type":"text","text":structured.to_string()}],
        "structuredContent":structured
    })
}

pub(crate) fn duplicate_execution_error(record: &AiExecutionRecord) -> Value {
    execution_error(
        record,
        "duplicate Provider request; original execution is not dispatched again",
    )
}

/// 结构化检查只在成功时到达此处；失败统一走 MCP 错误结果。
pub(crate) fn inspection_result(
    target: &AiTarget,
    operation: &str,
    output: String,
    record: Option<&AiExecutionRecord>,
) -> Value {
    // 一些第三方模型只消费 MCP 文本内容而忽略 structuredContent；显式标记空结果，
    // 避免把“成功但无匹配项”误判为工具没有返回数据并触发重复探测。
    let compatible_text = if output.is_empty() {
        "检查成功，结果为空。".to_string()
    } else {
        format!("检查成功，结果如下：\n{output}")
    };
    let structured = json!({
        "operation":operation,
        "output":output,
        "succeeded":true,
        "execution":execution_context(target, record)
    });
    json!({
        "content":[{"type":"text","text":compatible_text}],
        "structuredContent":structured
    })
}

#[cfg(test)]
mod tests {
    use super::{command_result, inspection_result};
    use crate::state::AiTarget;

    #[test]
    fn command_and_inspection_results_share_execution_facts() {
        let target = AiTarget::Ssh {
            connection_id: 7,
            workspace_session_id: "remote:7:test".into(),
            terminal_id: "ssh-test".into(),
            terminal_generation: 1,
        };
        let command = command_result(&target, "/root".into(), 0, None);
        let inspection = inspection_result(&target, "system_info", "host".into(), None);

        assert_eq!(command["structuredContent"]["succeeded"], true);
        assert_eq!(inspection["structuredContent"]["succeeded"], true);
        assert_eq!(
            inspection["content"][0]["text"],
            "检查成功，结果如下：\nhost"
        );
        assert_eq!(
            command["structuredContent"]["execution"],
            inspection["structuredContent"]["execution"]
        );
    }

    #[test]
    fn command_failure_keeps_output_and_sets_mcp_error() {
        let result = command_result(
            &AiTarget::Local {
                session_id: "local-one".into(),
                terminal_id: "terminal-one".into(),
            },
            "not found".into(),
            127,
            None,
        );

        assert_eq!(result["isError"], true);
        assert_eq!(result["structuredContent"]["output"], "not found");
        assert_eq!(result["structuredContent"]["exitCode"], 127);
    }

    #[test]
    fn empty_inspection_output_is_an_explicit_success_for_text_only_clients() {
        let inspection = inspection_result(
            &AiTarget::Ssh {
                connection_id: 7,
                workspace_session_id: "remote:7:test".into(),
                terminal_id: "ssh-test".into(),
                terminal_generation: 1,
            },
            "logs",
            "".into(),
            None,
        );

        assert_eq!(inspection["content"][0]["text"], "检查成功，结果为空。");
        assert_eq!(inspection["structuredContent"]["output"], "");
        assert_eq!(inspection["structuredContent"]["succeeded"], true);
    }
}
