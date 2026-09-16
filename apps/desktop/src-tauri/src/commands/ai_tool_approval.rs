//! 通用终端命令的逐次审批与单次执行状态机。

use std::{
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTool};
use serde_json::{Value, json};
use tauri::Emitter;

use crate::{
    commands::{
        ai_policy::{AI_APPROVAL_TIMEOUT, AI_EXECUTION_TIMEOUT},
        ai_terminal::{execute_local_sync, execute_ssh_sync},
        ai_tool_audit::{append as append_audit, elapsed_ms, execute_after_audit},
        ai_tool_gateway::{GatewayServices, ToolExecutionContext, execution_context},
    },
    dto::ai::{AiToolApprovalClosedEvent, AiToolApprovalEvent},
    state::{AiApprovalDecision, AiGatewayAccess, AiGatewayBinding, AiTarget},
};

static NEXT_APPROVAL_ID: AtomicU64 = AtomicU64::new(1);

impl GatewayServices {
    pub(super) fn execute_after_approval(
        &self,
        context: &ToolExecutionContext<'_>,
        command: &str,
    ) -> Value {
        self.execute_with_approval_callback(context, command, |command| {
            self.execute_command(
                context.access,
                context.audit_tool,
                command,
                AiAuditApproval::Approved,
                context.deadline,
            )
        })
    }

    /// 结构化工具和通用命令共用同一审批状态机，避免“每次确认”出现旁路。
    pub(super) fn execute_with_approval_callback<F>(
        &self,
        context: &ToolExecutionContext<'_>,
        command: &str,
        on_approved: F,
    ) -> Value
    where
        F: FnOnce(&str) -> Value,
    {
        let binding = &context.access.binding;
        let audit_tool = context.audit_tool;
        let command = command.trim();
        if command.is_empty() || command.len() > 4096 || command.contains('\0') {
            return self.approval_error(
                binding,
                audit_tool,
                AiAuditApproval::NotRequested,
                AiAuditOutcome::Failed,
                "AI_TOOL_ARGUMENTS_INVALID",
                "终端命令不能为空、不能包含空字符且不能超过 4096 个字符",
            );
        }
        let approval_id = format!(
            "ai-approval-{}",
            NEXT_APPROVAL_ID.fetch_add(1, AtomicOrdering::Relaxed)
        );
        let (target_kind, target_label) = match &binding.target {
            AiTarget::Ssh { connection_id } => match self.connection.get(*connection_id) {
                Ok(profile) => (
                    "ssh".to_string(),
                    format!(
                        "{}（{}@{}:{}）",
                        profile.name, profile.username, profile.host, profile.port
                    ),
                ),
                Err(error) => {
                    return self.approval_error(
                        binding,
                        audit_tool,
                        AiAuditApproval::NotRequested,
                        AiAuditOutcome::Failed,
                        "AI_TOOL_TARGET_UNAVAILABLE",
                        &error.message,
                    );
                }
            },
            AiTarget::Local {
                session_id,
                terminal_id,
            } => {
                if self
                    .local_registry
                    .terminal_for(session_id)
                    .is_none_or(|current| current != *terminal_id)
                {
                    return self.approval_error(
                        binding,
                        audit_tool,
                        AiAuditApproval::NotRequested,
                        AiAuditOutcome::Failed,
                        "AI_TOOL_TARGET_UNAVAILABLE",
                        "目标本地终端不存在或已重新连接",
                    );
                }
                ("local".to_string(), session_id.clone())
            }
        };
        // 审批事件先持久化，再向 UI 展示命令；审计不可用时不会产生可批准的请求。
        if let Err(error) = append_audit(
            &self.audit,
            binding,
            audit_tool,
            AiAuditApproval::Requested,
            AiAuditOutcome::Pending,
            None,
            None,
        ) {
            return tool_error(&error);
        }
        let receiver = match context
            .gateway
            .request_approval(context.token.to_string(), approval_id.clone())
        {
            Ok(receiver) => receiver,
            Err(error) => {
                return self.approval_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::Requested,
                    AiAuditOutcome::Failed,
                    "AI_TOOL_TASK_REVOKED",
                    &error,
                );
            }
        };
        let approval_deadline = (Instant::now() + AI_APPROVAL_TIMEOUT).min(context.deadline);
        let approval_wait = approval_deadline.saturating_duration_since(Instant::now());
        let expires_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .saturating_add(approval_wait.as_millis())
            .min(u128::from(u64::MAX)) as u64;
        if self
            .app
            .emit(
                "nocterm://ai-tool-approval",
                AiToolApprovalEvent {
                    approval_id: approval_id.clone(),
                    session_id: binding.session_id.clone(),
                    target_kind,
                    target_label,
                    command: command.to_string(),
                    expires_at_unix_ms,
                },
            )
            .is_err()
        {
            let _ = context
                .gateway
                .resolve_approval(&approval_id, &binding.session_id, false);
            return self.approval_error(
                binding,
                audit_tool,
                AiAuditApproval::Requested,
                AiAuditOutcome::Failed,
                "AI_TOOL_APPROVAL_UI_UNAVAILABLE",
                "无法向 Nocterm 界面发送命令确认请求",
            );
        }
        match receiver.recv_timeout(approval_wait) {
            Ok(AiApprovalDecision::Approved) => {
                self.emit_approval_closed(binding, &approval_id, "approved");
                on_approved(command)
            }
            Ok(AiApprovalDecision::Rejected) => {
                self.emit_approval_closed(binding, &approval_id, "rejected");
                self.approval_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_APPROVAL_REJECTED",
                    "用户拒绝执行该终端命令",
                )
            }
            Ok(AiApprovalDecision::Revoked) => {
                self.emit_approval_closed(binding, &approval_id, "revoked");
                self.approval_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_TASK_REVOKED",
                    "AI 任务已停止，终端命令未执行",
                )
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = context
                    .gateway
                    .resolve_approval(&approval_id, &binding.session_id, false);
                self.emit_approval_closed(binding, &approval_id, "timed_out");
                self.approval_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::TimedOut,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_APPROVAL_TIMEOUT",
                    "终端命令确认已超时",
                )
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.emit_approval_closed(binding, &approval_id, "revoked");
                self.approval_error(
                    binding,
                    audit_tool,
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_TASK_REVOKED",
                    "AI 任务已停止，终端命令未执行",
                )
            }
        }
    }

    /// UI 只接受同一 session/approval 的关闭事件；事件丢失时仍有 expiresAt 本地兜底。
    fn emit_approval_closed(
        &self,
        binding: &AiGatewayBinding,
        approval_id: &str,
        resolution: &str,
    ) {
        let _ = self.app.emit(
            "nocterm://ai-tool-approval-closed",
            AiToolApprovalClosedEvent {
                approval_id: approval_id.to_string(),
                session_id: binding.session_id.clone(),
                resolution: resolution.to_string(),
            },
        );
    }

    /// 策略允许跳过确认时仍使用同一审计和执行核心。
    pub(super) fn execute_command_without_approval(
        &self,
        access: &AiGatewayAccess,
        audit_tool: AiAuditTool,
        command: &str,
        call_deadline: Instant,
    ) -> Value {
        let command = match crate::commands::ai_policy::validate_approved_command(command) {
            Ok(command) => command,
            Err(error) => {
                return self.approval_error(
                    &access.binding,
                    audit_tool,
                    AiAuditApproval::NotRequested,
                    AiAuditOutcome::Failed,
                    "AI_TOOL_ARGUMENTS_INVALID",
                    &error,
                );
            }
        };
        self.execute_command(
            access,
            audit_tool,
            &command,
            AiAuditApproval::NotRequired,
            call_deadline,
        )
    }

    fn execute_command(
        &self,
        access: &AiGatewayAccess,
        audit_tool: AiAuditTool,
        command: &str,
        approval: AiAuditApproval,
        call_deadline: Instant,
    ) -> Value {
        let binding = &access.binding;
        let execution_deadline = (Instant::now() + AI_EXECUTION_TIMEOUT).min(call_deadline);
        // 无论是用户批准还是策略自动放行，命令都必须先写入 pending 审计再执行。
        let started_at = Instant::now();
        let result =
            match execute_after_audit(
                &self.audit,
                binding,
                audit_tool,
                approval,
                || match &binding.target {
                    AiTarget::Ssh { connection_id } => execute_ssh_sync(
                        &self.connection,
                        &self.terminal,
                        *connection_id,
                        command,
                        &access.cancellation,
                        execution_deadline,
                    ),
                    AiTarget::Local {
                        session_id,
                        terminal_id,
                    } => execute_local_sync(
                        &self.local_terminal,
                        &self.local_registry,
                        session_id,
                        terminal_id,
                        command,
                        &access.cancellation,
                        execution_deadline,
                    ),
                },
            ) {
                Ok(result) => result,
                Err(error) => return tool_error(&error),
            };
        let succeeded = result.as_ref().is_ok_and(|value| value.exit_code == 0);
        let (outcome, error_code) = if succeeded {
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
            return tool_error("终端命令可能已执行，但完成审计写入失败，请勿自动重试");
        }
        match result {
            Ok(result) => {
                let structured = json!({
                    "output":result.output,
                    "exitCode":result.exit_code,
                    "succeeded":succeeded,
                    "execution":execution_context(&binding.target)
                });
                json!({
                    "isError":!succeeded,
                    "content":[{"type":"text","text":structured.to_string()}],
                    "structuredContent":structured
                })
            }
            Err(error) => tool_error(&error),
        }
    }

    /// 审批失败只有在对应脱敏事件成功落库后才返回原始业务错误。
    fn approval_error(
        &self,
        binding: &AiGatewayBinding,
        tool: AiAuditTool,
        approval: AiAuditApproval,
        outcome: AiAuditOutcome,
        error_code: &'static str,
        message: &str,
    ) -> Value {
        match append_audit(
            &self.audit,
            binding,
            tool,
            approval,
            outcome,
            None,
            Some(error_code),
        ) {
            Ok(()) => tool_error(message),
            Err(error) => tool_error(&error),
        }
    }
}

fn tool_error(message: &str) -> Value {
    json!({"isError":true,"content":[{"type":"text","text":message}]})
}
