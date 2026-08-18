use nocterm_application::error::AppError;
use serde::Serialize;

/// IPC 错误只携带稳定契约，不暴露数据库和操作系统诊断信息。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    code: &'static str,
    message: String,
    retryable: bool,
}

impl From<AppError> for ErrorResponse {
    fn from(value: AppError) -> Self {
        Self {
            code: value.code,
            message: value.message,
            retryable: value.retryable,
        }
    }
}
