//! AI execution ledger 的 Tauri 适配层。
//! ID 生成、领域服务调用和事件投影属于宿主职责；合法状态转换仍由 Domain/Application 定义。
//!
//! Tracker 持有事件发射闭包而非具体 `AppHandle`：这样 Gateway 集成测试和 Domain/Application
//! 单元测试都能在不构造完整 Tauri 运行时的前提下验证生命周期，生产路径由调用方注入真正的事件发布器。

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use nocterm_application::ai_execution::AiExecutionService;
use nocterm_domain::{
    ai_audit::AiAuditTool,
    ai_execution::{
        AiExecutionCreateOutcome, AiExecutionListFilter, AiExecutionRecord, AiExecutionRequestKey,
        AiExecutionTarget, AiExecutionTransition, NewAiExecution,
    },
};
use tauri::{AppHandle, Emitter, State};

use crate::{
    dto::ai::AiToolExecutionEvent,
    state::{AiGatewayBinding, AiTarget, AppState},
};

/// 事件发射器抽象：生产路径通过 Tauri `AppHandle` 发布，测试可注入空实现。
pub(crate) trait ExecutionEmitter: Send + Sync {
    fn emit(&self, event: &AiToolExecutionEvent);
}

/// 生产实现：事件是可丢失投影；持久 ledger 与快照查询仍是恢复时的权威来源。
pub(crate) struct TauriExecutionEmitter {
    app: AppHandle,
}

impl TauriExecutionEmitter {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ExecutionEmitter for TauriExecutionEmitter {
    fn emit(&self, event: &AiToolExecutionEvent) {
        let _ = self.app.emit("nocterm://ai-tool-execution", event.clone());
    }
}

pub(crate) enum ExecutionStart {
    Created(ExecutionTracker),
    Existing(AiExecutionRecord),
}

/// Tracker 只缓存刚从 Application 返回的权威快照；每次转换都使用 sequence 做 CAS。
pub(crate) struct ExecutionTracker {
    service: AiExecutionService,
    emitter: Arc<dyn ExecutionEmitter>,
    record: AiExecutionRecord,
}

impl ExecutionTracker {
    pub(crate) fn start(
        service: AiExecutionService,
        emitter: Arc<dyn ExecutionEmitter>,
        binding: &AiGatewayBinding,
        provider_request_id: &str,
        tool: AiAuditTool,
    ) -> Result<ExecutionStart, String> {
        let input = NewAiExecution {
            execution_id: generate_execution_id()?,
            request_key: AiExecutionRequestKey {
                provider: binding.provider.clone(),
                provider_session_id: binding.session_id.clone(),
                provider_request_id: provider_request_id.to_string(),
                tool,
            },
            conversation_id: binding.conversation_id.clone(),
            target: execution_target(&binding.target),
            created_at_ms: current_unix_ms(),
        };
        match service
            .create_or_get(input)
            .map_err(|error| error.message)?
        {
            AiExecutionCreateOutcome::Created(record) => {
                let tracker = Self {
                    service,
                    emitter,
                    record,
                };
                tracker.emit();
                Ok(ExecutionStart::Created(tracker))
            }
            AiExecutionCreateOutcome::Existing(record) => Ok(ExecutionStart::Existing(record)),
        }
    }

    pub(crate) fn record(&self) -> &AiExecutionRecord {
        &self.record
    }

    pub(crate) fn transition(
        &mut self,
        transition: AiExecutionTransition,
    ) -> Result<&AiExecutionRecord, String> {
        self.record = self
            .service
            .conditional_transition(
                &self.record.execution_id,
                self.record.sequence,
                transition,
                current_unix_ms(),
            )
            .map_err(|error| error.message)?;
        self.emit();
        Ok(&self.record)
    }

    fn emit(&self) {
        self.emitter.emit(&AiToolExecutionEvent::from(&self.record));
    }
}

#[tauri::command]
pub fn ai_tool_execution_list(
    state: State<'_, AppState>,
    conversation_id: Option<String>,
    provider_session_id: Option<String>,
) -> Result<Vec<AiToolExecutionEvent>, String> {
    state
        .ai_execution_service()
        .list(&AiExecutionListFilter {
            conversation_id,
            provider_session_id,
            phases: Vec::new(),
        })
        .map(|records| records.iter().map(AiToolExecutionEvent::from).collect())
        .map_err(|error| error.message)
}

fn execution_target(target: &AiTarget) -> AiExecutionTarget {
    match target {
        AiTarget::Local {
            session_id,
            terminal_id,
        } => AiExecutionTarget::Local {
            workspace_session_id: session_id.clone(),
            terminal_id: terminal_id.clone(),
        },
        AiTarget::Ssh {
            connection_id,
            workspace_session_id,
            terminal_id,
            terminal_generation,
        } => AiExecutionTarget::Ssh {
            workspace_session_id: workspace_session_id.clone(),
            terminal_id: terminal_id.clone(),
            terminal_generation: *terminal_generation,
            connection_id: *connection_id,
        },
    }
}

fn generate_execution_id() -> Result<String, String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法生成 AI 执行标识".to_string())?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("ai-execution-{suffix}"))
}

pub(crate) fn current_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullEmitter;
    impl ExecutionEmitter for NullEmitter {
        fn emit(&self, _event: &AiToolExecutionEvent) {}
    }

    #[test]
    fn execution_ids_are_independent_lowercase_hex_values() {
        let first = generate_execution_id().expect("first id");
        let second = generate_execution_id().expect("second id");

        assert_eq!(first.len(), "ai-execution-".len() + 32);
        assert!(first.starts_with("ai-execution-"));
        assert!(
            first["ai-execution-".len()..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_ne!(first, second);
    }

    #[test]
    #[allow(clippy::redundant_clone)]
    fn duplicate_provider_request_returns_existing_without_dispatch() {
        use std::sync::Mutex;

        use nocterm_domain::ai_execution::{
            AiExecutionCreateOutcome, AiExecutionExpectation, AiExecutionRepository,
            AiExecutionRepositoryError, AiExecutionRepositoryErrorKind, AiExecutionUpdateOutcome,
        };

        #[derive(Default)]
        struct RecordingRepository {
            records: Mutex<Vec<AiExecutionRecord>>,
        }

        impl AiExecutionRepository for RecordingRepository {
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

        let _ = AiExecutionRepositoryErrorKind::Unavailable;
        let service = AiExecutionService::new(Arc::new(RecordingRepository::default()));
        let emitter: Arc<dyn ExecutionEmitter> = Arc::new(NullEmitter);
        let binding = AiGatewayBinding {
            target: AiTarget::Ssh {
                connection_id: 7,
                workspace_session_id: "remote:7:test".into(),
                terminal_id: "ssh-test".into(),
                terminal_generation: 1,
            },
            provider: "codex".into(),
            conversation_id: "conversation-1".into(),
            session_id: "ai-session-1".into(),
            command_policy: crate::state::AiCommandPolicy::AutoSafe,
        };

        let first = ExecutionTracker::start(
            service.clone(),
            emitter.clone(),
            &binding,
            "request-1",
            AiAuditTool::SshExec,
        )
        .expect("first start");
        let second = ExecutionTracker::start(
            service,
            emitter,
            &binding,
            "request-1",
            AiAuditTool::SshExec,
        )
        .expect("second start");

        assert!(matches!(first, ExecutionStart::Created(_)));
        match second {
            ExecutionStart::Existing(record) => {
                assert_eq!(
                    record.phase,
                    nocterm_domain::ai_execution::AiExecutionPhase::Requested
                );
            }
            ExecutionStart::Created(_) => panic!("duplicate request must not dispatch again"),
        }
    }
}
