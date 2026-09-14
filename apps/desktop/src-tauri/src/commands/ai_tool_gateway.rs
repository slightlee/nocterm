//! Nocterm MCP 工具清单与目标路由。
//! 传输协议留在 `ai_bridge`，本模块只处理一次工具调用的业务边界。

use std::{sync::Arc, time::Instant};

use nocterm_application::ai_audit::AiAuditService;
use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTool};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::{
    commands::{
        ai_policy::{
            AI_EXECUTION_TIMEOUT, AI_TOOL_CALL_TIMEOUT, AiExecutionDecision, AiOperationClass,
            execution_decision, is_safe_readonly_command,
        },
        ai_terminal::execute_ssh_inspection_sync,
        ai_tool_audit::{append as append_audit, elapsed_ms, execute_after_audit, tool_from_name},
        ai_tools::{
            ServerInspection, build_server_inspection, is_server_tool, server_tool_definitions,
        },
    },
    state::{
        AiGatewayAccess, AiGatewayBinding, AiGatewayState, AiTarget, AppState,
        LocalTerminalRegistry,
    },
};

/// 工具路由只依赖现有应用服务，不拥有 SSH、凭据或 Provider 生命周期。
pub(crate) struct GatewayServices {
    pub(super) app: AppHandle,
    pub(super) connection: nocterm_application::connection::ConnectionService,
    pub(super) terminal: nocterm_application::terminal::TerminalService,
    pub(super) local_terminal: nocterm_application::terminal::LocalTerminalService,
    pub(super) local_registry: Arc<LocalTerminalRegistry>,
    pub(super) audit: AiAuditService,
}

/// 一次工具调用的不可变安全上下文，统一传递授权身份、审计类型和绝对截止时间。
pub(super) struct ToolExecutionContext<'a> {
    pub gateway: &'a AiGatewayState,
    pub token: &'a str,
    pub access: &'a AiGatewayAccess,
    pub audit_tool: AiAuditTool,
    pub deadline: Instant,
}

impl GatewayServices {
    pub(crate) fn new(app: AppHandle, state: &AppState) -> Self {
        Self {
            app,
            connection: state.connection_service().clone(),
            terminal: state.terminal_service().clone(),
            local_terminal: state.local_terminal_service().clone(),
            local_registry: Arc::clone(state.local_terminals()),
            audit: state.ai_audit_service().clone(),
        }
    }

    pub(crate) fn tools_for_target(&self, target: &AiTarget) -> Value {
        tools_for_target(target)
    }

    pub(crate) fn call(
        &self,
        gateway: &AiGatewayState,
        token: &str,
        access: &AiGatewayAccess,
        request: &Value,
    ) -> Value {
        let binding = &access.binding;
        let deadline = Instant::now() + AI_TOOL_CALL_TIMEOUT;
        let target = &binding.target;
        let name = request
            .pointer("/params/name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let audit_tool = tool_from_name(name);
        if let Err(error) = gateway.check_tool_rate(token) {
            return self.audited_error(
                binding,
                audit_tool,
                AiAuditApproval::NotRequested,
                "AI_TOOL_RATE_LIMITED",
                &error,
            );
        }
        if name == "session_context" {
            return self.session_context(binding);
        }
        let context = ToolExecutionContext {
            gateway,
            token,
            access,
            audit_tool,
            deadline,
        };
        if is_server_tool(name) {
            return self.execute_server_tool(&context, name, request);
        }
        if name == execution_tool_name(target) {
            let command = request
                .pointer("/params/arguments/command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let operation = if is_safe_readonly_command(command) {
                AiOperationClass::ReadOnly
            } else {
                AiOperationClass::Unrestricted
            };
            return match execution_decision(binding.command_policy, operation) {
                AiExecutionDecision::Deny => self.audited_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::NotRequested,
                    "AI_TOOL_EXECUTION_DISABLED",
                    "当前会话已设置为仅分析，未执行终端命令",
                ),
                AiExecutionDecision::Confirm => self.execute_after_approval(&context, command),
                AiExecutionDecision::Execute => {
                    self.execute_command_without_approval(access, audit_tool, command, deadline)
                }
            };
        }
        self.audited_error(
            binding,
            AiAuditTool::Unknown,
            AiAuditApproval::NotRequested,
            "AI_TOOL_UNKNOWN",
            "unknown tool",
        )
    }

    /// 结构化工具只对 SSH 目标开放，命令由后端计划器生成后复用当前认证连接。
    fn execute_server_tool(
        &self,
        context: &ToolExecutionContext<'_>,
        name: &str,
        request: &Value,
    ) -> Value {
        let access = context.access;
        let audit_tool = context.audit_tool;
        let deadline = context.deadline;
        let binding = &access.binding;
        let AiTarget::Ssh { .. } = &binding.target else {
            return self.audited_error(
                binding,
                audit_tool,
                AiAuditApproval::NotRequired,
                "AI_TOOL_TARGET_INVALID",
                "服务器检查工具只能用于 SSH 目标",
            );
        };
        let arguments = request
            .pointer("/params/arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let plan = match build_server_inspection(name, &arguments) {
            Ok(plan) => plan,
            Err(error) => {
                return self.audited_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::NotRequired,
                    "AI_TOOL_ARGUMENTS_INVALID",
                    &error,
                );
            }
        };
        match execution_decision(binding.command_policy, AiOperationClass::ReadOnly) {
            AiExecutionDecision::Deny => self.audited_error(
                binding,
                audit_tool,
                AiAuditApproval::NotRequested,
                "AI_TOOL_EXECUTION_DISABLED",
                "当前会话已设置为仅分析，未访问终端",
            ),
            AiExecutionDecision::Confirm => {
                self.execute_with_approval_callback(context, &plan.command, |_| {
                    self.execute_server_inspection(
                        binding,
                        &access.cancellation,
                        audit_tool,
                        &plan,
                        AiAuditApproval::Approved,
                        deadline,
                    )
                })
            }
            AiExecutionDecision::Execute => self.execute_server_inspection(
                binding,
                &access.cancellation,
                audit_tool,
                &plan,
                AiAuditApproval::NotRequired,
                deadline,
            ),
        }
    }

    fn execute_server_inspection(
        &self,
        binding: &AiGatewayBinding,
        cancellation: &std::sync::atomic::AtomicBool,
        audit_tool: AiAuditTool,
        plan: &ServerInspection,
        approval: AiAuditApproval,
        deadline: Instant,
    ) -> Value {
        let AiTarget::Ssh { connection_id } = &binding.target else {
            return self.audited_error(
                binding,
                audit_tool,
                approval,
                "AI_TOOL_TARGET_INVALID",
                "服务器检查工具只能用于 SSH 目标",
            );
        };
        let execution_deadline = (Instant::now() + AI_EXECUTION_TIMEOUT).min(deadline);
        // 前置事件必须成功落库；否则远程 exec channel 不会被打开。
        let started_at = Instant::now();
        let result = match execute_after_audit(&self.audit, binding, audit_tool, approval, || {
            execute_ssh_inspection_sync(
                &self.connection,
                &self.terminal,
                *connection_id,
                &plan.command,
                cancellation,
                execution_deadline,
            )
        }) {
            Ok(result) => result,
            Err(error) => return tool_error(&error),
        };
        let (outcome, error_code) = if result.is_ok() {
            (AiAuditOutcome::Succeeded, None)
        } else {
            (AiAuditOutcome::Failed, Some("AI_TOOL_EXECUTION_FAILED"))
        };
        if append_audit(
            &self.audit,
            binding,
            audit_tool,
            approval,
            outcome,
            Some(elapsed_ms(started_at)),
            error_code,
        )
        .is_err()
        {
            // pending 前置事件仍能明确标识结果未知，且不会泄露已取得的远端输出。
            return tool_error("服务器检查已执行，但完成审计写入失败，请稍后核查且不要自动重试");
        }
        match result {
            Ok(output) => {
                let structured = json!({"operation":plan.operation,"output":output});
                json!({
                    "content":[{"type":"text","text":structured["output"]}],
                    "structuredContent":structured
                })
            }
            Err(error) => tool_error(&error),
        }
    }

    /// 会话上下文只由 token 对应的后端绑定生成，不接受 Provider 传入目标。
    fn session_context(&self, binding: &AiGatewayBinding) -> Value {
        let context = match &binding.target {
            AiTarget::Ssh { connection_id } => self.connection.get(*connection_id).map(|profile| {
                json!({
                    "targetKind":"ssh",
                    "targetSessionId":connection_id.to_string(),
                    "connectionId":connection_id,
                    "name":profile.name,
                    "host":profile.host,
                    "port":profile.port,
                    "username":profile.username
                })
            }),
            AiTarget::Local {
                session_id,
                terminal_id,
            } => {
                if self
                    .local_registry
                    .terminal_for(session_id)
                    .is_some_and(|current| current == *terminal_id)
                {
                    Ok(json!({
                        "targetKind":"local",
                        "targetSessionId":session_id,
                        "terminalId":terminal_id
                    }))
                } else {
                    Err(nocterm_application::error::AppError::new(
                        "LOCAL_TERMINAL_NOT_FOUND",
                        "目标本地终端不存在或已重新连接",
                        false,
                    ))
                }
            }
        };
        let (outcome, error_code) = if context.is_ok() {
            (AiAuditOutcome::Succeeded, None)
        } else {
            (AiAuditOutcome::Failed, Some("AI_TOOL_TARGET_UNAVAILABLE"))
        };
        if let Err(error) = append_audit(
            &self.audit,
            binding,
            AiAuditTool::SessionContext,
            AiAuditApproval::NotRequired,
            outcome,
            None,
            error_code,
        ) {
            return tool_error(&error);
        }
        match context {
            Ok(context) => json!({
                "content":[{"type":"text","text":context.to_string()}],
                "structuredContent":context
            }),
            Err(error) => tool_error(&error.message),
        }
    }

    fn audited_error(
        &self,
        binding: &AiGatewayBinding,
        tool: AiAuditTool,
        approval: AiAuditApproval,
        error_code: &'static str,
        message: &str,
    ) -> Value {
        match append_audit(
            &self.audit,
            binding,
            tool,
            approval,
            AiAuditOutcome::Failed,
            None,
            Some(error_code),
        ) {
            Ok(()) => tool_error(message),
            Err(error) => tool_error(&error),
        }
    }
}

fn tools_for_target(target: &AiTarget) -> Value {
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
            "在当前 Nocterm SSH 连接上执行通用命令；是否需要确认由当前会话权限决定。仅在结构化服务器检查无法完成任务时使用，不得改到本机执行"
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
                "exitCode":{"type":"integer"}
            },
            "required":["output","exitCode"],
            "additionalProperties":false
        },
        "annotations":{"readOnlyHint":false,"destructiveHint":true,"idempotentHint":false,"openWorldHint":false}
    }));
    json!({"tools":tools})
}

fn execution_tool_name(target: &AiTarget) -> &'static str {
    match target {
        AiTarget::Ssh { .. } => "ssh_exec",
        AiTarget::Local { .. } => "local_terminal_exec",
    }
}

fn tool_error(message: &str) -> Value {
    json!({"isError":true,"content":[{"type":"text","text":message}]})
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
