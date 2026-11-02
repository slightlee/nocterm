use std::sync::Arc;

use nocterm_domain::ai_execution::{
    AiExecution, AiExecutionCreateOutcome, AiExecutionExpectation, AiExecutionListFilter,
    AiExecutionPhase, AiExecutionRecord, AiExecutionRepository, AiExecutionRepositoryError,
    AiExecutionRepositoryErrorKind, AiExecutionTransition, AiExecutionUpdateOutcome,
    NewAiExecution,
};

use crate::error::AppError;

/// 执行服务只编排权威聚合与持久 ledger；命令、输出和凭据由运行期 Gateway 保管。
#[derive(Clone)]
pub struct AiExecutionService {
    repository: Arc<dyn AiExecutionRepository>,
}

impl AiExecutionService {
    pub fn new(repository: Arc<dyn AiExecutionRepository>) -> Self {
        Self { repository }
    }

    /// Adapter 必须在进入本用例前生成 execution_id；Repository 以 Provider 请求键原子去重。
    pub fn create_or_get(
        &self,
        input: NewAiExecution,
    ) -> Result<AiExecutionCreateOutcome, AppError> {
        let candidate = AiExecution::new(input)
            .map_err(|_| invalid_input_error())?
            .into_record();
        let outcome = self
            .repository
            .create_or_get(&candidate)
            .map_err(write_repository_error)?;
        validate_create_outcome(&candidate, outcome)
    }

    pub fn get(&self, execution_id: &str) -> Result<Option<AiExecutionRecord>, AppError> {
        if execution_id.trim().is_empty() {
            return Err(invalid_input_error());
        }
        self.repository
            .get(execution_id)
            .map_err(read_repository_error)?
            .map(validate_snapshot)
            .transpose()
    }

    pub fn list(&self, filter: &AiExecutionListFilter) -> Result<Vec<AiExecutionRecord>, AppError> {
        validate_filter(filter)?;
        self.repository
            .list(filter)
            .map_err(read_repository_error)?
            .into_iter()
            .map(validate_snapshot)
            .collect()
    }

    /// expected_sequence 让事件处理方显式拒绝重复、乱序和迟到更新。
    pub fn conditional_transition(
        &self,
        execution_id: &str,
        expected_sequence: u64,
        transition: AiExecutionTransition,
        updated_at_ms: i64,
    ) -> Result<AiExecutionRecord, AppError> {
        let current = self
            .get(execution_id)?
            .ok_or_else(execution_not_found_error)?;
        if current.sequence != expected_sequence {
            return Err(transition_conflict_error());
        }

        let expectation = AiExecutionExpectation {
            phase: current.phase,
            sequence: current.sequence,
        };
        let replacement = AiExecution::from_record(current)
            .map_err(|_| corrupt_data_error())?
            .transition(transition, updated_at_ms)
            .map_err(|_| invalid_transition_error())?
            .into_record();

        match self
            .repository
            .conditional_update(expectation, &replacement)
            .map_err(write_repository_error)?
        {
            AiExecutionUpdateOutcome::Updated(record) => validate_snapshot(record),
            AiExecutionUpdateOutcome::Conflict(_) => Err(transition_conflict_error()),
        }
    }

    /// 启动时绝不恢复运行期资源或重放命令：dispatch 前收敛为失败，边界上及之后收敛为结果未知。
    pub fn recover_after_startup(
        &self,
        updated_at_ms: i64,
    ) -> Result<Vec<AiExecutionRecord>, AppError> {
        let filter = AiExecutionListFilter {
            phases: nonterminal_phases().to_vec(),
            ..AiExecutionListFilter::default()
        };
        let snapshots = self.list(&filter)?;
        let mut recovered = Vec::with_capacity(snapshots.len());

        for snapshot in snapshots {
            let expectation = AiExecutionExpectation {
                phase: snapshot.phase,
                sequence: snapshot.sequence,
            };
            let replacement = AiExecution::from_record(snapshot)
                .map_err(|_| corrupt_data_error())?
                .transition(AiExecutionTransition::RecoverAfterStartup, updated_at_ms)
                .map_err(|_| invalid_transition_error())?
                .into_record();

            match self
                .repository
                .conditional_update(expectation, &replacement)
                .map_err(write_repository_error)?
            {
                AiExecutionUpdateOutcome::Updated(record) => {
                    recovered.push(validate_snapshot(record)?);
                }
                // 另一恢复者已推进到终态时无需覆盖；仍为非终态则要求调用方重新读取。
                AiExecutionUpdateOutcome::Conflict(Some(current))
                    if current.phase.is_terminal() => {}
                AiExecutionUpdateOutcome::Conflict(_) => {
                    return Err(transition_conflict_error());
                }
            }
        }

        Ok(recovered)
    }
}

const fn nonterminal_phases() -> &'static [AiExecutionPhase] {
    &[
        AiExecutionPhase::Requested,
        AiExecutionPhase::AwaitingApproval,
        AiExecutionPhase::Authorized,
        AiExecutionPhase::TerminalReserved,
        AiExecutionPhase::Dispatching,
        AiExecutionPhase::Dispatched,
    ]
}

fn validate_create_outcome(
    candidate: &AiExecutionRecord,
    outcome: AiExecutionCreateOutcome,
) -> Result<AiExecutionCreateOutcome, AppError> {
    let record = validate_snapshot(outcome.record().clone())?;
    if record.request_key != candidate.request_key {
        return Err(corrupt_data_error());
    }
    match outcome {
        AiExecutionCreateOutcome::Created(_) if record.execution_id == candidate.execution_id => {
            Ok(AiExecutionCreateOutcome::Created(record))
        }
        AiExecutionCreateOutcome::Existing(_) => Ok(AiExecutionCreateOutcome::Existing(record)),
        AiExecutionCreateOutcome::Created(_) => Err(corrupt_data_error()),
    }
}

fn validate_snapshot(record: AiExecutionRecord) -> Result<AiExecutionRecord, AppError> {
    AiExecution::from_record(record)
        .map(AiExecution::into_record)
        .map_err(|_| corrupt_data_error())
}

fn validate_filter(filter: &AiExecutionListFilter) -> Result<(), AppError> {
    let identifiers = [
        filter.conversation_id.as_deref(),
        filter.provider_session_id.as_deref(),
    ];
    if identifiers
        .into_iter()
        .flatten()
        .any(|value| value.trim().is_empty() || value.len() > 256)
    {
        return Err(invalid_input_error());
    }
    Ok(())
}

fn invalid_input_error() -> AppError {
    AppError::new("AI_EXECUTION_INPUT_INVALID", "AI 执行记录输入无效", false)
}

fn execution_not_found_error() -> AppError {
    AppError::new("AI_EXECUTION_NOT_FOUND", "AI 执行记录不存在或已清理", false)
}

fn invalid_transition_error() -> AppError {
    AppError::new(
        "AI_EXECUTION_TRANSITION_INVALID",
        "AI 执行状态变化不合法",
        false,
    )
}

fn transition_conflict_error() -> AppError {
    AppError::new(
        "AI_EXECUTION_TRANSITION_CONFLICT",
        "AI 执行状态已变化，请刷新后重试",
        true,
    )
}

fn corrupt_data_error() -> AppError {
    AppError::new(
        "AI_EXECUTION_DATA_INVALID",
        "AI 执行记录损坏，已阻止继续处理",
        false,
    )
}

fn read_repository_error(error: AiExecutionRepositoryError) -> AppError {
    repository_error(error, "AI_EXECUTION_LOAD_FAILED", "读取 AI 执行记录失败")
}

fn write_repository_error(error: AiExecutionRepositoryError) -> AppError {
    repository_error(
        error,
        "AI_EXECUTION_SAVE_FAILED",
        "保存 AI 执行记录失败，已阻止继续处理",
    )
}

fn repository_error(
    error: AiExecutionRepositoryError,
    unavailable_code: &'static str,
    unavailable_message: &'static str,
) -> AppError {
    // 原始数据库诊断可能包含本机路径或 SQL 内容，不能越过 Application 边界。
    match error.kind {
        AiExecutionRepositoryErrorKind::Unavailable => {
            AppError::new(unavailable_code, unavailable_message, true)
        }
        AiExecutionRepositoryErrorKind::CorruptData => corrupt_data_error(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nocterm_domain::{
        ai_audit::AiAuditTool,
        ai_execution::{
            AiDispatchReceipt, AiExecutionApproval, AiExecutionMode, AiExecutionRequestKey,
            AiExecutionResult, AiExecutionTarget,
        },
    };

    use super::*;

    #[derive(Default)]
    struct InMemoryRepository {
        records: Mutex<Vec<AiExecutionRecord>>,
    }

    impl InMemoryRepository {
        fn insert(&self, record: AiExecutionRecord) {
            self.records.lock().expect("records lock").push(record);
        }
    }

    impl AiExecutionRepository for InMemoryRepository {
        fn create_or_get(
            &self,
            candidate: &AiExecutionRecord,
        ) -> Result<AiExecutionCreateOutcome, AiExecutionRepositoryError> {
            let mut records = self.records.lock().expect("records lock");
            if let Some(existing) = records
                .iter()
                .find(|record| record.request_key == candidate.request_key)
            {
                return Ok(AiExecutionCreateOutcome::Existing(existing.clone()));
            }
            records.push(candidate.clone());
            Ok(AiExecutionCreateOutcome::Created(candidate.clone()))
        }

        fn get(
            &self,
            execution_id: &str,
        ) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError> {
            Ok(self
                .records
                .lock()
                .expect("records lock")
                .iter()
                .find(|record| record.execution_id == execution_id)
                .cloned())
        }

        fn list(
            &self,
            filter: &AiExecutionListFilter,
        ) -> Result<Vec<AiExecutionRecord>, AiExecutionRepositoryError> {
            Ok(self
                .records
                .lock()
                .expect("records lock")
                .iter()
                .filter(|record| {
                    filter
                        .conversation_id
                        .as_ref()
                        .is_none_or(|id| &record.conversation_id == id)
                        && filter
                            .provider_session_id
                            .as_ref()
                            .is_none_or(|id| &record.request_key.provider_session_id == id)
                        && (filter.phases.is_empty() || filter.phases.contains(&record.phase))
                })
                .cloned()
                .collect())
        }

        fn conditional_update(
            &self,
            expectation: AiExecutionExpectation,
            replacement: &AiExecutionRecord,
        ) -> Result<AiExecutionUpdateOutcome, AiExecutionRepositoryError> {
            let mut records = self.records.lock().expect("records lock");
            let Some(record) = records
                .iter_mut()
                .find(|record| record.execution_id == replacement.execution_id)
            else {
                return Ok(AiExecutionUpdateOutcome::Conflict(None));
            };
            if record.phase != expectation.phase || record.sequence != expectation.sequence {
                return Ok(AiExecutionUpdateOutcome::Conflict(Some(record.clone())));
            }
            *record = replacement.clone();
            Ok(AiExecutionUpdateOutcome::Updated(replacement.clone()))
        }
    }

    struct FailingRepository {
        kind: AiExecutionRepositoryErrorKind,
        diagnostic: &'static str,
    }

    impl FailingRepository {
        fn failure(&self) -> AiExecutionRepositoryError {
            AiExecutionRepositoryError::new(self.kind, self.diagnostic)
        }
    }

    impl AiExecutionRepository for FailingRepository {
        fn create_or_get(
            &self,
            _candidate: &AiExecutionRecord,
        ) -> Result<AiExecutionCreateOutcome, AiExecutionRepositoryError> {
            Err(self.failure())
        }

        fn get(
            &self,
            _execution_id: &str,
        ) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError> {
            Err(self.failure())
        }

        fn list(
            &self,
            _filter: &AiExecutionListFilter,
        ) -> Result<Vec<AiExecutionRecord>, AiExecutionRepositoryError> {
            Err(self.failure())
        }

        fn conditional_update(
            &self,
            _expectation: AiExecutionExpectation,
            _replacement: &AiExecutionRecord,
        ) -> Result<AiExecutionUpdateOutcome, AiExecutionRepositoryError> {
            Err(self.failure())
        }
    }

    struct ConflictingRepository {
        listed: AiExecutionRecord,
        current: AiExecutionRecord,
    }

    impl AiExecutionRepository for ConflictingRepository {
        fn create_or_get(
            &self,
            _candidate: &AiExecutionRecord,
        ) -> Result<AiExecutionCreateOutcome, AiExecutionRepositoryError> {
            Ok(AiExecutionCreateOutcome::Existing(self.current.clone()))
        }

        fn get(
            &self,
            _execution_id: &str,
        ) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError> {
            Ok(Some(self.current.clone()))
        }

        fn list(
            &self,
            _filter: &AiExecutionListFilter,
        ) -> Result<Vec<AiExecutionRecord>, AiExecutionRepositoryError> {
            Ok(vec![self.listed.clone()])
        }

        fn conditional_update(
            &self,
            _expectation: AiExecutionExpectation,
            _replacement: &AiExecutionRecord,
        ) -> Result<AiExecutionUpdateOutcome, AiExecutionRepositoryError> {
            Ok(AiExecutionUpdateOutcome::Conflict(Some(
                self.current.clone(),
            )))
        }
    }

    fn input(execution_id: &str, request_id: &str, timestamp: i64) -> NewAiExecution {
        NewAiExecution {
            execution_id: execution_id.to_string(),
            request_key: AiExecutionRequestKey {
                provider: "codex".to_string(),
                provider_session_id: "provider-session-1".to_string(),
                provider_request_id: request_id.to_string(),
                tool: AiAuditTool::SshExec,
            },
            conversation_id: "conversation-1".to_string(),
            target: AiExecutionTarget::Ssh {
                workspace_session_id: "remote:7:test".into(),
                terminal_id: "ssh-test".into(),
                terminal_generation: 1,
                connection_id: 7,
            },
            created_at_ms: timestamp,
        }
    }

    fn record_at_phase(
        execution_id: &str,
        request_id: &str,
        phase: AiExecutionPhase,
    ) -> AiExecutionRecord {
        let mut execution =
            AiExecution::new(input(execution_id, request_id, 100)).expect("new execution");
        let transitions = match phase {
            AiExecutionPhase::Requested => vec![],
            AiExecutionPhase::AwaitingApproval => vec![AiExecutionTransition::RequestApproval],
            AiExecutionPhase::Authorized => vec![AiExecutionTransition::AuthorizeAutomatically],
            AiExecutionPhase::TerminalReserved => vec![
                AiExecutionTransition::AuthorizeAutomatically,
                AiExecutionTransition::ReserveTerminal,
            ],
            AiExecutionPhase::Dispatching => vec![
                AiExecutionTransition::AuthorizeAutomatically,
                AiExecutionTransition::ReserveTerminal,
                AiExecutionTransition::BeginDispatch {
                    mode: AiExecutionMode::SshIndependentExec,
                    fallback_reason: None,
                },
            ],
            AiExecutionPhase::Dispatched => vec![
                AiExecutionTransition::AuthorizeAutomatically,
                AiExecutionTransition::ReserveTerminal,
                AiExecutionTransition::BeginDispatch {
                    mode: AiExecutionMode::SshIndependentExec,
                    fallback_reason: None,
                },
                AiExecutionTransition::ConfirmDispatch,
            ],
            _ => panic!("helper only creates nonterminal phases"),
        };
        for event in transitions {
            let timestamp = execution.record().updated_at_ms + 1;
            execution = execution
                .transition(event, timestamp)
                .expect("legal transition");
        }
        execution.into_record()
    }

    #[test]
    fn create_or_get_deduplicates_provider_request_identity() {
        let repository = Arc::new(InMemoryRepository::default());
        let service = AiExecutionService::new(repository.clone());

        let first = service
            .create_or_get(input("exec-1", "request-1", 100))
            .expect("create execution");
        assert!(matches!(first, AiExecutionCreateOutcome::Created(_)));

        let second = service
            .create_or_get(input("exec-2", "request-1", 101))
            .expect("get existing execution");
        assert!(matches!(second, AiExecutionCreateOutcome::Existing(_)));
        assert_eq!(second.record().execution_id, "exec-1");
        assert_eq!(repository.records.lock().expect("records lock").len(), 1);
    }

    #[test]
    fn conditional_transition_updates_sequence_and_returns_snapshots() {
        let repository = Arc::new(InMemoryRepository::default());
        let service = AiExecutionService::new(repository);
        service
            .create_or_get(input("exec-1", "request-1", 100))
            .expect("create execution");

        let authorized = service
            .conditional_transition(
                "exec-1",
                0,
                AiExecutionTransition::AuthorizeAutomatically,
                101,
            )
            .expect("authorize");
        assert_eq!(authorized.phase, AiExecutionPhase::Authorized);
        assert_eq!(authorized.approval, AiExecutionApproval::NotRequired);
        assert_eq!(authorized.sequence, 1);

        let listed = service
            .list(&AiExecutionListFilter {
                conversation_id: Some("conversation-1".to_string()),
                ..AiExecutionListFilter::default()
            })
            .expect("list snapshots");
        assert_eq!(listed, vec![authorized.clone()]);
        assert_eq!(service.get("exec-1").expect("get"), Some(authorized));
    }

    #[test]
    fn stale_sequences_and_illegal_transitions_have_distinct_stable_errors() {
        let repository = Arc::new(InMemoryRepository::default());
        let service = AiExecutionService::new(repository);
        service
            .create_or_get(input("exec-1", "request-1", 100))
            .expect("create execution");

        let stale = service
            .conditional_transition(
                "exec-1",
                9,
                AiExecutionTransition::AuthorizeAutomatically,
                101,
            )
            .expect_err("stale sequence must fail");
        assert_eq!(stale.code, "AI_EXECUTION_TRANSITION_CONFLICT");
        assert!(stale.retryable);

        let illegal = service
            .conditional_transition("exec-1", 0, AiExecutionTransition::ReserveTerminal, 101)
            .expect_err("illegal transition must fail");
        assert_eq!(illegal.code, "AI_EXECUTION_TRANSITION_INVALID");
        assert!(!illegal.retryable);
    }

    #[test]
    fn repository_compare_and_set_conflicts_are_exposed_without_overwrite() {
        let record = record_at_phase("exec-1", "request-1", AiExecutionPhase::Requested);
        let service = AiExecutionService::new(Arc::new(ConflictingRepository {
            listed: record.clone(),
            current: record,
        }));

        let error = service
            .conditional_transition(
                "exec-1",
                0,
                AiExecutionTransition::AuthorizeAutomatically,
                101,
            )
            .expect_err("compare-and-set must conflict");
        assert_eq!(error.code, "AI_EXECUTION_TRANSITION_CONFLICT");
    }

    #[test]
    fn startup_recovery_never_replays_and_marks_boundary_states_unknown() {
        let repository = Arc::new(InMemoryRepository::default());
        for (index, phase) in nonterminal_phases().iter().copied().enumerate() {
            repository.insert(record_at_phase(
                &format!("exec-{index}"),
                &format!("request-{index}"),
                phase,
            ));
        }
        let service = AiExecutionService::new(repository.clone());

        let recovered = service.recover_after_startup(1_000).expect("recover");
        assert_eq!(recovered.len(), nonterminal_phases().len());

        for record in recovered {
            match record.dispatch_receipt {
                AiDispatchReceipt::NotStarted => {
                    assert_eq!(record.phase, AiExecutionPhase::Failed);
                    assert_eq!(record.result, AiExecutionResult::Failed);
                    assert_eq!(
                        record.error_code.as_deref(),
                        Some("AI_EXECUTION_INTERRUPTED_BY_RESTART")
                    );
                }
                AiDispatchReceipt::IntentPersisted | AiDispatchReceipt::Confirmed => {
                    assert_eq!(record.phase, AiExecutionPhase::ResultUnknown);
                    assert_eq!(record.result, AiExecutionResult::Unknown);
                    assert_eq!(
                        record.error_code.as_deref(),
                        Some("AI_EXECUTION_RESULT_UNKNOWN_AFTER_RESTART")
                    );
                }
            }
        }
        assert!(
            service
                .recover_after_startup(2_000)
                .expect("idempotent recovery")
                .is_empty()
        );
    }

    #[test]
    fn terminal_conflict_during_recovery_is_already_reconciled() {
        let dispatched = record_at_phase("exec-1", "request-1", AiExecutionPhase::Dispatched);
        let terminal = AiExecution::from_record(dispatched.clone())
            .expect("valid record")
            .transition(AiExecutionTransition::RecoverAfterStartup, 200)
            .expect("recover")
            .into_record();
        let service = AiExecutionService::new(Arc::new(ConflictingRepository {
            listed: dispatched,
            current: terminal,
        }));

        // list 返回终态快照时过滤责任属于 Repository；真实 Adapter 不会走到 CAS。
        let recovered = service
            .recover_after_startup(300)
            .expect("terminal conflict");
        assert!(recovered.is_empty());
    }

    #[test]
    fn invalid_inputs_and_missing_records_map_to_stable_errors() {
        let service = AiExecutionService::new(Arc::new(InMemoryRepository::default()));
        let invalid = service
            .create_or_get(input(" ", "request-1", 100))
            .expect_err("invalid id");
        assert_eq!(invalid.code, "AI_EXECUTION_INPUT_INVALID");

        let missing = service
            .conditional_transition(
                "missing",
                0,
                AiExecutionTransition::AuthorizeAutomatically,
                100,
            )
            .expect_err("missing execution");
        assert_eq!(missing.code, "AI_EXECUTION_NOT_FOUND");

        let invalid_filter = service
            .list(&AiExecutionListFilter {
                conversation_id: Some(" ".to_string()),
                ..AiExecutionListFilter::default()
            })
            .expect_err("invalid filter");
        assert_eq!(invalid_filter.code, "AI_EXECUTION_INPUT_INVALID");
    }

    #[test]
    fn unavailable_repository_errors_hide_private_diagnostics() {
        let service = AiExecutionService::new(Arc::new(FailingRepository {
            kind: AiExecutionRepositoryErrorKind::Unavailable,
            diagnostic: "database at C:/Users/private/ledger.db is locked",
        }));

        let read_error = service.get("exec-1").expect_err("read must fail");
        assert_eq!(read_error.code, "AI_EXECUTION_LOAD_FAILED");
        assert!(!read_error.message.contains("C:/Users/private"));

        let write_error = service
            .create_or_get(input("exec-1", "request-1", 100))
            .expect_err("write must fail");
        assert_eq!(write_error.code, "AI_EXECUTION_SAVE_FAILED");
        assert!(!write_error.message.contains("ledger.db"));
    }

    #[test]
    fn corrupt_repository_data_uses_a_non_retryable_stable_error() {
        let service = AiExecutionService::new(Arc::new(FailingRepository {
            kind: AiExecutionRepositoryErrorKind::CorruptData,
            diagnostic: "unknown phase containing raw row data",
        }));

        let error = service
            .list(&AiExecutionListFilter::default())
            .expect_err("corrupt data must fail");
        assert_eq!(error.code, "AI_EXECUTION_DATA_INVALID");
        assert!(!error.retryable);
        assert!(!error.message.contains("raw row"));
    }

    #[test]
    fn malformed_repository_snapshots_are_rejected_before_transition() {
        let mut record = record_at_phase("exec-1", "request-1", AiExecutionPhase::Requested);
        record.phase = AiExecutionPhase::Completed;
        let service = AiExecutionService::new(Arc::new(ConflictingRepository {
            listed: record.clone(),
            current: record,
        }));

        let error = service.get("exec-1").expect_err("invalid snapshot");
        assert_eq!(error.code, "AI_EXECUTION_DATA_INVALID");
    }
}
