use std::{error::Error, fmt};

use crate::ai_audit::AiAuditTool;

/// 执行阶段显式覆盖外部副作用边界；终态不会被原 execution 重新打开。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiExecutionPhase {
    Requested,
    AwaitingApproval,
    Authorized,
    TerminalReserved,
    Dispatching,
    Dispatched,
    Completed,
    Failed,
    Denied,
    Cancelled,
    TimedOut,
    ResultUnknown,
}

impl AiExecutionPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Authorized => "authorized",
            Self::TerminalReserved => "terminal_reserved",
            Self::Dispatching => "dispatching",
            Self::Dispatched => "dispatched",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::ResultUnknown => "result_unknown",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed
                | Self::Failed
                | Self::Denied
                | Self::Cancelled
                | Self::TimedOut
                | Self::ResultUnknown
        )
    }

    /// 启动恢复只处理可能遗留运行期资源或待决审批的非终态记录。
    pub const fn needs_startup_recovery(self) -> bool {
        !self.is_terminal()
    }
}

/// 审批状态与执行阶段分离，保留自动授权、人工授权和撤销的事实差异。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiExecutionApproval {
    NotRequested,
    NotRequired,
    Pending,
    Approved,
    Rejected,
    TimedOut,
    Revoked,
}

impl AiExecutionApproval {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::NotRequired => "not_required",
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::TimedOut => "timed_out",
            Self::Revoked => "revoked",
        }
    }
}

/// 模式只记录执行语义，不包含命令、Shell 参数或环境变量。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiExecutionMode {
    Unselected,
    LocalVisibleOsc,
    LocalVisibleMarker,
    SshVisibleOsc,
    SshVisibleMarker,
    SshIndependentExec,
}

impl AiExecutionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unselected => "unselected",
            Self::LocalVisibleOsc => "local_visible_osc",
            Self::LocalVisibleMarker => "local_visible_marker",
            Self::SshVisibleOsc => "ssh_visible_osc",
            Self::SshVisibleMarker => "ssh_visible_marker",
            Self::SshIndependentExec => "ssh_independent_exec",
        }
    }

    pub const fn is_selected(self) -> bool {
        !matches!(self, Self::Unselected)
    }
}

/// 结果是面向投影的封闭分类；详细诊断只允许稳定错误码和数值元数据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiExecutionResult {
    Pending,
    Succeeded,
    Failed,
    Denied,
    Cancelled,
    TimedOut,
    Unknown,
}

impl AiExecutionResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Denied => "denied",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Unknown => "unknown",
        }
    }
}

/// Dispatch receipt 只证明本地持久化意图或 Adapter 已确认发送，不承诺外部 exactly-once。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AiDispatchReceipt {
    NotStarted,
    IntentPersisted,
    Confirmed,
}

impl AiDispatchReceipt {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::IntentPersisted => "intent_persisted",
            Self::Confirmed => "confirmed",
        }
    }
}

/// 目标只表达 Phase 1A 已存在的绑定事实，不保存主机、账号、目录或凭据。
/// SSH 的精确 terminal/generation 绑定属于后续控制面阶段，当前不得伪造占位值。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AiExecutionTarget {
    Local {
        workspace_session_id: String,
        terminal_id: String,
    },
    Ssh {
        workspace_session_id: String,
        terminal_id: String,
        terminal_generation: u64,
        connection_id: i64,
    },
}

impl AiExecutionTarget {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Local { .. } => "local",
            Self::Ssh { .. } => "ssh",
        }
    }

    fn validate(&self) -> Result<(), AiExecutionValidationError> {
        match self {
            Self::Local {
                workspace_session_id,
                terminal_id,
            } => {
                require_identifier("workspace_session_id", workspace_session_id)?;
                require_identifier("terminal_id", terminal_id)
            }
            Self::Ssh {
                workspace_session_id,
                terminal_id,
                terminal_generation,
                connection_id,
            } => {
                require_identifier("workspace_session_id", workspace_session_id)?;
                require_identifier("terminal_id", terminal_id)?;
                if *terminal_generation == 0 {
                    return Err(AiExecutionValidationError::new(
                        "terminal_generation must be positive",
                    ));
                }
                if *connection_id <= 0 {
                    return Err(AiExecutionValidationError::new(
                        "connection_id must be positive",
                    ));
                }
                Ok(())
            }
        }
    }
}

/// Provider request identity 是幂等键；同一 session/request/tool 必须只绑定一个 execution。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AiExecutionRequestKey {
    pub provider: String,
    pub provider_session_id: String,
    pub provider_request_id: String,
    pub tool: AiAuditTool,
}

impl AiExecutionRequestKey {
    fn validate(&self) -> Result<(), AiExecutionValidationError> {
        require_identifier("provider", &self.provider)?;
        require_identifier("provider_session_id", &self.provider_session_id)?;
        require_identifier("provider_request_id", &self.provider_request_id)
    }
}

/// 创建请求不含命令、参数、输出、Token 或凭据；execution_id 必须由可信后端生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAiExecution {
    pub execution_id: String,
    pub request_key: AiExecutionRequestKey,
    pub conversation_id: String,
    pub target: AiExecutionTarget,
    pub created_at_ms: i64,
}

/// 持久 ledger 的完整安全记录，也是 Application 查询返回的权威快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExecutionRecord {
    pub execution_id: String,
    pub request_key: AiExecutionRequestKey,
    pub conversation_id: String,
    pub target: AiExecutionTarget,
    pub phase: AiExecutionPhase,
    pub approval: AiExecutionApproval,
    pub mode: AiExecutionMode,
    /// dispatch 前确定的安全降级原因；不包含命令、输出或远端身份信息。
    pub fallback_reason: Option<String>,
    pub result: AiExecutionResult,
    pub dispatch_receipt: AiDispatchReceipt,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub sequence: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// 所有状态变化都通过封闭事件进入聚合，调用方不能直接拼装下一阶段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiExecutionTransition {
    RequestApproval,
    DenyByPolicy,
    AuthorizeAutomatically,
    Approve,
    Reject,
    ApprovalTimedOut,
    RevokeApproval,
    ReserveTerminal,
    BeginDispatch {
        mode: AiExecutionMode,
        fallback_reason: Option<String>,
    },
    ConfirmDispatch,
    Complete {
        exit_code: Option<i32>,
        duration_ms: u64,
    },
    FailBeforeDispatch {
        error_code: String,
    },
    FailAfterDispatch {
        exit_code: Option<i32>,
        duration_ms: u64,
        error_code: String,
    },
    Cancel {
        duration_ms: u64,
    },
    TimeOut {
        duration_ms: u64,
    },
    MarkResultUnknown {
        duration_ms: Option<u64>,
        error_code: String,
    },
    RecoverAfterStartup,
}

/// 聚合集中维护状态机与记录不变量，Repository 只负责原子创建和条件写入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExecution {
    record: AiExecutionRecord,
}

impl AiExecution {
    pub fn new(input: NewAiExecution) -> Result<Self, AiExecutionValidationError> {
        require_identifier("execution_id", &input.execution_id)?;
        input.request_key.validate()?;
        require_identifier("conversation_id", &input.conversation_id)?;
        input.target.validate()?;
        if input.created_at_ms < 0 {
            return Err(AiExecutionValidationError::new(
                "created_at_ms must not be negative",
            ));
        }

        Ok(Self {
            record: AiExecutionRecord {
                execution_id: input.execution_id,
                request_key: input.request_key,
                conversation_id: input.conversation_id,
                target: input.target,
                phase: AiExecutionPhase::Requested,
                approval: AiExecutionApproval::NotRequested,
                mode: AiExecutionMode::Unselected,
                fallback_reason: None,
                result: AiExecutionResult::Pending,
                dispatch_receipt: AiDispatchReceipt::NotStarted,
                exit_code: None,
                duration_ms: None,
                error_code: None,
                sequence: 0,
                created_at_ms: input.created_at_ms,
                updated_at_ms: input.created_at_ms,
            },
        })
    }

    /// 从持久化记录恢复时重新验证不变量，损坏数据不能进入业务状态机。
    pub fn from_record(record: AiExecutionRecord) -> Result<Self, AiExecutionValidationError> {
        validate_record(&record)?;
        Ok(Self { record })
    }

    pub const fn record(&self) -> &AiExecutionRecord {
        &self.record
    }

    pub fn into_record(self) -> AiExecutionRecord {
        self.record
    }

    pub fn transition(
        &self,
        transition: AiExecutionTransition,
        updated_at_ms: i64,
    ) -> Result<Self, AiExecutionTransitionError> {
        if updated_at_ms < self.record.updated_at_ms {
            return Err(AiExecutionTransitionError::new(
                self.record.phase,
                "updated_at_ms moved backwards",
            ));
        }
        if self.record.phase.is_terminal() {
            return Err(AiExecutionTransitionError::new(
                self.record.phase,
                "terminal execution cannot transition",
            ));
        }

        let mut next = self.record.clone();
        apply_transition(&mut next, transition)?;
        next.sequence = next.sequence.checked_add(1).ok_or_else(|| {
            AiExecutionTransitionError::new(self.record.phase, "sequence overflow")
        })?;
        next.updated_at_ms = updated_at_ms;
        validate_record(&next).map_err(|error| {
            AiExecutionTransitionError::new(self.record.phase, error.to_string())
        })?;
        Ok(Self { record: next })
    }
}

fn apply_transition(
    next: &mut AiExecutionRecord,
    transition: AiExecutionTransition,
) -> Result<(), AiExecutionTransitionError> {
    use AiExecutionPhase as Phase;

    let from = next.phase;
    match (from, transition) {
        (Phase::Requested, AiExecutionTransition::RequestApproval) => {
            next.phase = Phase::AwaitingApproval;
            next.approval = AiExecutionApproval::Pending;
        }
        (Phase::Requested, AiExecutionTransition::DenyByPolicy) => {
            deny(next, AiExecutionApproval::NotRequested);
        }
        (Phase::Requested, AiExecutionTransition::AuthorizeAutomatically) => {
            next.phase = Phase::Authorized;
            next.approval = AiExecutionApproval::NotRequired;
        }
        (Phase::AwaitingApproval, AiExecutionTransition::Approve) => {
            next.phase = Phase::Authorized;
            next.approval = AiExecutionApproval::Approved;
        }
        (Phase::AwaitingApproval, AiExecutionTransition::Reject) => {
            deny(next, AiExecutionApproval::Rejected);
        }
        (Phase::AwaitingApproval, AiExecutionTransition::ApprovalTimedOut) => {
            deny(next, AiExecutionApproval::TimedOut);
        }
        (Phase::AwaitingApproval, AiExecutionTransition::RevokeApproval) => {
            deny(next, AiExecutionApproval::Revoked);
        }
        (Phase::Authorized, AiExecutionTransition::ReserveTerminal) => {
            next.phase = Phase::TerminalReserved;
        }
        (
            Phase::TerminalReserved,
            AiExecutionTransition::BeginDispatch {
                mode,
                fallback_reason,
            },
        ) if mode.is_selected() => {
            if let Some(reason) = &fallback_reason {
                validate_error_code(reason)
                    .map_err(|error| AiExecutionTransitionError::new(from, error.to_string()))?;
            }
            next.phase = Phase::Dispatching;
            next.mode = mode;
            next.fallback_reason = fallback_reason;
            next.dispatch_receipt = AiDispatchReceipt::IntentPersisted;
        }
        (Phase::Dispatching, AiExecutionTransition::ConfirmDispatch) => {
            next.phase = Phase::Dispatched;
            next.dispatch_receipt = AiDispatchReceipt::Confirmed;
        }
        (
            Phase::Dispatched,
            AiExecutionTransition::Complete {
                exit_code,
                duration_ms,
            },
        ) => {
            next.phase = Phase::Completed;
            next.result = AiExecutionResult::Succeeded;
            next.exit_code = exit_code;
            next.duration_ms = Some(duration_ms);
        }
        (
            Phase::Requested | Phase::Authorized | Phase::TerminalReserved,
            AiExecutionTransition::FailBeforeDispatch { error_code },
        ) => {
            validate_error_code(&error_code)
                .map_err(|error| AiExecutionTransitionError::new(from, error.to_string()))?;
            next.phase = Phase::Failed;
            next.result = AiExecutionResult::Failed;
            next.error_code = Some(error_code);
        }
        (
            Phase::Dispatched,
            AiExecutionTransition::FailAfterDispatch {
                exit_code,
                duration_ms,
                error_code,
            },
        ) => {
            validate_error_code(&error_code)
                .map_err(|error| AiExecutionTransitionError::new(from, error.to_string()))?;
            next.phase = Phase::Failed;
            next.result = AiExecutionResult::Failed;
            next.exit_code = exit_code;
            next.duration_ms = Some(duration_ms);
            next.error_code = Some(error_code);
        }
        (Phase::Dispatching | Phase::Dispatched, AiExecutionTransition::Cancel { duration_ms }) => {
            next.phase = Phase::Cancelled;
            next.result = AiExecutionResult::Cancelled;
            next.duration_ms = Some(duration_ms);
        }
        (
            Phase::Dispatching | Phase::Dispatched,
            AiExecutionTransition::TimeOut { duration_ms },
        ) => {
            next.phase = Phase::TimedOut;
            next.result = AiExecutionResult::TimedOut;
            next.duration_ms = Some(duration_ms);
        }
        (
            Phase::Dispatching | Phase::Dispatched,
            AiExecutionTransition::MarkResultUnknown {
                duration_ms,
                error_code,
            },
        ) => {
            validate_error_code(&error_code)
                .map_err(|error| AiExecutionTransitionError::new(from, error.to_string()))?;
            next.phase = Phase::ResultUnknown;
            next.result = AiExecutionResult::Unknown;
            next.duration_ms = duration_ms;
            next.error_code = Some(error_code);
        }
        (
            Phase::Requested
            | Phase::AwaitingApproval
            | Phase::Authorized
            | Phase::TerminalReserved,
            AiExecutionTransition::RecoverAfterStartup,
        ) => {
            next.phase = Phase::Failed;
            next.result = AiExecutionResult::Failed;
            next.error_code = Some("AI_EXECUTION_INTERRUPTED_BY_RESTART".to_string());
            if next.approval == AiExecutionApproval::Pending {
                next.approval = AiExecutionApproval::Revoked;
            }
        }
        (Phase::Dispatching | Phase::Dispatched, AiExecutionTransition::RecoverAfterStartup) => {
            next.phase = Phase::ResultUnknown;
            next.result = AiExecutionResult::Unknown;
            next.error_code = Some("AI_EXECUTION_RESULT_UNKNOWN_AFTER_RESTART".to_string());
        }
        _ => {
            return Err(AiExecutionTransitionError::new(
                from,
                "transition is not legal from the current phase",
            ));
        }
    }
    Ok(())
}

fn deny(record: &mut AiExecutionRecord, approval: AiExecutionApproval) {
    record.phase = AiExecutionPhase::Denied;
    record.approval = approval;
    record.result = AiExecutionResult::Denied;
}

fn validate_record(record: &AiExecutionRecord) -> Result<(), AiExecutionValidationError> {
    require_identifier("execution_id", &record.execution_id)?;
    record.request_key.validate()?;
    require_identifier("conversation_id", &record.conversation_id)?;
    record.target.validate()?;
    if record.created_at_ms < 0 || record.updated_at_ms < record.created_at_ms {
        return Err(AiExecutionValidationError::new(
            "execution timestamps are inconsistent",
        ));
    }
    if let Some(error_code) = &record.error_code {
        validate_error_code(error_code)?;
    }
    if let Some(fallback_reason) = &record.fallback_reason {
        validate_error_code(fallback_reason)?;
    }

    use AiExecutionPhase as Phase;
    let authorized = matches!(
        record.approval,
        AiExecutionApproval::NotRequired | AiExecutionApproval::Approved
    );
    let valid = match record.phase {
        Phase::Requested => {
            record.approval == AiExecutionApproval::NotRequested && pending_before_dispatch(record)
        }
        Phase::AwaitingApproval => {
            record.approval == AiExecutionApproval::Pending && pending_before_dispatch(record)
        }
        Phase::Authorized | Phase::TerminalReserved => {
            authorized && pending_before_dispatch(record)
        }
        Phase::Dispatching => {
            authorized
                && record.mode.is_selected()
                && record.result == AiExecutionResult::Pending
                && record.dispatch_receipt == AiDispatchReceipt::IntentPersisted
                && result_metadata_is_empty(record)
        }
        Phase::Dispatched => {
            authorized
                && record.mode.is_selected()
                && record.result == AiExecutionResult::Pending
                && record.dispatch_receipt == AiDispatchReceipt::Confirmed
                && result_metadata_is_empty(record)
        }
        Phase::Completed => {
            authorized
                && completed_after_dispatch(record, AiExecutionResult::Succeeded)
                && record.error_code.is_none()
        }
        Phase::Failed => {
            record.result == AiExecutionResult::Failed
                && record.error_code.is_some()
                && ((record.mode == AiExecutionMode::Unselected
                    && record.dispatch_receipt == AiDispatchReceipt::NotStarted
                    && record.exit_code.is_none()
                    && record.duration_ms.is_none())
                    || (authorized
                        && record.mode.is_selected()
                        && record.dispatch_receipt == AiDispatchReceipt::Confirmed))
        }
        Phase::Denied => {
            matches!(
                record.approval,
                AiExecutionApproval::NotRequested
                    | AiExecutionApproval::Rejected
                    | AiExecutionApproval::TimedOut
                    | AiExecutionApproval::Revoked
            ) && record.result == AiExecutionResult::Denied
                && record.mode == AiExecutionMode::Unselected
                && record.dispatch_receipt == AiDispatchReceipt::NotStarted
                && result_metadata_is_empty(record)
        }
        Phase::Cancelled => {
            authorized && completed_after_dispatch(record, AiExecutionResult::Cancelled)
        }
        Phase::TimedOut => {
            authorized && completed_after_dispatch(record, AiExecutionResult::TimedOut)
        }
        Phase::ResultUnknown => {
            authorized
                && record.mode.is_selected()
                && record.result == AiExecutionResult::Unknown
                && matches!(
                    record.dispatch_receipt,
                    AiDispatchReceipt::IntentPersisted | AiDispatchReceipt::Confirmed
                )
                && record.exit_code.is_none()
                && record.error_code.is_some()
        }
    };

    if valid {
        Ok(())
    } else {
        Err(AiExecutionValidationError::new(
            "execution record violates phase invariants",
        ))
    }
}

fn pending_before_dispatch(record: &AiExecutionRecord) -> bool {
    record.mode == AiExecutionMode::Unselected
        && record.result == AiExecutionResult::Pending
        && record.dispatch_receipt == AiDispatchReceipt::NotStarted
        && result_metadata_is_empty(record)
}

fn completed_after_dispatch(record: &AiExecutionRecord, result: AiExecutionResult) -> bool {
    record.mode.is_selected()
        && record.result == result
        && record.dispatch_receipt == AiDispatchReceipt::Confirmed
        && record.duration_ms.is_some()
        && record.error_code.is_none()
}

fn result_metadata_is_empty(record: &AiExecutionRecord) -> bool {
    record.exit_code.is_none() && record.duration_ms.is_none() && record.error_code.is_none()
}

fn require_identifier(name: &str, value: &str) -> Result<(), AiExecutionValidationError> {
    if value.trim().is_empty() || value.len() > 256 {
        return Err(AiExecutionValidationError::new(format!(
            "{name} must contain 1 to 256 characters"
        )));
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), AiExecutionValidationError> {
    let valid = !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(AiExecutionValidationError::new(
            "error_code must be a stable uppercase identifier",
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExecutionValidationError {
    message: String,
}

impl AiExecutionValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AiExecutionValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiExecutionValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExecutionTransitionError {
    pub from: AiExecutionPhase,
    message: String,
}

impl AiExecutionTransitionError {
    fn new(from: AiExecutionPhase, message: impl Into<String>) -> Self {
        Self {
            from,
            message: message.into(),
        }
    }
}

impl fmt::Display for AiExecutionTransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiExecutionTransitionError {}

/// SQLite Adapter 应以 request_key 唯一约束原子实现该结果，避免先查后插竞态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiExecutionCreateOutcome {
    Created(AiExecutionRecord),
    Existing(AiExecutionRecord),
}

impl AiExecutionCreateOutcome {
    pub const fn record(&self) -> &AiExecutionRecord {
        match self {
            Self::Created(record) | Self::Existing(record) => record,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AiExecutionExpectation {
    pub phase: AiExecutionPhase,
    pub sequence: u64,
}

/// 条件更新未命中时返回当前记录，使 Application 可区分丢失记录与并发推进。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiExecutionUpdateOutcome {
    Updated(AiExecutionRecord),
    Conflict(Option<AiExecutionRecord>),
}

/// 查询过滤器只接受安全主键和封闭阶段；空过滤器表示列出全部快照。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AiExecutionListFilter {
    pub conversation_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub phases: Vec<AiExecutionPhase>,
}

/// Repository 以原子 create-or-get 与 compare-and-set 支持 SQLite 事务和请求幂等。
pub trait AiExecutionRepository: Send + Sync {
    fn create_or_get(
        &self,
        candidate: &AiExecutionRecord,
    ) -> Result<AiExecutionCreateOutcome, AiExecutionRepositoryError>;

    fn get(
        &self,
        execution_id: &str,
    ) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError>;

    fn list(
        &self,
        filter: &AiExecutionListFilter,
    ) -> Result<Vec<AiExecutionRecord>, AiExecutionRepositoryError>;

    fn conditional_update(
        &self,
        expectation: AiExecutionExpectation,
        replacement: &AiExecutionRecord,
    ) -> Result<AiExecutionUpdateOutcome, AiExecutionRepositoryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiExecutionRepositoryErrorKind {
    Unavailable,
    CorruptData,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiExecutionRepositoryError {
    pub kind: AiExecutionRepositoryErrorKind,
    message: String,
}

impl AiExecutionRepositoryError {
    pub fn new(kind: AiExecutionRepositoryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for AiExecutionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiExecutionRepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(id: &str) -> NewAiExecution {
        NewAiExecution {
            execution_id: id.to_string(),
            request_key: AiExecutionRequestKey {
                provider: "codex".to_string(),
                provider_session_id: "provider-session-1".to_string(),
                provider_request_id: format!("request-{id}"),
                tool: AiAuditTool::LocalTerminalExec,
            },
            conversation_id: "conversation-1".to_string(),
            target: AiExecutionTarget::Local {
                workspace_session_id: "workspace-1".to_string(),
                terminal_id: "terminal-1".to_string(),
            },
            created_at_ms: 100,
        }
    }

    fn transition(execution: AiExecution, event: AiExecutionTransition) -> AiExecution {
        let timestamp = execution.record().updated_at_ms + 1;
        execution
            .transition(event, timestamp)
            .expect("legal transition")
    }

    fn dispatched() -> AiExecution {
        let execution = AiExecution::new(input("exec-dispatched")).expect("new execution");
        let execution = transition(execution, AiExecutionTransition::AuthorizeAutomatically);
        let execution = transition(execution, AiExecutionTransition::ReserveTerminal);
        let execution = transition(
            execution,
            AiExecutionTransition::BeginDispatch {
                mode: AiExecutionMode::LocalVisibleOsc,
                fallback_reason: None,
            },
        );
        transition(execution, AiExecutionTransition::ConfirmDispatch)
    }

    #[test]
    fn new_execution_contains_only_safe_requested_metadata() {
        let execution = AiExecution::new(input("exec-1")).expect("new execution");
        let record = execution.record();

        assert_eq!(record.phase, AiExecutionPhase::Requested);
        assert_eq!(record.approval, AiExecutionApproval::NotRequested);
        assert_eq!(record.mode, AiExecutionMode::Unselected);
        assert_eq!(record.result, AiExecutionResult::Pending);
        assert_eq!(record.dispatch_receipt, AiDispatchReceipt::NotStarted);
        assert_eq!(record.sequence, 0);
        assert!(record.error_code.is_none());
    }

    #[test]
    fn approval_and_dispatch_path_preserves_the_side_effect_boundary() {
        let execution = AiExecution::new(input("exec-2")).expect("new execution");
        let execution = transition(execution, AiExecutionTransition::RequestApproval);
        assert_eq!(execution.record().approval, AiExecutionApproval::Pending);
        let execution = transition(execution, AiExecutionTransition::Approve);
        let execution = transition(execution, AiExecutionTransition::ReserveTerminal);
        let execution = transition(
            execution,
            AiExecutionTransition::BeginDispatch {
                mode: AiExecutionMode::SshIndependentExec,
                fallback_reason: None,
            },
        );
        assert_eq!(execution.record().phase, AiExecutionPhase::Dispatching);
        assert_eq!(
            execution.record().dispatch_receipt,
            AiDispatchReceipt::IntentPersisted
        );
        let execution = transition(execution, AiExecutionTransition::ConfirmDispatch);
        assert_eq!(execution.record().phase, AiExecutionPhase::Dispatched);
        assert_eq!(
            execution.record().dispatch_receipt,
            AiDispatchReceipt::Confirmed
        );
        assert_eq!(execution.record().sequence, 5);
    }

    #[test]
    fn denial_variants_are_terminal_and_never_select_a_mode() {
        let cases = [
            (AiExecutionTransition::Reject, AiExecutionApproval::Rejected),
            (
                AiExecutionTransition::ApprovalTimedOut,
                AiExecutionApproval::TimedOut,
            ),
            (
                AiExecutionTransition::RevokeApproval,
                AiExecutionApproval::Revoked,
            ),
        ];

        for (event, expected_approval) in cases {
            let execution = AiExecution::new(input("exec-denied")).expect("new execution");
            let execution = transition(execution, AiExecutionTransition::RequestApproval);
            let execution = transition(execution, event);
            assert_eq!(execution.record().phase, AiExecutionPhase::Denied);
            assert_eq!(execution.record().approval, expected_approval);
            assert_eq!(execution.record().result, AiExecutionResult::Denied);
            assert_eq!(execution.record().mode, AiExecutionMode::Unselected);
            assert!(
                execution
                    .transition(AiExecutionTransition::ReserveTerminal, 200)
                    .is_err()
            );
        }
    }

    #[test]
    fn dispatched_execution_supports_all_known_final_results() {
        let completed = transition(
            dispatched(),
            AiExecutionTransition::Complete {
                exit_code: Some(0),
                duration_ms: 45,
            },
        );
        assert_eq!(completed.record().result, AiExecutionResult::Succeeded);

        let failed = transition(
            dispatched(),
            AiExecutionTransition::FailAfterDispatch {
                exit_code: Some(7),
                duration_ms: 50,
                error_code: "AI_EXECUTION_EXIT_NONZERO".to_string(),
            },
        );
        assert_eq!(failed.record().phase, AiExecutionPhase::Failed);

        let cancelled = transition(
            dispatched(),
            AiExecutionTransition::Cancel { duration_ms: 12 },
        );
        assert_eq!(cancelled.record().result, AiExecutionResult::Cancelled);

        let timed_out = transition(
            dispatched(),
            AiExecutionTransition::TimeOut {
                duration_ms: 60_000,
            },
        );
        assert_eq!(timed_out.record().result, AiExecutionResult::TimedOut);

        let unknown = transition(
            dispatched(),
            AiExecutionTransition::MarkResultUnknown {
                duration_ms: Some(10),
                error_code: "AI_EXECUTION_CHANNEL_LOST".to_string(),
            },
        );
        assert_eq!(unknown.record().result, AiExecutionResult::Unknown);
    }

    #[test]
    fn dispatching_can_only_finish_as_dispatched_or_unknown() {
        let execution = AiExecution::new(input("exec-3")).expect("new execution");
        let execution = transition(execution, AiExecutionTransition::AuthorizeAutomatically);
        let execution = transition(execution, AiExecutionTransition::ReserveTerminal);
        let dispatching = transition(
            execution,
            AiExecutionTransition::BeginDispatch {
                mode: AiExecutionMode::LocalVisibleMarker,
                fallback_reason: None,
            },
        );

        assert!(
            dispatching
                .transition(
                    AiExecutionTransition::Complete {
                        exit_code: Some(0),
                        duration_ms: 1,
                    },
                    200,
                )
                .is_err()
        );
        let unknown = transition(
            dispatching,
            AiExecutionTransition::MarkResultUnknown {
                duration_ms: None,
                error_code: "AI_EXECUTION_DISPATCH_UNCERTAIN".to_string(),
            },
        );
        assert_eq!(
            unknown.record().dispatch_receipt,
            AiDispatchReceipt::IntentPersisted
        );
    }

    #[test]
    fn startup_recovery_distinguishes_pre_and_post_dispatch_records() {
        let awaiting = AiExecution::new(input("exec-awaiting")).expect("new execution");
        let awaiting = transition(awaiting, AiExecutionTransition::RequestApproval);
        let interrupted = transition(awaiting, AiExecutionTransition::RecoverAfterStartup);
        assert_eq!(interrupted.record().phase, AiExecutionPhase::Failed);
        assert_eq!(interrupted.record().approval, AiExecutionApproval::Revoked);
        assert_eq!(
            interrupted.record().error_code.as_deref(),
            Some("AI_EXECUTION_INTERRUPTED_BY_RESTART")
        );

        let unknown = transition(dispatched(), AiExecutionTransition::RecoverAfterStartup);
        assert_eq!(unknown.record().phase, AiExecutionPhase::ResultUnknown);
        assert_eq!(
            unknown.record().error_code.as_deref(),
            Some("AI_EXECUTION_RESULT_UNKNOWN_AFTER_RESTART")
        );
    }

    #[test]
    fn illegal_transitions_and_unselected_dispatch_modes_are_rejected() {
        let execution = AiExecution::new(input("exec-4")).expect("new execution");
        assert!(
            execution
                .transition(AiExecutionTransition::ReserveTerminal, 101)
                .is_err()
        );

        let execution = transition(execution, AiExecutionTransition::AuthorizeAutomatically);
        let execution = transition(execution, AiExecutionTransition::ReserveTerminal);
        assert!(
            execution
                .transition(
                    AiExecutionTransition::BeginDispatch {
                        mode: AiExecutionMode::Unselected,
                        fallback_reason: None,
                    },
                    103,
                )
                .is_err()
        );
    }

    #[test]
    fn malformed_ids_timestamps_and_error_codes_are_rejected() {
        let mut invalid = input(" ");
        assert!(AiExecution::new(invalid.clone()).is_err());
        invalid.execution_id = "exec-valid".to_string();
        invalid.created_at_ms = -1;
        assert!(AiExecution::new(invalid).is_err());

        let execution = AiExecution::new(input("exec-5")).expect("new execution");
        assert!(
            execution
                .transition(
                    AiExecutionTransition::FailBeforeDispatch {
                        error_code: "contains secret text".to_string(),
                    },
                    101,
                )
                .is_err()
        );
    }

    #[test]
    fn persisted_records_are_revalidated_before_use() {
        let execution = AiExecution::new(input("exec-6")).expect("new execution");
        let mut corrupted = execution.into_record();
        corrupted.phase = AiExecutionPhase::Completed;
        assert!(AiExecution::from_record(corrupted).is_err());
    }

    #[test]
    fn timestamps_cannot_move_backwards() {
        let execution = AiExecution::new(input("exec-7")).expect("new execution");
        let error = execution
            .transition(AiExecutionTransition::AuthorizeAutomatically, 99)
            .expect_err("backwards timestamp must fail");
        assert_eq!(error.from, AiExecutionPhase::Requested);
    }
}
