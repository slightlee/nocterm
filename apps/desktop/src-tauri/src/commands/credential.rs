use std::io::Read;

use nocterm_application::connection::CredentialInput;
use tauri::State;

use crate::{dto::error::ErrorResponse, state::AppState};

#[tauri::command]
pub fn credential_store(
    state: State<'_, AppState>,
    connection_id: String,
    credential_kind: String,
    secret: String,
) -> Result<(), ErrorResponse> {
    let id = validate_credential_request(&state, &connection_id, &credential_kind)?;
    if !can_store_secret_credential(&credential_kind) {
        return Err(error(
            "CREDENTIAL_KIND_INVALID",
            "SSH Agent 不需要保存系统凭据",
            false,
        ));
    }
    state
        .connection_service()
        .store_credential(
            id,
            CredentialInput {
                kind: credential_kind,
                secret,
            },
        )
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn credential_delete(
    state: State<'_, AppState>,
    connection_id: String,
    credential_kind: String,
) -> Result<(), ErrorResponse> {
    let id = validate_credential_request(&state, &connection_id, &credential_kind)?;
    state
        .connection_service()
        .delete_credential(id, &credential_kind)
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub fn credential_store_file(
    state: State<'_, AppState>,
    connection_id: String,
    credential_kind: String,
    path: String,
) -> Result<(), ErrorResponse> {
    let id = validate_credential_request(&state, &connection_id, &credential_kind)?;
    if credential_kind != "private_key" {
        return Err(error(
            "CREDENTIAL_KIND_INVALID",
            "仅支持保存私钥文件",
            false,
        ));
    }
    state
        .connection_service()
        .store_credential(
            id,
            CredentialInput {
                kind: credential_kind,
                secret: read_private_key_file(&path)?,
            },
        )
        .map_err(ErrorResponse::from)
}

pub fn read_secret(
    state: &AppState,
    connection_id: &str,
    credential_kind: &str,
) -> Result<String, ErrorResponse> {
    let id = connection_id
        .parse::<i64>()
        .map_err(|_| error("CONNECTION_ID_INVALID", "凭据请求缺少有效连接标识", false))?;
    state
        .connection_service()
        .read_credential(id, credential_kind)
        .map_err(ErrorResponse::from)
}

/// 路径只在 Tauri 边界读取，私钥明文随后立即交给 Application 用例且不会回传前端。
pub fn read_private_key_file(path: &str) -> Result<String, ErrorResponse> {
    const MAX_PRIVATE_KEY_BYTES: u64 = 1024 * 1024;
    let metadata = std::fs::metadata(path)
        .map_err(|_| error("CREDENTIAL_FILE_READ_FAILED", "读取私钥文件失败", false))?;
    if !metadata.is_file() || metadata.len() > MAX_PRIVATE_KEY_BYTES {
        return Err(error(
            "CREDENTIAL_FILE_INVALID",
            "私钥文件无效或超过 1 MiB 限制",
            false,
        ));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| {
            file.by_ref()
                .take(MAX_PRIVATE_KEY_BYTES + 1)
                .read_to_end(&mut bytes)
        })
        .map_err(|_| error("CREDENTIAL_FILE_READ_FAILED", "读取私钥文件失败", false))?;
    if bytes.len() as u64 > MAX_PRIVATE_KEY_BYTES {
        return Err(error(
            "CREDENTIAL_FILE_INVALID",
            "私钥文件无效或超过 1 MiB 限制",
            false,
        ));
    }
    let secret = String::from_utf8(bytes).map_err(|_| {
        error(
            "CREDENTIAL_FILE_INVALID",
            "私钥文件必须是 UTF-8 文本",
            false,
        )
    })?;
    if secret.trim().is_empty() {
        return Err(error("CREDENTIAL_EMPTY", "私钥文件内容不能为空", false));
    }
    Ok(secret)
}

fn validate_credential_request(
    state: &AppState,
    connection_id: &str,
    credential_kind: &str,
) -> Result<i64, ErrorResponse> {
    let id = connection_id
        .parse::<i64>()
        .map_err(|_| error("CONNECTION_ID_INVALID", "凭据请求缺少有效连接标识", false))?;
    if id <= 0 {
        return Err(error(
            "CONNECTION_ID_INVALID",
            "凭据请求缺少有效连接标识",
            false,
        ));
    }
    if !is_supported_credential_kind(credential_kind) {
        return Err(error("CREDENTIAL_KIND_INVALID", "凭据类型不受支持", false));
    }
    state
        .connection_service()
        .get(id)
        .map(|_| id)
        .map_err(ErrorResponse::from)
}

fn is_supported_credential_kind(value: &str) -> bool {
    matches!(value, "password" | "private_key" | "ssh_agent")
}

fn can_store_secret_credential(value: &str) -> bool {
    matches!(value, "password" | "private_key")
}

fn error(code: &'static str, message: &'static str, retryable: bool) -> ErrorResponse {
    nocterm_application::error::AppError::new(code, message, retryable).into()
}

#[cfg(test)]
mod tests {
    use super::{can_store_secret_credential, is_supported_credential_kind};

    #[test]
    fn accepts_only_supported_credential_kinds() {
        assert!(is_supported_credential_kind("password"));
        assert!(is_supported_credential_kind("private_key"));
        assert!(is_supported_credential_kind("ssh_agent"));
        assert!(!is_supported_credential_kind("token"));
    }

    #[test]
    fn does_not_store_a_secret_for_ssh_agent() {
        assert!(can_store_secret_credential("password"));
        assert!(can_store_secret_credential("private_key"));
        assert!(!can_store_secret_credential("ssh_agent"));
    }
}
