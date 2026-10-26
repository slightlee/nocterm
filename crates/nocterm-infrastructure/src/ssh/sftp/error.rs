//! SFTP 错误归一化：把协议层失败收敛为携带前端稳定 `code` 的 `SftpError`，
//! 与 `__NOCTERM_SFTP_ERROR__` 契约保持一致；连接断开类错误额外标记 `evict`，
//! 触发会话池剔除以便下次重连。

use russh_sftp::{client::error::Error as SftpProtoError, protocol::StatusCode};

/// SFTP 操作错误：`code` 为前端稳定错误码（与 `sftp-error.ts` 词表一致），
/// `message` 面向诊断，`evict` 表示该错误意味着连接已不可用需从会话池剔除。
#[derive(Debug, Clone)]
pub struct SftpError {
    pub code: &'static str,
    pub message: String,
    pub evict: bool,
}

impl SftpError {
    /// 常规错误：不触发会话剔除，连接仍可继续复用。
    pub(super) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            evict: false,
        }
    }

    /// 连接断开类错误：向前端归为 `unknown`，同时标记剔除以便下次自动重连。
    pub(super) fn lost(message: impl Into<String>) -> Self {
        Self {
            code: "unknown",
            message: message.into(),
            evict: true,
        }
    }

    /// 用户主动取消：命令层以取消标志判定最终状态，此处仅用于中断内部流程。
    pub(super) fn cancelled() -> Self {
        Self::new("cancelled", "传输已取消")
    }
}

impl std::fmt::Display for SftpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SftpError {}

/// 把 russh-sftp 协议错误归一化为携带前端稳定 `code` 的 `SftpError`。
pub(super) fn map_sftp_error(error: SftpProtoError) -> SftpError {
    match error {
        SftpProtoError::Status(status) => match status.status_code {
            // 路径不存在：前端据此提示目标缺失。
            StatusCode::NoSuchFile => SftpError::new("pathMissing", status.error_message),
            // 权限不足：区分于路径缺失，便于前端给出针对性提示。
            StatusCode::PermissionDenied => {
                SftpError::new("permissionDenied", status.error_message)
            }
            // 连接层面异常：标记剔除以触发下次重连。
            StatusCode::ConnectionLost | StatusCode::NoConnection => {
                SftpError::lost(status.error_message)
            }
            // 其余协议失败（Failure/BadMessage/OpUnsupported 等）统一归为未知。
            _ => SftpError::new("unknown", status.error_message),
        },
        // 底层 IO 错误通常意味着连接已断，标记剔除。
        SftpProtoError::IO(message) => SftpError::lost(format!("SFTP IO 错误：{message}")),
        // 请求超时：前端有独立的超时提示词条。
        SftpProtoError::Timeout => SftpError::new("timeout", "SFTP 操作超时"),
        // 其余协议异常（限流、意外报文、意外行为）归为未知。
        other => SftpError::new("unknown", format!("SFTP 协议错误：{other}")),
    }
}
