//! SQLite 原生 Provider 会话绑定。该表是后端控制面状态，不通过前端会话 DTO 暴露。

use nocterm_domain::ai_native_session::{
    AiNativeSessionIdentity, AiNativeSessionRecord, AiNativeSessionRepository,
    AiNativeSessionRepositoryError,
};
use rusqlite::{OptionalExtension, params};

use super::sqlite_connection_repository::SqliteConnectionRepository;

impl AiNativeSessionRepository for SqliteConnectionRepository {
    fn get(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AiNativeSessionRecord>, AiNativeSessionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| native_session_error("database lock poisoned"))?;
        connection
            .query_row(
                "SELECT native_session_id, provider, model, reasoning_effort, fast_mode,
                        connection_id, target_session_id, updated_at_ms
                 FROM ai_native_sessions
                 WHERE conversation_id = ?1",
                [conversation_id],
                |row| {
                    Ok(AiNativeSessionRecord {
                        conversation_id: conversation_id.to_string(),
                        native_session_id: row.get(0)?,
                        identity: AiNativeSessionIdentity {
                            provider: row.get(1)?,
                            model: row.get(2)?,
                            reasoning_effort: row.get(3)?,
                            fast_mode: row.get::<_, Option<i64>>(4)?.map(|value| value != 0),
                            connection_id: row.get(5)?,
                            target_session_id: row.get(6)?,
                        },
                        updated_at_ms: row.get(7)?,
                    })
                },
            )
            .optional()
            .map_err(native_session_error)
    }

    fn upsert(&self, record: &AiNativeSessionRecord) -> Result<(), AiNativeSessionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| native_session_error("database lock poisoned"))?;
        connection
            .execute(
                "INSERT INTO ai_native_sessions
                     (conversation_id, native_session_id, provider, model, reasoning_effort,
                      fast_mode, connection_id, target_session_id, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(conversation_id) DO UPDATE SET
                     native_session_id = excluded.native_session_id,
                     provider = excluded.provider,
                     model = excluded.model,
                     reasoning_effort = excluded.reasoning_effort,
                     fast_mode = excluded.fast_mode,
                     connection_id = excluded.connection_id,
                     target_session_id = excluded.target_session_id,
                     updated_at_ms = excluded.updated_at_ms",
                params![
                    record.conversation_id,
                    record.native_session_id,
                    record.identity.provider,
                    record.identity.model,
                    record.identity.reasoning_effort,
                    record
                        .identity
                        .fast_mode
                        .map(|value| if value { 1_i64 } else { 0_i64 }),
                    record.identity.connection_id,
                    record.identity.target_session_id,
                    record.updated_at_ms,
                ],
            )
            .map_err(native_session_error)?;
        Ok(())
    }

    fn delete(&self, conversation_id: &str) -> Result<(), AiNativeSessionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| native_session_error("database lock poisoned"))?;
        connection
            .execute(
                "DELETE FROM ai_native_sessions WHERE conversation_id = ?1",
                [conversation_id],
            )
            .map_err(native_session_error)?;
        Ok(())
    }
}

fn native_session_error(error: impl std::fmt::Display) -> AiNativeSessionRepositoryError {
    AiNativeSessionRepositoryError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> AiNativeSessionRecord {
        AiNativeSessionRecord {
            conversation_id: "conversation-one".into(),
            native_session_id: "provider-session-one".into(),
            identity: AiNativeSessionIdentity {
                provider: "claude-code".into(),
                model: Some("sonnet".into()),
                reasoning_effort: Some("high".into()),
                fast_mode: Some(false),
                connection_id: Some(7),
                target_session_id: Some("remote:7:terminal".into()),
            },
            updated_at_ms: 10,
        }
    }

    #[test]
    fn native_session_bindings_round_trip_and_replace_atomically() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.upsert(&record()).expect("insert binding");
        assert_eq!(repository.get("conversation-one").unwrap(), Some(record()));

        let mut replaced = record();
        replaced.native_session_id = "provider-session-two".into();
        replaced.updated_at_ms = 20;
        repository.upsert(&replaced).expect("replace binding");
        assert_eq!(repository.get("conversation-one").unwrap(), Some(replaced));

        repository
            .delete("conversation-one")
            .expect("delete binding");
        assert_eq!(repository.get("conversation-one").unwrap(), None);
    }
}
