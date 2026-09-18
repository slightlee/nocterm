//! SQLite AI 审计适配器。迁移仍由统一数据库入口负责，本模块只实现事件写入与保留。

use nocterm_domain::ai_audit::{AiAuditRepository, AiAuditRepositoryError, AiToolAuditEvent};
use rusqlite::params;

use super::sqlite_connection_repository::SqliteConnectionRepository;

const AI_AUDIT_RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;
const AI_AUDIT_MAX_EVENTS: i64 = 10_000;

impl AiAuditRepository for SqliteConnectionRepository {
    fn append(&self, event: &AiToolAuditEvent) -> Result<(), AiAuditRepositoryError> {
        let duration_ms = event
            .duration_ms
            .map(i64::try_from)
            .transpose()
            .map_err(ai_audit_error)?;
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ai_audit_error("database lock poisoned"))?;
        // 写入和两种保留策略在同一事务内完成，失败时不会留下未清理的半成品。
        let transaction = connection.transaction().map_err(ai_audit_error)?;
        transaction
            .execute(
                "INSERT INTO ai_tool_audit_events
                 (provider, session_id, target_kind, connection_id, tool_name,
                  approval_state, outcome, duration_ms, error_code)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    event.provider,
                    event.session_id,
                    event.target.kind(),
                    event.target.connection_id(),
                    event.tool.as_str(),
                    event.approval.as_str(),
                    event.outcome.as_str(),
                    duration_ms,
                    event.error_code,
                ],
            )
            .map_err(ai_audit_error)?;
        transaction
            .execute(
                "DELETE FROM ai_tool_audit_events
                 WHERE occurred_at < unixepoch() - ?1",
                [AI_AUDIT_RETENTION_SECONDS],
            )
            .map_err(ai_audit_error)?;
        transaction
            .execute(
                "DELETE FROM ai_tool_audit_events
                 WHERE id IN (
                     SELECT id FROM ai_tool_audit_events
                     ORDER BY id DESC LIMIT -1 OFFSET ?1
                 )",
                [AI_AUDIT_MAX_EVENTS],
            )
            .map_err(ai_audit_error)?;
        transaction.commit().map_err(ai_audit_error)
    }
}

fn ai_audit_error(error: impl std::fmt::Display) -> AiAuditRepositoryError {
    AiAuditRepositoryError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use nocterm_domain::ai_audit::{AiAuditApproval, AiAuditOutcome, AiAuditTarget, AiAuditTool};

    use super::*;

    fn audit_event() -> AiToolAuditEvent {
        AiToolAuditEvent {
            provider: "codex".into(),
            session_id: "ai-audit-test".into(),
            target: AiAuditTarget::Ssh { connection_id: 7 },
            tool: AiAuditTool::GetSystemInfo,
            approval: AiAuditApproval::NotRequired,
            outcome: AiAuditOutcome::Succeeded,
            duration_ms: Some(12),
            error_code: None,
        }
    }

    #[test]
    fn audit_events_store_only_the_redacted_contract() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository
            .append(&audit_event())
            .expect("append audit event");
        let connection = repository.connection.lock().expect("database lock");
        let columns = connection
            .prepare("PRAGMA table_info(ai_tool_audit_events)")
            .expect("prepare table info")
            .query_map([], |row| row.get::<_, String>(1))
            .expect("query columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect columns");
        let stored: (String, String, String, i64, String, String, String, i64) = connection
            .query_row(
                "SELECT provider, session_id, target_kind, connection_id, tool_name,
                        approval_state, outcome, duration_ms
                 FROM ai_tool_audit_events",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .expect("read audit event");

        assert_eq!(
            columns,
            vec![
                "id",
                "occurred_at",
                "provider",
                "session_id",
                "target_kind",
                "connection_id",
                "tool_name",
                "approval_state",
                "outcome",
                "duration_ms",
                "error_code",
            ]
        );
        assert_eq!(
            stored,
            (
                "codex".into(),
                "ai-audit-test".into(),
                "ssh".into(),
                7,
                "get_system_info".into(),
                "not_required".into(),
                "succeeded".into(),
                12,
            )
        );
        assert!(
            columns
                .iter()
                .all(|column| !["command", "arguments", "output", "token"]
                    .contains(&column.as_str()))
        );
    }

    #[test]
    fn audit_retention_removes_expired_and_excess_events_atomically() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        {
            let mut connection = repository.connection.lock().expect("database lock");
            let transaction = connection.transaction().expect("begin seed transaction");
            transaction
                .execute(
                    "INSERT INTO ai_tool_audit_events
                     (occurred_at, provider, session_id, target_kind, connection_id, tool_name,
                      approval_state, outcome)
                     VALUES (unixepoch() - ?1 - 1, 'codex', 'ai-expired', 'ssh', 7,
                             'get_system_info', 'not_required', 'succeeded')",
                    [AI_AUDIT_RETENTION_SECONDS],
                )
                .expect("seed expired event");
            for sequence in 0..AI_AUDIT_MAX_EVENTS {
                transaction
                    .execute(
                        "INSERT INTO ai_tool_audit_events
                         (provider, session_id, target_kind, connection_id, tool_name,
                          approval_state, outcome)
                         VALUES ('codex', ?1, 'ssh', 7, 'get_system_info',
                                 'not_required', 'succeeded')",
                        [format!("ai-{sequence}")],
                    )
                    .expect("seed retained event");
            }
            transaction.commit().expect("commit seed events");
        }

        repository
            .append(&audit_event())
            .expect("append audited event");

        let connection = repository.connection.lock().expect("database lock");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM ai_tool_audit_events", [], |row| {
                row.get(0)
            })
            .expect("count audit events");
        let expired: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM ai_tool_audit_events WHERE session_id = 'ai-expired'",
                [],
                |row| row.get(0),
            )
            .expect("count expired events");

        assert_eq!(count, AI_AUDIT_MAX_EVENTS);
        assert_eq!(expired, 0);
    }
}
