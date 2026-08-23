use std::io::Read;

use nocterm_application::connection::CredentialInput;
use nocterm_domain::connection::ConnectionProfile;
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
            "只有密码需要写入系统凭据库：SSH Agent 不需要凭据，私钥请绑定文件路径",
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

/// 绑定私钥文件：只把路径写进连接资料，密钥内容留在用户自己的文件里。
///
/// 命名上不再叫 `credential_store_file`——私钥根本不进系统凭据库，
/// 沿用旧名字会让人以为密钥被复制到了 Keychain/凭据管理器里。
#[tauri::command]
pub fn connection_bind_private_key(
    state: State<'_, AppState>,
    connection_id: String,
    path: String,
) -> Result<(), ErrorResponse> {
    let id = validate_credential_request(&state, &connection_id, "private_key")?;
    // 先确认文件此刻可读，避免把一个用不了的路径写进资料。内容读出后立即丢弃。
    read_private_key_file(&path)?;
    state
        .connection_service()
        .bind_private_key_path(id, &path)
        .map(|_| ())
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

/// 建连时解析私钥内容：优先按资料里的路径读取文件，其次回落到系统凭据库。
///
/// 回落分支只为兼容早于"路径引用"改造的老连接（那时私钥内容被写进了凭据库），
/// 新绑定一律走路径。两条分支都不在磁盘上留下私钥副本。
pub fn resolve_private_key(
    state: &AppState,
    profile: &ConnectionProfile,
) -> Result<String, ErrorResponse> {
    if let Some(path) = profile
        .private_key_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return read_private_key_file(path);
    }
    read_secret(state, &profile.id.to_string(), "private_key").map_err(|_| {
        error(
            "PRIVATE_KEY_REQUIRED",
            "该连接尚未绑定私钥文件，请编辑连接重新选择私钥",
            false,
        )
    })
}

/// 路径只在 Tauri 边界读取，私钥明文随后立即交给 Application 用例且不会回传前端。
pub fn read_private_key_file(path: &str) -> Result<String, ErrorResponse> {
    const MAX_PRIVATE_KEY_BYTES: u64 = 1024 * 1024;
    // 提示里带上路径与系统原因："文件被移动/改名"和"权限不足"的处置完全不同，
    // 只说"读取私钥文件失败"用户无从下手。路径是用户自己选的，不属于敏感内容；
    // 文件内容始终不进错误信息。
    let describe = |reason: std::io::Error| {
        error(
            "CREDENTIAL_FILE_READ_FAILED",
            format!("读取私钥文件失败（{path}）：{reason}"),
            false,
        )
    };
    let metadata = std::fs::metadata(path).map_err(describe)?;
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
        .map_err(describe)?;
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

/// 只有密码是"写入系统凭据库的密钥"。SSH Agent 无凭据可存；
/// 私钥按文件路径引用，凭据管理器也装不下（单条 blob 上限 2560 字节）。
fn can_store_secret_credential(value: &str) -> bool {
    matches!(value, "password")
}

/// 统一构造带稳定错误码的响应。`message` 取 `impl Into<String>` 而非 `&'static str`，
/// 好让调用方能把路径、系统原因等诊断信息拼进提示——把它们折叠掉等于放弃定位能力。
fn error(code: &'static str, message: impl Into<String>, retryable: bool) -> ErrorResponse {
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
    fn does_not_store_a_secret_for_ssh_agent_or_private_key() {
        assert!(can_store_secret_credential("password"));
        // 私钥走 `connection_bind_private_key` 记录路径，绝不写入系统凭据库。
        assert!(!can_store_secret_credential("private_key"));
        assert!(!can_store_secret_credential("ssh_agent"));
    }
}
