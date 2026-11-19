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
    // 大小先于内容检查；读写和 force 共用规则，失败时不能触及保存事务。
    if bytes.len() as u64 > EDIT_MAX_BYTES {
        return Err(AppError::new(
            "SFTP_FILE_TOO_LARGE",
            "文件超过在线编辑上限（2 MiB）",
            false,
        ));
    }
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() || has_binary_header(bytes) {
        return Err(not_text());
    }
    Ok(())
}

/// 识别常见容器签名，补足“合法 UTF-8 且无 NUL”仍可能是二进制的情况。
/// 这是有限格式识别，不按扩展名判断，也不尝试有损解码或自动转换内容。
fn has_binary_header(bytes: &[u8]) -> bool {
    // 仅在检测时跳过 BOM，真正返回和保存的文本仍保留原始 BOM。
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    const HEADERS: &[&[u8]] = &[
        b"%PDF-",
        b"GIF87a",
        b"GIF89a",
        b"PK\x03\x04",
        b"PK\x05\x06",
        b"PK\x07\x08",
        b"!<arch>\n",
        b"\x7fELF",
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff",
        b"\x1f\x8b",
        b"7z\xbc\xaf\x27\x1c",
        b"Rar!\x1a\x07",
    ];
    if HEADERS.iter().any(|header| bytes.starts_with(header)) {
        return true;
    }
    // RIFF 本身不足以判定，必须同时匹配固定偏移上的实际媒体格式。
    bytes.starts_with(b"RIFF") && matches!(bytes.get(8..12), Some(b"WEBP" | b"WAVE" | b"AVI "))
}

fn not_text() -> AppError {
    AppError::new(
        "SFTP_FILE_NOT_TEXT",
        "文件是二进制或非 UTF-8 编码，不支持在线编辑",
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

    #[test]
    fn rejects_binary_formats_that_can_otherwise_pass_utf8_validation() {
        // PDF、GIF 和容器头可能全部为合法 UTF-8，不能只靠 NUL 或编码拒绝。
        let fixtures: &[&[u8]] = &[
            b"%PDF-1.4\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n",
            b"GIF89aAAAAAAAAAA",
            b"PK\x03\x04AAAAAAAAAA",
            b"!<arch>\nAAAAAAAAAA",
            b"RIFFAAAAWEBPVP8 AAAAAAAA",
        ];
        for bytes in fixtures {
            assert!(std::str::from_utf8(bytes).is_ok());
            assert!(!bytes.contains(&0));
            assert_eq!(
                validate_content(bytes).unwrap_err().code,
                "SFTP_FILE_NOT_TEXT"
            );
        }
        assert_eq!(
            validate_content(b"\xef\xbb\xbf%PDF-1.7\n%%EOF")
                .unwrap_err()
                .code,
            "SFTP_FILE_NOT_TEXT"
        );
    }

    #[test]
    fn preserves_supported_text_without_extension_assumptions() {
        for bytes in [
            b"".as_slice(),
            "配置=中文\r\n".as_bytes(),
            b"#!/bin/sh\nprintf 'hello\\n'\n",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"><path /></svg>",
            b"\xef\xbb\xbf<svg />",
            b"MZ_CONFIG=production\nRIFF is a container format\n",
        ] {
            assert!(validate_content(bytes).is_ok());
        }
    }

    #[test]
    fn force_cannot_write_a_binary_header_as_text() {
        let service = FileEditService(RejectWrite(AtomicBool::new(false)));
        let version = FileVersion {
            size: None,
            modified_seconds: None,
            modified_nanos: None,
        };
        assert_eq!(
            service
                .write("disguised.txt", "%PDF-1.7\n%%EOF", version, true)
                .unwrap_err()
                .code,
            "SFTP_FILE_NOT_TEXT"
        );
        assert!(!service.0.0.load(Ordering::Relaxed));
    }
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
