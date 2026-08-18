use std::{path::Path, sync::Mutex};

use nocterm_domain::connection::{
    AuthenticationMethod, ConnectionGroup, ConnectionImportProfile, ConnectionImportResult,
    ConnectionProfile, ConnectionRepository, ConnectionRepositoryError, ImportedConnection,
    NewConnectionGroup, NewConnectionProfile,
};
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA_VERSION: i64 = 3;

const PROFILE_COLUMNS: &str = "connection_profiles.id, connection_profiles.name, connection_profiles.host, connection_profiles.port, connection_profiles.username, connection_profiles.authentication, connection_profiles.created_at, connection_profiles.updated_at,
    connection_profiles.group_id, connection_profiles.remark, connection_profiles.remote_initial_path, connection_profiles.icon, connection_profiles.sort_order,
    cg.name AS group_name,
    (SELECT credential_kind FROM credential_bindings WHERE connection_id = CAST(connection_profiles.id AS TEXT)) AS credential_kind,
    COALESCE((SELECT credential_status FROM credential_bindings WHERE connection_id = CAST(connection_profiles.id AS TEXT)), 'missing') AS credential_status";

/// 单进程桌面应用通过互斥连接串行访问 SQLite，避免把连接对象泄露给上层。
pub struct SqliteConnectionRepository {
    connection: Mutex<Connection>,
}

impl SqliteConnectionRepository {
    /// 打开应用数据库并在提供服务前原子执行所有待处理迁移。
    pub fn open(path: &Path) -> Result<Self, ConnectionRepositoryError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(repository_error)?;
        }

        let mut connection = Connection::open(path).map_err(repository_error)?;
        configure_connection(&connection)?;
        migrate(&mut connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn upsert_credential_binding(
        &self,
        connection_id: &str,
        credential_kind: &str,
        credential_status: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        connection
            .execute(
                "INSERT INTO credential_bindings (connection_id, credential_kind, credential_status)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(connection_id) DO UPDATE SET
                   credential_kind = excluded.credential_kind,
                   credential_status = excluded.credential_status,
                   updated_at = unixepoch()",
                params![connection_id, credential_kind, credential_status],
            )
            .map_err(repository_error)?;
        Ok(())
    }

    pub fn delete_credential_binding(
        &self,
        connection_id: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        connection
            .execute(
                "DELETE FROM credential_bindings WHERE connection_id = ?1",
                [connection_id],
            )
            .map_err(repository_error)?;
        Ok(())
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self, ConnectionRepositoryError> {
        let mut connection = Connection::open_in_memory().map_err(repository_error)?;
        configure_connection(&connection)?;
        migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }
}

impl ConnectionRepository for SqliteConnectionRepository {
    /// 固定排序保证新建连接在列表前方，时间相同时使用主键消除不确定性。
    fn list(&self) -> Result<Vec<ConnectionProfile>, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        let mut statement = connection
            .prepare(&format!(
                "SELECT {PROFILE_COLUMNS}
                 FROM connection_profiles
                 LEFT JOIN connection_groups cg ON cg.id = connection_profiles.group_id
                 ORDER BY connection_profiles.sort_order IS NULL, connection_profiles.sort_order ASC, connection_profiles.created_at DESC, connection_profiles.id DESC"
            ))
            .map_err(repository_error)?;
        let profiles = statement
            .query_map([], map_profile)
            .map_err(repository_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repository_error)?;

        Ok(profiles)
    }

    fn create(
        &self,
        profile: NewConnectionProfile,
    ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        // INSERT RETURNING 保证 UI 收到的对象与数据库实际默认时间戳完全一致。
        let profile = connection
            .query_row(
                "INSERT INTO connection_profiles
                 (name, host, port, username, authentication, group_id, remark, remote_initial_path, icon)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 RETURNING id, name, host, port, username, authentication, created_at, updated_at,
                 group_id, remark, remote_initial_path, icon, sort_order, NULL, NULL, 'missing'",
                params![
                    profile.name,
                    profile.host,
                    profile.port,
                    profile.username,
                    profile.authentication.as_str(),
                    profile.group_id,
                    profile.remark,
                    profile.remote_initial_path,
                    profile.icon,
                ],
                map_profile,
            )
            .map_err(repository_error)?;
        let id = profile.id;
        drop(connection);
        self.get(id)?
            .ok_or_else(|| ConnectionRepositoryError::new("connection not found"))
    }

    fn get(&self, id: i64) -> Result<Option<ConnectionProfile>, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        connection
            .query_row(
                &format!("SELECT {PROFILE_COLUMNS} FROM connection_profiles LEFT JOIN connection_groups cg ON cg.id = connection_profiles.group_id WHERE connection_profiles.id = ?1"),
                [id],
                map_profile,
            )
            .optional()
            .map_err(repository_error)
    }

    fn update(
        &self,
        id: i64,
        profile: NewConnectionProfile,
    ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        connection
            .execute(
                "UPDATE connection_profiles SET
                   name = ?1, host = ?2, port = ?3, username = ?4, authentication = ?5,
                   group_id = ?6, remark = ?7, remote_initial_path = ?8, icon = ?9,
                   updated_at = unixepoch()
                 WHERE id = ?10",
                params![
                    profile.name,
                    profile.host,
                    profile.port,
                    profile.username,
                    profile.authentication.as_str(),
                    profile.group_id,
                    profile.remark,
                    profile.remote_initial_path,
                    profile.icon,
                    id,
                ],
            )
            .map_err(repository_error)?;
        drop(connection);
        self.get(id)?
            .ok_or_else(|| ConnectionRepositoryError::new("connection not found"))
    }

    fn delete(&self, id: i64) -> Result<(), ConnectionRepositoryError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        let transaction = connection.transaction().map_err(repository_error)?;
        let deleted = transaction
            .execute("DELETE FROM connection_profiles WHERE id = ?1", [id])
            .map_err(repository_error)?;
        if deleted == 0 {
            return Err(ConnectionRepositoryError::new("connection not found"));
        }
        transaction
            .execute(
                "DELETE FROM credential_bindings WHERE connection_id = ?1",
                [id.to_string()],
            )
            .map_err(repository_error)?;
        transaction.commit().map_err(repository_error)
    }

    fn list_groups(&self) -> Result<Vec<ConnectionGroup>, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        let mut statement = connection
            .prepare(
                "SELECT id, name, sort_order FROM connection_groups
                 ORDER BY sort_order IS NULL, sort_order ASC, name COLLATE NOCASE ASC",
            )
            .map_err(repository_error)?;
        statement
            .query_map([], |row| {
                Ok(ConnectionGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    sort_order: row.get(2)?,
                })
            })
            .map_err(repository_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(repository_error)
    }

    fn upsert_group(
        &self,
        group: NewConnectionGroup,
    ) -> Result<ConnectionGroup, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        connection
            .execute(
                "INSERT INTO connection_groups (id, name, sort_order)
                 VALUES (?1, ?2, COALESCE(?3, (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM connection_groups)))
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, sort_order = COALESCE(excluded.sort_order, connection_groups.sort_order), updated_at = unixepoch()",
                params![group.id, group.name, group.sort_order],
            )
            .map_err(repository_error)?;
        connection
            .query_row(
                "SELECT id, name, sort_order FROM connection_groups WHERE id = ?1",
                [group.id],
                |row| {
                    Ok(ConnectionGroup {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        sort_order: row.get(2)?,
                    })
                },
            )
            .map_err(repository_error)
    }

    fn delete_group(&self, id: &str) -> Result<(), ConnectionRepositoryError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        let transaction = connection.transaction().map_err(repository_error)?;
        transaction
            .execute(
                "UPDATE connection_profiles SET group_id = NULL, updated_at = unixepoch() WHERE group_id = ?1",
                [id],
            )
            .map_err(repository_error)?;
        let deleted = transaction
            .execute("DELETE FROM connection_groups WHERE id = ?1", [id])
            .map_err(repository_error)?;
        if deleted == 0 {
            return Err(ConnectionRepositoryError::new("connection group not found"));
        }
        transaction.commit().map_err(repository_error)
    }

    fn update_sort_order(
        &self,
        id: i64,
        group_id: Option<&str>,
        sort_order: f64,
    ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        connection
            .execute(
                "UPDATE connection_profiles SET group_id = ?1, sort_order = ?2, updated_at = unixepoch() WHERE id = ?3",
                params![group_id, sort_order, id],
            )
            .map_err(repository_error)?;
        drop(connection);
        self.get(id)?
            .ok_or_else(|| ConnectionRepositoryError::new("connection not found"))
    }

    fn upsert_credential_binding(
        &self,
        connection_id: &str,
        credential_kind: &str,
        credential_status: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        Self::upsert_credential_binding(self, connection_id, credential_kind, credential_status)
    }

    fn delete_credential_binding(
        &self,
        connection_id: &str,
    ) -> Result<(), ConnectionRepositoryError> {
        Self::delete_credential_binding(self, connection_id)
    }

    fn import_backup(
        &self,
        groups: Vec<NewConnectionGroup>,
        profiles: Vec<ConnectionImportProfile>,
    ) -> Result<ConnectionImportResult, ConnectionRepositoryError> {
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| ConnectionRepositoryError::new("database lock poisoned"))?;
        let transaction = connection.transaction().map_err(repository_error)?;
        let group_count = groups.len();
        let connection_count = profiles.len();

        for group in groups {
            transaction
                .execute(
                    "INSERT INTO connection_groups (id, name, sort_order)
                     VALUES (?1, ?2, ?3)
                     ON CONFLICT(id) DO UPDATE SET
                       name = excluded.name,
                       sort_order = COALESCE(excluded.sort_order, connection_groups.sort_order),
                       updated_at = unixepoch()",
                    params![group.id, group.name, group.sort_order],
                )
                .map_err(repository_error)?;
        }

        let mut credential_count = 0;
        let mut imported_connections = Vec::with_capacity(connection_count);
        for imported in profiles {
            let source_id = imported.source_id;
            let profile = imported.profile;
            transaction
                .execute(
                    "INSERT INTO connection_profiles
                     (name, host, port, username, authentication, group_id, remark,
                      remote_initial_path, icon, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        profile.name,
                        profile.host,
                        profile.port,
                        profile.username,
                        profile.authentication.as_str(),
                        profile.group_id,
                        profile.remark,
                        profile.remote_initial_path,
                        profile.icon,
                        imported.sort_order,
                    ],
                )
                .map_err(repository_error)?;
            let connection_id = transaction.last_insert_rowid();
            imported_connections.push(ImportedConnection {
                source_id,
                id: connection_id,
            });

            if let (Some(kind), Some(status)) =
                (imported.credential_kind, imported.credential_status)
                && status != "missing"
            {
                // 备份不含 secret；仅 SSH Agent 的 bound 状态可以安全保留。
                let restored_status = if kind == "ssh_agent" && status == "bound" {
                    "bound"
                } else {
                    "metadata_only"
                };
                transaction
                    .execute(
                        "INSERT INTO credential_bindings
                         (connection_id, credential_kind, credential_status)
                         VALUES (?1, ?2, ?3)",
                        params![connection_id.to_string(), kind, restored_status],
                    )
                    .map_err(repository_error)?;
                credential_count += 1;
            }
        }

        transaction.commit().map_err(repository_error)?;
        Ok(ConnectionImportResult {
            groups: group_count,
            connections: connection_count,
            credentials: credential_count,
            imported_connections,
        })
    }
}

fn configure_connection(connection: &Connection) -> Result<(), ConnectionRepositoryError> {
    // 所有连接统一启用约束和等待策略，避免 Repository 各自形成不同数据库语义。
    connection
        .execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(repository_error)
}

fn migrate(connection: &mut Connection) -> Result<(), ConnectionRepositoryError> {
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

    transaction.commit().map_err(repository_error)
}

fn map_profile(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConnectionProfile> {
    let authentication = row.get::<_, String>(5)?;
    let authentication = AuthenticationMethod::parse(&authentication).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;

    Ok(ConnectionProfile {
        id: row.get(0)?,
        name: row.get(1)?,
        host: row.get(2)?,
        port: row.get(3)?,
        username: row.get(4)?,
        authentication,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        group_id: row.get(8)?,
        remark: row.get(9)?,
        remote_initial_path: row.get(10)?,
        icon: row.get(11)?,
        sort_order: row.get(12)?,
        group_name: row.get(13)?,
        credential_kind: row.get(14)?,
        credential_status: row.get(15)?,
        sync_mode: "local_only".to_string(),
        execution_target: "remote_terminal".to_string(),
    })
}

fn repository_error(error: impl std::fmt::Display) -> ConnectionRepositoryError {
    // 诊断信息只停留在基础设施边界；Application 会转换成面向 UI 的稳定错误。
    ConnectionRepositoryError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn draft(name: &str) -> NewConnectionProfile {
        NewConnectionProfile::try_new(
            name,
            "server.example.com",
            22,
            "deploy",
            AuthenticationMethod::PrivateKey,
        )
        .expect("valid draft")
    }

    #[test]
    fn creates_schema_and_persists_connections() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        let created = repository.create(draft("Production")).expect("create");
        let listed = repository.list().expect("list");

        assert!(created.id > 0);
        assert_eq!(listed, vec![created]);
    }

    #[test]
    fn persists_groups_and_connection_membership() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        let group = repository
            .upsert_group(NewConnectionGroup::try_new("production", "生产环境").expect("group"))
            .expect("save group");
        let mut profile = draft("Production");
        profile.group_id = Some(group.id.clone());
        let created = repository.create(profile).expect("create");

        assert_eq!(created.group_name.as_deref(), Some("生产环境"));
        assert_eq!(repository.list_groups().expect("list groups"), vec![group]);

        let reordered = repository
            .update_sort_order(created.id, None, 10.0)
            .expect("reorder");
        assert_eq!(reordered.group_id, None);
        repository.delete_group("production").expect("delete group");
        assert!(repository.list_groups().expect("list groups").is_empty());
    }

    #[test]
    fn persists_group_sort_order_updates() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        for (id, name, sort_order) in [
            ("first", "一组", 1000.0),
            ("second", "二组", 2000.0),
            ("third", "三组", 3000.0),
        ] {
            let mut group = NewConnectionGroup::try_new(id, name).expect("group");
            group.sort_order = Some(sort_order);
            repository.upsert_group(group).expect("save group");
        }

        let mut moved = NewConnectionGroup::try_new("third", "三组").expect("group");
        moved.sort_order = Some(1500.0);
        repository.upsert_group(moved).expect("move group");

        let ids = repository
            .list_groups()
            .expect("list groups")
            .into_iter()
            .map(|group| group.id)
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["first", "third", "second"]);
    }

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
    fn database_constraint_rejects_invalid_port() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        let error = repository
            .connection
            .lock()
            .expect("database lock")
            .execute(
                "INSERT INTO connection_profiles
                 (name, host, port, username, authentication)
                 VALUES ('Invalid', 'host', 70000, 'root', 'password')",
                [],
            )
            .expect_err("port constraint must fail");

        assert!(error.to_string().contains("CHECK constraint failed"));
    }

    #[test]
    fn connection_survives_database_reopen() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nocterm-connection-test-{}-{unique}.db",
            std::process::id()
        ));

        {
            let repository = SqliteConnectionRepository::open(&path).expect("open first time");
            repository.create(draft("Persistent")).expect("create");
        }
        let reopened = SqliteConnectionRepository::open(&path).expect("reopen database");
        let profiles = reopened.list().expect("list after reopen");

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Persistent");

        // 测试只清理自己创建的唯一临时数据库及 SQLite 辅助文件。
        drop(reopened);
        for suffix in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
        }
    }

    #[test]
    fn credential_binding_is_reflected_in_connection_list() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.create(draft("Credentialed")).expect("create");
        repository
            .upsert_credential_binding("1", "password", "bound")
            .expect("bind credential");

        let listed = repository.list().expect("list");
        assert_eq!(listed[0].credential_kind.as_deref(), Some("password"));
        assert_eq!(listed[0].credential_status, "bound");
    }

    #[test]
    fn deleting_connection_removes_its_credential_binding_atomically() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        repository.create(draft("Credentialed")).expect("create");
        repository
            .upsert_credential_binding("1", "private_key", "bound")
            .expect("bind credential");

        repository.delete(1).expect("delete connection");

        let binding_count: i64 = repository
            .connection
            .lock()
            .expect("database lock")
            .query_row(
                "SELECT COUNT(*) FROM credential_bindings WHERE connection_id = '1'",
                [],
                |row| row.get(0),
            )
            .expect("count bindings");
        assert_eq!(binding_count, 0);
    }

    #[test]
    fn imports_backup_in_one_transaction_and_downgrades_secret_credentials() {
        let repository = SqliteConnectionRepository::open_in_memory().expect("open database");
        let group = NewConnectionGroup::try_new("production", "生产环境").expect("group");
        let mut profile = draft("Imported");
        profile.group_id = Some("production".to_string());

        let result = repository
            .import_backup(
                vec![group],
                vec![ConnectionImportProfile {
                    source_id: "legacy-1".to_string(),
                    profile,
                    sort_order: Some(10.0),
                    credential_kind: Some("private_key".to_string()),
                    credential_status: Some("bound".to_string()),
                }],
            )
            .expect("import backup");
        let imported = repository.list().expect("list profiles");

        assert_eq!(result.connections, 1);
        assert_eq!(imported[0].group_name.as_deref(), Some("生产环境"));
        assert_eq!(imported[0].credential_kind.as_deref(), Some("private_key"));
        assert_eq!(imported[0].credential_status, "metadata_only");
    }
}
