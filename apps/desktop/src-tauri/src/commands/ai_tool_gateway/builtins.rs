//! 后端受控工具的参数规划、执行和审计。

use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTool};
use serde_json::{Value, json};

use crate::{
    commands::{
        ai_policy::{AiExecutionDecision, AiOperationClass, execution_decision},
        ai_tool_contract::{execution_context, inspection_result},
        ai_tools::build_server_inspection,
    },
    state::{AiGatewayBinding, AiTarget},
};

use super::{GatewayServices, ToolExecutionContext, tool_error};

impl GatewayServices {
    /// 结构化工具只对 SSH 目标开放，命令由后端计划器生成后复用当前认证连接。
    pub(super) fn execute_server_tool(
        &self,
        context: &mut ToolExecutionContext<'_>,
        name: &str,
        request: &Value,
    ) -> Value {
        let access = context.access;
        let audit_tool = context.audit_tool;
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
            AiExecutionDecision::Deny => {
                if let Err(error) = context
                    .execution
                    .transition(nocterm_domain::ai_execution::AiExecutionTransition::DenyByPolicy)
                {
                    return tool_error(&error.to_string());
                }
                self.audited_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::NotRequested,
                    "AI_TOOL_EXECUTION_DISABLED",
                    "当前会话已设置为仅分析，未访问终端",
                )
            }
            AiExecutionDecision::Confirm => {
                // 与通用终端命令同一时序：先进入等待审批，批准回调里才 Approve。
                if let Err(error) = context.execution.transition(
                    nocterm_domain::ai_execution::AiExecutionTransition::RequestApproval,
                ) {
                    return tool_error(&error.to_string());
                }
                match self.execute_after_approval(context, &plan.command) {
                    Ok(result) if result.exit_code == 0 => inspection_result(
                        &binding.target,
                        plan.operation,
                        result.output,
                        Some(context.execution.record()),
                    ),
                    Ok(result) => tool_error(&format!(
                        "服务器检查失败（退出码 {}）：{}",
                        result.exit_code, result.output
                    )),
                    Err(error) => tool_error(&error),
                }
            }
            AiExecutionDecision::Execute => {
                if let Err(error) = context.execution.transition(
                    nocterm_domain::ai_execution::AiExecutionTransition::AuthorizeAutomatically,
                ) {
                    return tool_error(&error.to_string());
                }
                match self.execute_command_without_approval(context, &plan.command) {
                    Ok(result) if result.exit_code == 0 => inspection_result(
                        &binding.target,
                        plan.operation,
                        result.output,
                        Some(context.execution.record()),
                    ),
                    Ok(result) => tool_error(&format!(
                        "服务器检查失败（退出码 {}）：{}",
                        result.exit_code, result.output
                    )),
                    Err(error) => tool_error(&error),
                }
            }
        }
    }

    /// 会话上下文只由 token 对应的后端绑定生成，不接受 Provider 传入目标。
    pub(super) fn session_context(&self, binding: &AiGatewayBinding) -> Value {
        let context = match &binding.target {
            AiTarget::Ssh { connection_id, .. } => {
                self.connection.get(*connection_id).map(|profile| {
                    json!({
                        "targetKind":"ssh",
                        "targetSessionId":connection_id.to_string(),
                        "connectionId":connection_id,
                        "name":profile.name,
                        "host":profile.host,
                        "port":profile.port,
                        "username":profile.username,
                        "execution":execution_context(&binding.target, None)
                    })
                })
            }
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
                        "terminalId":terminal_id,
                        "execution":execution_context(&binding.target, None)
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
        if let Err(error) = crate::commands::ai_tool_audit::append(
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
}
