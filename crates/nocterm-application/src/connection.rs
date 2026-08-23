use std::sync::Arc;

use nocterm_domain::connection::{
    AuthenticationMethod, ConnectionGroup, ConnectionImportProfile, ConnectionImportResult,
    ConnectionProfile, ConnectionRepository, NewConnectionGroup, NewConnectionProfile,
};
use nocterm_domain::credential::CredentialStore;

use crate::error::AppError;

/// Application 输入不复用 IPC DTO，避免 Tauri 类型进入业务用例。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: String,
    pub group_id: Option<String>,
    pub remark: Option<String>,
    pub remote_initial_path: Option<String>,
    pub icon: Option<String>,
    /// 私钥文件路径；与密码不同，它是元数据而非密钥内容，可以随资料一起持久化。
    pub private_key_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImportConnection {
    pub source_id: String,
    pub connection: CreateConnection,
    pub sort_order: Option<f64>,
    pub credential_kind: Option<String>,
    pub credential_status: Option<String>,
}

/// 连接用例只依赖领域 Port，便于替换 SQLite 并独立验证业务行为。
#[derive(Clone)]
pub struct ConnectionService {
    repository: Arc<dyn ConnectionRepository>,
    credential_store: Option<Arc<dyn CredentialStore>>,
}

impl ConnectionService {
    pub fn new(repository: Arc<dyn ConnectionRepository>) -> Self {
        Self {
            repository,
            credential_store: None,
        }
    }

    /// 凭据依赖由装配层注入；纯连接用例和单测仍可只使用 Repository。
    pub fn with_credential_store(
        repository: Arc<dyn ConnectionRepository>,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            repository,
            credential_store: Some(credential_store),
        }
    }

    /// 列表读取失败时只暴露稳定错误，不把 SQL 或本机路径传递到 UI。
    pub fn list(&self) -> Result<Vec<ConnectionProfile>, AppError> {
        self.repository.list().map_err(|_| {
            AppError::new(
                "CONNECTION_LIST_FAILED",
                "读取连接列表失败，请稍后重试",
                true,
            )
        })
    }

    /// SSH 会话按本地连接主键读取资料，不接受前端重复传入主机参数。
    pub fn get(&self, id: i64) -> Result<ConnectionProfile, AppError> {
        self.repository
            .get(id)
            .map_err(|_| AppError::new("CONNECTION_READ_FAILED", "读取连接资料失败", true))?
            .ok_or_else(|| AppError::new("CONNECTION_NOT_FOUND", "连接资料不存在或已删除", false))
    }

    pub fn create(&self, input: CreateConnection) -> Result<ConnectionProfile, AppError> {
        let profile = build_profile(input)?;

        // 基础设施诊断不会越过此边界，UI 只接收安全且可重试的错误。
        let profile = self.repository.create(profile).map_err(|_| {
            AppError::new("CONNECTION_CREATE_FAILED", "保存连接失败，请稍后重试", true)
        })?;
        Ok(profile)
    }

    pub fn update(&self, id: i64, input: CreateConnection) -> Result<ConnectionProfile, AppError> {
        let profile = build_profile(input)?;
        self.repository.update(id, profile).map_err(|_| {
            AppError::new("CONNECTION_UPDATE_FAILED", "更新连接失败，请稍后重试", true)
        })
    }

    /// 将资料变更与系统凭据切换作为一个可补偿用例，避免 UI 多次 IPC 造成半成品。
    pub fn update_with_credential(
        &self,
        id: i64,
        input: CreateConnection,
        credential: Option<CredentialInput>,
    ) -> Result<ConnectionProfile, AppError> {
        let previous = self.get(id)?;
        let next = build_profile(input)?;
        let store = self.credential_store()?;
        validate_credential_input(&next, credential.as_ref())?;

        let next_kind = credential.as_ref().map(|value| value.kind.as_str());
        let previous_secret = next_kind.and_then(|kind| store.read(&id.to_string(), kind).ok());
        if let Some(credential) = credential.as_ref() {
            store
                .store(&id.to_string(), &credential.kind, &credential.secret)
                .map_err(store_credential_error)?;
        }

        let updated = match self.repository.update(id, next.clone()) {
            Ok(updated) => updated,
            Err(_) => {
                restore_credential(store.as_ref(), id, next_kind, previous_secret.as_deref());
                return Err(AppError::new(
                    "CONNECTION_UPDATE_FAILED",
                    "更新连接失败，请稍后重试",
                    true,
                ));
            }
        };

        let binding = credential
            .as_ref()
            .map(|value| (value.kind.as_str(), "bound"))
            .or_else(|| {
                (previous.authentication != updated.authentication).then_some(
                    match updated.authentication {
                        AuthenticationMethod::SshAgent => ("ssh_agent", "bound"),
                        _ => ("", "missing"),
                    },
                )
            });
        let binding_result = match binding {
            Some(("", _)) => self.repository.delete_credential_binding(&id.to_string()),
            Some((kind, status)) => {
                self.repository
                    .upsert_credential_binding(&id.to_string(), kind, status)
            }
            None => Ok(()),
        };
        if binding_result.is_err() {
            let _ = self.repository.update(id, profile_as_new(&previous));
            restore_credential(store.as_ref(), id, next_kind, previous_secret.as_deref());
            return Err(credential_error(
                "CREDENTIAL_BIND_FAILED",
                "保存凭据状态失败",
                true,
            ));
        }

        if previous.authentication != updated.authentication
            && let Some(previous_kind) = secret_credential_kind(previous.authentication)
            && store.delete(&id.to_string(), previous_kind).is_err()
        {
            let _ = restore_binding(&*self.repository, &previous);
            let _ = self.repository.update(id, profile_as_new(&previous));
            restore_credential(store.as_ref(), id, next_kind, previous_secret.as_deref());
            return Err(credential_error(
                "CREDENTIAL_DELETE_FAILED",
                "清理旧系统凭据失败",
                true,
            ));
        }
        Ok(updated)
    }

    /// 绑定私钥文件：只记录路径，不复制密钥内容。
    ///
    /// 这与 OpenSSH 的 `IdentityFile` 同义，也是 PuTTY、Xshell、MobaXterm 的共同做法——
    /// 没有任何主流客户端把私钥字节塞进系统凭据库。Windows 上更是硬约束：
    /// 凭据管理器单条 blob 上限 2560 字节，2048 位 RSA 私钥（PEM 约 1.7 KB，
    /// UTF-16 编码后翻倍）必然被拒。路径引用还顺带避免了密钥出现第二份副本。
    ///
    /// 路径的可读性由调用方（Tauri 命令层）在绑定前校验，用例层不做文件 I/O。
    pub fn bind_private_key_path(
        &self,
        connection_id: i64,
        path: &str,
    ) -> Result<ConnectionProfile, AppError> {
        let profile = self.get(connection_id)?;
        if profile.authentication != AuthenticationMethod::PrivateKey {
            return Err(credential_error(
                "CREDENTIAL_KIND_INVALID",
                "凭据类型与连接认证方式不匹配",
                false,
            ));
        }
        let path = normalize_private_key_path(Some(path.to_string()))
            .ok_or_else(|| credential_error("CREDENTIAL_EMPTY", "私钥文件路径不能为空", false))?;
        let mut next = profile_as_new(&profile);
        next.private_key_path = Some(path);
        self.repository.update(connection_id, next).map_err(|_| {
            AppError::new("CONNECTION_UPDATE_FAILED", "更新连接失败，请稍后重试", true)
        })
    }

    pub fn store_credential(
        &self,
        connection_id: i64,
        credential: CredentialInput,
    ) -> Result<(), AppError> {
        let profile = self.get(connection_id)?;
        validate_credential_input_for_authentication(profile.authentication, &credential)?;
        let store = self.credential_store()?;
        let prior = store
            .read(&connection_id.to_string(), &credential.kind)
            .ok();
        store
            .store(
                &connection_id.to_string(),
                &credential.kind,
                &credential.secret,
            )
            .map_err(store_credential_error)?;
        if self
            .repository
            .upsert_credential_binding(&connection_id.to_string(), &credential.kind, "bound")
            .is_err()
        {
            restore_credential(
                store.as_ref(),
                connection_id,
                Some(credential.kind.as_str()),
                prior.as_deref(),
            );
            return Err(credential_error(
                "CREDENTIAL_BIND_FAILED",
                "保存凭据状态失败",
                true,
            ));
        }
        Ok(())
    }

    pub fn read_credential(&self, connection_id: i64, kind: &str) -> Result<String, AppError> {
        let profile = self.get(connection_id)?;
        if secret_credential_kind(profile.authentication) != Some(kind) {
            return Err(credential_error(
                "CREDENTIAL_KIND_INVALID",
                "凭据类型与连接认证方式不匹配",
                false,
            ));
        }
        self.credential_store()?
            .read(&connection_id.to_string(), kind)
            .map_err(|_| credential_error("CREDENTIAL_READ_FAILED", "读取系统凭据失败", true))
    }

    pub fn delete_credential(&self, connection_id: i64, kind: &str) -> Result<(), AppError> {
        let store = self.credential_store()?;
        let previous = store.read(&connection_id.to_string(), kind).ok();
        store
            .delete(&connection_id.to_string(), kind)
            .map_err(|_| credential_error("CREDENTIAL_DELETE_FAILED", "删除系统凭据失败", true))?;
        if self
            .repository
            .delete_credential_binding(&connection_id.to_string())
            .is_err()
        {
            if let Some(secret) = previous {
                let _ = store.store(&connection_id.to_string(), kind, &secret);
            }
            return Err(credential_error(
                "CREDENTIAL_DELETE_FAILED",
                "删除凭据状态失败",
                true,
            ));
        }
        Ok(())
    }

    pub fn delete_with_credentials(&self, id: i64) -> Result<(), AppError> {
        let store = self.credential_store()?;
        let mut removed = Vec::new();
        for kind in ["password", "private_key"] {
            if let Ok(secret) = store.read(&id.to_string(), kind) {
                store.delete(&id.to_string(), kind).map_err(|_| {
                    credential_error("CREDENTIAL_DELETE_FAILED", "删除系统凭据失败", true)
                })?;
                removed.push((kind, secret));
            }
        }
        if let Err(error) = self.delete(id) {
            for (kind, secret) in removed {
                let _ = store.store(&id.to_string(), kind, &secret);
            }
            return Err(error);
        }
        Ok(())
    }

    pub fn sync_ssh_agent(
        &self,
        id: i64,
        authentication: AuthenticationMethod,
    ) -> Result<(), AppError> {
        if authentication != AuthenticationMethod::SshAgent {
            return Ok(());
        }
        self.repository
            .upsert_credential_binding(&id.to_string(), "ssh_agent", "bound")
            .map_err(|_| {
                credential_error("CREDENTIAL_BIND_FAILED", "保存 SSH Agent 状态失败", true)
            })
    }

    fn credential_store(&self) -> Result<&Arc<dyn CredentialStore>, AppError> {
        self.credential_store.as_ref().ok_or_else(|| {
            credential_error("CREDENTIAL_STORE_UNAVAILABLE", "系统凭据服务不可用", true)
        })
    }

    pub fn delete(&self, id: i64) -> Result<(), AppError> {
        self.repository.delete(id).map_err(|_| {
            AppError::new("CONNECTION_DELETE_FAILED", "删除连接失败，请稍后重试", true)
        })
    }

    pub fn list_groups(&self) -> Result<Vec<ConnectionGroup>, AppError> {
        self.repository.list_groups().map_err(|_| {
            AppError::new(
                "CONNECTION_GROUP_LIST_FAILED",
                "读取连接分组失败，请稍后重试",
                true,
            )
        })
    }

    pub fn upsert_group(&self, group: NewConnectionGroup) -> Result<ConnectionGroup, AppError> {
        self.repository.upsert_group(group).map_err(|_| {
            AppError::new(
                "CONNECTION_GROUP_SAVE_FAILED",
                "保存连接分组失败，请稍后重试",
                true,
            )
        })
    }

    pub fn delete_group(&self, id: &str) -> Result<(), AppError> {
        self.repository.delete_group(id).map_err(|_| {
            AppError::new(
                "CONNECTION_GROUP_DELETE_FAILED",
                "删除连接分组失败，请稍后重试",
                true,
            )
        })
    }

    pub fn update_sort_order(
        &self,
        id: i64,
        group_id: Option<&str>,
        sort_order: f64,
    ) -> Result<ConnectionProfile, AppError> {
        self.repository
            .update_sort_order(id, group_id, sort_order)
            .map_err(|_| {
                AppError::new(
                    "CONNECTION_REORDER_FAILED",
                    "调整连接顺序失败，请稍后重试",
                    true,
                )
            })
    }

    /// 所有输入先经过领域校验，再交由 Repository 原子写入。
    pub fn import_backup(
        &self,
        groups: Vec<NewConnectionGroup>,
        connections: Vec<ImportConnection>,
    ) -> Result<ConnectionImportResult, AppError> {
        let profiles = connections
            .into_iter()
            .map(|connection| {
                let profile = build_profile(connection.connection)?;
                validate_credential_metadata(
                    connection.credential_kind.as_deref(),
                    connection.credential_status.as_deref(),
                )?;
                Ok(ConnectionImportProfile {
                    source_id: connection.source_id,
                    profile,
                    sort_order: connection.sort_order,
                    credential_kind: connection.credential_kind,
                    credential_status: connection.credential_status,
                })
            })
            .collect::<Result<Vec<_>, AppError>>()?;

        self.repository
            .import_backup(groups, profiles)
            .map_err(|_| {
                AppError::new(
                    "CONNECTION_BACKUP_IMPORT_FAILED",
                    "恢复连接备份失败，未写入任何数据",
                    true,
                )
            })
    }
}

/// IPC 输入只在 Application 内短暂存在，禁止落入连接资料或日志。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialInput {
    pub kind: String,
    pub secret: String,
}

fn validate_credential_input(
    profile: &NewConnectionProfile,
    credential: Option<&CredentialInput>,
) -> Result<(), AppError> {
    if let Some(credential) = credential {
        validate_credential_input_for_authentication(profile.authentication, credential)?;
    }
    Ok(())
}

fn validate_credential_input_for_authentication(
    authentication: AuthenticationMethod,
    credential: &CredentialInput,
) -> Result<(), AppError> {
    if credential.secret.is_empty() {
        return Err(credential_error(
            "CREDENTIAL_EMPTY",
            "凭据内容不能为空",
            false,
        ));
    }
    // 私钥内容不再写入系统凭据库：Windows 凭据管理器单条 blob 上限 2560 字节，
    // 常见 RSA 私钥（2048 位 PEM 约 1.7 KB，UTF-16 编码后翻倍）必然被拒。
    // 私钥统一按文件路径引用（`bind_private_key_path`），与 OpenSSH `IdentityFile` 一致。
    // 遗留条目仍可读取与删除，只是不再新增。
    if credential.kind == "private_key" {
        return Err(credential_error(
            "CREDENTIAL_KIND_INVALID",
            "私钥不写入系统凭据库，请改为绑定私钥文件",
            false,
        ));
    }
    if secret_credential_kind(authentication) != Some(credential.kind.as_str()) {
        return Err(credential_error(
            "CREDENTIAL_KIND_INVALID",
            "凭据类型与连接认证方式不匹配",
            false,
        ));
    }
    Ok(())
}

fn secret_credential_kind(authentication: AuthenticationMethod) -> Option<&'static str> {
    match authentication {
        AuthenticationMethod::Password => Some("password"),
        AuthenticationMethod::PrivateKey => Some("private_key"),
        AuthenticationMethod::SshAgent => None,
    }
}

fn profile_as_new(profile: &ConnectionProfile) -> NewConnectionProfile {
    NewConnectionProfile {
        name: profile.name.clone(),
        host: profile.host.clone(),
        port: profile.port,
        username: profile.username.clone(),
        authentication: profile.authentication,
        group_id: profile.group_id.clone(),
        remark: profile.remark.clone(),
        remote_initial_path: profile.remote_initial_path.clone(),
        icon: profile.icon.clone(),
        private_key_path: profile.private_key_path.clone(),
    }
}

fn restore_binding(
    repository: &dyn ConnectionRepository,
    profile: &ConnectionProfile,
) -> Result<(), nocterm_domain::connection::ConnectionRepositoryError> {
    match profile.credential_kind.as_deref() {
        Some(kind) => repository.upsert_credential_binding(
            &profile.id.to_string(),
            kind,
            &profile.credential_status,
        ),
        None => repository.delete_credential_binding(&profile.id.to_string()),
    }
}

fn restore_credential(
    store: &dyn CredentialStore,
    connection_id: i64,
    kind: Option<&str>,
    previous: Option<&str>,
) {
    let Some(kind) = kind else {
        return;
    };
    match previous {
        Some(secret) => {
            let _ = store.store(&connection_id.to_string(), kind, secret);
        }
        None => {
            let _ = store.delete(&connection_id.to_string(), kind);
        }
    }
}

fn credential_error(code: &'static str, message: &'static str, retryable: bool) -> AppError {
    AppError::new(code, message, retryable)
}

/// 平台写入失败的原因必须带到 UI：长度超限、权限不足和存储不可用的处置方式完全不同，
/// 统一折叠成"保存系统凭据失败"会让用户和排查者都无从下手。
/// 适配器只回传平台诊断文本，不包含凭据本身，因此可以安全跨 IPC。
fn store_credential_error(source: String) -> AppError {
    AppError::new(
        "CREDENTIAL_STORE_FAILED",
        format!("保存系统凭据失败：{source}"),
        true,
    )
}

fn validate_credential_metadata(kind: Option<&str>, status: Option<&str>) -> Result<(), AppError> {
    if kind.is_some_and(|value| !matches!(value, "password" | "private_key" | "ssh_agent")) {
        return Err(AppError::new(
            "CONNECTION_BACKUP_CREDENTIAL_INVALID",
            "备份文件包含无效的凭据类型",
            false,
        ));
    }
    if status.is_some_and(|value| !matches!(value, "missing" | "bound" | "metadata_only")) {
        return Err(AppError::new(
            "CONNECTION_BACKUP_CREDENTIAL_INVALID",
            "备份文件包含无效的凭据状态",
            false,
        ));
    }
    Ok(())
}

fn build_profile(input: CreateConnection) -> Result<NewConnectionProfile, AppError> {
    let authentication = AuthenticationMethod::parse(&input.authentication)
        .map_err(|error| AppError::new(error.code, error.message, false))?;
    let mut profile = NewConnectionProfile::try_new(
        input.name,
        input.host,
        input.port,
        input.username,
        authentication,
    )
    .map_err(|error| AppError::new(error.code, error.message, false))?;
    profile.group_id = input.group_id;
    profile.remark = input.remark;
    profile.remote_initial_path = input.remote_initial_path;
    profile.icon = input.icon;
    // 只有私钥登录才保留路径：切换认证方式后残留的旧路径会让"是否已绑定私钥"的判断失真。
    profile.private_key_path = match authentication {
        AuthenticationMethod::PrivateKey => normalize_private_key_path(input.private_key_path),
        _ => None,
    };
    Ok(profile)
}

/// 空串与纯空白等同于"未绑定"，避免把用户清空后的输入框存成一个不存在的路径。
fn normalize_private_key_path(path: Option<String>) -> Option<String> {
    path.map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Mutex};

    use nocterm_domain::connection::ConnectionRepositoryError;
    use nocterm_domain::credential::CredentialStore;

    use super::*;

    #[derive(Default)]
    struct FakeRepository {
        profiles: Mutex<Vec<ConnectionProfile>>,
        fail_binding: bool,
    }

    #[derive(Default)]
    struct FakeCredentialStore {
        values: Mutex<HashMap<(String, String), String>>,
    }

    impl CredentialStore for FakeCredentialStore {
        fn store(&self, connection_id: &str, kind: &str, secret: &str) -> Result<(), String> {
            self.values.lock().expect("credential lock").insert(
                (connection_id.to_string(), kind.to_string()),
                secret.to_string(),
            );
            Ok(())
        }

        fn read(&self, connection_id: &str, kind: &str) -> Result<String, String> {
            self.values
                .lock()
                .expect("credential lock")
                .get(&(connection_id.to_string(), kind.to_string()))
                .cloned()
                .ok_or_else(|| "credential missing".to_string())
        }

        fn delete(&self, connection_id: &str, kind: &str) -> Result<(), String> {
            self.values
                .lock()
                .expect("credential lock")
                .remove(&(connection_id.to_string(), kind.to_string()));
            Ok(())
        }
    }

    impl ConnectionRepository for FakeRepository {
        fn list(&self) -> Result<Vec<ConnectionProfile>, ConnectionRepositoryError> {
            Ok(self.profiles.lock().expect("profiles lock").clone())
        }

        fn create(
            &self,
            profile: NewConnectionProfile,
        ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
            let mut profiles = self.profiles.lock().expect("profiles lock");
            let saved = ConnectionProfile {
                id: 1,
                name: profile.name,
                host: profile.host,
                port: profile.port,
                username: profile.username,
                authentication: profile.authentication,
                group_id: profile.group_id,
                remark: profile.remark,
                remote_initial_path: profile.remote_initial_path,
                icon: profile.icon,
                private_key_path: profile.private_key_path,
                created_at: 1,
                updated_at: 1,
                group_name: None,
                sync_mode: "local_only".to_string(),
                execution_target: "remote_terminal".to_string(),
                sort_order: None,
                credential_kind: None,
                credential_status: "missing".to_string(),
            };
            profiles.push(saved.clone());
            Ok(saved)
        }

        fn get(&self, id: i64) -> Result<Option<ConnectionProfile>, ConnectionRepositoryError> {
            Ok(self
                .profiles
                .lock()
                .expect("profiles lock")
                .iter()
                .find(|profile| profile.id == id)
                .cloned())
        }

        fn update(
            &self,
            id: i64,
            profile: NewConnectionProfile,
        ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
            let mut profiles = self.profiles.lock().expect("profiles lock");
            let current = profiles
                .iter_mut()
                .find(|item| item.id == id)
                .ok_or_else(|| ConnectionRepositoryError::new("not found"))?;
            current.name = profile.name;
            current.host = profile.host;
            current.port = profile.port;
            current.username = profile.username;
            current.authentication = profile.authentication;
            current.group_id = profile.group_id;
            current.remark = profile.remark;
            current.remote_initial_path = profile.remote_initial_path;
            current.icon = profile.icon;
            current.private_key_path = profile.private_key_path;
            Ok(current.clone())
        }

        fn delete(&self, id: i64) -> Result<(), ConnectionRepositoryError> {
            let mut profiles = self.profiles.lock().expect("profiles lock");
            let before = profiles.len();
            profiles.retain(|item| item.id != id);
            if profiles.len() == before {
                return Err(ConnectionRepositoryError::new("not found"));
            }
            Ok(())
        }

        fn list_groups(&self) -> Result<Vec<ConnectionGroup>, ConnectionRepositoryError> {
            Ok(Vec::new())
        }

        fn upsert_group(
            &self,
            group: NewConnectionGroup,
        ) -> Result<ConnectionGroup, ConnectionRepositoryError> {
            Ok(ConnectionGroup {
                id: group.id,
                name: group.name,
                sort_order: group.sort_order,
            })
        }

        fn delete_group(&self, _id: &str) -> Result<(), ConnectionRepositoryError> {
            Ok(())
        }

        fn update_sort_order(
            &self,
            id: i64,
            group_id: Option<&str>,
            sort_order: f64,
        ) -> Result<ConnectionProfile, ConnectionRepositoryError> {
            self.get(id)?
                .map(|mut profile| {
                    profile.group_id = group_id.map(str::to_string);
                    profile.sort_order = Some(sort_order);
                    profile
                })
                .ok_or_else(|| ConnectionRepositoryError::new("not found"))
        }

        fn upsert_credential_binding(
            &self,
            _connection_id: &str,
            _credential_kind: &str,
            _credential_status: &str,
        ) -> Result<(), ConnectionRepositoryError> {
            if self.fail_binding {
                return Err(ConnectionRepositoryError::new("binding unavailable"));
            }
            Ok(())
        }

        fn delete_credential_binding(
            &self,
            _connection_id: &str,
        ) -> Result<(), ConnectionRepositoryError> {
            Ok(())
        }

        fn import_backup(
            &self,
            groups: Vec<NewConnectionGroup>,
            profiles: Vec<ConnectionImportProfile>,
        ) -> Result<ConnectionImportResult, ConnectionRepositoryError> {
            let group_count = groups.len();
            let connection_count = profiles.len();
            let credentials = profiles
                .iter()
                .filter(|profile| profile.credential_kind.is_some())
                .count();
            for profile in profiles {
                self.create(profile.profile)?;
            }
            Ok(ConnectionImportResult {
                groups: group_count,
                connections: connection_count,
                credentials,
                imported_connections: Vec::new(),
            })
        }
    }

    #[test]
    fn creates_and_lists_a_connection() {
        let service = ConnectionService::new(Arc::new(FakeRepository::default()));
        let created = service
            .create(CreateConnection {
                name: "Production".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "private_key".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: None,
            })
            .expect("create connection");

        assert_eq!(created.name, "Production");
        assert_eq!(service.list().expect("list connections"), vec![created]);
    }

    #[test]
    fn returns_stable_validation_error() {
        let service = ConnectionService::new(Arc::new(FakeRepository::default()));
        let error = service
            .create(CreateConnection {
                name: "".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "password".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: None,
            })
            .expect_err("blank name must fail");

        assert_eq!(error.code, "CONNECTION_NAME_REQUIRED");
        assert!(!error.retryable);
    }

    #[test]
    fn restores_new_secret_and_profile_when_binding_update_fails() {
        let repository = Arc::new(FakeRepository {
            fail_binding: true,
            ..Default::default()
        });
        let credentials = Arc::new(FakeCredentialStore::default());
        let service = ConnectionService::with_credential_store(repository, credentials.clone());
        let created = service
            .create(CreateConnection {
                name: "Production".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "private_key".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: None,
            })
            .expect("create connection");

        // 用"私钥切密码"而非反向，是因为私钥内容已不再允许写入系统凭据库；
        // 补偿逻辑本身与凭据类型无关，密码同样能覆盖"写密钥成功但绑定状态失败"的路径。
        let error = service
            .update_with_credential(
                created.id,
                CreateConnection {
                    authentication: "password".to_string(),
                    name: created.name.clone(),
                    host: created.host.clone(),
                    port: created.port,
                    username: created.username.clone(),
                    group_id: None,
                    remark: None,
                    remote_initial_path: None,
                    icon: None,
                    private_key_path: None,
                },
                Some(CredentialInput {
                    kind: "password".to_string(),
                    secret: "s3cret".to_string(),
                }),
            )
            .expect_err("binding failure must compensate");

        assert_eq!(error.code, "CREDENTIAL_BIND_FAILED");
        assert_eq!(
            service
                .get(created.id)
                .expect("read connection")
                .authentication,
            AuthenticationMethod::PrivateKey
        );
        assert!(credentials.read("1", "password").is_err());
    }

    #[test]
    fn refuses_to_write_private_key_content_into_the_credential_store() {
        let credentials = Arc::new(FakeCredentialStore::default());
        let service = ConnectionService::with_credential_store(
            Arc::new(FakeRepository::default()),
            credentials.clone(),
        );
        let created = service
            .create(CreateConnection {
                name: "Production".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "private_key".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: None,
            })
            .expect("create connection");

        let error = service
            .store_credential(
                created.id,
                CredentialInput {
                    kind: "private_key".to_string(),
                    secret: "-----BEGIN OPENSSH PRIVATE KEY-----".to_string(),
                },
            )
            .expect_err("private key content must be rejected");

        assert_eq!(error.code, "CREDENTIAL_KIND_INVALID");
        // 拒绝必须发生在触碰平台存储之前，否则超长写入仍会在 Windows 上炸掉。
        assert!(credentials.read("1", "private_key").is_err());
    }

    #[test]
    fn binds_a_private_key_by_path_without_storing_its_content() {
        let credentials = Arc::new(FakeCredentialStore::default());
        let service = ConnectionService::with_credential_store(
            Arc::new(FakeRepository::default()),
            credentials.clone(),
        );
        let created = service
            .create(CreateConnection {
                name: "Production".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "private_key".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: None,
            })
            .expect("create connection");

        let bound = service
            .bind_private_key_path(created.id, "  D:\\keys\\deploy.pem  ")
            .expect("bind private key path");

        // 路径两端的空白来自用户粘贴，落库前必须规整，否则后续读文件会失败。
        assert_eq!(
            bound.private_key_path.as_deref(),
            Some("D:\\keys\\deploy.pem")
        );
        assert!(credentials.read("1", "private_key").is_err());
    }

    #[test]
    fn refuses_to_bind_a_private_key_path_for_password_login() {
        let service = ConnectionService::with_credential_store(
            Arc::new(FakeRepository::default()),
            Arc::new(FakeCredentialStore::default()),
        );
        let created = service
            .create(CreateConnection {
                name: "Production".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "password".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: None,
            })
            .expect("create connection");

        let error = service
            .bind_private_key_path(created.id, "D:\\keys\\deploy.pem")
            .expect_err("password login has no private key to bind");

        assert_eq!(error.code, "CREDENTIAL_KIND_INVALID");
    }

    #[test]
    fn drops_a_stale_private_key_path_when_switching_away_from_key_login() {
        let service = ConnectionService::new(Arc::new(FakeRepository::default()));
        // 认证方式改成密码后仍保留旧路径，会让"是否已绑定私钥"的判断长期失真。
        let created = service
            .create(CreateConnection {
                name: "Production".to_string(),
                host: "server.example.com".to_string(),
                port: 22,
                username: "deploy".to_string(),
                authentication: "password".to_string(),
                group_id: None,
                remark: None,
                remote_initial_path: None,
                icon: None,
                private_key_path: Some("D:\\keys\\deploy.pem".to_string()),
            })
            .expect("create connection");

        assert_eq!(created.private_key_path, None);
    }
}
