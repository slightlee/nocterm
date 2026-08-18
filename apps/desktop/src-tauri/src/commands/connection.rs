use std::path::{Path, PathBuf};

use nocterm_application::{
    connection::{CreateConnection, CredentialInput, ImportConnection},
    error::AppError,
};
use nocterm_domain::connection::NewConnectionGroup;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::{
    dto::{
        connection::{
            ConnectionBackupImportRequest, ConnectionCreateRequest, ConnectionGroupRequest,
            ConnectionGroupResponse, ConnectionImportResultResponse, ConnectionResponse,
            CredentialInputRequest, ImportedConnectionResponse,
        },
        error::ErrorResponse,
    },
    state::AppState,
};

const MAX_CONNECTION_FILE_SIZE: u64 = 5 * 1024 * 1024;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedConnectionFile {
    path: String,
    content: String,
}

#[tauri::command]
pub fn connection_list(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionResponse>, ErrorResponse> {
    state
        .connection_service()
        .list()
        .map(|profiles| profiles.into_iter().map(ConnectionResponse::from).collect())
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_create(
    state: State<'_, AppState>,
    request: ConnectionCreateRequest,
) -> Result<ConnectionResponse, ErrorResponse> {
    let created = state
        .connection_service()
        .create(CreateConnection::from(request))
        .map_err(ErrorResponse::from)?;
    if let Err(error) = state
        .connection_service()
        .sync_ssh_agent(created.id, created.authentication)
        .map_err(ErrorResponse::from)
    {
        // Agent 绑定元数据和连接资料必须同时成功，避免留下永远显示“未就绪”的半成品。
        let _ = state.connection_service().delete(created.id);
        return Err(error);
    }
    state
        .connection_service()
        .get(created.id)
        .map(ConnectionResponse::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_update(
    state: State<'_, AppState>,
    request: ConnectionCreateRequest,
    credential: Option<CredentialInputRequest>,
) -> Result<ConnectionResponse, ErrorResponse> {
    let id = request.id.ok_or_else(|| {
        ErrorResponse::from(nocterm_application::error::AppError::new(
            "CONNECTION_ID_REQUIRED",
            "更新连接缺少 ID",
            false,
        ))
    })?;
    let credential = credential
        .map(|credential| {
            let secret = match (credential.secret, credential.private_key_path) {
                (Some(secret), None) => Ok(secret),
                (None, Some(path)) if credential.kind == "private_key" => {
                    crate::commands::credential::read_private_key_file(&path)
                }
                _ => Err(ErrorResponse::from(AppError::new(
                    "CREDENTIAL_INPUT_INVALID",
                    "凭据请求无效",
                    false,
                ))),
            }?;
            Ok::<CredentialInput, ErrorResponse>(CredentialInput {
                kind: credential.kind,
                secret,
            })
        })
        .transpose()?;
    let updated = state
        .connection_service()
        .update_with_credential(id, CreateConnection::from(request), credential)
        .map_err(ErrorResponse::from)?;
    Ok(ConnectionResponse::from(updated))
}

#[cfg(test)]
mod tests {
    use super::{validate_connection_backup_write_path, validate_connection_read_path};

    #[test]
    fn accepts_standard_ssh_config_only_for_reading() {
        // Build absolute paths with the platform's native prefix so this test
        // exercises the same validation branch on Unix and Windows.
        let ssh_config = std::env::temp_dir().join(".ssh").join("config");
        let backup_file = std::env::temp_dir().join("backup.json");

        assert!(validate_connection_read_path(ssh_config.to_string_lossy().as_ref()).is_ok());
        assert!(
            validate_connection_backup_write_path(ssh_config.to_string_lossy().as_ref()).is_err()
        );
        assert!(
            validate_connection_backup_write_path(backup_file.to_string_lossy().as_ref()).is_ok()
        );
    }
}

#[tauri::command]
pub fn connection_delete(state: State<'_, AppState>, id: i64) -> Result<(), ErrorResponse> {
    state
        .terminal_service()
        .close_connection(id)
        .map_err(ErrorResponse::from)?;
    state
        .connection_service()
        .delete_with_credentials(id)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_group_list(
    state: State<'_, AppState>,
) -> Result<Vec<ConnectionGroupResponse>, ErrorResponse> {
    state
        .connection_service()
        .list_groups()
        .map(|groups| {
            groups
                .into_iter()
                .map(ConnectionGroupResponse::from)
                .collect()
        })
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_group_upsert(
    state: State<'_, AppState>,
    request: ConnectionGroupRequest,
) -> Result<ConnectionGroupResponse, ErrorResponse> {
    let mut group = NewConnectionGroup::try_new(request.id, request.name).map_err(|error| {
        ErrorResponse::from(nocterm_application::error::AppError::new(
            error.code,
            error.message,
            false,
        ))
    })?;
    group.sort_order = request.sort_order;
    state
        .connection_service()
        .upsert_group(group)
        .map(ConnectionGroupResponse::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_group_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), ErrorResponse> {
    state
        .connection_service()
        .delete_group(&id)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_reorder(
    state: State<'_, AppState>,
    id: i64,
    group_id: Option<String>,
    sort_order: f64,
) -> Result<ConnectionResponse, ErrorResponse> {
    state
        .connection_service()
        .update_sort_order(id, group_id.as_deref(), sort_order)
        .map(ConnectionResponse::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn connection_backup_import(
    state: State<'_, AppState>,
    request: ConnectionBackupImportRequest,
) -> Result<ConnectionImportResultResponse, ErrorResponse> {
    let groups = request
        .groups
        .into_iter()
        .map(|request| {
            let mut group =
                NewConnectionGroup::try_new(request.id, request.name).map_err(|error| {
                    ErrorResponse::from(AppError::new(error.code, error.message, false))
                })?;
            group.sort_order = request.sort_order;
            Ok(group)
        })
        .collect::<Result<Vec<_>, ErrorResponse>>()?;
    let connections = request
        .connections
        .into_iter()
        .map(|entry| ImportConnection {
            source_id: entry.source_id,
            connection: CreateConnection {
                name: entry.name,
                host: entry.host,
                port: entry.port,
                username: entry.username,
                authentication: entry.authentication,
                group_id: entry.group_id,
                remark: entry.remark,
                remote_initial_path: entry.remote_initial_path,
                icon: entry.icon,
            },
            sort_order: entry.sort_order,
            credential_kind: entry.credential_kind,
            credential_status: entry.credential_status,
        })
        .collect();

    state
        .connection_service()
        .import_backup(groups, connections)
        .map(|result| ConnectionImportResultResponse {
            groups: result.groups,
            connections: result.connections,
            credentials: result.credentials,
            imported_connections: result
                .imported_connections
                .into_iter()
                .map(|connection| ImportedConnectionResponse {
                    source_id: connection.source_id,
                    id: connection.id,
                })
                .collect(),
        })
        .map_err(ErrorResponse::from)
}

/// 文件路径来自原生选择器；命令仍限制扩展名和体积，避免成为通用文件读取接口。
/// 文件选择与读取在同一受控命令内完成，前端不能构造任意本机路径请求。
#[tauri::command]
pub async fn connection_backup_pick_and_read(
    app: AppHandle,
) -> Result<Option<String>, ErrorResponse> {
    let selected = run_file_task(move || {
        app.dialog()
            .file()
            .add_filter("Nocterm Connection Backup", &["json"])
            .blocking_pick_file()
            .map(|file| selected_file_path(file.into_path()))
            .transpose()
    })
    .await?;
    let Some(path) = selected else {
        return Ok(None);
    };
    run_file_task(move || read_connection_file(path))
        .await
        .map(Some)
}

/// SSH Config 保留来源路径，仅供前端解析相对 IdentityFile，不再开放通用读文件 IPC。
#[tauri::command]
pub async fn connection_ssh_config_pick_and_read(
    app: AppHandle,
) -> Result<Option<SelectedConnectionFile>, ErrorResponse> {
    let selected = run_file_task(move || {
        app.dialog()
            .file()
            .blocking_pick_file()
            .map(|file| selected_file_path(file.into_path()))
            .transpose()
    })
    .await?;
    let Some(path) = selected else {
        return Ok(None);
    };
    let path_for_read = path.clone();
    let content = run_file_task(move || read_connection_file(path_for_read)).await?;
    Ok(Some(SelectedConnectionFile {
        path: path.to_string_lossy().to_string(),
        content,
    }))
}

/// 导出由后端选择目标并立即写入，避免 IPC 获得任意文件覆盖能力。
#[tauri::command]
pub async fn connection_backup_save(
    app: AppHandle,
    content: String,
    default_file_name: String,
) -> Result<bool, ErrorResponse> {
    if content.len() as u64 > MAX_CONNECTION_FILE_SIZE {
        return Err(ErrorResponse::from(AppError::new(
            "CONNECTION_BACKUP_TOO_LARGE",
            "连接备份不能超过 5 MB",
            false,
        )));
    }
    let selected = run_file_task(move || {
        app.dialog()
            .file()
            .add_filter("Nocterm Connection Backup", &["json"])
            .set_file_name(default_file_name)
            .blocking_save_file()
            .map(|file| selected_file_path(file.into_path()))
            .transpose()
    })
    .await?;
    let Some(path) = selected else {
        return Ok(false);
    };
    run_file_task(move || write_connection_file(path, content)).await?;
    Ok(true)
}

/// 原生对话框和文件 I/O 不应占用 Tauri UI 线程，避免窗口在等待选择时卡顿。
async fn run_file_task<T>(
    task: impl FnOnce() -> Result<T, ErrorResponse> + Send + 'static,
) -> Result<T, ErrorResponse>
where
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|_| {
            ErrorResponse::from(AppError::new(
                "CONNECTION_FILE_OPERATION_INTERRUPTED",
                "文件操作被中断，请重试",
                true,
            ))
        })?
}

fn selected_file_path(
    path: Result<PathBuf, impl std::fmt::Debug>,
) -> Result<PathBuf, ErrorResponse> {
    path.map_err(|_| {
        ErrorResponse::from(AppError::new(
            "CONNECTION_FILE_PATH_INVALID",
            "所选文件不是本地路径",
            false,
        ))
    })
}

fn read_connection_file(path: PathBuf) -> Result<String, ErrorResponse> {
    let path = validate_connection_read_path(path.to_string_lossy().as_ref())?;
    let metadata = std::fs::metadata(&path).map_err(|_| {
        ErrorResponse::from(AppError::new(
            "CONNECTION_BACKUP_READ_FAILED",
            "无法读取所选连接文件",
            true,
        ))
    })?;
    if metadata.len() > MAX_CONNECTION_FILE_SIZE {
        return Err(ErrorResponse::from(AppError::new(
            "CONNECTION_BACKUP_TOO_LARGE",
            "连接文件不能超过 5 MB",
            false,
        )));
    }
    std::fs::read_to_string(path).map_err(|_| {
        ErrorResponse::from(AppError::new(
            "CONNECTION_BACKUP_READ_FAILED",
            "连接文件不是有效的 UTF-8 文本",
            false,
        ))
    })
}

fn write_connection_file(path: PathBuf, content: String) -> Result<(), ErrorResponse> {
    let path = validate_connection_backup_write_path(path.to_string_lossy().as_ref())?;
    std::fs::write(path, content).map_err(|_| {
        ErrorResponse::from(AppError::new(
            "CONNECTION_BACKUP_WRITE_FAILED",
            "写入连接备份失败，请检查目标位置权限",
            true,
        ))
    })
}

fn validate_connection_read_path(path: &str) -> Result<PathBuf, ErrorResponse> {
    let path = Path::new(path);
    let extension_is_supported = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("json")
                || extension.eq_ignore_ascii_case("conf")
                || extension.eq_ignore_ascii_case("config")
        });
    let is_standard_ssh_config = path.file_name().is_some_and(|name| name == "config");
    if !path.is_absolute() || !(extension_is_supported || is_standard_ssh_config) {
        return Err(ErrorResponse::from(AppError::new(
            "CONNECTION_FILE_PATH_INVALID",
            "请选择 JSON 备份或 SSH Config 文件",
            false,
        )));
    }
    Ok(path.to_path_buf())
}

fn validate_connection_backup_write_path(path: &str) -> Result<PathBuf, ErrorResponse> {
    let path = Path::new(path);
    let is_json = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"));
    if !path.is_absolute() || !is_json {
        return Err(ErrorResponse::from(AppError::new(
            "CONNECTION_FILE_PATH_INVALID",
            "请选择 JSON 备份文件保存位置",
            false,
        )));
    }
    Ok(path.to_path_buf())
}
