//! 文本编辑契约：只包含内容版本、错误和存储 Port，不依赖文件系统或 SSH 实现。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// 远端可能不提供完整 stat；未知字段以 None 保留，不用 0 伪造已知版本。
/// 纳秒字段只由本地 Adapter 提供，避免降低本地冲突检测精度。
pub struct FileVersion {
    pub size: Option<u64>,
    pub modified_seconds: Option<u64>,
    pub modified_nanos: Option<u32>,
}

pub struct FileSnapshot {
    pub identity_path: String,
    pub bytes: Vec<u8>,
    pub version: FileVersion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEditError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl FileEditError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

/// Adapter 必须限制读取量，写入要复检版本并保证失败时保留可恢复的原文件。
/// force 只跳过内容冲突，不能跳过文件类型、目标身份或权限检查。
pub trait TextFilePort: Send + Sync {
    fn read(&self, path: &str, max_bytes: u64) -> Result<FileSnapshot, FileEditError>;
    fn write(
        &self,
        path: &str,
        content: &[u8],
        baseline: FileVersion,
        force: bool,
    ) -> Result<FileVersion, FileEditError>;
}
