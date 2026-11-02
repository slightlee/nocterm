//! AI 工具审计映射。这里只把 Gateway 的运行时身份转换为脱敏领域事件。

use std::time::{Duration, Instant};

use crate::state::{AiGatewayBinding, AiTarget};
use nocterm_application::ai_audit::AiAuditService;
use nocterm_domain::ai_audit::{
    AiAuditApproval, AiAuditOutcome, AiAuditTarget, AiAuditTool, AiToolAuditEvent,
};

/// 未识别的 Provider 输入统一记为 `unknown`，不把任意工具名写入本地数据库。
pub(crate) fn tool_from_name(name: &str) -> AiAuditTool {
    match name {
        "session_context" => AiAuditTool::SessionContext,
        "get_system_info" => AiAuditTool::GetSystemInfo,
        "list_processes" => AiAuditTool::ListProcesses,
        "list_listening_ports" => AiAuditTool::ListListeningPorts,
        "get_service_status" => AiAuditTool::GetServiceStatus,
        "read_service_logs" => AiAuditTool::ReadServiceLogs,
        "get_disk_usage" => AiAuditTool::GetDiskUsage,
        "get_memory_usage" => AiAuditTool::GetMemoryUsage,
        "list_docker_containers" => AiAuditTool::ListDockerContainers,
        "get_docker_info" => AiAuditTool::GetDockerInfo,
        "get_docker_container_status" => AiAuditTool::GetDockerContainerStatus,
        "read_docker_logs" => AiAuditTool::ReadDockerLogs,
        "ssh_exec" => AiAuditTool::SshExec,
        "local_terminal_exec" => AiAuditTool::LocalTerminalExec,
        _ => AiAuditTool::Unknown,
    }
}

pub(crate) fn append(
    service: &AiAuditService,
    binding: &AiGatewayBinding,
    tool: AiAuditTool,
    approval: AiAuditApproval,
    outcome: AiAuditOutcome,
    duration_ms: Option<u64>,
    error_code: Option<&'static str>,
) -> Result<(), String> {
    append_with_execution(
        service,
        binding,
        tool,
        approval,
        outcome,
        duration_ms,
        error_code,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn append_with_execution(
    service: &AiAuditService,
    binding: &AiGatewayBinding,
    tool: AiAuditTool,
    approval: AiAuditApproval,
    outcome: AiAuditOutcome,
    duration_ms: Option<u64>,
    error_code: Option<&'static str>,
    execution_id: Option<&str>,
) -> Result<(), String> {
    let target = match &binding.target {
        AiTarget::Ssh { connection_id, .. } => AiAuditTarget::Ssh {
            connection_id: *connection_id,
        },
        AiTarget::Local { .. } => AiAuditTarget::Local,
    };
    service
        .append(&AiToolAuditEvent {
            execution_id: execution_id.map(str::to_string),
            provider: binding.provider.clone(),
            session_id: binding.session_id.clone(),
            target,
            tool,
            approval,
            outcome,
            duration_ms,
            error_code,
        })
        .map_err(|error| error.message)
}

/// 守卫语义的测试验证：前置审计失败时执行闭包不被调用。
/// 生产路径由 ledger 生命周期（TerminalExecution::run）接管同一时序。
#[cfg(test)]
pub(crate) fn execute_after_audit<T>(
    service: &AiAuditService,
    binding: &AiGatewayBinding,
    tool: AiAuditTool,
    approval: AiAuditApproval,
    execute: impl FnOnce() -> T,
) -> Result<T, String> {
    append(
        service,
        binding,
        tool,
        approval,
        AiAuditOutcome::Pending,
        None,
        None,
    )?;
    Ok(execute())
}

pub(crate) fn elapsed_ms(started_at: Instant) -> u64 {
    duration_ms(started_at.elapsed())
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use crate::state::{AiCommandPolicy, AiGatewayBinding, AiTarget};
    use nocterm_domain::ai_audit::{AiAuditRepository, AiAuditRepositoryError, AiToolAuditEvent};

    use super::*;

    struct FailingRepository;

    impl AiAuditRepository for FailingRepository {
        fn append(&self, _event: &AiToolAuditEvent) -> Result<(), AiAuditRepositoryError> {
            Err(AiAuditRepositoryError::new("audit unavailable"))
        }
    }

    #[test]
    fn unknown_tool_names_are_redacted() {
        assert_eq!(tool_from_name("ssh_exec; secret"), AiAuditTool::Unknown);
    }

    #[test]
    fn elapsed_duration_conversion_is_bounded() {
        assert_eq!(duration_ms(Duration::from_millis(42)), 42);
        assert_eq!(duration_ms(Duration::MAX), u64::MAX);
    }

    #[test]
    fn failed_pre_execution_audit_prevents_the_operation() {
        let service = AiAuditService::new(Arc::new(FailingRepository));
        let binding = AiGatewayBinding {
            target: AiTarget::Ssh {
                connection_id: 7,
                workspace_session_id: "remote:7:test".into(),
                terminal_id: "ssh-test".into(),
                terminal_generation: 1,
            },
            provider: "codex".into(),
            conversation_id: "conversation-one".into(),
            session_id: "ai-one".into(),
            command_policy: AiCommandPolicy::AutoSafe,
        };
        let executed = AtomicBool::new(false);

        let result = execute_after_audit(
            &service,
            &binding,
            AiAuditTool::GetSystemInfo,
            AiAuditApproval::NotRequired,
            || executed.store(true, Ordering::Relaxed),
        );

        assert!(result.is_err());
        assert!(!executed.load(Ordering::Relaxed));
    }
}
