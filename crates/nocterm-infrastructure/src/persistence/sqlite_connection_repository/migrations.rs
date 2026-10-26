//! SQLite 数据库迁移系统：统一启用 WAL/外键/等待策略，按版本化增量把库结构
//! 升级到 `SCHEMA_VERSION`。迁移在单个事务内原子执行，幂等可重放；版本超前
//! 直接拒绝，避免新库被旧程序打开后损坏。

use std::path::Path;

use nocterm_domain::connection::ConnectionRepositoryError;
use rusqlite::{Connection, OptionalExtension};

/// 当前数据库结构版本；每次结构变更递增并追加一段 `current_version < N` 增量。
pub(super) const SCHEMA_VERSION: i64 = 8;

/// 打开应用数据库并在提供服务前原子执行所有待处理迁移。
pub(super) fn open_database(path: &Path) -> Result<Connection, ConnectionRepositoryError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(repository_error)?;
    }

    let mut connection = Connection::open(path).map_err(repository_error)?;
    configure_connection(&connection)?;
    migrate(&mut connection)?;

    Ok(connection)
}

/// 所有连接统一启用约束和等待策略，避免 Repository 各自形成不同数据库语义。
pub(super) fn configure_connection(
    connection: &Connection,
) -> Result<(), ConnectionRepositoryError> {
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(repository_error)
}

/// 按版本化增量把数据库结构升级到 `SCHEMA_VERSION`；整个迁移在单个事务内完成。
pub(super) fn migrate(connection: &mut Connection) -> Result<(), ConnectionRepositoryError> {
    let transaction = connection.transaction().map_err(repository_error)?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at INTEGER NOT NULL DEFAULT (unixepoch())
             );",
        )
        .map_err(repository_error)?;

    let current_version = transaction
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get::<_, Option<i64>>(0)
        })
        .optional()
        .map_err(repository_error)?
        .flatten()
        .unwrap_or(0);

    if current_version > SCHEMA_VERSION {
        return Err(ConnectionRepositoryError::new(format!(
            "database schema version {current_version} is newer than supported {SCHEMA_VERSION}"
        )));
    }

    if current_version < 1 {
        // v1 只包含阶段 1.1 所需字段，凭据、分组和排序由后续功能独立迁移。
        transaction
            .execute_batch(
                "CREATE TABLE connection_profiles (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name TEXT NOT NULL CHECK (length(trim(name)) > 0),
                     host TEXT NOT NULL CHECK (length(trim(host)) > 0),
                     port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
                     username TEXT NOT NULL CHECK (length(trim(username)) > 0),
                     authentication TEXT NOT NULL CHECK (
                         authentication IN ('password', 'private_key', 'ssh_agent')
                     ),
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO schema_migrations (version) VALUES (1);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 2 {
        transaction
            .execute_batch(
                "ALTER TABLE connection_profiles ADD COLUMN group_id TEXT;
             ALTER TABLE connection_profiles ADD COLUMN remark TEXT;
             ALTER TABLE connection_profiles ADD COLUMN remote_initial_path TEXT;
             ALTER TABLE connection_profiles ADD COLUMN icon TEXT;
             ALTER TABLE connection_profiles ADD COLUMN sort_order REAL;
             CREATE TABLE IF NOT EXISTS credential_bindings (
                 connection_id TEXT PRIMARY KEY NOT NULL,
                 credential_kind TEXT NOT NULL,
                 credential_status TEXT NOT NULL,
                 updated_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             INSERT INTO schema_migrations (version) VALUES (2);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 3 {
        transaction
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS connection_groups (
                     id TEXT PRIMARY KEY NOT NULL,
                     name TEXT NOT NULL CHECK (length(trim(name)) > 0),
                     sort_order REAL,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO schema_migrations (version) VALUES (3);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 4 {
        // 私钥改为按路径引用（等价于 OpenSSH 的 IdentityFile）：
        // Windows 凭据管理器单条 blob 上限 2560 字节，装不下常见 RSA 私钥，
        // 因此密钥字节留在用户文件里，库中只记录路径。
        transaction
            .execute_batch(
                "ALTER TABLE connection_profiles ADD COLUMN private_key_path TEXT;
                 INSERT INTO schema_migrations (version) VALUES (4);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 5 {
        // 设置采用稳定键值保存：新增外观选项不应继续改变连接资料表结构。
        // 值仍由 Domain/Application 解析和校验，SQLite 不承载展示文案。
        transaction
            .execute_batch(
                "CREATE TABLE app_settings (
                     key TEXT PRIMARY KEY NOT NULL CHECK (length(trim(key)) > 0),
                     value TEXT NOT NULL,
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO schema_migrations (version) VALUES (5);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 6 {
        // AI 审计是脱敏的不可变事件流：不保存命令、参数、输出、Token、主机或账号。
        // connection_id 不设外键，连接资料删除后仍需保留既有安全审计记录。
        transaction
            .execute_batch(
                "CREATE TABLE ai_tool_audit_events (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     occurred_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     provider TEXT NOT NULL CHECK (length(provider) BETWEEN 1 AND 64),
                     session_id TEXT NOT NULL CHECK (length(session_id) BETWEEN 1 AND 64),
                     target_kind TEXT NOT NULL CHECK (target_kind IN ('ssh', 'local')),
                     connection_id INTEGER,
                     tool_name TEXT NOT NULL CHECK (length(tool_name) BETWEEN 1 AND 64),
                     approval_state TEXT NOT NULL CHECK (
                         approval_state IN ('not_requested', 'not_required', 'requested', 'approved', 'rejected', 'timed_out')
                     ),
                     outcome TEXT NOT NULL CHECK (
                         outcome IN ('pending', 'succeeded', 'failed', 'denied')
                     ),
                     duration_ms INTEGER CHECK (duration_ms IS NULL OR duration_ms >= 0),
                     error_code TEXT CHECK (error_code IS NULL OR length(error_code) BETWEEN 1 AND 64),
                     CHECK (
                         (target_kind = 'ssh' AND connection_id IS NOT NULL AND connection_id > 0)
                         OR (target_kind = 'local' AND connection_id IS NULL)
                     )
                 );
                 CREATE INDEX idx_ai_tool_audit_occurred_at
                     ON ai_tool_audit_events (occurred_at);
                 CREATE INDEX idx_ai_tool_audit_connection_time
                     ON ai_tool_audit_events (connection_id, occurred_at);
                 INSERT INTO schema_migrations (version) VALUES (6);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 7 {
        // AI 会话历史是用户业务数据：时间戳沿用前端的毫秒精度，由调用方提供，
        // 数据库不做时区换算。消息随会话级联删除，清空历史不产生孤儿行。
        transaction
            .execute_batch(
                "CREATE TABLE ai_conversations (
                     id TEXT PRIMARY KEY NOT NULL CHECK (length(id) BETWEEN 1 AND 64),
                     title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 200),
                     provider TEXT NOT NULL CHECK (provider IN ('codex', 'claude-code', 'grok')),
                     command_policy TEXT CHECK (
                         command_policy IS NULL OR
                         command_policy IN ('deny_all', 'confirm_each', 'auto_safe', 'full_access')
                     ),
                     created_at_ms INTEGER NOT NULL CHECK (created_at_ms > 0),
                     updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms > 0)
                 );
                 CREATE TABLE ai_messages (
                     id TEXT PRIMARY KEY NOT NULL CHECK (length(id) BETWEEN 1 AND 64),
                     conversation_id TEXT NOT NULL REFERENCES ai_conversations (id) ON DELETE CASCADE,
                     role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
                     content TEXT NOT NULL CHECK (length(content) <= 262144),
                     parts_json TEXT CHECK (parts_json IS NULL OR length(parts_json) <= 262144),
                     created_at_ms INTEGER NOT NULL CHECK (created_at_ms > 0)
                 );
                 CREATE INDEX idx_ai_messages_conversation_time
                     ON ai_messages (conversation_id, created_at_ms);
                 INSERT INTO schema_migrations (version) VALUES (7);",
            )
            .map_err(repository_error)?;
    }

    if current_version < 8 {
        // 旧会话无法可靠推断原终端，新增列保持 NULL；前端将其作为未绑定只读历史，
        // 禁止在当前活动终端中静默续聊。目标字段不参与执行授权。
        transaction
            .execute_batch(
                "ALTER TABLE ai_conversations ADD COLUMN target_kind TEXT CHECK (
                     target_kind IS NULL OR target_kind IN ('local', 'ssh')
                 );
                 ALTER TABLE ai_conversations ADD COLUMN target_id TEXT CHECK (
                     target_id IS NULL OR length(target_id) BETWEEN 1 AND 128
                 );
                 ALTER TABLE ai_conversations ADD COLUMN target_label TEXT CHECK (
                     target_label IS NULL OR length(target_label) BETWEEN 1 AND 200
                 );
                 INSERT INTO schema_migrations (version) VALUES (8);",
            )
            .map_err(repository_error)?;
    }

    transaction.commit().map_err(repository_error)
}

/// 仓储层错误的统一包装：诊断信息只停留在基础设施边界，Application 负责转换。
pub(super) fn repository_error(error: impl std::fmt::Display) -> ConnectionRepositoryError {
    ConnectionRepositoryError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_is_idempotent() {
        let mut connection = Connection::open_in_memory().expect("open database");
        configure_connection(&connection).expect("configure");
        migrate(&mut connection).expect("first migration");
        migrate(&mut connection).expect("second migration");

        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read schema version");
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn fresh_database_creates_the_ai_audit_schema() {
        let mut connection = Connection::open_in_memory().expect("open database");
        configure_connection(&connection).expect("configure");
        migrate(&mut connection).expect("migrate fresh database");
        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read schema version");
        let table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name = 'ai_tool_audit_events'",
                [],
                |row| row.get(0),
            )
            .expect("read audit table");

        assert_eq!(version, 8);
        assert_eq!(table, "ai_tool_audit_events");
    }

    #[test]
    fn upgrades_a_v5_database_with_the_ai_audit_schema() {
        let mut connection = Connection::open_in_memory().expect("open database");
        configure_connection(&connection).expect("configure");
        // 先由正式迁移生成完整结构，再移除 v6 增量，避免只伪造版本号而漏掉旧表共存问题。
        migrate(&mut connection).expect("create current schema");
        connection
            .execute_batch(
                "DROP TABLE ai_messages;
                 DROP TABLE ai_conversations;
                 DROP TABLE ai_tool_audit_events;
                 DELETE FROM schema_migrations WHERE version >= 6;",
            )
            .expect("restore complete v5 schema");

        migrate(&mut connection).expect("migrate v5 database");

        let version: i64 = connection
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .expect("read schema version");
        let table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name = 'ai_tool_audit_events'",
                [],
                |row| row.get(0),
            )
            .expect("read audit table");

        assert_eq!(version, 8);
        assert_eq!(table, "ai_tool_audit_events");
    }

    #[test]
    fn upgrades_v7_ai_conversations_without_changing_existing_history() {
        let mut connection = Connection::open_in_memory().expect("open database");
        configure_connection(&connection).expect("configure");
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     applied_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE ai_conversations (
                     id TEXT PRIMARY KEY NOT NULL,
                     title TEXT NOT NULL,
                     provider TEXT NOT NULL,
                     command_policy TEXT,
                     created_at_ms INTEGER NOT NULL,
                     updated_at_ms INTEGER NOT NULL
                 );
                 CREATE TABLE ai_messages (
                     id TEXT PRIMARY KEY NOT NULL,
                     conversation_id TEXT NOT NULL REFERENCES ai_conversations (id) ON DELETE CASCADE,
                     role TEXT NOT NULL,
                     content TEXT NOT NULL,
                     parts_json TEXT,
                     created_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO schema_migrations (version) VALUES (7);
                 INSERT INTO ai_conversations
                     (id, title, provider, command_policy, created_at_ms, updated_at_ms)
                 VALUES ('legacy', '旧会话', 'codex', 'auto_safe', 1000, 2000);
                 INSERT INTO ai_messages
                     (id, conversation_id, role, content, parts_json, created_at_ms)
                 VALUES ('message-1', 'legacy', 'user', '保留这条消息', NULL, 1500);",
            )
            .expect("seed v7 schema and history");

        migrate(&mut connection).expect("migrate v7 database");

        let row: (String, Option<String>, Option<String>, Option<String>) = connection
            .query_row(
                "SELECT title, target_kind, target_id, target_label
                 FROM ai_conversations WHERE id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .expect("read migrated history");
        assert_eq!(row, ("旧会话".into(), None, None, None));
        let message: String = connection
            .query_row(
                "SELECT content FROM ai_messages WHERE conversation_id = 'legacy'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated message");
        assert_eq!(message, "保留这条消息");
    }

    #[test]
    fn upgrades_a_pre_v4_database_by_adding_the_private_key_path_column() {
        let mut connection = Connection::open_in_memory().expect("open database");
        configure_connection(&connection).expect("configure");
        // 复刻 v3 结构：老库里没有 private_key_path，迁移必须补列而不是要求重建数据库。
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                     version INTEGER PRIMARY KEY,
                     applied_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE TABLE connection_profiles (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     name TEXT NOT NULL,
                     host TEXT NOT NULL,
                     port INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
                     username TEXT NOT NULL,
                     authentication TEXT NOT NULL,
                     created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     updated_at INTEGER NOT NULL DEFAULT (unixepoch()),
                     group_id TEXT,
                     remark TEXT,
                     sort_order REAL,
                     remote_initial_path TEXT,
                     icon TEXT
                 );
                 INSERT INTO schema_migrations (version) VALUES (1), (2), (3);
                 INSERT INTO connection_profiles (name, host, port, username, authentication)
                 VALUES ('Legacy', 'legacy.example.com', 22, 'deploy', 'private_key');",
            )
            .expect("seed legacy schema");

        migrate(&mut connection).expect("migrate legacy database");

        let path: Option<String> = connection
            .query_row(
                "SELECT private_key_path FROM connection_profiles WHERE name = 'Legacy'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated column");
        // 老连接迁移后路径为空，由 resolve_private_key 回落到凭据库里的遗留密钥。
        assert_eq!(path, None);

        let settings_table: String = connection
            .query_row(
                "SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'app_settings'",
                [],
                |row| row.get(0),
            )
            .expect("read migrated settings table");
        assert_eq!(settings_table, "app_settings");
    }
}
