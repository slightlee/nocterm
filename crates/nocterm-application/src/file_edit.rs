//! 本地与远程共享文本规则；读写和平台事务由注入的 Port 执行。
use crate::error::AppError;
use nocterm_domain::file_edit::{FileEditError, FileVersion, TextFilePort};

pub const EDIT_MAX_BYTES: u64 = 2 * 1024 * 1024;

pub struct FileContent {
    pub identity_path: String,
    pub content: String,
    pub version: FileVersion,
}

pub struct FileEditService<P>(pub P);

impl<P: TextFilePort> FileEditService<P> {
    pub fn read(&self, path: &str) -> Result<FileContent, AppError> {
        // 上限下传到读取器，不能先载入整个文件再在 UI 判定大小。
        // 仍在用例层复核，覆盖 stat 后增长与不完整的 Adapter 元数据。
        let snapshot = self.0.read(path, EDIT_MAX_BYTES).map_err(app_error)?;
        validate_content(&snapshot.bytes)?;
        let content = String::from_utf8(snapshot.bytes).map_err(|_| not_text())?;
        Ok(FileContent {
            identity_path: snapshot.identity_path,
            content,
            version: snapshot.version,
        })
    }

    pub fn write(
        &self,
        path: &str,
        content: &str,
        baseline: FileVersion,
        force: bool,
    ) -> Result<FileVersion, AppError> {
        // 任何 force 保存都先过文本规则，覆盖冲突不代表允许二进制或超限写入。
        // 校验在触及存储前完成，失败时不创建任何暂存资源。
        validate_content(content.as_bytes())?;
        self.0
            .write(path, content.as_bytes(), baseline, force)
            .map_err(app_error)
    }
}

pub fn validate_content(bytes: &[u8]) -> Result<(), AppError> {
    // UTF-8 合法也可能包含 NUL；这类文件不交给文本编辑器往返改写。
    if bytes.len() as u64 > EDIT_MAX_BYTES {
        return Err(AppError::new(
            "SFTP_FILE_TOO_LARGE",
            "文件超过在线编辑上限（2 MiB）",
            false,
        ));
    }
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return Err(not_text());
    }
    Ok(())
}

fn not_text() -> AppError {
    AppError::new(
        "SFTP_FILE_NOT_TEXT",
        "文件不是合法 UTF-8 文本，不支持在线编辑",
        false,
    )
}
fn app_error(error: FileEditError) -> AppError {
    AppError::new(error.code, error.message, error.retryable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    struct RejectWrite(AtomicBool);
    impl TextFilePort for RejectWrite {
        fn read(
            &self,
            _: &str,
            _: u64,
        ) -> Result<nocterm_domain::file_edit::FileSnapshot, FileEditError> {
            unreachable!()
        }
        fn write(
            &self,
            _: &str,
            _: &[u8],
            _: FileVersion,
            _: bool,
        ) -> Result<FileVersion, FileEditError> {
            self.0.store(true, Ordering::Relaxed);
            unreachable!("invalid text must not reach storage")
        }
    }
    #[test]
    fn rejects_invalid_content_before_touching_storage() {
        let service = FileEditService(RejectWrite(AtomicBool::new(false)));
        let version = FileVersion {
            size: None,
            modified_seconds: None,
            modified_nanos: None,
        };
        assert_eq!(
            service
                .write("test", "a\0b", version, true)
                .unwrap_err()
                .code,
            "SFTP_FILE_NOT_TEXT"
        );
        assert!(!service.0.0.load(Ordering::Relaxed));
        assert_eq!(
            validate_content(&vec![b'x'; EDIT_MAX_BYTES as usize + 1])
                .unwrap_err()
                .code,
            "SFTP_FILE_TOO_LARGE"
        );
    }
}
