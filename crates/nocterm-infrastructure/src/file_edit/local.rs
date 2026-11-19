//! 本地文件 Adapter：受限读取、冲突复检、私有暂存与可恢复提交。
use nocterm_domain::file_edit::{FileEditError, FileSnapshot, FileVersion, TextFilePort};
use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[cfg(test)]
#[path = "local_tests.rs"]
mod tests;

fn error(code: &'static str, message: impl Into<String>, retryable: bool) -> FileEditError {
    FileEditError::new(code, message, retryable)
}
fn local_read_error(err: &std::io::Error) -> FileEditError {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        error(
            "SFTP_LOCAL_PERMISSION_DENIED",
            "没有访问本地文件的权限",
            true,
        )
    } else {
        error("SFTP_LOCAL_READ_FAILED", "读取本地文件失败", true)
    }
}
fn remove_path(path: &Path) {
    let _ = fs::remove_file(path);
}
fn temp_name(name: &str, id: &str, suffix: &str) -> String {
    format!(".{name}.nocterm-{id}.{suffix}")
}

pub struct LocalTextFiles;
// Adapter 只处理文件系统身份、元数据和提交事务，文本规则由 Application 统一校验。
impl TextFilePort for LocalTextFiles {
    fn read(&self, path: &str, max_bytes: u64) -> Result<FileSnapshot, FileEditError> {
        read_local_file(path.to_string(), max_bytes)
    }
    fn write(
        &self,
        path: &str,
        content: &[u8],
        baseline: FileVersion,
        force: bool,
    ) -> Result<FileVersion, FileEditError> {
        write_local_file(
            path.to_string(),
            content,
            baseline.size,
            baseline.modified_seconds,
            baseline.modified_nanos,
            force,
        )
    }
}
fn resolve_local(path: Option<String>) -> Result<PathBuf, FileEditError> {
    // 仅展开用户主目录；符号链接的最终组件由类型校验拒绝，读取成功再返回身份路径。
    let home_dir = || {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
    };
    let raw = path.unwrap_or_else(|| "~".to_string());
    if raw == "~" {
        return home_dir()
            .ok_or_else(|| error("SFTP_LOCAL_PATH_INVALID", "无法定位用户主目录", false));
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir()
            .map(|home| home.join(rest))
            .ok_or_else(|| error("SFTP_LOCAL_PATH_INVALID", "无法定位用户主目录", false));
    }
    Ok(PathBuf::from(raw))
}

#[derive(Clone, Copy)]
struct LocalCommitExpectation {
    size: Option<u64>,
    modified_seconds: Option<u64>,
    modified_nanos: Option<u32>,
    force: bool,
}

/// 目标已改名为备份后恢复原路径；恢复失败时把备份位置写进错误，
/// 便于诊断和人工恢复，不能静默吞掉数据保护失败。
fn rollback_local_backup(
    backup: &Path,
    target: &Path,
    kind: std::io::ErrorKind,
    reason: &str,
) -> std::io::Error {
    match fs::rename(backup, target) {
        Ok(()) => std::io::Error::new(kind, reason),
        Err(rollback_error) => std::io::Error::new(
            rollback_error.kind(),
            format!(
                "{reason}; rollback failed: {rollback_error}; backup remains at {}",
                backup.display()
            ),
        ),
    }
}

/// 目标已移到备份名后复检快照；任何 stat/类型/冲突错误都先恢复原路径。
fn verify_local_backup_snapshot(
    backup: &Path,
    target: &Path,
    expected: LocalCommitExpectation,
) -> std::io::Result<()> {
    let current = match fs::symlink_metadata(backup) {
        Ok(metadata) => metadata,
        Err(stat_error) => {
            return Err(rollback_local_backup(
                backup,
                target,
                stat_error.kind(),
                "could not verify local backup before commit",
            ));
        }
    };
    if current.file_type().is_symlink() || !current.is_file() {
        return Err(rollback_local_backup(
            backup,
            target,
            std::io::ErrorKind::InvalidInput,
            "local target changed to a symlink or non-regular file during save",
        ));
    }
    let (seconds, nanos) = split_modified(&current);
    let unchanged = Some(current.len()) == expected.size
        && Some(seconds) == expected.modified_seconds
        && Some(nanos) == expected.modified_nanos;
    if !unchanged && !expected.force {
        return Err(rollback_local_backup(
            backup,
            target,
            std::io::ErrorKind::WouldBlock,
            "local target changed during save",
        ));
    }
    Ok(())
}

/// 先把旧目标移到任务专属备份，再提交新目标；提交失败时恢复旧目标。
/// 目标改名到备份后再检查一次，缩小最后一次 stat 与替换之间的 TOCTOU 窗口。
/// 该事务仅由编辑保存调用，不改变现有目录下载的替换行为。
fn replace_local_path_checked(
    staged: &Path,
    target: &Path,
    backup: &Path,
    expectation: LocalCommitExpectation,
) -> std::io::Result<()> {
    let had_target = fs::symlink_metadata(target).is_ok();
    if !had_target {
        if !expectation.force {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "local target disappeared during save",
            ));
        }
    } else {
        fs::rename(target, backup)?;
        // 元数据读取本身也可能失败；先恢复备份，避免目标只剩隐藏文件名。
        verify_local_backup_snapshot(backup, target, expectation)?;
    }

    if let Err(commit_error) = fs::rename(staged, target) {
        if had_target {
            return Err(rollback_local_backup(
                backup,
                target,
                commit_error.kind(),
                &format!("commit failed: {commit_error}"),
            ));
        }
        return Err(commit_error);
    }

    if had_target {
        // 内容已提交后才清理回滚点；删除失败不会删除新的目标文件。
        remove_path(backup);
    }
    Ok(())
}

fn split_modified(metadata: &fs::Metadata) -> (u64, u32) {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok());
    match modified {
        Some(duration) => (duration.as_secs(), duration.subsec_nanos()),
        None => (0, 0),
    }
}

/// 校验本地待编辑路径：必须是常规文件且最终组件不是符号链接。
/// 符号链接编辑会混淆「改的是链接还是目标」，V1 明确拒绝并提示打开真实文件。
fn validate_local_edit_target(path: &Path) -> Result<(), FileEditError> {
    let metadata = fs::symlink_metadata(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            error("SFTP_LOCAL_PATH_INVALID", "本地文件不存在", false)
        } else {
            local_read_error(&err)
        }
    })?;
    if metadata.file_type().is_symlink() {
        return Err(error(
            "SFTP_LOCAL_FILE_SYMLINK",
            "目标是符号链接，请直接打开链接指向的真实文件",
            false,
        ));
    }
    if !metadata.is_file() {
        return Err(error(
            "SFTP_LOCAL_PATH_INVALID",
            "目标不是常规文件，无法作为文本编辑",
            false,
        ));
    }
    Ok(())
}

/// 本地冲突校验：size 与精确到纳秒的 mtime 任一变化即视为已变更，
/// 误报只多弹一次确认，漏报会静默覆盖外部改动。
fn local_file_unchanged(
    metadata: &fs::Metadata,
    baseline_size: Option<u64>,
    baseline_modified_seconds: Option<u64>,
    baseline_modified_nanos: Option<u32>,
) -> bool {
    let (seconds, nanos) = split_modified(metadata);
    Some(metadata.len()) == baseline_size
        && Some(seconds) == baseline_modified_seconds
        && Some(nanos) == baseline_modified_nanos
}

/// 读取本地文本文件供在线编辑：stat 先拒绝超限，最多只读上限+1 字节；
/// 读取前后元数据一致才返回，避免把正在变化的内容和错误基线交给前端。
fn read_local_file(path: String, max_bytes: u64) -> Result<FileSnapshot, FileEditError> {
    let resolved = resolve_local(Some(path))?;
    validate_local_edit_target(&resolved)?;
    let before = fs::metadata(&resolved).map_err(|err| local_read_error(&err))?;
    if before.len() > max_bytes {
        return Err(error(
            "SFTP_FILE_TOO_LARGE",
            "文件超过在线编辑上限，请使用本地编辑器处理",
            false,
        ));
    }
    let mut file = fs::File::open(&resolved).map_err(|err| local_read_error(&err))?;
    let mut bytes = Vec::with_capacity(before.len() as usize);
    std::io::Read::by_ref(&mut file)
        .take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| local_read_error(&err))?;
    // 读取完成后重新 stat：size/mtime 任一变化、路径被换成链接或其它类型都拒绝，
    // 防止最终组件在读取期间被替换后仍返回错误身份的内容。
    let after = fs::symlink_metadata(&resolved).map_err(|err| local_read_error(&err))?;
    let (before_seconds, before_nanos) = split_modified(&before);
    let (modified_seconds, modified_nanos) = split_modified(&after);
    if after.file_type().is_symlink()
        || !after.is_file()
        || before.len() != after.len()
        || before_seconds != modified_seconds
        || before_nanos != modified_nanos
    {
        return Err(error(
            "SFTP_LOCAL_READ_CHANGED",
            "本地文件在读取期间发生变化，请重试",
            true,
        ));
    }
    Ok(FileSnapshot {
        bytes,
        // 返回真实绝对路径，让相对路径和祖先目录别名合并为同一编辑标签。
        // canonicalize 在最终组件校验后执行，不能用它绕过符号链接拒绝。
        identity_path: fs::canonicalize(&resolved)
            .map_err(|err| local_read_error(&err))?
            .to_string_lossy()
            .into_owned(),
        version: FileVersion {
            size: Some(after.len()),
            modified_seconds: Some(modified_seconds),
            modified_nanos: Some(modified_nanos),
        },
    })
}

/// 保存本地文本文件：先按基线做冲突校验（force 仅跳过冲突），同目录写隐藏
/// 暂存文件并同步落盘，保留原文件普通权限后原子替换；任何失败保留原文件。
fn write_local_file(
    path: String,
    content: &[u8],
    baseline_size: Option<u64>,
    baseline_modified_seconds: Option<u64>,
    baseline_modified_nanos: Option<u32>,
    force: bool,
) -> Result<FileVersion, FileEditError> {
    // 内容已由 Application 校验，Adapter 只处理身份、权限与文件事务。
    let resolved = resolve_local(Some(path))?;
    validate_local_edit_target(&resolved)?;
    // 直接探测写权限：只读目标在这里失败，而不是靠父目录可写掩盖提交失败。
    // append 无写入语义之外的效果，仅为不截断地探测文件可写性。
    if fs::OpenOptions::new().append(true).open(&resolved).is_err() {
        return Err(error(
            "SFTP_LOCAL_PERMISSION_DENIED",
            "没有写入该本地文件的权限，请检查文件只读状态",
            true,
        ));
    }

    let original_metadata = fs::metadata(&resolved).map_err(|err| local_read_error(&err))?;
    if !force
        && !local_file_unchanged(
            &original_metadata,
            baseline_size,
            baseline_modified_seconds,
            baseline_modified_nanos,
        )
    {
        return Err(error(
            "SFTP_LOCAL_FILE_CHANGED",
            "本地文件已被修改或删除，保存可能覆盖外部改动",
            true,
        ));
    }

    let dir = resolved
        .parent()
        .ok_or_else(|| error("SFTP_LOCAL_PATH_INVALID", "无法定位本地父目录", false))?
        .to_path_buf();
    // 暂存文件与目标同目录保证同文件系统内原子改名；create_new 防止并发覆盖。
    let staged = dir.join(temp_name(
        resolved
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or_else(|| error("SFTP_LOCAL_PATH_INVALID", "无法识别本地文件名", false))?,
        &super::unique_suffix()
            .map_err(|_| error("SFTP_LOCAL_WRITE_FAILED", "无法创建安全暂存名称", true))?,
        "part",
    ));
    let save_result = (|| -> std::io::Result<FileVersion> {
        // create_new + 0600 从首字节开始保护内容，原文件权限在提交前单独恢复。
        // 随机名称不代替排他创建，两者共同防止预置链接或意外覆盖。
        let mut file = super::private_file(&staged)?;
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;
        // Unix 保留原文件权限位；Windows 暂存文件本身不继承只读属性，
        // 原文件只读已被前置写探测拒绝，替换后无需再降级权限。
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(
                std::os::unix::fs::MetadataExt::mode(&original_metadata) & 0o7777,
            ))?;
            file.sync_all()?;
        }
        drop(file);
        // 暂存文件就是本次实际写入内容，先记录它的基线再提交；不能提交后 stat 目标，
        // 否则另一进程若紧随提交修改，会被错误地当作本次保存结果。
        let staged_metadata = fs::metadata(&staged)?;
        let (staged_seconds, staged_nanos) = split_modified(&staged_metadata);
        let committed_baseline = FileVersion {
            size: Some(staged_metadata.len()),
            modified_seconds: Some(staged_seconds),
            modified_nanos: Some(staged_nanos),
        };
        // 提交前最后一次校验：staging 写盘期间文件仍可能被外部修改或替换为链接。
        // 用 symlink_metadata 明确拒绝最终组件变成符号链接，不能跟随其目标覆盖。
        let current = fs::symlink_metadata(&resolved)?;
        if current.file_type().is_symlink()
            || !current.is_file()
            || (!force
                && !local_file_unchanged(
                    &current,
                    baseline_size,
                    baseline_modified_seconds,
                    baseline_modified_nanos,
                ))
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "local file changed during save",
            ));
        }
        // 借用下载链路的备份-替换-回滚语义：提交失败时恢复原目标。
        let backup = dir.join(temp_name(
            resolved
                .file_name()
                .and_then(|v| v.to_str())
                .unwrap_or("target"),
            &super::unique_suffix()?,
            "backup",
        ));
        replace_local_path_checked(
            &staged,
            &resolved,
            &backup,
            LocalCommitExpectation {
                size: baseline_size,
                modified_seconds: baseline_modified_seconds,
                modified_nanos: baseline_modified_nanos,
                force,
            },
        )?;
        Ok(committed_baseline)
    })();
    // 暂存文件在成功提交后已被改名消费；失败路径必须清理，避免残留隐藏文件。
    if save_result.is_err() {
        remove_path(&staged);
    }
    save_result.map_err(|err| {
        if err.kind() == std::io::ErrorKind::WouldBlock {
            error(
                "SFTP_LOCAL_FILE_CHANGED",
                "本地文件在保存期间被修改，请重新加载后再试",
                true,
            )
        } else {
            error(
                "SFTP_LOCAL_WRITE_FAILED",
                format!("保存本地文件失败：{err}"),
                true,
            )
        }
    })
}
