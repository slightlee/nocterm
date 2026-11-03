//! SQLite AI execution ledger adapter. The ledger stores only control-plane metadata and
//! deliberately excludes commands, arguments, output, credentials, and provider tokens.

use nocterm_domain::{
    ai_audit::AiAuditTool,
    ai_execution::{
        AiDispatchReceipt, AiExecution, AiExecutionApproval, AiExecutionCreateOutcome,
        AiExecutionExpectation, AiExecutionListFilter, AiExecutionMode, AiExecutionPhase,
        AiExecutionRecord, AiExecutionRepository, AiExecutionRepositoryError,
        AiExecutionRepositoryErrorKind, AiExecutionRequestKey, AiExecutionResult,
        AiExecutionTarget, AiExecutionUpdateOutcome,
    },
};
use rusqlite::{OptionalExtension, params, params_from_iter, types::Value};

use super::sqlite_connection_repository::SqliteConnectionRepository;

const AI_EXECUTION_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const AI_EXECUTION_MAX_TERMINAL_RECORDS: i64 = 10_000;
const TERMINAL_PHASES_SQL: &str =
    "'completed', 'failed', 'denied', 'cancelled', 'timed_out', 'result_unknown'";

impl AiExecutionRepository for SqliteConnectionRepository {
    fn create_or_get(
        &self,
        candidate: &AiExecutionRecord,
    ) -> Result<AiExecutionCreateOutcome, AiExecutionRepositoryError> {
        validate_record(candidate)?;
        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(repository_error)?;
        let inserted = insert_record(&transaction, candidate).map_err(repository_error)?;

        let outcome = if inserted == 1 {
            prune_terminal_records(&transaction)?;
            AiExecutionCreateOutcome::Created(candidate.clone())
        } else {
            // A request identity conflict is normal idempotence. A primary-key-only conflict is
            // not: returning an unrelated execution would permit identity confusion.
            let existing = get_by_request_key(&transaction, &candidate.request_key)?;
            let Some(existing) = existing else {
                return Err(corrupt_error(
                    "execution_id is already bound to another provider request",
                ));
            };
            AiExecutionCreateOutcome::Existing(existing)
        };
        transaction.commit().map_err(repository_error)?;
        Ok(outcome)
    }

    fn get(
        &self,
        execution_id: &str,
    ) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError> {
        let connection = self.connection.lock().map_err(lock_error)?;
        get_by_execution_id(&connection, execution_id)
    }

    fn list(
        &self,
        filter: &AiExecutionListFilter,
    ) -> Result<Vec<AiExecutionRecord>, AiExecutionRepositoryError> {
        let connection = self.connection.lock().map_err(lock_error)?;
        let mut sql = format!("SELECT {EXECUTION_COLUMNS} FROM ai_tool_executions WHERE 1 = 1");
        let mut values = Vec::new();
        if let Some(conversation_id) = &filter.conversation_id {
            sql.push_str(" AND conversation_id = ?");
            values.push(Value::Text(conversation_id.clone()));
        }
        if let Some(provider_session_id) = &filter.provider_session_id {
            sql.push_str(" AND provider_session_id = ?");
            values.push(Value::Text(provider_session_id.clone()));
        }
        if !filter.phases.is_empty() {
            sql.push_str(" AND phase IN (");
            sql.push_str(&vec!["?"; filter.phases.len()].join(", "));
            sql.push(')');
            values.extend(
                filter
                    .phases
                    .iter()
                    .map(|phase| Value::Text(phase.as_str().to_string())),
            );
        }
        sql.push_str(" ORDER BY updated_at_ms DESC, execution_id ASC");

        let mut statement = connection.prepare(&sql).map_err(repository_error)?;
        statement
            .query_map(params_from_iter(values), map_record)
            .map_err(repository_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(map_read_error)
    }

    fn conditional_update(
        &self,
        expectation: AiExecutionExpectation,
        replacement: &AiExecutionRecord,
    ) -> Result<AiExecutionUpdateOutcome, AiExecutionRepositoryError> {
        validate_record(replacement)?;
        let expected_sequence = i64::try_from(expectation.sequence).map_err(repository_error)?;
        let replacement_sequence = i64::try_from(replacement.sequence).map_err(repository_error)?;
        if replacement.sequence != expectation.sequence.saturating_add(1) {
            return Err(corrupt_error(
                "replacement sequence must advance the expected sequence exactly once",
            ));
        }

        let mut connection = self.connection.lock().map_err(lock_error)?;
        let transaction = connection.transaction().map_err(repository_error)?;
        let current = get_by_execution_id(&transaction, &replacement.execution_id)?;
        let Some(current) = current else {
            transaction.commit().map_err(repository_error)?;
            return Ok(AiExecutionUpdateOutcome::Conflict(None));
        };
        if current.phase != expectation.phase || current.sequence != expectation.sequence {
            transaction.commit().map_err(repository_error)?;
            return Ok(AiExecutionUpdateOutcome::Conflict(Some(current)));
        }
        if !same_identity(&current, replacement) {
            return Err(corrupt_error(
                "conditional update attempted to change immutable execution identity",
            ));
        }

        let duration_ms = replacement
            .duration_ms
            .map(i64::try_from)
            .transpose()
            .map_err(repository_error)?;
        let updated = transaction
            .execute(
                "UPDATE ai_tool_executions SET
                     phase = ?1, approval_state = ?2, execution_mode = ?3,
                     fallback_reason = ?4, result_state = ?5, dispatch_receipt = ?6,
                     exit_code = ?7, duration_ms = ?8, error_code = ?9, sequence = ?10,
                     updated_at_ms = ?11
                 WHERE execution_id = ?12 AND phase = ?13 AND sequence = ?14",
                params![
                    replacement.phase.as_str(),
                    replacement.approval.as_str(),
                    replacement.mode.as_str(),
                    replacement.fallback_reason,
                    replacement.result.as_str(),
                    replacement.dispatch_receipt.as_str(),
                    replacement.exit_code,
                    duration_ms,
                    replacement.error_code,
                    replacement_sequence,
                    replacement.updated_at_ms,
                    replacement.execution_id,
                    expectation.phase.as_str(),
                    expected_sequence,
                ],
            )
            .map_err(repository_error)?;
        if updated != 1 {
            let conflict = get_by_execution_id(&transaction, &replacement.execution_id)?;
            transaction.commit().map_err(repository_error)?;
            return Ok(AiExecutionUpdateOutcome::Conflict(conflict));
        }

        prune_terminal_records(&transaction)?;
        let stored = get_by_execution_id(&transaction, &replacement.execution_id)?
            .ok_or_else(|| corrupt_error("updated execution disappeared before commit"))?;
        transaction.commit().map_err(repository_error)?;
        Ok(AiExecutionUpdateOutcome::Updated(stored))
    }
}

const EXECUTION_COLUMNS: &str = "execution_id, provider, provider_session_id, provider_request_id,
    conversation_id, target_kind, workspace_session_id, terminal_id, terminal_generation,
    connection_id, tool_name, phase, approval_state, execution_mode, fallback_reason,
    result_state, dispatch_receipt, exit_code, duration_ms, error_code, sequence,
    created_at_ms, updated_at_ms";

fn insert_record(
    connection: &rusqlite::Connection,
    record: &AiExecutionRecord,
) -> rusqlite::Result<usize> {
    let (workspace_session_id, terminal_id, terminal_generation, connection_id) =
        target_columns(&record.target);
    let duration_ms = record
        .duration_ms
        .map(i64::try_from)
        .transpose()
        .map_err(|error| conversion_error(18, rusqlite::types::Type::Integer, error))?;
    let sequence = i64::try_from(record.sequence)
        .map_err(|error| conversion_error(20, rusqlite::types::Type::Integer, error))?;
    let terminal_generation = terminal_generation
        .map(i64::try_from)
        .transpose()
        .map_err(|error| conversion_error(8, rusqlite::types::Type::Integer, error))?;

    connection.execute(
        "INSERT OR IGNORE INTO ai_tool_executions
         (execution_id, provider, provider_session_id, provider_request_id, conversation_id,
          target_kind, workspace_session_id, terminal_id, terminal_generation, connection_id,
          tool_name, phase, approval_state, execution_mode, fallback_reason, result_state,
          dispatch_receipt, exit_code, duration_ms, error_code, sequence, created_at_ms,
          updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                 ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
        params![
            record.execution_id,
            record.request_key.provider,
            record.request_key.provider_session_id,
            record.request_key.provider_request_id,
            record.conversation_id,
            record.target.kind(),
            workspace_session_id,
            terminal_id,
            terminal_generation,
            connection_id,
            record.request_key.tool.as_str(),
            record.phase.as_str(),
            record.approval.as_str(),
            record.mode.as_str(),
            record.fallback_reason,
            record.result.as_str(),
            record.dispatch_receipt.as_str(),
            record.exit_code,
            duration_ms,
            record.error_code,
            sequence,
            record.created_at_ms,
            record.updated_at_ms,
        ],
    )
}

fn target_columns(
    target: &AiExecutionTarget,
) -> (Option<&str>, Option<&str>, Option<u64>, Option<i64>) {
    match target {
        AiExecutionTarget::Local {
            workspace_session_id,
            terminal_id,
        } => (Some(workspace_session_id), Some(terminal_id), None, None),
        AiExecutionTarget::Ssh {
            workspace_session_id,
            terminal_id,
            terminal_generation,
            connection_id,
        } => (
            Some(workspace_session_id),
            Some(terminal_id),
            Some(*terminal_generation),
            Some(*connection_id),
        ),
    }
}

fn same_identity(current: &AiExecutionRecord, replacement: &AiExecutionRecord) -> bool {
    current.execution_id == replacement.execution_id
        && current.request_key == replacement.request_key
        && current.conversation_id == replacement.conversation_id
        && current.target == replacement.target
        && current.created_at_ms == replacement.created_at_ms
}

fn prune_terminal_records(
    connection: &rusqlite::Connection,
) -> Result<(), AiExecutionRepositoryError> {
    // Active records are never pruned: startup recovery must retain every unresolved side effect.
    connection
        .execute(
            &format!(
                "DELETE FROM ai_tool_executions
                 WHERE phase IN ({TERMINAL_PHASES_SQL})
                   AND updated_at_ms < unixepoch() * 1000 - ?1"
            ),
            [AI_EXECUTION_RETENTION_MS],
        )
        .map_err(repository_error)?;
    connection
        .execute(
            &format!(
                "DELETE FROM ai_tool_executions
                 WHERE execution_id IN (
                     SELECT execution_id FROM ai_tool_executions
                     WHERE phase IN ({TERMINAL_PHASES_SQL})
                     ORDER BY updated_at_ms DESC, execution_id DESC
                     LIMIT -1 OFFSET ?1
                 )"
            ),
            [AI_EXECUTION_MAX_TERMINAL_RECORDS],
        )
        .map_err(repository_error)?;
    Ok(())
}

fn get_by_execution_id(
    connection: &rusqlite::Connection,
    execution_id: &str,
) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError> {
    connection
        .query_row(
            &format!("SELECT {EXECUTION_COLUMNS} FROM ai_tool_executions WHERE execution_id = ?1"),
            [execution_id],
            map_record,
        )
        .optional()
        .map_err(map_read_error)
}

fn get_by_request_key(
    connection: &rusqlite::Connection,
    key: &AiExecutionRequestKey,
) -> Result<Option<AiExecutionRecord>, AiExecutionRepositoryError> {
    connection
        .query_row(
            &format!(
                "SELECT {EXECUTION_COLUMNS} FROM ai_tool_executions
                 WHERE provider = ?1 AND provider_session_id = ?2
                   AND provider_request_id = ?3 AND tool_name = ?4"
            ),
            params![
                key.provider,
                key.provider_session_id,
                key.provider_request_id,
                key.tool.as_str()
            ],
            map_record,
        )
        .optional()
        .map_err(map_read_error)
}

fn map_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiExecutionRecord> {
    let target_kind = row.get::<_, String>(5)?;
    let workspace_session_id = row.get::<_, Option<String>>(6)?;
    let terminal_id = row.get::<_, Option<String>>(7)?;
    let terminal_generation = row.get::<_, Option<i64>>(8)?;
    let connection_id = row.get::<_, Option<i64>>(9)?;
    let target = match (
        target_kind.as_str(),
        workspace_session_id,
        terminal_id,
        terminal_generation,
        connection_id,
    ) {
        ("local", Some(workspace_session_id), Some(terminal_id), None, None) => {
            AiExecutionTarget::Local {
                workspace_session_id,
                terminal_id,
            }
        }
        (
            "ssh",
            Some(workspace_session_id),
            Some(terminal_id),
            Some(terminal_generation),
            Some(connection_id),
        ) => AiExecutionTarget::Ssh {
            workspace_session_id,
            terminal_id,
            terminal_generation: u64::try_from(terminal_generation)
                .map_err(|error| conversion_error(8, rusqlite::types::Type::Integer, error))?,
            connection_id,
        },
        _ => return Err(invalid_text(5, "invalid execution target binding")),
    };
    let duration_ms = row
        .get::<_, Option<i64>>(18)?
        .map(|value| {
            u64::try_from(value)
                .map_err(|error| conversion_error(18, rusqlite::types::Type::Integer, error))
        })
        .transpose()?;
    let record = AiExecutionRecord {
        execution_id: row.get(0)?,
        request_key: AiExecutionRequestKey {
            provider: row.get(1)?,
            provider_session_id: row.get(2)?,
            provider_request_id: row.get(3)?,
            tool: parse_tool(&row.get::<_, String>(10)?)
                .ok_or_else(|| invalid_text(10, "unknown tool_name"))?,
        },
        conversation_id: row.get(4)?,
        target,
        phase: parse_phase(&row.get::<_, String>(11)?)
            .ok_or_else(|| invalid_text(11, "unknown phase"))?,
        approval: parse_approval(&row.get::<_, String>(12)?)
            .ok_or_else(|| invalid_text(12, "unknown approval_state"))?,
        mode: parse_mode(&row.get::<_, String>(13)?)
            .ok_or_else(|| invalid_text(13, "unknown execution_mode"))?,
        fallback_reason: row.get(14)?,
        result: parse_result(&row.get::<_, String>(15)?)
            .ok_or_else(|| invalid_text(15, "unknown result_state"))?,
        dispatch_receipt: parse_receipt(&row.get::<_, String>(16)?)
            .ok_or_else(|| invalid_text(16, "unknown dispatch_receipt"))?,
        exit_code: row.get(17)?,
        duration_ms,
        error_code: row.get(19)?,
        sequence: read_u64(row, 20)?,
        created_at_ms: row.get(21)?,
        updated_at_ms: row.get(22)?,
    };
    AiExecution::from_record(record)
        .map(AiExecution::into_record)
        .map_err(|error| conversion_error(0, rusqlite::types::Type::Text, error))
}

fn read_u64(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value = row.get::<_, i64>(index)?;
    u64::try_from(value)
        .map_err(|error| conversion_error(index, rusqlite::types::Type::Integer, error))
}

fn parse_phase(value: &str) -> Option<AiExecutionPhase> {
    Some(match value {
        "requested" => AiExecutionPhase::Requested,
        "awaiting_approval" => AiExecutionPhase::AwaitingApproval,
        "authorized" => AiExecutionPhase::Authorized,
        "terminal_reserved" => AiExecutionPhase::TerminalReserved,
        "dispatching" => AiExecutionPhase::Dispatching,
        "dispatched" => AiExecutionPhase::Dispatched,
        "completed" => AiExecutionPhase::Completed,
        "failed" => AiExecutionPhase::Failed,
        "denied" => AiExecutionPhase::Denied,
        "cancelled" => AiExecutionPhase::Cancelled,
        "timed_out" => AiExecutionPhase::TimedOut,
        "result_unknown" => AiExecutionPhase::ResultUnknown,
        _ => return None,
    })
}

fn parse_approval(value: &str) -> Option<AiExecutionApproval> {
    Some(match value {
        "not_requested" => AiExecutionApproval::NotRequested,
        "not_required" => AiExecutionApproval::NotRequired,
        "pending" => AiExecutionApproval::Pending,
        "approved" => AiExecutionApproval::Approved,
        "rejected" => AiExecutionApproval::Rejected,
        "timed_out" => AiExecutionApproval::TimedOut,
        "revoked" => AiExecutionApproval::Revoked,
        _ => return None,
    })
}

fn parse_mode(value: &str) -> Option<AiExecutionMode> {
    Some(match value {
        "unselected" => AiExecutionMode::Unselected,
        "local_visible_osc" => AiExecutionMode::LocalVisibleOsc,
        "local_visible_marker" => AiExecutionMode::LocalVisibleMarker,
        "ssh_visible_osc" => AiExecutionMode::SshVisibleOsc,
        "ssh_visible_marker" => AiExecutionMode::SshVisibleMarker,
        "ssh_independent_exec" => AiExecutionMode::SshIndependentExec,
        _ => return None,
    })
}

fn parse_result(value: &str) -> Option<AiExecutionResult> {
    Some(match value {
        "pending" => AiExecutionResult::Pending,
        "succeeded" => AiExecutionResult::Succeeded,
        "failed" => AiExecutionResult::Failed,
        "denied" => AiExecutionResult::Denied,
        "cancelled" => AiExecutionResult::Cancelled,
        "timed_out" => AiExecutionResult::TimedOut,
        "unknown" => AiExecutionResult::Unknown,
        _ => return None,
    })
}

fn parse_receipt(value: &str) -> Option<AiDispatchReceipt> {
    Some(match value {
        "not_started" => AiDispatchReceipt::NotStarted,
        "intent_persisted" => AiDispatchReceipt::IntentPersisted,
        "confirmed" => AiDispatchReceipt::Confirmed,
        _ => return None,
    })
}

fn parse_tool(value: &str) -> Option<AiAuditTool> {
    Some(match value {
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
        "unknown" => AiAuditTool::Unknown,
        _ => return None,
    })
}

fn validate_record(record: &AiExecutionRecord) -> Result<(), AiExecutionRepositoryError> {
    AiExecution::from_record(record.clone())
        .map(|_| ())
        .map_err(|error| corrupt_error(error.to_string()))
}

fn invalid_text(index: usize, message: &'static str) -> rusqlite::Error {
    conversion_error(
        index,
        rusqlite::types::Type::Text,
        std::io::Error::other(message),
    )
}

fn conversion_error(
    index: usize,
    value_type: rusqlite::types::Type,
    error: impl std::error::Error + Send + Sync + 'static,
) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(index, value_type, Box::new(error))
}

fn map_read_error(error: rusqlite::Error) -> AiExecutionRepositoryError {
    match error {
        rusqlite::Error::FromSqlConversionFailure(_, _, _) => corrupt_error(error.to_string()),
        _ => repository_error(error),
    }
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> AiExecutionRepositoryError {
    repository_error("database lock poisoned")
}

fn repository_error(error: impl std::fmt::Display) -> AiExecutionRepositoryError {
    AiExecutionRepositoryError::new(
        AiExecutionRepositoryErrorKind::Unavailable,
        error.to_string(),
    )
}

fn corrupt_error(error: impl std::fmt::Display) -> AiExecutionRepositoryError {
    AiExecutionRepositoryError::new(
        AiExecutionRepositoryErrorKind::CorruptData,
        error.to_string(),
    )
}
