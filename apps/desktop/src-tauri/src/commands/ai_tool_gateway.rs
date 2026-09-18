//! Nocterm MCP 工具清单与目标路由。
//! 传输协议留在 `ai_bridge`，本模块只处理一次工具调用的业务边界。

mod builtins;
mod catalog;

use std::{sync::Arc, time::Instant};

use nocterm_application::ai_audit::AiAuditService;
use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTool};
use serde_json::{Value, json};
use tauri::AppHandle;

use crate::{
    commands::{
        ai_policy::{
            AI_TOOL_CALL_TIMEOUT, AiExecutionDecision, AiOperationClass, execution_decision,
            is_safe_readonly_command,
        },
        ai_terminal::TerminalCommandResult,
        ai_tool_audit::{append as append_audit, tool_from_name},
        ai_tool_contract::command_result,
        ai_tools::is_server_tool,
    },
    state::{
        AiGatewayAccess, AiGatewayBinding, AiGatewayState, AiTarget, AppState,
        LocalTerminalRegistry,
    },
};

use self::catalog::{execution_tool_name, tools_for_target};

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
pub(crate) struct ToolExecutionContext<'a> {
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
        let is_known_tool = name == "session_context"
            || is_server_tool(name)
            || name == execution_tool_name(target);
        if is_known_tool && let Err(error) = gateway.mark_tool_call(token) {
            return self.audited_error(
                binding,
                audit_tool,
                AiAuditApproval::NotRequested,
                "AI_TOOL_STATE_UNAVAILABLE",
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
            return match self.execute_terminal_command(&context, command) {
                Ok(result) => command_result(&binding.target, result.output, result.exit_code),
                Err(error) => tool_error(&error),
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

    /// MCP 与 ACP 共用这一条类型化命令入口；协议层只负责各自的结果编码。
    pub(crate) fn execute_terminal_command(
        &self,
        context: &ToolExecutionContext<'_>,
        command: &str,
    ) -> Result<TerminalCommandResult, String> {
        let operation = if is_safe_readonly_command(command) {
            AiOperationClass::ReadOnly
        } else {
            AiOperationClass::Unrestricted
        };
        match execution_decision(context.access.binding.command_policy, operation) {
            AiExecutionDecision::Deny => Err(self.audited_failure(
                &context.access.binding,
                context.audit_tool,
                AiAuditApproval::NotRequested,
                "AI_TOOL_EXECUTION_DISABLED",
                "当前会话已设置为仅分析，未执行终端命令",
            )),
            AiExecutionDecision::Confirm => self.execute_after_approval(context, command),
            AiExecutionDecision::Execute => self.execute_command_without_approval(
                context.access,
                context.audit_tool,
                command,
                context.deadline,
            ),
        }
    }

    pub(super) fn audited_error(
        &self,
        binding: &AiGatewayBinding,
        tool: AiAuditTool,
        approval: AiAuditApproval,
        error_code: &'static str,
        message: &str,
    ) -> Value {
        tool_error(&self.audited_failure(binding, tool, approval, error_code, message))
    }

    pub(crate) fn audited_failure(
        &self,
        binding: &AiGatewayBinding,
        tool: AiAuditTool,
        approval: AiAuditApproval,
        error_code: &'static str,
        message: &str,
    ) -> String {
        match append_audit(
            &self.audit,
            binding,
            tool,
            approval,
            AiAuditOutcome::Failed,
            None,
            Some(error_code),
        ) {
            Ok(()) => message.to_string(),
            Err(error) => error,
        }
    }
}

pub(super) fn tool_error(message: &str) -> Value {
    json!({"isError":true,"content":[{"type":"text","text":message}]})
}
