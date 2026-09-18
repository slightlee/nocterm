//! 后端受控工具的参数规划、执行和审计。

use std::{sync::atomic::AtomicBool, time::Instant};

use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTool};
use serde_json::{Value, json};

use crate::{
    commands::{
        ai_policy::{
            AI_EXECUTION_TIMEOUT, AiExecutionDecision, AiOperationClass, execution_decision,
        },
        ai_terminal::execute_ssh_inspection_sync,
        ai_tool_audit::{append as append_audit, elapsed_ms, execute_after_audit},
        ai_tool_contract::{execution_context, inspection_result},
        ai_tools::{ServerInspection, build_server_inspection},
    },
    state::{AiGatewayBinding, AiTarget},
};

use super::{GatewayServices, ToolExecutionContext, tool_error};

impl GatewayServices {
    /// 结构化工具只对 SSH 目标开放，命令由后端计划器生成后复用当前认证连接。
    pub(super) fn execute_server_tool(
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
                match self.execute_with_approval_callback(context, &plan.command, |_| {
                    self.execute_server_inspection_result(
                        binding,
                        &access.cancellation,
                        audit_tool,
                        &plan,
                        AiAuditApproval::Approved,
                        deadline,
                    )
                }) {
                    Ok(result) => inspection_result(&binding.target, plan.operation, result.output),
                    Err(error) => tool_error(&error),
                }
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
        cancellation: &AtomicBool,
        audit_tool: AiAuditTool,
        plan: &ServerInspection,
        approval: AiAuditApproval,
        deadline: Instant,
    ) -> Value {
        match self.execute_server_inspection_result(
            binding,
            cancellation,
            audit_tool,
            plan,
            approval,
            deadline,
        ) {
            Ok(result) => inspection_result(&binding.target, plan.operation, result.output),
            Err(error) => tool_error(&error),
        }
    }

    fn execute_server_inspection_result(
        &self,
        binding: &AiGatewayBinding,
        cancellation: &AtomicBool,
        audit_tool: AiAuditTool,
        plan: &ServerInspection,
        approval: AiAuditApproval,
        deadline: Instant,
    ) -> Result<crate::commands::ai_terminal::TerminalCommandResult, String> {
        let AiTarget::Ssh { connection_id } = &binding.target else {
            return Err("服务器检查工具只能用于 SSH 目标".to_string());
        };
        let execution_deadline = (Instant::now() + AI_EXECUTION_TIMEOUT).min(deadline);
        // 前置事件必须成功落库；否则远程 exec channel 不会被打开。
        let started_at = Instant::now();
        let result = execute_after_audit(&self.audit, binding, audit_tool, approval, || {
            execute_ssh_inspection_sync(
                &self.connection,
                &self.terminal,
                *connection_id,
                &plan.command,
                cancellation,
                execution_deadline,
            )
        })?;
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
            return Err(
                "服务器检查已执行，但完成审计写入失败，请稍后核查且不要自动重试".to_string(),
            );
        }
        match result {
            Ok(output) => Ok(crate::commands::ai_terminal::TerminalCommandResult {
                output,
                exit_code: 0,
            }),
            Err(error) => Err(error),
        }
    }

    /// 会话上下文只由 token 对应的后端绑定生成，不接受 Provider 传入目标。
    pub(super) fn session_context(&self, binding: &AiGatewayBinding) -> Value {
        let context = match &binding.target {
            AiTarget::Ssh { connection_id } => self.connection.get(*connection_id).map(|profile| {
                json!({
                    "targetKind":"ssh",
                    "targetSessionId":connection_id.to_string(),
                    "connectionId":connection_id,
                    "name":profile.name,
                    "host":profile.host,
                    "port":profile.port,
                    "username":profile.username,
                    "execution":execution_context(&binding.target)
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
                        "terminalId":terminal_id,
                        "execution":execution_context(&binding.target)
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
}
