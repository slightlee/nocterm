//! SQLite AI 会话历史适配器。迁移仍由统一数据库入口负责，本模块只实现会话快照的读写与删除。

use nocterm_domain::ai_conversation::{
    AiConversationRecord, AiConversationRepository, AiConversationRepositoryError, AiMessageRecord,
    AiMessageRole,
};
use rusqlite::params;

use super::sqlite_connection_repository::SqliteConnectionRepository;

/// 单会话消息数上限：正常对话远低于该值；超限视为异常客户端输入直接拒绝。
const MAX_MESSAGES_PER_CONVERSATION: usize = 1_000;

impl AiConversationRepository for SqliteConnectionRepository {
    /// 按最近更新排序返回全部会话；消息随会话一次性读出，前端无需二次查询。
    fn list(&self) -> Result<Vec<AiConversationRecord>, AiConversationRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ai_conversation_error("database lock poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT id, title, provider, command_policy, created_at_ms, updated_at_ms
                 FROM ai_conversations
                 ORDER BY updated_at_ms DESC",
            )
            .map_err(ai_conversation_error)?;
        let mut conversations = statement
            .query_map([], map_conversation_row)
            .map_err(ai_conversation_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ai_conversation_error)?;

        for conversation in &mut conversations {
            conversation.messages = self.load_messages(&connection, &conversation.id)?;
        }
        Ok(conversations)
    }

    /// 整体覆盖语义：先写会话行，再重建消息列表；任一步失败整体回滚，
    /// 避免出现“会话已更新但消息列表停留在旧状态”的中间结果。
    fn upsert(
        &self,
        conversation: &AiConversationRecord,
    ) -> Result<(), AiConversationRepositoryError> {
        if conversation.messages.len() > MAX_MESSAGES_PER_CONVERSATION {
            return Err(AiConversationRepositoryError::new(
                "conversation exceeds the maximum number of messages",
            ));
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ai_conversation_error("database lock poisoned"))?;
        let transaction = connection.transaction().map_err(ai_conversation_error)?;
        transaction
            .execute(
                "INSERT INTO ai_conversations
                     (id, title, provider, command_policy, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET
                     title = excluded.title,
                     provider = excluded.provider,
                     command_policy = excluded.command_policy,
                     created_at_ms = excluded.created_at_ms,
                     updated_at_ms = excluded.updated_at_ms",
                params![
                    conversation.id,
                    conversation.title,
                    conversation.provider,
                    conversation.command_policy,
                    conversation.created_at_ms,
                    conversation.updated_at_ms,
                ],
            )
            .map_err(ai_conversation_error)?;
        transaction
            .execute(
                "DELETE FROM ai_messages WHERE conversation_id = ?1",
                [conversation.id.clone()],
            )
            .map_err(ai_conversation_error)?;
        for message in &conversation.messages {
            transaction
                .execute(
                    "INSERT INTO ai_messages
                         (id, conversation_id, role, content, parts_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        message.id,
                        conversation.id,
                        message.role.as_str(),
                        message.content,
                        message.parts_json,
                        message.created_at_ms,
                    ],
                )
                .map_err(ai_conversation_error)?;
        }
        transaction.commit().map_err(ai_conversation_error)
    }

    fn delete(&self, id: &str) -> Result<(), AiConversationRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ai_conversation_error("database lock poisoned"))?;
        connection
            .execute("DELETE FROM ai_conversations WHERE id = ?1", [id])
            .map_err(ai_conversation_error)?;
        Ok(())
    }
}

impl SqliteConnectionRepository {
    /// 消息按创建时间升序返回，保持与前端追加顺序一致。
    fn load_messages(
        &self,
        connection: &rusqlite::Connection,
        conversation_id: &str,
    ) -> Result<Vec<AiMessageRecord>, AiConversationRepositoryError> {
        let mut statement = connection
            .prepare(
                "SELECT id, role, content, parts_json, created_at_ms
                 FROM ai_messages
                 WHERE conversation_id = ?1
                 ORDER BY created_at_ms ASC, rowid ASC",
            )
            .map_err(ai_conversation_error)?;
        let messages = statement
            .query_map([conversation_id], |row| {
                let role = row.get::<_, String>(1)?;
                Ok(AiMessageRecord {
                    id: row.get(0)?,
                    role: AiMessageRole::parse(&role).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    content: row.get(2)?,
                    parts_json: row.get(3)?,
                    created_at_ms: row.get(4)?,
                })
            })
            .map_err(ai_conversation_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(ai_conversation_error)?;
        Ok(messages)
    }
}

fn map_conversation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiConversationRecord> {
    Ok(AiConversationRecord {
        id: row.get(0)?,
        title: row.get(1)?,
        provider: row.get(2)?,
        command_policy: row.get(3)?,
        created_at_ms: row.get(4)?,
        updated_at_ms: row.get(5)?,
        // 消息在 list 主流程中单独装载，这里先留空。
        messages: Vec::new(),
    })
}

fn ai_conversation_error(error: impl std::fmt::Display) -> AiConversationRepositoryError {
    AiConversationRepositoryError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use rusqlite::OptionalExtension;

    use super::*;

    fn conversation(id: &str) -> AiConversationRecord {
        AiConversationRecord {
            id: id.into(),
            title: "排查磁盘".into(),
            provider: "codex".into(),
            command_policy: Some("auto_safe".into()),
            created_at_ms: 1_000,
            updated_at_ms: 2_000,
            messages: vec![
                AiMessageRecord {
                    id: format!("{id}-u1"),
                    role: AiMessageRole::User,
                    content: "查看磁盘占用".into(),
                    parts_json: None,
                    created_at_ms: 1_100,
                },
                AiMessageRecord {
                    id: format!("{id}-a1"),
                    role: AiMessageRole::Assistant,
                    content: "占用最高的是 /var/log".into(),
                    parts_json: Some(r#"[{"type":"text","content":"结论"}]"#.into()),
                    created_at_ms: 1_200,
                },
            ],
        }
    }

    #[test]
    fn conversations_round_trip_with_messages_in_order() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.upsert(&conversation("conv-1")).expect("upsert");
        // 第二个会话更新时间更晚，list 必须按最近更新排在前面。
        let mut newer = conversation("conv-2");
        newer.updated_at_ms = 3_000;
        repository.upsert(&newer).expect("upsert second");

        let listed = repository.list().expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "conv-2");
        assert_eq!(listed[1].id, "conv-1");
        assert_eq!(listed[1].messages.len(), 2);
        assert_eq!(listed[1].messages[0].role, AiMessageRole::User);
        assert_eq!(
            listed[1].messages[1].parts_json.as_deref(),
            Some(r#"[{"type":"text","content":"结论"}]"#)
        );
    }

    #[test]
    fn upsert_replaces_the_previous_message_list_atomically() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.upsert(&conversation("conv-1")).expect("seed");

        let mut cleared = conversation("conv-1");
        cleared.title = "新话题".into();
        cleared.messages = vec![AiMessageRecord {
            id: "conv-1-u2".into(),
            role: AiMessageRole::User,
            content: "重新开始".into(),
            parts_json: None,
            created_at_ms: 9_000,
        }];
        repository.upsert(&cleared).expect("upsert cleared");

        let listed = repository.list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "新话题");
        assert_eq!(
            listed[0]
                .messages
                .iter()
                .map(|message| message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["conv-1-u2"]
        );
    }

    #[test]
    fn deleting_a_conversation_cascades_to_its_messages() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.upsert(&conversation("conv-1")).expect("seed");
        repository.delete("conv-1").expect("delete");

        let listed = repository.list().expect("list");
        assert!(listed.is_empty());
        let connection = repository.connection.lock().expect("database lock");
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM ai_messages", [], |row| row.get(0))
            .expect("count messages");
        assert_eq!(remaining, 0);
    }

    #[test]
    fn deleting_an_unknown_conversation_is_a_no_op() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.upsert(&conversation("conv-1")).expect("seed");
        repository.delete("conv-missing").expect("delete missing");
        assert_eq!(repository.list().expect("list").len(), 1);
    }

    #[test]
    fn schema_migration_creates_ai_conversation_tables() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        let connection = repository.connection.lock().expect("database lock");
        let table: Option<String> = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'ai_conversations'",
                [],
                |row| row.get(0),
            )
            .optional()
            .expect("query table");
        assert_eq!(table.as_deref(), Some("ai_conversations"));
    }
}
