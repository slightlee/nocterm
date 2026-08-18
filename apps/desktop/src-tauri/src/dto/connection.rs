use nocterm_application::connection::CreateConnection;
use nocterm_domain::connection::{ConnectionGroup, ConnectionProfile};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionCreateRequest {
    #[serde(default)]
    pub id: Option<i64>,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub remote_initial_path: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
}

/// 凭据只随单次写入请求传递，连接资料 DTO 永不包含明文。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialInputRequest {
    pub kind: String,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub private_key_path: Option<String>,
}

impl From<ConnectionCreateRequest> for CreateConnection {
    fn from(value: ConnectionCreateRequest) -> Self {
        Self {
            name: value.name,
            host: value.host,
            port: value.port,
            username: value.username,
            authentication: value.authentication,
            group_id: value.group_id,
            remark: value.remark,
            remote_initial_path: value.remote_initial_path,
            icon: value.icon,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionResponse {
    id: i64,
    name: String,
    host: String,
    port: u16,
    username: String,
    authentication: &'static str,
    created_at: i64,
    updated_at: i64,
    group_id: Option<String>,
    group_name: Option<String>,
    remark: Option<String>,
    sync_mode: String,
    execution_target: String,
    remote_initial_path: Option<String>,
    icon: Option<String>,
    sort_order: Option<f64>,
    credential_kind: Option<String>,
    credential_status: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionGroupRequest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionGroupResponse {
    pub id: String,
    pub name: String,
    pub sort_order: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionBackupImportEntry {
    pub source_id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub authentication: String,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub remark: Option<String>,
    #[serde(default)]
    pub remote_initial_path: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub sort_order: Option<f64>,
    #[serde(default)]
    pub credential_kind: Option<String>,
    #[serde(default)]
    pub credential_status: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionBackupImportRequest {
    pub groups: Vec<ConnectionGroupRequest>,
    pub connections: Vec<ConnectionBackupImportEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionImportResultResponse {
    pub groups: usize,
    pub connections: usize,
    pub credentials: usize,
    pub imported_connections: Vec<ImportedConnectionResponse>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedConnectionResponse {
    pub source_id: String,
    pub id: i64,
}

impl From<ConnectionGroup> for ConnectionGroupResponse {
    fn from(value: ConnectionGroup) -> Self {
        Self {
            id: value.id,
            name: value.name,
            sort_order: value.sort_order,
        }
    }
}

impl From<ConnectionProfile> for ConnectionResponse {
    fn from(value: ConnectionProfile) -> Self {
        Self {
            id: value.id,
            name: value.name,
            host: value.host,
            port: value.port,
            username: value.username,
            authentication: value.authentication.as_str(),
            created_at: value.created_at,
            updated_at: value.updated_at,
            group_id: value.group_id,
            group_name: value.group_name,
            remark: value.remark,
            sync_mode: value.sync_mode,
            execution_target: value.execution_target,
            remote_initial_path: value.remote_initial_path,
            icon: value.icon,
            sort_order: value.sort_order,
            credential_kind: value.credential_kind,
            credential_status: value.credential_status,
        }
    }
}
