//! 通用终端命令的逐次审批与单次执行状态机。

use std::{
    sync::atomic::{AtomicU64, Ordering as AtomicOrdering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use nocterm_application::{
    ai_audit::AiAuditService,
    connection::ConnectionService,
    terminal::{LocalTerminalService, TerminalService},
};
use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTool};
use tauri::Emitter;

use crate::{
    commands::{
        ai_policy::{AI_APPROVAL_TIMEOUT, AI_EXECUTION_TIMEOUT},
        ai_terminal::{TerminalCommandResult, execute_local_sync, execute_ssh_sync},
        ai_tool_audit::{append as append_audit, elapsed_ms, execute_after_audit},
        ai_tool_gateway::{GatewayServices, ToolExecutionContext},
    },
    dto::ai::{AiToolApprovalClosedEvent, AiToolApprovalEvent},
    state::{
        AiApprovalDecision, AiGatewayAccess, AiGatewayBinding, AiGatewayState, AiTarget,
        LocalTerminalRegistry,
    },
};

static NEXT_APPROVAL_ID: AtomicU64 = AtomicU64::new(1);

impl GatewayServices {
    pub(super) fn execute_after_approval(
        &self,
        context: &ToolExecutionContext<'_>,
        command: &str,
    ) -> Result<TerminalCommandResult, String> {
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
    ) -> Result<TerminalCommandResult, String>
    where
        F: FnOnce(&str) -> Result<TerminalCommandResult, String>,
    {
        // 状态机主体在 ApprovalFlow 中实现；这里只注入真正的事件发布闭包。
        ApprovalFlow {
            audit: &self.audit,
            gateway: context.gateway,
            connection: &self.connection,
            local_registry: &self.local_registry,
            token: context.token,
            binding: &context.access.binding,
            audit_tool: context.audit_tool,
            deadline: context.deadline,
        }
        .run(
            command,
            |event| self.app.emit("nocterm://ai-tool-approval", event).is_ok(),
            // UI 只接受同一 session/approval 的关闭事件；事件丢失时前端仍有
            // expiresAt 本地兜底，这里尽力发送即可。
            |approval_id, resolution| {
                let _ = self.app.emit(
                    "nocterm://ai-tool-approval-closed",
                    AiToolApprovalClosedEvent {
                        approval_id: approval_id.to_string(),
                        session_id: context.access.binding.session_id.clone(),
                        resolution: resolution.to_string(),
                    },
                );
            },
            on_approved,
        )
    }

    /// 策略允许跳过确认时仍使用同一审计和执行核心。
    pub(super) fn execute_command_without_approval(
        &self,
        access: &AiGatewayAccess,
        audit_tool: AiAuditTool,
        command: &str,
        call_deadline: Instant,
    ) -> Result<TerminalCommandResult, String> {
        self.terminal_execution()
            .without_approval(access, audit_tool, command, call_deadline)
    }

    fn execute_command(
        &self,
        access: &AiGatewayAccess,
        audit_tool: AiAuditTool,
        command: &str,
        approval: AiAuditApproval,
        call_deadline: Instant,
    ) -> Result<TerminalCommandResult, String> {
        self.terminal_execution()
            .run(access, audit_tool, command, approval, call_deadline)
    }

    fn terminal_execution(&self) -> TerminalExecution<'_> {
        TerminalExecution {
            audit: &self.audit,
            connection: &self.connection,
            terminal: &self.terminal,
            local_terminal: &self.local_terminal,
            local_registry: &self.local_registry,
        }
    }
}

/// 一次逐次审批流程的全部协作依赖。
///
/// UI 事件发布通过闭包注入而非直接持有 `AppHandle`：批准/拒绝/超时/撤销的
/// 状态转移与“先审计后展示”的时序是安全契约，必须能在无 Tauri 运行时的
/// 单元测试中完整验证；生产路径由 `GatewayServices` 提供真正的发布闭包。
struct ApprovalFlow<'a> {
    audit: &'a AiAuditService,
    gateway: &'a AiGatewayState,
    connection: &'a ConnectionService,
    local_registry: &'a LocalTerminalRegistry,
    token: &'a str,
    binding: &'a AiGatewayBinding,
    audit_tool: AiAuditTool,
    /// 整次工具调用的绝对截止时间；审批等待不得超过它。
    deadline: Instant,
}

impl ApprovalFlow<'_> {
    /// 执行审批状态机。
    ///
    /// `emit_request` 返回 false 表示界面不可达，等待请求会立即以拒绝收尾；
    /// `emit_closed` 携带 (approval_id, resolution) 通知 UI 关闭确认卡片。
    fn run(
        self,
        command: &str,
        emit_request: impl FnOnce(&AiToolApprovalEvent) -> bool,
        emit_closed: impl Fn(&str, &str),
        on_approved: impl FnOnce(&str) -> Result<TerminalCommandResult, String>,
    ) -> Result<TerminalCommandResult, String> {
        let binding = self.binding;
        let audit_tool = self.audit_tool;
        let command = command.trim();
        if command.is_empty() || command.len() > 4096 || command.contains('\0') {
            return Err(audited_denial(
                self.audit,
                binding,
                audit_tool,
                AiAuditApproval::NotRequested,
                AiAuditOutcome::Failed,
                "AI_TOOL_ARGUMENTS_INVALID",
                "终端命令不能为空、不能包含空字符且不能超过 4096 个字符",
            ));
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
                    return Err(audited_denial(
                        self.audit,
                        binding,
                        audit_tool,
                        AiAuditApproval::NotRequested,
                        AiAuditOutcome::Failed,
                        "AI_TOOL_TARGET_UNAVAILABLE",
                        &error.message,
                    ));
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
                    return Err(audited_denial(
                        self.audit,
                        binding,
                        audit_tool,
                        AiAuditApproval::NotRequested,
                        AiAuditOutcome::Failed,
                        "AI_TOOL_TARGET_UNAVAILABLE",
                        "目标本地终端不存在或已重新连接",
                    ));
                }
                ("local".to_string(), session_id.clone())
            }
        };
        // 审批事件先持久化，再向 UI 展示命令；审计不可用时不会产生可批准的请求。
        append_audit(
            self.audit,
            binding,
            audit_tool,
            AiAuditApproval::Requested,
            AiAuditOutcome::Pending,
            None,
            None,
        )?;
        let receiver = match self
            .gateway
            .request_approval(self.token.to_string(), approval_id.clone())
        {
            Ok(receiver) => receiver,
            Err(error) => {
                return Err(audited_denial(
                    self.audit,
                    binding,
                    audit_tool,
                    AiAuditApproval::Requested,
                    AiAuditOutcome::Failed,
                    "AI_TOOL_TASK_REVOKED",
                    &error,
                ));
            }
        };
        let approval_deadline = (Instant::now() + AI_APPROVAL_TIMEOUT).min(self.deadline);
        let approval_wait = approval_deadline.saturating_duration_since(Instant::now());
        let expires_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .saturating_add(approval_wait.as_millis())
            .min(u128::from(u64::MAX)) as u64;
        let event = AiToolApprovalEvent {
            approval_id: approval_id.clone(),
            session_id: binding.session_id.clone(),
            target_kind,
            target_label,
            command: command.to_string(),
            expires_at_unix_ms,
        };
        // 界面不可达时立即以拒绝收尾，不能留下可被迟到的批准解锁的请求。
        if !emit_request(&event) {
            let _ = self
                .gateway
                .resolve_approval(&approval_id, &binding.session_id, false);
            return Err(audited_denial(
                self.audit,
                binding,
                audit_tool,
                AiAuditApproval::Requested,
                AiAuditOutcome::Failed,
                "AI_TOOL_APPROVAL_UI_UNAVAILABLE",
                "无法向 Nocterm 界面发送命令确认请求",
            ));
        }
        match receiver.recv_timeout(approval_wait) {
            Ok(AiApprovalDecision::Approved) => {
                emit_closed(&approval_id, "approved");
                on_approved(command)
            }
            Ok(AiApprovalDecision::Rejected) => {
                emit_closed(&approval_id, "rejected");
                Err(audited_denial(
                    self.audit,
                    binding,
                    audit_tool,
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_APPROVAL_REJECTED",
                    "用户拒绝执行该终端命令",
                ))
            }
            Ok(AiApprovalDecision::Revoked) => {
                emit_closed(&approval_id, "revoked");
                Err(audited_denial(
                    self.audit,
                    binding,
                    audit_tool,
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_TASK_REVOKED",
                    "AI 任务已停止，终端命令未执行",
                ))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let _ = self
                    .gateway
                    .resolve_approval(&approval_id, &binding.session_id, false);
                emit_closed(&approval_id, "timed_out");
                Err(audited_denial(
                    self.audit,
                    binding,
                    audit_tool,
                    AiAuditApproval::TimedOut,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_APPROVAL_TIMEOUT",
                    "终端命令确认已超时",
                ))
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                emit_closed(&approval_id, "revoked");
                Err(audited_denial(
                    self.audit,
                    binding,
                    audit_tool,
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    "AI_TOOL_TASK_REVOKED",
                    "AI 任务已停止，终端命令未执行",
                ))
            }
        }
    }
}

/// 终端命令执行与完成审计的协作依赖。
///
/// 不含任何 UI 依赖：用户批准与策略放行走同一套“pending 审计 → 执行 →
/// 结果审计”核心，该时序同样要求在无窗口环境下可测。
struct TerminalExecution<'a> {
    audit: &'a AiAuditService,
    connection: &'a ConnectionService,
    terminal: &'a TerminalService,
    local_terminal: &'a LocalTerminalService,
    local_registry: &'a LocalTerminalRegistry,
}

impl TerminalExecution<'_> {
    fn without_approval(
        &self,
        access: &AiGatewayAccess,
        audit_tool: AiAuditTool,
        command: &str,
        call_deadline: Instant,
    ) -> Result<TerminalCommandResult, String> {
        let command = match crate::commands::ai_policy::validate_approved_command(command) {
            Ok(command) => command,
            Err(error) => {
                return Err(audited_denial(
                    self.audit,
                    &access.binding,
                    audit_tool,
                    AiAuditApproval::NotRequested,
                    AiAuditOutcome::Failed,
                    "AI_TOOL_ARGUMENTS_INVALID",
                    &error,
                ));
            }
        };
        self.run(
            access,
            audit_tool,
            &command,
            AiAuditApproval::NotRequired,
            call_deadline,
        )
    }

    fn run(
        &self,
        access: &AiGatewayAccess,
        audit_tool: AiAuditTool,
        command: &str,
        approval: AiAuditApproval,
        call_deadline: Instant,
    ) -> Result<TerminalCommandResult, String> {
        let binding = &access.binding;
        let execution_deadline = (Instant::now() + AI_EXECUTION_TIMEOUT).min(call_deadline);
        // 无论是用户批准还是策略自动放行，命令都必须先写入 pending 审计再执行。
        let started_at = Instant::now();
        let result =
            execute_after_audit(
                self.audit,
                binding,
                audit_tool,
                approval,
                || match &binding.target {
                    AiTarget::Ssh { connection_id } => execute_ssh_sync(
                        self.connection,
                        self.terminal,
                        *connection_id,
                        command,
                        &access.cancellation,
                        execution_deadline,
                    ),
                    AiTarget::Local {
                        session_id,
                        terminal_id,
                    } => execute_local_sync(
                        self.local_terminal,
                        self.local_registry,
                        session_id,
                        terminal_id,
                        command,
                        &access.cancellation,
                        execution_deadline,
                    ),
                },
            )?;
        let succeeded = result.as_ref().is_ok_and(|value| value.exit_code == 0);
        let (outcome, error_code) = if succeeded {
            (AiAuditOutcome::Succeeded, None)
        } else {
            (AiAuditOutcome::Failed, Some("AI_TOOL_EXECUTION_FAILED"))
        };
        if append_audit(
            self.audit,
            binding,
            audit_tool,
            approval,
            outcome,
            Some(elapsed_ms(started_at)),
            error_code,
        )
        .is_err()
        {
            return Err("终端命令可能已执行，但完成审计写入失败，请勿自动重试".to_string());
        }
        result
    }
}

/// 审批失败只有在对应脱敏事件成功落库后才返回原始业务错误。
fn audited_denial(
    audit: &AiAuditService,
    binding: &AiGatewayBinding,
    tool: AiAuditTool,
    approval: AiAuditApproval,
    outcome: AiAuditOutcome,
    error_code: &'static str,
    message: &str,
) -> String {
    match append_audit(
        audit,
        binding,
        tool,
        approval,
        outcome,
        None,
        Some(error_code),
    ) {
        Ok(()) => message.to_string(),
        Err(error) => error,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        sync::{Arc, Mutex, atomic::AtomicBool},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use nocterm_application::{
        ai_audit::AiAuditService,
        connection::ConnectionService,
        terminal::{LocalTerminalService, TerminalService},
    };
    use nocterm_domain::ai_audit::{
        AiAuditApproval, AiAuditOutcome, AiAuditRepository, AiAuditRepositoryError, AiAuditTarget,
        AiToolAuditEvent,
    };
    use nocterm_domain::connection::{
        AuthenticationMethod, ConnectionGroup, ConnectionImportProfile, ConnectionImportResult,
        ConnectionProfile, ConnectionRepository, ConnectionRepositoryError, NewConnectionGroup,
        NewConnectionProfile,
    };
    use nocterm_domain::terminal::{LocalTerminalPort, OpenedTerminal, SshTerminalPort};

    use crate::state::{
        AiCommandPolicy, AiGatewayAccess, AiGatewayBinding, AiGatewayState, AiTarget,
        LocalTerminalRegistry,
    };

    use super::{ApprovalFlow, TerminalExecution};
    use crate::commands::ai_terminal::TerminalCommandResult;

    /// 记录全部审计事件；写入数达到 `fail_after` 后开始失败，用于验证审计失败路径。
    struct RecordingAudit {
        state: Mutex<(Vec<AiToolAuditEvent>, usize)>,
    }

    impl RecordingAudit {
        fn never_failing() -> Self {
            Self {
                state: Mutex::new((Vec::new(), usize::MAX)),
            }
        }

        fn failing_after(count: usize) -> Self {
            Self {
                state: Mutex::new((Vec::new(), count)),
            }
        }

        fn events(&self) -> Vec<AiToolAuditEvent> {
            self.state.lock().unwrap().0.clone()
        }
    }

    impl AiAuditRepository for RecordingAudit {
        fn append(&self, event: &AiToolAuditEvent) -> Result<(), AiAuditRepositoryError> {
            let mut state = self.state.lock().unwrap();
            if state.0.len() >= state.1 {
                return Err(AiAuditRepositoryError::new("audit unavailable"));
            }
            state.0.push(event.clone());
            Ok(())
        }
    }

    /// 只回答 `get` 的连接仓库桩；其余方法不在被测路径上，统一返回错误。
    struct StubConnectionRepository {
        profile: Option<ConnectionProfile>,
    }

    fn unused() -> ConnectionRepositoryError {
        ConnectionRepositoryError::new("测试桩未实现该方法")
    }

    impl ConnectionRepository for StubConnectionRepository {
        fn get(&self, _id: i64) -> Result<Option<ConnectionProfile>, ConnectionRepositoryError> {
            match &self.profile {
                Some(profile) => Ok(Some(profile.clone())),
                None => Err(unused()),
            }
        }

        fn list(&self) -> Result<Vec<ConnectionProfile>, ConnectionRepositoryError> {
            Err(unused())
        }

        fn create(
            &self,
            _profile: NewConnectionProfile,
        ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
            Err(unused())
        }

        fn update(
            &self,
            _id: i64,
            _profile: NewConnectionProfile,
        ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
            Err(unused())
        }

        fn delete(&self, _id: i64) -> Result<(), ConnectionRepositoryError> {
            Err(unused())
        }

        fn list_groups(&self) -> Result<Vec<ConnectionGroup>, ConnectionRepositoryError> {
            Err(unused())
        }

        fn upsert_group(
            &self,
            _group: NewConnectionGroup,
        ) -> Result<ConnectionGroup, ConnectionRepositoryError> {
            Err(unused())
        }

        fn delete_group(&self, _id: &str) -> Result<(), ConnectionRepositoryError> {
            Err(unused())
        }

        fn update_sort_order(
            &self,
            _id: i64,
            _group_id: Option<&str>,
            _sort_order: f64,
        ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
            Err(unused())
        }

        fn upsert_credential_binding(
            &self,
            _connection_id: &str,
            _credential_kind: &str,
            _credential_status: &str,
        ) -> Result<(), ConnectionRepositoryError> {
            Err(unused())
        }

        fn delete_credential_binding(
            &self,
            _connection_id: &str,
        ) -> Result<(), ConnectionRepositoryError> {
            Err(unused())
        }

        fn import_backup(
            &self,
            _groups: Vec<NewConnectionGroup>,
            _profiles: Vec<ConnectionImportProfile>,
        ) -> Result<ConnectionImportResult, ConnectionRepositoryError> {
            Err(unused())
        }
    }

    /// SSH 端口桩：被测路径在到达端口前就会失败；`exec_existing` 用默认实现即可。
    struct UnavailableSshPort;

    impl SshTerminalPort for UnavailableSshPort {
        fn open(
            &self,
            _profile: &ConnectionProfile,
            _cols: u16,
            _rows: u16,
            _password: Option<&str>,
            _private_key: Option<&str>,
        ) -> Result<OpenedTerminal, String> {
            Err("测试桩不可用".to_string())
        }

        fn write(&self, _terminal_id: &str, _data: &str) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }

        fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }

        fn close(&self, _terminal_id: &str) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }

        fn close_connection(&self, _connection_id: i64) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }
    }

    /// 本地终端端口桩：审批状态机测试不会执行本地 PTY。
    struct UnavailableLocalPort;

    impl LocalTerminalPort for UnavailableLocalPort {
        fn open(&self, _cols: u16, _rows: u16) -> Result<OpenedTerminal, String> {
            Err("测试桩不可用".to_string())
        }

        fn write(&self, _terminal_id: &str, _data: &str) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }

        fn completion_command(&self, _terminal_id: &str, _marker: &str) -> Result<String, String> {
            Err("测试桩不可用".to_string())
        }

        fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }

        fn close(&self, _terminal_id: &str) -> Result<(), String> {
            Err("测试桩不可用".to_string())
        }
    }

    /// 装配被测核心的全部协作者：审计仓库、网关状态、服务桩与本地注册表。
    struct Harness {
        audit: Arc<RecordingAudit>,
        audit_service: AiAuditService,
        gateway: AiGatewayState,
        connection: ConnectionService,
        terminal: TerminalService,
        local_terminal: LocalTerminalService,
        local_registry: Arc<LocalTerminalRegistry>,
    }

    impl Harness {
        fn new(profile: Option<ConnectionProfile>) -> Self {
            Self::with_audit(profile, Arc::new(RecordingAudit::never_failing()))
        }

        fn with_audit(profile: Option<ConnectionProfile>, audit: Arc<RecordingAudit>) -> Self {
            Self {
                // 服务与测试夹具共享同一份审计记录，便于断言完整事件序列。
                audit_service: AiAuditService::new(audit.clone()),
                gateway: AiGatewayState::default(),
                connection: ConnectionService::new(Arc::new(StubConnectionRepository { profile })),
                terminal: TerminalService::new(Arc::new(UnavailableSshPort)),
                local_terminal: LocalTerminalService::new(Arc::new(UnavailableLocalPort)),
                local_registry: Arc::new(LocalTerminalRegistry::default()),
                audit,
            }
        }

        fn flow<'a>(
            &'a self,
            token: &'a str,
            binding: &'a AiGatewayBinding,
            deadline: Instant,
        ) -> ApprovalFlow<'a> {
            ApprovalFlow {
                audit: &self.audit_service,
                gateway: &self.gateway,
                connection: &self.connection,
                local_registry: &self.local_registry,
                token,
                binding,
                audit_tool: nocterm_domain::ai_audit::AiAuditTool::SshExec,
                deadline,
            }
        }

        fn execution(&self) -> TerminalExecution<'_> {
            TerminalExecution {
                audit: &self.audit_service,
                connection: &self.connection,
                terminal: &self.terminal,
                local_terminal: &self.local_terminal,
                local_registry: &self.local_registry,
            }
        }

        fn events(&self) -> Vec<AiToolAuditEvent> {
            self.audit.events()
        }
    }

    fn ssh_profile() -> ConnectionProfile {
        ConnectionProfile {
            id: 7,
            name: "生产机组".to_string(),
            host: "10.0.0.9".to_string(),
            port: 2222,
            username: "ops".to_string(),
            authentication: AuthenticationMethod::Password,
            created_at: 0,
            updated_at: 0,
            group_id: None,
            group_name: None,
            remark: None,
            sync_mode: "manual".to_string(),
            execution_target: "ssh".to_string(),
            remote_initial_path: None,
            icon: None,
            sort_order: None,
            private_key_path: None,
            credential_kind: None,
            credential_status: "none".to_string(),
        }
    }

    fn ssh_binding() -> AiGatewayBinding {
        AiGatewayBinding {
            target: AiTarget::Ssh { connection_id: 7 },
            provider: "codex".to_string(),
            session_id: "ai-test".to_string(),
            command_policy: AiCommandPolicy::ConfirmEach,
        }
    }

    fn local_binding() -> AiGatewayBinding {
        AiGatewayBinding {
            target: AiTarget::Local {
                session_id: "local:one".to_string(),
                terminal_id: "local-1".to_string(),
            },
            provider: "codex".to_string(),
            session_id: "ai-test".to_string(),
            command_policy: AiCommandPolicy::ConfirmEach,
        }
    }

    fn bind_active(gateway: &AiGatewayState, token: &str, binding: &AiGatewayBinding) {
        gateway.bind(token.to_string(), binding.clone()).unwrap();
        gateway
            .activate_session(token, binding.session_id.clone(), binding.command_policy)
            .unwrap();
    }

    /// 只比较安全契约相关字段：审批状态、结果与错误码。
    fn assert_audit(
        events: &[AiToolAuditEvent],
        expected: &[(AiAuditApproval, AiAuditOutcome, Option<&'static str>)],
    ) {
        assert_eq!(events.len(), expected.len(), "审计序列不符：{events:?}");
        for (event, (approval, outcome, error_code)) in events.iter().zip(expected) {
            assert_eq!(event.approval, *approval);
            assert_eq!(event.outcome, *outcome);
            assert_eq!(event.error_code, *error_code);
        }
    }

    fn never_emits() -> impl FnOnce(&crate::dto::ai::AiToolApprovalEvent) -> bool {
        |_| panic!("该路径不应向 UI 发布审批请求")
    }

    fn never_closes() -> impl Fn(&str, &str) {
        |_, _| panic!("该路径不应发布关闭事件")
    }

    fn never_executes() -> impl FnOnce(&str) -> Result<TerminalCommandResult, String> {
        |command: &str| panic!("该路径不应执行命令：{command}")
    }

    #[test]
    fn approval_flow_executes_the_trimmed_command_once_after_user_approval() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let executed: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let closed: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let approval_id = RefCell::new(String::new());
        let expires_at = RefCell::new(None);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let result = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(30),
            )
            .run(
                "  uptime ",
                |event| {
                    assert!(event.approval_id.starts_with("ai-approval-"));
                    assert_eq!(event.session_id, "ai-test");
                    assert_eq!(event.target_kind, "ssh");
                    assert_eq!(event.target_label, "生产机组（ops@10.0.0.9:2222）");
                    // 展示给用户的命令必须是已修剪的最终形态。
                    assert_eq!(event.command, "uptime");
                    *approval_id.borrow_mut() = event.approval_id.clone();
                    *expires_at.borrow_mut() = Some(event.expires_at_unix_ms);
                    // 等待者已注册：事件发布阶段同步批准，决议在等待开始前排入通道。
                    assert!(
                        harness
                            .gateway
                            .resolve_approval(&event.approval_id, "ai-test", true)
                            .unwrap()
                    );
                    true
                },
                |id, resolution| closed.lock().unwrap().push(format!("{id}:{resolution}")),
                |command| {
                    executed.lock().unwrap().push(command.to_string());
                    Ok(TerminalCommandResult {
                        output: "load average: 0.02".to_string(),
                        exit_code: 0,
                    })
                },
            )
            .unwrap();

        assert_eq!(result.exit_code, 0);
        assert_eq!(*executed.lock().unwrap(), ["uptime"]);
        // 关闭决议恰好一条，且与审批请求一一对应。
        let closed = closed.into_inner().unwrap();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].ends_with(":approved"));
        // expiresAt 受调用截止时间钳制，UI 倒计时不得超过真实等待窗口。
        let expires_at = expires_at.into_inner().expect("expires captured");
        assert!(expires_at >= now_ms);
        assert!(
            expires_at <= now_ms + 32_000,
            "expiresAt 未被调用截止时间钳制"
        );
        // 注入执行回调的路径只产生“请求”审计；执行审计属于 TerminalExecution。
        assert_audit(
            &harness.events(),
            &[(AiAuditApproval::Requested, AiAuditOutcome::Pending, None)],
        );
        // 同一审批只能被决议一次；迟到的重复批准不会解锁第二次执行。
        assert!(
            !harness
                .gateway
                .resolve_approval(&approval_id.into_inner(), "ai-test", true)
                .unwrap()
        );
    }

    #[test]
    fn rejection_denies_execution_and_audits_the_denial() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let executed: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let closed: Mutex<Vec<String>> = Mutex::new(Vec::new());

        let error = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(30),
            )
            .run(
                "uptime",
                |event| {
                    harness
                        .gateway
                        .resolve_approval(&event.approval_id, "ai-test", false)
                        .unwrap();
                    true
                },
                |id, resolution| closed.lock().unwrap().push(format!("{id}:{resolution}")),
                |command| {
                    executed.lock().unwrap().push(command.to_string());
                    Ok(TerminalCommandResult {
                        output: String::new(),
                        exit_code: 0,
                    })
                },
            )
            .unwrap_err();

        assert_eq!(error, "用户拒绝执行该终端命令");
        assert!(executed.into_inner().unwrap().is_empty());
        let closed = closed.into_inner().unwrap();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].ends_with(":rejected"));
        assert_audit(
            &harness.events(),
            &[
                (AiAuditApproval::Requested, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    Some("AI_TOOL_APPROVAL_REJECTED"),
                ),
            ],
        );
    }

    #[test]
    fn stopping_the_task_revokes_a_pending_approval() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let closed: Mutex<Vec<String>> = Mutex::new(Vec::new());

        let error = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(30),
            )
            .run(
                "uptime",
                |event| {
                    // 停止任务即撤销整个 turn 的授权，包括等待中的审批。
                    assert!(harness.gateway.deactivate_session("ai-test").unwrap());
                    let _ = event;
                    true
                },
                |id, resolution| closed.lock().unwrap().push(format!("{id}:{resolution}")),
                never_executes(),
            )
            .unwrap_err();

        assert_eq!(error, "AI 任务已停止，终端命令未执行");
        let closed = closed.into_inner().unwrap();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].ends_with(":revoked"));
        assert_audit(
            &harness.events(),
            &[
                (AiAuditApproval::Requested, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::Rejected,
                    AiAuditOutcome::Denied,
                    Some("AI_TOOL_TASK_REVOKED"),
                ),
            ],
        );
    }

    #[test]
    fn an_expired_call_deadline_times_out_and_clears_the_waiter() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let approval_id = RefCell::new(String::new());
        let closed: Mutex<Vec<String>> = Mutex::new(Vec::new());

        // 调用截止时间已过：审批等待窗口为零，无需真实等待 60 秒即可触发超时。
        let error = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() - Duration::from_secs(1),
            )
            .run(
                "uptime",
                |event| {
                    *approval_id.borrow_mut() = event.approval_id.clone();
                    true
                },
                |id, resolution| closed.lock().unwrap().push(format!("{id}:{resolution}")),
                never_executes(),
            )
            .unwrap_err();

        assert_eq!(error, "终端命令确认已超时");
        let closed = closed.into_inner().unwrap();
        assert_eq!(closed.len(), 1);
        assert!(closed[0].ends_with(":timed_out"));
        assert_audit(
            &harness.events(),
            &[
                (AiAuditApproval::Requested, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::TimedOut,
                    AiAuditOutcome::Denied,
                    Some("AI_TOOL_APPROVAL_TIMEOUT"),
                ),
            ],
        );
        // 超时路径必须移除等待者：迟到的批准不能再解锁执行。
        assert!(
            !harness
                .gateway
                .resolve_approval(&approval_id.into_inner(), "ai-test", true)
                .unwrap()
        );
    }

    #[test]
    fn an_unreachable_ui_fails_the_request_without_leaving_a_waiter() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let approval_id = RefCell::new(String::new());
        let closed: Mutex<Vec<String>> = Mutex::new(Vec::new());

        let error = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(30),
            )
            .run(
                "uptime",
                |event| {
                    *approval_id.borrow_mut() = event.approval_id.clone();
                    false
                },
                |id, resolution| closed.lock().unwrap().push(format!("{id}:{resolution}")),
                never_executes(),
            )
            .unwrap_err();

        assert_eq!(error, "无法向 Nocterm 界面发送命令确认请求");
        // UI 不可达属于失败而非等待：不发布关闭事件，也不留下等待者。
        assert!(closed.into_inner().unwrap().is_empty());
        assert!(
            !harness
                .gateway
                .resolve_approval(&approval_id.into_inner(), "ai-test", true)
                .unwrap()
        );
        assert_audit(
            &harness.events(),
            &[
                (AiAuditApproval::Requested, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::Requested,
                    AiAuditOutcome::Failed,
                    Some("AI_TOOL_APPROVAL_UI_UNAVAILABLE"),
                ),
            ],
        );
    }

    #[test]
    fn invalid_command_arguments_never_request_approval() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let invalid: Vec<String> = vec![
            "   ".to_string(),
            "echo \0 boom".to_string(),
            "x".repeat(4097),
        ];

        for command in &invalid {
            let error = harness
                .flow(
                    "task-one",
                    &binding,
                    Instant::now() + Duration::from_secs(5),
                )
                .run(command, never_emits(), never_closes(), never_executes())
                .unwrap_err();
            assert_eq!(
                error,
                "终端命令不能为空、不能包含空字符且不能超过 4096 个字符"
            );
        }
        assert_audit(
            &harness.events(),
            &[(
                AiAuditApproval::NotRequested,
                AiAuditOutcome::Failed,
                Some("AI_TOOL_ARGUMENTS_INVALID"),
            ); 3],
        );

        // 参数校验失败不占用审批通道：同任务仍可立即发起下一条审批。
        let result = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(5),
            )
            .run(
                "uptime",
                |event| {
                    harness
                        .gateway
                        .resolve_approval(&event.approval_id, "ai-test", true)
                        .unwrap();
                    true
                },
                |_, _| {},
                |_| {
                    Ok(TerminalCommandResult {
                        output: String::new(),
                        exit_code: 0,
                    })
                },
            )
            .unwrap();
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn unavailable_targets_fail_before_an_approval_is_requested() {
        // SSH 目标：连接资料读取失败。
        let harness = Harness::new(None);
        let binding = ssh_binding();
        bind_active(&harness.gateway, "task-one", &binding);
        let error = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(5),
            )
            .run("uptime", never_emits(), never_closes(), never_executes())
            .unwrap_err();
        assert_eq!(error, "读取连接资料失败");
        assert_audit(
            &harness.events(),
            &[(
                AiAuditApproval::NotRequested,
                AiAuditOutcome::Failed,
                Some("AI_TOOL_TARGET_UNAVAILABLE"),
            )],
        );

        // 本地目标：会话不存在或已重连。
        let harness = Harness::new(Some(ssh_profile()));
        let binding = local_binding();
        bind_active(&harness.gateway, "task-two", &binding);
        let error = harness
            .flow(
                "task-two",
                &binding,
                Instant::now() + Duration::from_secs(5),
            )
            .run("uptime", never_emits(), never_closes(), never_executes())
            .unwrap_err();
        assert_eq!(error, "目标本地终端不存在或已重新连接");
        assert_audit(
            &harness.events(),
            &[(
                AiAuditApproval::NotRequested,
                AiAuditOutcome::Failed,
                Some("AI_TOOL_TARGET_UNAVAILABLE"),
            )],
        );
    }

    #[test]
    fn a_bound_local_target_reports_the_local_session_label() {
        let harness = Harness::new(Some(ssh_profile()));
        harness
            .local_registry
            .bind("local:one".to_string(), "local-1".to_string());
        let binding = local_binding();
        bind_active(&harness.gateway, "task-one", &binding);

        let result = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(5),
            )
            .run(
                "uptime",
                |event| {
                    assert_eq!(event.target_kind, "local");
                    assert_eq!(event.target_label, "local:one");
                    harness
                        .gateway
                        .resolve_approval(&event.approval_id, "ai-test", true)
                        .unwrap();
                    true
                },
                |_, _| {},
                |_| {
                    Ok(TerminalCommandResult {
                        output: String::new(),
                        exit_code: 0,
                    })
                },
            )
            .unwrap();

        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn an_ended_task_token_fails_the_approval_request() {
        let harness = Harness::new(Some(ssh_profile()));
        let binding = ssh_binding();
        // 只绑定未激活：等价于任务已结束的 token。
        harness
            .gateway
            .bind("task-one".to_string(), binding.clone())
            .unwrap();

        let error = harness
            .flow(
                "task-one",
                &binding,
                Instant::now() + Duration::from_secs(5),
            )
            .run("uptime", never_emits(), never_closes(), never_executes())
            .unwrap_err();

        assert_eq!(error, "AI Bridge 任务已结束，请重新发起请求");
        assert_audit(
            &harness.events(),
            &[
                (AiAuditApproval::Requested, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::Requested,
                    AiAuditOutcome::Failed,
                    Some("AI_TOOL_TASK_REVOKED"),
                ),
            ],
        );
    }

    #[test]
    fn ssh_execution_failure_keeps_pending_then_failed_audit_order() {
        let harness = Harness::new(None);
        let binding = ssh_binding();
        let access = AiGatewayAccess {
            binding: binding.clone(),
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let error = harness
            .execution()
            .run(
                &access,
                nocterm_domain::ai_audit::AiAuditTool::SshExec,
                "uptime",
                AiAuditApproval::Approved,
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap_err();

        assert_eq!(error, "读取连接资料失败");
        let events = harness.events();
        assert_audit(
            &events,
            &[
                (AiAuditApproval::Approved, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::Approved,
                    AiAuditOutcome::Failed,
                    Some("AI_TOOL_EXECUTION_FAILED"),
                ),
            ],
        );
        // 执行时长只出现在完成审计上。
        assert_eq!(events[0].duration_ms, None);
        assert!(events[1].duration_ms.is_some());
        assert_eq!(events[0].target, AiAuditTarget::Ssh { connection_id: 7 });
        // 审计不携带命令内容。
        assert!(!format!("{events:?}").contains("uptime"));
    }

    #[test]
    fn an_unwritable_final_audit_reports_an_unreliable_result() {
        let harness = Harness::with_audit(None, Arc::new(RecordingAudit::failing_after(1)));
        let binding = ssh_binding();
        let access = AiGatewayAccess {
            binding: binding.clone(),
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        let error = harness
            .execution()
            .run(
                &access,
                nocterm_domain::ai_audit::AiAuditTool::SshExec,
                "uptime",
                AiAuditApproval::Approved,
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap_err();

        // pending 审计已写入、命令已失败，但完成审计缺失：必须禁止调用方自动重试。
        assert_eq!(
            error,
            "终端命令可能已执行，但完成审计写入失败，请勿自动重试"
        );
        assert_audit(
            &harness.events(),
            &[(AiAuditApproval::Approved, AiAuditOutcome::Pending, None)],
        );
    }

    #[test]
    fn policy_shortcut_execution_validates_arguments_and_audits_not_required() {
        let harness = Harness::new(None);
        let binding = ssh_binding();
        let access = AiGatewayAccess {
            binding: binding.clone(),
            cancellation: Arc::new(AtomicBool::new(false)),
        };

        // 策略放行不豁免参数校验。
        let error = harness
            .execution()
            .without_approval(
                &access,
                nocterm_domain::ai_audit::AiAuditTool::SshExec,
                "echo \0 boom",
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap_err();
        assert_eq!(error, "终端命令包含不允许的空字符");
        assert_audit(
            &harness.events(),
            &[(
                AiAuditApproval::NotRequested,
                AiAuditOutcome::Failed,
                Some("AI_TOOL_ARGUMENTS_INVALID"),
            )],
        );

        // 合法命令免确认执行，但仍写入 NotRequired 的完整审计对。
        let error = harness
            .execution()
            .without_approval(
                &access,
                nocterm_domain::ai_audit::AiAuditTool::SshExec,
                "uptime",
                Instant::now() + Duration::from_secs(5),
            )
            .unwrap_err();
        assert_eq!(error, "读取连接资料失败");
        assert_audit(
            &harness.events(),
            &[
                (
                    AiAuditApproval::NotRequested,
                    AiAuditOutcome::Failed,
                    Some("AI_TOOL_ARGUMENTS_INVALID"),
                ),
                (AiAuditApproval::NotRequired, AiAuditOutcome::Pending, None),
                (
                    AiAuditApproval::NotRequired,
                    AiAuditOutcome::Failed,
                    Some("AI_TOOL_EXECUTION_FAILED"),
                ),
            ],
        );
    }
}
