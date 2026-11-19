//! SFTP 文件传输：上传/下载的文件与目录树链路、原子提交、取消与进度，
//! 以及本地磁盘 IO 的阻塞池派发。
//!
//! 传输在命令层的独立线程中调用，进度经原子计数轮询上报，取消经 `AtomicBool`
//! 协作式检查；慢速磁盘（网络盘、外置盘）上的本地读写全部走阻塞池，不占用
//! SFTP 运行时的工作线程。

use std::{
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use russh_sftp::{
    client::SftpSession,
    protocol::{FileAttributes, OpenFlags},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    error::{SftpError, map_sftp_error},
    path::join_remote,
    session::SftpConnection,
};
use crate::ssh::sftp::TRANSFER_CHUNK_BYTES;

#[cfg(test)]
#[path = "transfer_tests.rs"]
mod tests;

/// 把本地磁盘 IO 派发到阻塞线程池执行。
///
/// SFTP 运行时的工作线程只负责推进网络协议；慢速磁盘（网络盘、外置盘）上的
/// 元数据调用、目录枚举和逐块读写若直接占用工作线程，会饿死同一运行时上其他
/// SFTP 会话的异步任务。本地 IO 的失败语义（`localReadFailed` / `localWriteFailed`）
/// 由闭包自行映射；本函数只兜底闭包线程异常终止（panic）的情况，归一化为
/// `unknown` 且不触发会话剔除——本地任务崩溃不代表网络连接已断。
pub(super) async fn run_local_io<T>(
    op: impl FnOnce() -> Result<T, SftpError> + Send + 'static,
) -> Result<T, SftpError>
where
    T: Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|_| SftpError::new("unknown", "本地文件任务异常终止"))?
}

/// 任务名称跨应用实例唯一；随机源失败时拒绝写入，不退回可预测名称。
fn task_scoped_remote_name(name: &str, suffix: &str) -> Result<String, SftpError> {
    let unique = crate::file_edit::unique_suffix()
        .map_err(|_| SftpError::new("unknown", "无法创建安全暂存名称"))?;
    Ok(format!(".{name}.nocterm-{unique}.{suffix}"))
}

/// 上传入口：以本地路径的文件名作为远程目标名，按类型分派到文件或目录处理。
pub(super) async fn do_upload(
    connection: &SftpConnection,
    local_path: PathBuf,
    remote_dir: String,
    cancel: Arc<AtomicBool>,
    transferred: Arc<AtomicU64>,
) -> Result<(), SftpError> {
    let name = local_path
        .file_name()
        .map(|value| value.to_string_lossy().to_string())
        .ok_or_else(|| SftpError::new("invalidPath", "本地路径缺少文件名"))?;
    // 用 symlink_metadata 判断本地类型，符号链接目录不被误当作目录递归；
    // 探测走阻塞池，网络盘上单次 stat 也可能长时间不返回。
    let metadata = {
        let probe_path = local_path.clone();
        run_local_io(move || {
            std::fs::symlink_metadata(&probe_path)
                .map_err(|error| SftpError::new("localReadFailed", error.to_string()))
        })
        .await?
    };
    if metadata.is_dir() {
        let dest = join_remote(&remote_dir, &name);
        upload_tree(&connection.sftp, local_path, dest, &cancel, &transferred).await
    } else {
        upload_file_committed(
            &connection.sftp,
            &local_path,
            &remote_dir,
            &name,
            None,
            &cancel,
            &transferred,
        )
        .await
        .map(|_| ())
    }
}

/// 编辑保存直接从内存分块写远端私有暂存，不把远程文本落到本机临时目录。
pub(super) async fn do_save_file(
    connection: &SftpConnection,
    remote_path: String,
    content: Vec<u8>,
    expectation: RemoteCommitExpectation,
    cancel: Arc<AtomicBool>,
) -> Result<RemoteCommittedStat, SftpError> {
    save_text(&connection.sftp, remote_path, content, expectation, cancel).await
}

/// 协议层仅依赖 SFTP 会话，失败路径可用内存字节流测试，无需访问真实服务器。
async fn save_text(
    sftp: &SftpSession,
    remote_path: String,
    content: Vec<u8>,
    expectation: RemoteCommitExpectation,
    cancel: Arc<AtomicBool>,
) -> Result<RemoteCommittedStat, SftpError> {
    let (parent, name) = remote_path
        .rsplit_once('/')
        .filter(|(_, name)| !name.is_empty())
        .ok_or_else(|| SftpError::new("invalidPath", "远程路径缺少文件名"))?;
    let parent = if parent.is_empty() { "/" } else { parent };
    let temp = join_remote(parent, &task_scoped_remote_name(name, "part")?);
    // CREATE+EXCLUDE 不能跟随预先布置的链接；权限随 OPEN 发出，先保护再写第一字节。
    let mut file = sftp
        .open_with_flags_and_attributes(
            temp.clone(),
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            FileAttributes {
                permissions: Some(0o600),
                ..FileAttributes::default()
            },
        )
        .await
        .map_err(map_sftp_error)?;
    let result = async {
        for chunk in content.chunks(TRANSFER_CHUNK_BYTES) {
            if cancel.load(Ordering::Relaxed) {
                return Err(SftpError::cancelled());
            }
            file.write_all(chunk)
                .await
                .map_err(|error| SftpError::lost(format!("写入远程失败：{error}")))?;
        }
        file.flush()
            .await
            .map_err(|error| SftpError::lost(format!("刷新远程失败：{error}")))?;
        file.shutdown()
            .await
            .map_err(|error| SftpError::lost(format!("关闭远程失败：{error}")))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(SftpError::cancelled());
        }
        commit_remote(sftp, &temp, &remote_path, Some(expectation)).await
    }
    .await;
    if result.is_err() {
        let _ = file.shutdown().await;
        let _ = sftp.remove_file(temp).await;
    }
    result
}

/// 文件上传：先流式写入临时文件，未取消再原子提交为目标名；取消或失败清理临时文件。
async fn upload_file_committed(
    sftp: &SftpSession,
    local_path: &Path,
    remote_dir: &str,
    name: &str,
    expectation: Option<RemoteCommitExpectation>,
    cancel: &Arc<AtomicBool>,
    transferred: &Arc<AtomicU64>,
) -> Result<RemoteCommittedStat, SftpError> {
    let dest = join_remote(remote_dir, name);
    let temp = join_remote(remote_dir, &task_scoped_remote_name(name, "part")?);
    if let Err(error) = upload_file_raw(sftp, local_path, &temp, cancel, transferred).await {
        // 失败即尽力清理临时文件，避免残留污染远程目录。
        let _ = sftp.remove_file(temp.clone()).await;
        return Err(error);
    }
    // 提交前再判一次取消：已取消则不落盘，清理临时文件后返回取消。
    if cancel.load(Ordering::Relaxed) {
        let _ = sftp.remove_file(temp.clone()).await;
        return Err(SftpError::cancelled());
    }
    // commit 返回临时文件自身的 stat：这是本次写入的基线，不能在目标路径再 stat，
    // 否则提交后别人立刻改文件时会把对方版本误认成本次保存结果。
    match commit_remote(sftp, &temp, &dest, expectation).await {
        Ok(stat) => Ok(stat),
        Err(error) => {
            let _ = sftp.remove_file(temp).await;
            Err(error)
        }
    }
}

/// 提交完成前从临时文件读取的 stat：代表本次实际写入内容的版本。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RemoteCommittedStat {
    pub size: Option<u64>,
    pub modified_at: Option<u64>,
}

/// 编辑保存提交前的目标快照：`force=false` 时 size/mtime 必须仍与打开时一致。
#[derive(Debug, Clone, Copy)]
pub(super) struct RemoteCommitExpectation {
    pub size: Option<u64>,
    pub modified_at: Option<u64>,
    pub force: bool,
}

/// 原子提交：目标已存在则先备份再改名，失败回滚备份；目标不存在则直接改名。
/// 编辑保存可携带预期快照，在临时文件完整写入之后再做最终冲突校验。
async fn commit_remote(
    sftp: &SftpSession,
    temp: &str,
    dest: &str,
    expectation: Option<RemoteCommitExpectation>,
) -> Result<RemoteCommittedStat, SftpError> {
    // 编辑器使用本次写入的临时文件作新基线。先占位，等权限 SETSTAT 完成后
    // 再读取临时文件 metadata；普通上传不消费基线，避免额外 stat。
    let mut committed = RemoteCommittedStat {
        size: None,
        modified_at: None,
    };
    // 编辑保存看最终组件本身并拒绝链接；普通上传保持既有 metadata 跟随语义，
    // 避免本次功能顺带改变同名符号链接上传行为。一次 metadata 同时承担存在性
    // 与类型判断，消除 try_exists→metadata 之间目标被删除的额外竞态窗口。
    let metadata_result = if expectation.is_some() {
        sftp.symlink_metadata(dest.to_string())
            .await
            .map_err(map_sftp_error)
    } else {
        sftp.metadata(dest.to_string())
            .await
            .map_err(map_sftp_error)
    };
    let metadata = match metadata_result {
        Ok(metadata) => metadata,
        Err(error) if error.code == "pathMissing" => {
            if expectation.is_some_and(|expected| !expected.force) {
                return Err(SftpError::new("remoteChanged", "远端文件在保存期间被删除"));
            }
            if expectation.is_some() {
                let staged = sftp
                    .metadata(temp.to_string())
                    .await
                    .map_err(map_sftp_error)?;
                committed = RemoteCommittedStat {
                    size: staged.size,
                    modified_at: staged.mtime.map(u64::from),
                };
            }
            sftp.rename(temp.to_string(), dest.to_string())
                .await
                .map_err(map_sftp_error)?;
            return Ok(committed);
        }
        Err(error) => return Err(error),
    };
    if expectation.is_some() && metadata.is_symlink() {
        return Err(SftpError::new(
            "fileSymlink",
            format!("远端目标 {dest} 是符号链接，请直接编辑真实文件"),
        ));
    }
    if metadata.is_dir() {
        return Err(SftpError::new(
            "invalidPath",
            format!("远端已存在同名目录 {dest}，请改用其它文件名或先删除该目录"),
        ));
    }
    if let Some(expected) = expectation {
        let changed =
            metadata.size != expected.size || metadata.mtime.map(u64::from) != expected.modified_at;
        if !expected.force && changed {
            return Err(SftpError::new("remoteChanged", "远端文件在保存期间被修改"));
        }
        // 临时文件由 create 新建，默认 mode 可能与目标不同；提交前复制普通权限位，
        // 避免脚本丢失可执行位或敏感配置被意外放宽。uid/gid 不复制（通常无权限）。
        if let Some(permissions) = metadata.permissions {
            sftp.set_metadata(
                temp.to_string(),
                FileAttributes {
                    permissions: Some(permissions & 0o7777),
                    ..FileAttributes::default()
                },
            )
            .await
            .map_err(map_sftp_error)?;
        }
        let staged = sftp
            .metadata(temp.to_string())
            .await
            .map_err(map_sftp_error)?;
        committed = RemoteCommittedStat {
            size: staged.size,
            modified_at: staged.mtime.map(u64::from),
        };
    }
    let dest_name = dest
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("target");
    let parent = dest
        .rsplit_once('/')
        .map(|(dir, _)| if dir.is_empty() { "/" } else { dir })
        .unwrap_or(".");
    let backup = join_remote(parent, &task_scoped_remote_name(dest_name, "backup")?);
    // 任务专属备份名不存在共享清理步骤，并发提交不会删除彼此回滚点。
    if let Err(error) = sftp.rename(dest.to_string(), backup.clone()).await {
        let _ = sftp.remove_file(temp.to_string()).await;
        return Err(map_sftp_error(error));
    }
    match sftp.rename(temp.to_string(), dest.to_string()).await {
        Ok(()) => {
            // 提交成功后删除备份；删除失败不影响正确性，忽略即可。
            let _ = sftp.remove_file(backup).await;
            Ok(committed)
        }
        Err(error) => {
            // 回滚：把备份改回目标名；若回滚失败，不能静默丢掉备份路径。
            let mapped = map_sftp_error(error);
            if let Err(rollback_error) = sftp.rename(backup.clone(), dest.to_string()).await {
                return Err(SftpError::new(
                    "unknown",
                    format!(
                        "远端提交失败：{}；回滚失败：{}；备份保留在 {}",
                        mapped.message, rollback_error, backup
                    ),
                ));
            }
            let _ = sftp.remove_file(temp.to_string()).await;
            Err(mapped)
        }
    }
}

/// 流式上传单个文件：本地按块读取（阻塞池）→ 远程异步写入，逐块累计进度并检查取消。
async fn upload_file_raw(
    sftp: &SftpSession,
    local_path: &Path,
    remote: &str,
    cancel: &Arc<AtomicBool>,
    transferred: &Arc<AtomicU64>,
) -> Result<(), SftpError> {
    // 打开本地文件同样可能阻塞在慢速磁盘上，与逐块读取一样走阻塞池。
    let mut local = {
        let open_path = local_path.to_path_buf();
        run_local_io(move || {
            std::fs::File::open(&open_path)
                .map_err(|error| SftpError::new("localReadFailed", error.to_string()))
        })
        .await?
    };
    // create 语义为 创建+截断+写入，等价覆盖式新建，用于写入临时文件。
    let mut remote_file = sftp
        .create(remote.to_string())
        .await
        .map_err(map_sftp_error)?;
    let mut buffer = vec![0u8; TRANSFER_CHUNK_BYTES];
    loop {
        // 每块前检查取消：中断则关闭已写部分并返回取消（临时文件由上层清理）。
        if cancel.load(Ordering::Relaxed) {
            let _ = remote_file.shutdown().await;
            return Err(SftpError::cancelled());
        }
        // 读取在阻塞池执行；文件句柄与缓冲区随任务往返（仅所有权传递，无数据拷贝），
        // 同一时刻同一句柄只有一个读取在途，无需额外同步。
        let (local_returned, buffer_returned, read) = {
            run_local_io(move || {
                let read = std::io::Read::read(&mut local, &mut buffer)
                    .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
                Ok((local, buffer, read))
            })
            .await?
        };
        local = local_returned;
        buffer = buffer_returned;
        if read == 0 {
            break;
        }
        remote_file
            .write_all(&buffer[..read])
            .await
            .map_err(|error| SftpError::lost(format!("写入远程失败：{error}")))?;
        transferred.fetch_add(read as u64, Ordering::Relaxed);
    }
    // flush + shutdown 确保缓冲落盘并正常关闭远程句柄。
    remote_file
        .flush()
        .await
        .map_err(|error| SftpError::lost(format!("刷新远程写入失败：{error}")))?;
    remote_file
        .shutdown()
        .await
        .map_err(|error| SftpError::lost(format!("关闭远程文件失败：{error}")))?;
    Ok(())
}

/// 收集本地目录的直接子项（阻塞调用，必须经 `run_local_io` 派发）：一次性取回
/// 路径、无损名称与类型，避免在递归 await 之间持有 `ReadDir` 迭代器，也让目录
/// 枚举的磁盘阻塞集中发生在阻塞池内。
pub(super) fn collect_local_children(
    local_dir: &Path,
) -> Result<Vec<(PathBuf, String, bool)>, SftpError> {
    let read = std::fs::read_dir(local_dir)
        .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
    let mut children = Vec::new();
    for entry in read {
        let entry = entry.map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
        // 本地名转远端名必须无损：`to_string_lossy` 会把非 UTF-8 字节换成 U+FFFD，
        // 于是"上传成功"但远端文件名已被悄悄改写。显式拒绝比静默改名好。
        let raw_name = entry.file_name();
        let Some(name) = raw_name.to_str().map(str::to_string) else {
            return Err(SftpError::new(
                "localReadFailed",
                format!(
                    "本地文件名不是合法 UTF-8，无法无损映射到远端：{}",
                    raw_name.to_string_lossy()
                ),
            ));
        };
        let file_type = entry
            .file_type()
            .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
        children.push((entry.path(), name, file_type.is_dir()));
    }
    Ok(children)
}

/// 递归上传目录：确保远程目录存在后逐项处理，子目录递归、文件直接流式上传。
fn upload_tree<'a>(
    sftp: &'a SftpSession,
    local_dir: PathBuf,
    remote_dir: String,
    cancel: &'a Arc<AtomicBool>,
    transferred: &'a Arc<AtomicU64>,
) -> Pin<Box<dyn Future<Output = Result<(), SftpError>> + Send + 'a>> {
    Box::pin(async move {
        // 目录不存在才创建，已存在则复用（区别于权限等真实错误由 map_sftp_error 上抛）。
        if !sftp
            .try_exists(remote_dir.clone())
            .await
            .map_err(map_sftp_error)?
        {
            sftp.create_dir(remote_dir.clone())
                .await
                .map_err(map_sftp_error)?;
        }
        // 先在阻塞池内收齐子项再逐个上传：枚举期间不再持有跨 await 的本地迭代器。
        let children = {
            let scan_dir = local_dir.clone();
            run_local_io(move || collect_local_children(&scan_dir)).await?
        };
        for (child_local, name, is_dir) in children {
            if cancel.load(Ordering::Relaxed) {
                return Err(SftpError::cancelled());
            }
            let child_remote = join_remote(&remote_dir, &name);
            if is_dir {
                upload_tree(sftp, child_local, child_remote, cancel, transferred).await?;
            } else {
                // 目录内文件直接写入目标名（不逐个临时提交），语义与整目录传输一致。
                upload_file_raw(sftp, &child_local, &child_remote, cancel, transferred).await?;
            }
        }
        Ok(())
    })
}

/// 下载入口：以远程 stat 为准判断类型（跟随符号链接），分派到文件或目录处理。
pub(super) async fn do_download(
    connection: &SftpConnection,
    remote_path: String,
    staged_local: PathBuf,
    cancel: Arc<AtomicBool>,
    transferred: Arc<AtomicU64>,
) -> Result<(), SftpError> {
    // metadata 跟随符号链接，链接指向目录则按目录镜像，指向文件则按文件下载。
    let metadata = connection
        .sftp
        .metadata(remote_path.clone())
        .await
        .map_err(map_sftp_error)?;
    if metadata.is_dir() {
        download_tree(
            &connection.sftp,
            remote_path,
            staged_local,
            &cancel,
            &transferred,
        )
        .await
    } else {
        download_file_raw(
            &connection.sftp,
            &remote_path,
            &staged_local,
            &cancel,
            &transferred,
        )
        .await
    }
}

/// 流式下载单个文件到本地暂存路径：远程异步读取 → 本地写入（阻塞池），逐块累计进度与取消。
async fn download_file_raw(
    sftp: &SftpSession,
    remote: &str,
    staged_local: &Path,
    cancel: &Arc<AtomicBool>,
    transferred: &Arc<AtomicU64>,
) -> Result<(), SftpError> {
    // 确保暂存文件的父目录存在，避免因中间目录缺失而写入失败；目录创建走阻塞池。
    if let Some(parent) = staged_local.parent() {
        let parent = parent.to_path_buf();
        run_local_io(move || {
            std::fs::create_dir_all(&parent)
                .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))
        })
        .await?;
    }
    let mut remote_file = sftp
        .open(remote.to_string())
        .await
        .map_err(map_sftp_error)?;
    let mut local = {
        let create_path = staged_local.to_path_buf();
        run_local_io(move || {
            std::fs::File::create(&create_path)
                .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))
        })
        .await?
    };
    let mut buffer = vec![0u8; TRANSFER_CHUNK_BYTES];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(SftpError::cancelled());
        }
        let read = remote_file
            .read(&mut buffer)
            .await
            .map_err(|error| SftpError::lost(format!("读取远程失败：{error}")))?;
        if read == 0 {
            break;
        }
        // 本地写入在阻塞池执行；句柄与缓冲区随任务往返（仅所有权传递，无数据拷贝），
        // 同一时刻同一句柄只有一个写入在途。
        let (local_returned, buffer_returned) = {
            run_local_io(move || {
                std::io::Write::write_all(&mut local, &buffer[..read])
                    .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
                Ok((local, buffer))
            })
            .await?
        };
        local = local_returned;
        buffer = buffer_returned;
        transferred.fetch_add(read as u64, Ordering::Relaxed);
    }
    // flush 同样可能阻塞在慢速磁盘；完成后句柄在闭包内自然关闭。
    run_local_io(move || {
        std::io::Write::flush(&mut local)
            .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
        Ok(())
    })
    .await?;
    Ok(())
}

/// 受限读取远程文件：先 stat 判型并预检大小，再打开流式读取，逐块累计超过
/// `max_bytes` 立即拒绝——大文件不进入内存。stat 快照与内容出自同一次操作，
/// 供命令层把 size/mtime 作为编辑冲突基线返回给前端。
pub(super) async fn do_read_remote_file(
    connection: &SftpConnection,
    remote_path: String,
    max_bytes: u64,
) -> Result<super::RemoteFileRead, SftpError> {
    // 在线编辑拒绝最终符号链接：若先跟随读取目标，保存时原子改名会替换链接本身，
    // 读写语义不一致且可能破坏目录结构。
    let metadata = connection
        .sftp
        .symlink_metadata(remote_path.clone())
        .await
        .map_err(map_sftp_error)?;
    if metadata.is_symlink() {
        return Err(SftpError::new(
            "fileSymlink",
            format!("目标是符号链接，请直接打开真实文件：{remote_path}"),
        ));
    }
    if metadata.is_dir() {
        return Err(SftpError::new(
            "invalidPath",
            format!("目标是目录，无法作为文本文件读取：{remote_path}"),
        ));
    }
    // 服务端已报告大小且超限时直接拒绝，避免为注定失败的读取建立数据流。
    if metadata.size.is_some_and(|size| size > max_bytes) {
        return Err(file_too_large(max_bytes));
    }
    let mut remote_file = connection
        .sftp
        .open(remote_path.clone())
        .await
        .map_err(map_sftp_error)?;
    let mut bytes = Vec::new();
    let mut buffer = vec![0u8; TRANSFER_CHUNK_BYTES];
    loop {
        let read = remote_file
            .read(&mut buffer)
            .await
            .map_err(|error| SftpError::lost(format!("读取远程失败：{error}")))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        // stat 与 open 之间文件可能被替换或增长，读取侧仍需守住上限。
        if bytes.len() as u64 > max_bytes {
            return Err(file_too_large(max_bytes));
        }
    }
    // 读取完成后再次 lstat：链接/类型/size/mtime 任一变化都拒绝本次快照。
    let after = connection
        .sftp
        .symlink_metadata(remote_path)
        .await
        .map_err(map_sftp_error)?;
    if after.is_symlink()
        || after.is_dir()
        || after.size != metadata.size
        || after.mtime != metadata.mtime
    {
        return Err(SftpError::new(
            "remoteReadChanged",
            "远端文件在读取期间发生变化，请重试",
        ));
    }
    Ok(super::RemoteFileRead {
        bytes,
        size: after.size,
        modified_at: after.mtime.map(u64::from),
    })
}

/// 构造统一文案的「超过编辑上限」错误，stat 预检与流式读取两处共用。
fn file_too_large(max_bytes: u64) -> SftpError {
    SftpError::new(
        "fileTooLarge",
        format!("文件超过 {max_bytes} 字节的在线编辑上限，请下载后本地编辑"),
    )
}

/// 递归下载目录到本地暂存目录：创建本地目录后逐项镜像，子目录递归、文件流式下载。
fn download_tree<'a>(
    sftp: &'a SftpSession,
    remote_dir: String,
    local_dir: PathBuf,
    cancel: &'a Arc<AtomicBool>,
    transferred: &'a Arc<AtomicU64>,
) -> Pin<Box<dyn Future<Output = Result<(), SftpError>> + Send + 'a>> {
    Box::pin(async move {
        // 本地目录创建走阻塞池，网络盘上的递归建目录同样可能长时间阻塞。
        {
            let create_dir = local_dir.clone();
            run_local_io(move || {
                std::fs::create_dir_all(&create_dir)
                    .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))
            })
            .await?;
        }
        let read = sftp
            .read_dir(remote_dir.clone())
            .await
            .map_err(map_sftp_error)?;
        // 先收集条目再逐个处理，避免在递归 await 期间持有 ReadDir 迭代器。
        let children: Vec<(String, bool)> = read
            .map(|item| (item.file_name(), item.file_type().is_dir()))
            .collect();
        for (name, is_dir) in children {
            if cancel.load(Ordering::Relaxed) {
                return Err(SftpError::cancelled());
            }
            // 拼本地路径前先校验：远端名字决定写到哪个本地文件。
            super::path::ensure_safe_child_name(&name)?;
            let child_remote = join_remote(&remote_dir, &name);
            let child_local = local_dir.join(&name);
            if is_dir {
                download_tree(sftp, child_remote, child_local, cancel, transferred).await?;
            } else {
                download_file_raw(sftp, &child_remote, &child_local, cancel, transferred).await?;
            }
        }
        Ok(())
    })
}
