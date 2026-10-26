//! 进程内 SFTP 后端：在 russh 连接之上请求 `sftp` 子系统，用 `russh-sftp` 完成
//! 结构化的目录浏览、文件管理与流式传输，复用同一套建连、认证与主机密钥策略。
//!
//! 模块结构：
//! - `error`：`SftpError` 与协议错误归一化（含会话剔除标记）；
//! - `path`：远程路径拼接与本地写入安全校验；
//! - `session`：会话池、建连与复用；
//! - `transfer`：上传/下载链路、原子提交、取消/进度与本地 IO 阻塞池派发。
//!
//! 设计要点：
//! - 独立的多线程 tokio 运行时承载所有 SFTP 会话，避免阻塞终端运行时；
//! - 每个连接维护一个会话（含底层 russh 连接句柄），按 `connection_id` 汇聚到
//!   会话池中复用，避免每次目录操作都重新建连；
//! - 所有对外方法都是同步的：内部把异步任务派发到运行时，再用同步通道桥接结果，
//!   保持与命令层既有的同步调用契约一致。

mod error;
mod path;
mod session;
mod transfer;

pub use error::SftpError;

use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64},
    },
};

use nocterm_domain::connection::ConnectionProfile;
use russh_sftp::client::SftpSession;
use tokio::runtime::Runtime;

use error::map_sftp_error;
use path::join_remote;
use session::{SessionPool, connect_params, ensure_session, evict_session};
use transfer::{do_download, do_upload};

/// 单次读写的块大小。取 russh-sftp 默认 `max_packet_len`（256 KiB）留出协议头后的
/// 安全值：下载路径在库内是**严格串行**的（同一时刻只有一个 SSH_FXP_READ 在途），
/// 吞吐上限约为"块大小 ÷ RTT"，64 KiB 在 30 ms 链路上只有 ~2 MB/s，加大块能直接翻倍。
/// 不取满 256 KiB 是因为 SFTP 数据包头与 SSH 通道窗口都要占额度，贴边容易被对端拒收。
pub(super) const TRANSFER_CHUNK_BYTES: usize = 128 * 1024;

/// 远程目录项：字段与命令层 `RemoteFileEntry` 对齐，避免跨层再做结构转换。
#[derive(Debug, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified_at: Option<u64>,
}

/// 目录列举结果：`path` 为服务端规范化后的绝对路径（等价 `pwd -P`）。
#[derive(Debug, Clone)]
pub struct SftpListing {
    pub path: String,
    pub entries: Vec<SftpEntry>,
}

/// SFTP 会话管理器：统一持有运行时与会话池，向命令层暴露同步操作接口。
pub struct SftpManager {
    runtime: Runtime,
    sessions: SessionPool,
}

impl Default for SftpManager {
    fn default() -> Self {
        // 多线程运行时保证同步方法阻塞等待结果时，会话任务仍能在其它工作线程推进。
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("构建 SFTP 运行时失败");
        Self {
            runtime,
            sessions: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }
}

impl SftpManager {
    /// 在运行时上派发异步任务并同步阻塞等待其结果；任务异常退出归一化为未知错误。
    fn run<T>(
        &self,
        future: impl Future<Output = Result<T, SftpError>> + Send + 'static,
    ) -> Result<T, SftpError>
    where
        T: Send + 'static,
    {
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        self.runtime.spawn(async move {
            let _ = result_tx.send(future.await);
        });
        result_rx
            .recv()
            .unwrap_or_else(|_| Err(SftpError::new("unknown", "SFTP 操作异常终止")))
    }

    /// 获取（或新建）连接会话后执行操作；操作若报连接断开则从池中剔除以便下次重连。
    fn run_op<T, F, Fut>(
        &self,
        connection_id: i64,
        params: session::ConnectParams,
        op: F,
    ) -> Result<T, SftpError>
    where
        F: FnOnce(Arc<session::SftpConnection>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, SftpError>> + Send,
        T: Send + 'static,
    {
        let sessions = self.sessions.clone();
        self.run(async move {
            let connection = ensure_session(&sessions, connection_id, params).await?;
            match op(connection).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    if error.evict {
                        evict_session(&sessions, connection_id);
                    }
                    Err(error)
                }
            }
        })
    }

    /// 列举远程目录：返回服务端规范化后的绝对路径与排序前的原始条目集合。
    pub fn list_dir(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        path: &str,
    ) -> Result<SftpListing, SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let path = path.to_string();
        self.run_op(profile.id, params, move |connection| async move {
            do_list_dir(connection.as_ref(), &path).await
        })
    }

    /// 判断远程路径是否存在；跟随符号链接，父目录不可访问时以错误形式上抛。
    pub fn exists(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        path: &str,
    ) -> Result<bool, SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let path = path.to_string();
        self.run_op(profile.id, params, move |connection| async move {
            connection
                .sftp
                .try_exists(path)
                .await
                .map_err(map_sftp_error)
        })
    }

    /// 创建远程目录（单层）。父目录不存在或权限不足时由错误码区分。
    pub fn create_dir(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        path: &str,
    ) -> Result<(), SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let path = path.to_string();
        self.run_op(profile.id, params, move |connection| async move {
            connection
                .sftp
                .create_dir(path)
                .await
                .map_err(map_sftp_error)
        })
    }

    /// 重命名（移动）远程路径。
    pub fn rename(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        from: &str,
        to: &str,
    ) -> Result<(), SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let (from, to) = (from.to_string(), to.to_string());
        self.run_op(profile.id, params, move |connection| async move {
            connection
                .sftp
                .rename(from, to)
                .await
                .map_err(map_sftp_error)
        })
    }

    /// 递归删除远程路径：目录先逐项清空再删除目录，符号链接按自身删除不跟随。
    pub fn remove(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        path: &str,
    ) -> Result<(), SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let path = path.to_string();
        self.run_op(profile.id, params, move |connection| async move {
            remove_recursive(&connection.sftp, path).await
        })
    }

    /// 上传本地文件或目录到远程目录内（以本地名为目标名）：文件走临时+提交的原子替换，
    /// 目录递归创建并逐项流式上传；`cancel` 协作式取消，`transferred` 累计已传字节供轮询。
    // 参数均为独立且必需的入口信息（连接、三类凭据、双端路径、取消与进度信号），
    // 强行合并成结构体只会把间接层贯穿整条递归辅助函数链，故在入口处放行该风格 lint。
    #[allow(clippy::too_many_arguments)]
    pub fn upload(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        local_path: &Path,
        remote_dir: &str,
        cancel: Arc<AtomicBool>,
        transferred: Arc<AtomicU64>,
    ) -> Result<(), SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let local_path = local_path.to_path_buf();
        let remote_dir = remote_dir.to_string();
        self.run_op(profile.id, params, move |connection| async move {
            do_upload(
                connection.as_ref(),
                local_path,
                remote_dir,
                cancel,
                transferred,
            )
            .await
        })
    }

    /// 下载远程文件或目录到本地暂存路径（原子替换由命令层完成）：文件流式写入，
    /// 目录递归镜像；类型以远程 stat 为准，`cancel`/`transferred` 语义同上传。
    // 同 `upload`：入参各自独立且必需，放行 too_many_arguments 风格 lint。
    #[allow(clippy::too_many_arguments)]
    pub fn download(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
        remote_path: &str,
        staged_local: &Path,
        cancel: Arc<AtomicBool>,
        transferred: Arc<AtomicU64>,
    ) -> Result<(), SftpError> {
        let params = connect_params(profile, password, private_key)?;
        let remote_path = remote_path.to_string();
        let staged_local = staged_local.to_path_buf();
        self.run_op(profile.id, params, move |connection| async move {
            do_download(
                connection.as_ref(),
                remote_path,
                staged_local,
                cancel,
                transferred,
            )
            .await
        })
    }

    /// 关闭并剔除某连接的 SFTP 会话；不存在视为成功，保证重复调用安全。
    pub fn close_connection(&self, connection_id: i64) {
        evict_session(&self.sessions, connection_id);
    }

    /// 剔除全部会话，用于应用退出时统一释放底层连接。
    pub fn shutdown(&self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.clear();
        }
    }
}

/// 列举远程目录：先规范化出绝对路径，再遍历条目并抽取类型、大小与修改时间。
async fn do_list_dir(
    connection: &session::SftpConnection,
    path: &str,
) -> Result<SftpListing, SftpError> {
    // 规范化为服务端绝对路径（等价 pwd -P），便于前端展示与后续拼接。
    let canonical = connection
        .sftp
        .canonicalize(path.to_string())
        .await
        .map_err(map_sftp_error)?;
    let read_dir = connection
        .sftp
        .read_dir(canonical.clone())
        .await
        .map_err(map_sftp_error)?;
    let mut entries = Vec::new();
    for item in read_dir {
        // read_dir 已自动跳过 "." 与 ".."，此处直接采集有效条目。
        let file_type = item.file_type();
        let metadata = item.metadata();
        entries.push(SftpEntry {
            name: item.file_name(),
            is_dir: file_type.is_dir(),
            size: metadata.size,
            // mtime 为秒级 u32，统一放大为 u64 与本地条目字段对齐。
            modified_at: metadata.mtime.map(u64::from),
        });
    }
    Ok(SftpListing {
        path: canonical,
        entries,
    })
}

/// 递归删除远程路径：目录先逐项清空再删自身，文件/符号链接按自身删除不跟随。
fn remove_recursive<'a>(
    sftp: &'a SftpSession,
    path: String,
) -> Pin<Box<dyn Future<Output = Result<(), SftpError>> + Send + 'a>> {
    Box::pin(async move {
        // 用 symlink_metadata 判断类型，确保符号链接按自身删除而非跟随其目标。
        let metadata = sftp
            .symlink_metadata(path.clone())
            .await
            .map_err(map_sftp_error)?;
        if metadata.is_dir() {
            // 目录：先递归清空每个子项，再删除目录本身。
            let read_dir = sftp.read_dir(path.clone()).await.map_err(map_sftp_error)?;
            let children: Vec<String> = read_dir
                .map(|item| join_remote(&path, &item.file_name()))
                .collect();
            for child in children {
                remove_recursive(sftp, child).await?;
            }
            sftp.remove_dir(path).await.map_err(map_sftp_error)?;
        } else {
            sftp.remove_file(path).await.map_err(map_sftp_error)?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::path::{ensure_safe_child_name, join_remote};
    use super::transfer::collect_local_children;

    #[test]
    fn joins_remote_paths_without_duplicating_the_separator() {
        assert_eq!(join_remote("/home/demo", "a.txt"), "/home/demo/a.txt");
        assert_eq!(join_remote("/", "a.txt"), "/a.txt");
    }

    /// 构造仅本测试使用的临时目录并在结束时清理；同进程内以测试名保证唯一。
    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("nocterm-sftp-test-{}-{}", std::process::id(), tag));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    #[test]
    fn collects_local_children_with_paths_names_and_types() {
        let base = scratch_dir("collect");
        std::fs::create_dir_all(base.join("sub")).expect("创建临时子目录");
        std::fs::write(base.join("a.txt"), b"x").expect("写入临时文件");

        let children = collect_local_children(&base).expect("收集子项");
        // read_dir 顺序随平台变化，统一排序后断言名称与类型被无损收集。
        let mut summary: Vec<(String, bool)> = children
            .into_iter()
            .map(|(path, name, is_dir)| {
                assert!(path.starts_with(&base), "子项路径必须位于被扫描目录内");
                (name, is_dir)
            })
            .collect();
        summary.sort();
        assert_eq!(
            summary,
            vec![("a.txt".to_string(), false), ("sub".to_string(), true)]
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn collect_local_children_reports_missing_directory_with_stable_code() {
        let missing = scratch_dir("missing");
        let error = collect_local_children(&missing).expect_err("缺失目录应报错");
        assert_eq!(error.code, "localReadFailed");
    }

    #[test]
    fn accepts_ordinary_remote_entry_names() {
        for name in [
            "a.txt",
            "中文 文件.log",
            ".bashrc",
            "..hidden",
            "a-b_c.tar.gz",
        ] {
            assert!(ensure_safe_child_name(name).is_ok(), "{name} 应被接受");
        }
    }

    #[test]
    fn rejects_entry_names_that_would_escape_the_target_directory() {
        for name in ["", ".", "..", "a/b", "/etc/passwd", "../../evil", "a\0b"] {
            let error = ensure_safe_child_name(name)
                .expect_err(&format!("{name} 应被拒绝"))
                .code;
            assert_eq!(error, "invalidPath");
        }
    }

    /// Windows 上 `\` 是分隔符、冒号是备用数据流语法，两者都必须在拼接前拦下。
    #[cfg(windows)]
    #[test]
    fn rejects_windows_specific_path_syntax() {
        for name in [r"a\b", r"C:\evil", r"..\evil", "notes.txt:secret"] {
            assert!(ensure_safe_child_name(name).is_err(), "{name} 应被拒绝");
        }
    }

    /// Unix 上反斜杠与冒号都是合法文件名字符，按 Windows 规则一律拒绝会误伤。
    #[cfg(unix)]
    #[test]
    fn keeps_unix_legal_names_that_windows_would_reject() {
        assert!(ensure_safe_child_name(r"a\b").is_ok());
        assert!(ensure_safe_child_name("notes.txt:secret").is_ok());
    }
}
