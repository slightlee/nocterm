//! 远程文本 Port 复用 SFTP 会话和提交协议；内容全程驻留内存，不在本机暂存。
use crate::ssh::sftp::{SftpError, SftpManager, SftpSaveExpectation};
use nocterm_domain::{
    connection::ConnectionProfile,
    file_edit::{FileEditError, FileSnapshot, FileVersion, TextFilePort},
};
use std::sync::{Arc, atomic::AtomicBool};

pub struct RemoteTextFiles {
    // 凭据只在一次用例的内存载荷里使用，不派生 Debug、不缓存内容、不写本机暂存。
    pub manager: Arc<SftpManager>,
    pub profile: ConnectionProfile,
    pub password: Option<String>,
    pub private_key: Option<String>,
    pub cancel: Arc<AtomicBool>,
}

impl TextFilePort for RemoteTextFiles {
    fn read(&self, path: &str, max_bytes: u64) -> Result<FileSnapshot, FileEditError> {
        // 协议层限制累计字节并在读取前后复核 stat，不能只信打开前的目录大小。
        valid_path(path)?;
        let result = self
            .manager
            .read_remote_file(
                &self.profile,
                self.password.as_deref(),
                self.private_key.as_deref(),
                path,
                max_bytes,
            )
            .map_err(map_error)?;
        Ok(FileSnapshot {
            identity_path: path.to_string(),
            bytes: result.bytes,
            version: FileVersion {
                size: result.size,
                modified_seconds: result.modified_at,
                modified_nanos: None,
            },
        })
    }

    fn write(
        &self,
        path: &str,
        content: &[u8],
        baseline: FileVersion,
        force: bool,
    ) -> Result<FileVersion, FileEditError> {
        valid_path(path)?;
        // 在上传之前拒绝已知冲突，提交协议还会在内容完整写入后再次检查。
        // 第一次检查减少无效上传，第二次检查覆盖上传期间的外部修改。
        // 目标消失也算冲突；只有明确覆盖才允许重建普通文件。
        match self.manager.stat_remote_file(
            &self.profile,
            self.password.as_deref(),
            self.private_key.as_deref(),
            path,
        ) {
            Ok(stat)
                if !force
                    && (stat.size != baseline.size
                        || stat.modified_at != baseline.modified_seconds) =>
            {
                return Err(changed());
            }
            Err(error) if error.code == "pathMissing" && force => {}
            Err(error) if error.code == "pathMissing" => return Err(changed()),
            Err(error) => return Err(map_error(error)),
            Ok(_) => {}
        }
        let result = self
            .manager
            .save_text(
                // 版本与取消标记属于本次保存，不从可见面板重新取当前连接。
                &self.profile,
                self.password.as_deref(),
                self.private_key.as_deref(),
                path,
                content,
                SftpSaveExpectation {
                    size: baseline.size,
                    modified_at: baseline.modified_seconds,
                    force,
                },
                self.cancel.clone(),
            )
            .map_err(map_error)?;
        Ok(FileVersion {
            size: result.size,
            modified_seconds: result.modified_at,
            modified_nanos: None,
        })
    }
}

fn valid_path(path: &str) -> Result<(), FileEditError> {
    // IPC 必须携带绝对文件路径，拒绝目录和会改变父目录语义的最终组件。
    let name = path.rsplit('/').next().unwrap_or_default();
    if !path.starts_with('/')
        || path.contains(['\n', '\r', '\0'])
        || name.is_empty()
        || matches!(name, "." | "..")
    {
        return Err(FileEditError::new(
            "SFTP_INVALID_PATH",
            "远程文件路径无效",
            false,
        ));
    }
    Ok(())
}

fn changed() -> FileEditError {
    FileEditError::new(
        "SFTP_REMOTE_CHANGED",
        "远端文件已被修改或删除，请重新加载或确认覆盖",
        true,
    )
}
fn map_error(error: SftpError) -> FileEditError {
    // 冲突使用独立命令错误码，控制器据此进入对话，不能解析服务端文本。
    // 其余分类继续兼容文件浏览器已有的错误词表。
    let code = match error.code {
        "remoteChanged" => return changed(),
        "fileSymlink" => "SFTP_FILE_SYMLINK",
        "fileTooLarge" => "SFTP_FILE_TOO_LARGE",
        "remoteReadChanged" => "SFTP_REMOTE_READ_CHANGED",
        "permissionDenied" => "SFTP_REMOTE_PERMISSION_DENIED",
        _ => "SFTP_REMOTE_READ_FAILED",
    };
    // 原始路径、服务端消息不越过适配边界，保留既有稳定错误词表供 UI 本地化。
    FileEditError::new(
        code,
        format!("__NOCTERM_SFTP_ERROR__\t{}\t", error.code),
        true,
    )
}
