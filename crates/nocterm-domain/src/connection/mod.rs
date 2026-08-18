use std::{error::Error, fmt};

/// 连接资料只描述如何定位主机，不保存密码或私钥明文。
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionProfile {
    pub id: i64,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: AuthenticationMethod,
    pub created_at: i64,
    pub updated_at: i64,
    pub group_id: Option<String>,
    pub group_name: Option<String>,
    pub remark: Option<String>,
    pub sync_mode: String,
    pub execution_target: String,
    pub remote_initial_path: Option<String>,
    pub icon: Option<String>,
    pub sort_order: Option<f64>,
    pub credential_kind: Option<String>,
    pub credential_status: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionGroup {
    pub id: String,
    pub name: String,
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewConnectionGroup {
    pub id: String,
    pub name: String,
    pub sort_order: Option<f64>,
}

impl NewConnectionGroup {
    pub fn try_new(
        id: impl Into<String>,
        name: impl Into<String>,
    ) -> Result<Self, ConnectionValidationError> {
        let id = id.into().trim().to_string();
        let name = name.into().trim().to_string();
        if id.is_empty() {
            return Err(ConnectionValidationError::new(
                "CONNECTION_GROUP_ID_REQUIRED",
                "分组标识不能为空",
            ));
        }
        if name.is_empty() {
            return Err(ConnectionValidationError::new(
                "CONNECTION_GROUP_NAME_REQUIRED",
                "请输入分组名称",
            ));
        }
        Ok(Self {
            id,
            name,
            sort_order: None,
        })
    }
}

/// 认证方式属于连接资料；真正的凭据内容由 Credential 领域独立管理。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthenticationMethod {
    Password,
    PrivateKey,
    SshAgent,
}

impl AuthenticationMethod {
    /// 返回可持久化的稳定代码，禁止直接保存展示文案。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::PrivateKey => "private_key",
            Self::SshAgent => "ssh_agent",
        }
    }

    /// 从持久化或 IPC 代码恢复领域枚举，未知值必须显式失败。
    pub fn parse(value: &str) -> Result<Self, ConnectionValidationError> {
        match value {
            "password" => Ok(Self::Password),
            "private_key" => Ok(Self::PrivateKey),
            "ssh_agent" => Ok(Self::SshAgent),
            _ => Err(ConnectionValidationError::new(
                "CONNECTION_AUTHENTICATION_INVALID",
                "请选择受支持的认证方式",
            )),
        }
    }
}

/// 创建参数在进入 Repository 前完成规范化，避免各存储实现重复业务校验。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewConnectionProfile {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: AuthenticationMethod,
    pub group_id: Option<String>,
    pub remark: Option<String>,
    pub remote_initial_path: Option<String>,
    pub icon: Option<String>,
}

/// 备份导入项保留来源 ID，仅用于在事务内关联无密钥的凭据元数据。
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionImportProfile {
    pub source_id: String,
    pub profile: NewConnectionProfile,
    pub sort_order: Option<f64>,
    pub credential_kind: Option<String>,
    pub credential_status: Option<String>,
}

/// 导入映射只服务同一批后续凭据绑定，避免 UI 猜测新建连接的数据库 ID。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedConnection {
    pub source_id: String,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionImportResult {
    pub groups: usize,
    pub connections: usize,
    pub credentials: usize,
    pub imported_connections: Vec<ImportedConnection>,
}

impl NewConnectionProfile {
    /// 在领域边界统一清理空白并验证必填项，Repository 仅承担持久化约束。
    pub fn try_new(
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        username: impl Into<String>,
        authentication: AuthenticationMethod,
    ) -> Result<Self, ConnectionValidationError> {
        let name = name.into().trim().to_string();
        let host = host.into().trim().to_string();
        let username = username.into().trim().to_string();

        if name.is_empty() {
            return Err(ConnectionValidationError::new(
                "CONNECTION_NAME_REQUIRED",
                "请输入连接名称",
            ));
        }
        if host.is_empty() {
            return Err(ConnectionValidationError::new(
                "CONNECTION_HOST_REQUIRED",
                "请输入主机地址",
            ));
        }
        if host.starts_with('-')
            || host
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(ConnectionValidationError::new(
                "CONNECTION_HOST_INVALID",
                "主机地址不能包含空白字符或以短横线开头",
            ));
        }
        if username.is_empty() {
            return Err(ConnectionValidationError::new(
                "CONNECTION_USERNAME_REQUIRED",
                "请输入用户名",
            ));
        }
        if username.starts_with('-')
            || username
                .chars()
                .any(|character| character.is_whitespace() || character.is_control())
        {
            return Err(ConnectionValidationError::new(
                "CONNECTION_USERNAME_INVALID",
                "用户名不能包含空白字符或以短横线开头",
            ));
        }
        if port == 0 {
            return Err(ConnectionValidationError::new(
                "CONNECTION_PORT_INVALID",
                "端口必须在 1 到 65535 之间",
            ));
        }

        Ok(Self {
            name,
            host,
            port,
            username,
            authentication,
            group_id: None,
            remark: None,
            remote_initial_path: None,
            icon: None,
        })
    }
}

/// 校验错误代码可直接映射为稳定的 Application 错误契约。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionValidationError {
    pub code: &'static str,
    pub message: &'static str,
}

impl ConnectionValidationError {
    const fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

impl fmt::Display for ConnectionValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for ConnectionValidationError {}

/// Repository 隐藏 SQLite 等基础设施细节，并为用例提供稳定失败边界。
pub trait ConnectionRepository: Send + Sync {
    fn list(&self) -> Result<Vec<ConnectionProfile>, ConnectionRepositoryError>;
    fn get(&self, id: i64) -> Result<Option<ConnectionProfile>, ConnectionRepositoryError>;
    fn create(
        &self,
        profile: NewConnectionProfile,
    ) -> Result<ConnectionProfile, ConnectionRepositoryError>;
    fn update(
        &self,
        id: i64,
        profile: NewConnectionProfile,
    ) -> Result<ConnectionProfile, ConnectionRepositoryError>;
    fn delete(&self, id: i64) -> Result<(), ConnectionRepositoryError>;
    fn list_groups(&self) -> Result<Vec<ConnectionGroup>, ConnectionRepositoryError>;
    fn upsert_group(
        &self,
        group: NewConnectionGroup,
    ) -> Result<ConnectionGroup, ConnectionRepositoryError>;
    fn delete_group(&self, id: &str) -> Result<(), ConnectionRepositoryError>;
    fn update_sort_order(
        &self,
        id: i64,
        group_id: Option<&str>,
        sort_order: f64,
    ) -> Result<ConnectionProfile, ConnectionRepositoryError>;
    fn upsert_credential_binding(
        &self,
        connection_id: &str,
        credential_kind: &str,
        credential_status: &str,
    ) -> Result<(), ConnectionRepositoryError>;
    fn delete_credential_binding(
        &self,
        connection_id: &str,
    ) -> Result<(), ConnectionRepositoryError>;
    /// 分组、连接和凭据元数据必须在同一事务内导入，失败时不留下半成品。
    fn import_backup(
        &self,
        groups: Vec<NewConnectionGroup>,
        profiles: Vec<ConnectionImportProfile>,
    ) -> Result<ConnectionImportResult, ConnectionRepositoryError>;
}

/// Repository 错误保留给 Application 转换，不允许直接跨 IPC 暴露。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionRepositoryError {
    pub message: String,
}

impl ConnectionRepositoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ConnectionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ConnectionRepositoryError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_connection_input() {
        let profile = NewConnectionProfile::try_new(
            "  Production  ",
            " server.example.com ",
            22,
            " deploy ",
            AuthenticationMethod::PrivateKey,
        )
        .expect("valid profile");

        assert_eq!(profile.name, "Production");
        assert_eq!(profile.host, "server.example.com");
        assert_eq!(profile.username, "deploy");
    }

    #[test]
    fn rejects_missing_required_fields() {
        let error =
            NewConnectionProfile::try_new(" ", "host", 22, "root", AuthenticationMethod::Password)
                .expect_err("blank name must fail");

        assert_eq!(error.code, "CONNECTION_NAME_REQUIRED");
    }

    #[test]
    fn rejects_ssh_argument_like_host_and_username() {
        let host_error = NewConnectionProfile::try_new(
            "Production",
            "-oProxyCommand=bad",
            22,
            "root",
            AuthenticationMethod::Password,
        )
        .expect_err("option-like host must fail");
        assert_eq!(host_error.code, "CONNECTION_HOST_INVALID");

        let username_error = NewConnectionProfile::try_new(
            "Production",
            "server.example.com",
            22,
            "deploy user",
            AuthenticationMethod::Password,
        )
        .expect_err("whitespace username must fail");
        assert_eq!(username_error.code, "CONNECTION_USERNAME_INVALID");
    }
}
