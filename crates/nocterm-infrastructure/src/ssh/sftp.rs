//! 进程内 SFTP 后端：在 russh 连接之上请求 `sftp` 子系统，用 `russh-sftp` 完成
//! 结构化的目录浏览、文件管理与流式传输，复用同一套建连、认证与主机密钥策略。
//!
//! 设计要点：
//! - 独立的多线程 tokio 运行时承载所有 SFTP 会话，避免阻塞终端运行时；
//! - 每个连接维护一个 `SftpSession`（含底层 russh 连接句柄），按 `connection_id`
//!   汇聚到会话池中复用，避免每次目录操作都重新建连；
//! - 所有对外方法都是同步的：内部把异步任务派发到运行时，再用同步通道桥接结果，
//!   保持与命令层既有的同步调用契约一致；
//! - 传输在命令层的独立线程中调用，进度经原子计数轮询上报，取消经 `AtomicBool`
//!   协作式检查，配合 select 让阻塞读写也能及时中断；
//! - 错误统一归一化为携带前端稳定 `code` 的 `SftpError`，与 `__NOCTERM_SFTP_ERROR__`
//!   契约保持一致；连接断开类错误额外标记 `evict`，触发会话池剔除以便下次重连。

use std::{
    collections::HashMap,
    future::Future,
    path::{Component, Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use nocterm_domain::connection::ConnectionProfile;
use russh::client::Handle;
use russh_sftp::{
    client::{SftpSession, error::Error as SftpProtoError},
    protocol::StatusCode,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    runtime::Runtime,
};

use super::{Auth, ClientHandler, build_auth, connect_authenticated_coded};

/// 单个 SFTP 请求的等待上限，替换 russh-sftp 的 10 秒默认值。
const SFTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// 单次读写的块大小。取 russh-sftp 默认 `max_packet_len`（256 KiB）留出协议头后的
/// 安全值：下载路径在库内是**严格串行**的（同一时刻只有一个 SSH_FXP_READ 在途），
/// 吞吐上限约为"块大小 ÷ RTT"，64 KiB 在 30 ms 链路上只有 ~2 MB/s，加大块能直接翻倍。
/// 不取满 256 KiB 是因为 SFTP 数据包头与 SSH 通道窗口都要占额度，贴边容易被对端拒收。
const TRANSFER_CHUNK_BYTES: usize = 128 * 1024;

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
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            evict: false,
        }
    }

    /// 连接断开类错误：向前端归为 `unknown`，同时标记剔除以便下次自动重连。
    fn lost(message: impl Into<String>) -> Self {
        Self {
            code: "unknown",
            message: message.into(),
            evict: true,
        }
    }

    /// 用户主动取消：命令层以取消标志判定最终状态，此处仅用于中断内部流程。
    fn cancelled() -> Self {
        Self::new("cancelled", "传输已取消")
    }
}

impl std::fmt::Display for SftpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SftpError {}

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

/// 建连所需的运行期参数，从连接资料与已解出的凭据装配后移动进异步任务。
struct ConnectParams {
    host: String,
    port: u16,
    username: String,
    auth: Auth,
}

/// 池中的单个 SFTP 会话：持有 russh 连接句柄以维持底层连接，附带其上的 SFTP 会话。
struct SftpConnection {
    /// 保活连接句柄；一旦释放，russh 会断开连接并关闭 SFTP 通道。
    _handle: Handle<ClientHandler>,
    sftp: SftpSession,
}

/// 会话池类型别名：按连接标识汇聚，供多个操作以 `Arc` 并发共享同一 SFTP 会话。
type SessionPool = Arc<Mutex<HashMap<i64, Arc<SftpConnection>>>>;

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
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl SftpManager {
    /// 依据连接资料与凭据装配建连参数；缺失必要凭据时给出明确提示。
    fn connect_params(
        &self,
        profile: &ConnectionProfile,
        password: Option<&str>,
        private_key: Option<&str>,
    ) -> Result<ConnectParams, SftpError> {
        let auth = build_auth(profile, password, private_key)
            .map_err(|error| SftpError::new("authFailed", error.to_string()))?;
        Ok(ConnectParams {
            host: profile.host.clone(),
            port: profile.port,
            username: profile.username.clone(),
            auth,
        })
    }

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
        params: ConnectParams,
        op: F,
    ) -> Result<T, SftpError>
    where
        F: FnOnce(Arc<SftpConnection>) -> Fut + Send + 'static,
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
        let params = self.connect_params(profile, password, private_key)?;
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
        let params = self.connect_params(profile, password, private_key)?;
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
        let params = self.connect_params(profile, password, private_key)?;
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
        let params = self.connect_params(profile, password, private_key)?;
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
        let params = self.connect_params(profile, password, private_key)?;
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
        let params = self.connect_params(profile, password, private_key)?;
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
        let params = self.connect_params(profile, password, private_key)?;
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

/// 从会话池取出已有连接；不存在则新建并缓存后返回，供并发操作以 `Arc` 共享。
async fn ensure_session(
    sessions: &SessionPool,
    connection_id: i64,
    params: ConnectParams,
) -> Result<Arc<SftpConnection>, SftpError> {
    // 已有会话直接复用：命中即返回，避免重复建连与主机密钥再次校验。
    if let Some(existing) = sessions
        .lock()
        .ok()
        .and_then(|pool| pool.get(&connection_id).cloned())
    {
        return Ok(existing);
    }
    // 未命中则实际建连；建连在锁外进行，避免长时间持锁阻塞其它连接的操作。
    let connection = Arc::new(establish(params).await?);
    if let Ok(mut pool) = sessions.lock() {
        // 双重检查：并发建连时以先入者为准，丢弃本次多建的连接。
        let entry = pool
            .entry(connection_id)
            .or_insert_with(|| connection.clone());
        return Ok(entry.clone());
    }
    Ok(connection)
}

/// 实际建连：认证后打开会话通道、请求 `sftp` 子系统，并在其上初始化 SFTP 会话。
async fn establish(params: ConnectParams) -> Result<SftpConnection, SftpError> {
    let handle =
        connect_authenticated_coded(&params.host, params.port, &params.username, params.auth)
            .await
            .map_err(|failure| SftpError::new(failure.code, failure.message))?;
    // 打开一个会话通道并请求 sftp 子系统；want_reply=true 让服务端确认子系统可用。
    let channel = handle
        .channel_open_session()
        .await
        .map_err(|error| SftpError::lost(format!("打开 SFTP 通道失败：{error}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|error| SftpError::lost(format!("请求 sftp 子系统失败：{error}")))?;
    // 将通道转为字节流交给 russh-sftp 完成协议初始化，得到可用的 SFTP 会话。
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|error| SftpError::lost(format!("初始化 SFTP 会话失败：{error}")))?;
    // russh-sftp 默认每个请求只等 10 秒。传输是按 `TRANSFER_CHUNK_BYTES` 逐块发请求的，
    // 在跨境链路或家用上行带宽下，单块耗时超过 10 秒并不罕见——默认值会把一次
    // 本来能成功的传输判成超时。放宽到 60 秒仍能在服务端真的失联时收口。
    sftp.set_timeout(SFTP_REQUEST_TIMEOUT.as_secs());
    Ok(SftpConnection {
        _handle: handle,
        sftp,
    })
}

/// 从会话池剔除并释放某连接；释放句柄即断开底层连接，下次操作会自动重连。
fn evict_session(sessions: &SessionPool, connection_id: i64) {
    if let Ok(mut pool) = sessions.lock() {
        pool.remove(&connection_id);
    }
}

/// 把 russh-sftp 协议错误归一化为携带前端稳定 `code` 的 `SftpError`。
fn map_sftp_error(error: SftpProtoError) -> SftpError {
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

/// 拼接远程路径：以 `/` 分隔，规避根目录下产生重复分隔符。
fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// 列举远程目录：先规范化出绝对路径，再遍历条目并抽取类型、大小与修改时间。
async fn do_list_dir(connection: &SftpConnection, path: &str) -> Result<SftpListing, SftpError> {
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

/// 上传临时文件名的单调序列，保证同目录并发上传的临时名互不冲突。
static UPLOAD_SEQ: AtomicU64 = AtomicU64::new(0);

/// 生成隐藏的上传临时名：`.{原名}.nocterm-{序号}.part`，提交成功后被改名为目标名。
fn temp_upload_name(name: &str) -> String {
    let seq = UPLOAD_SEQ.fetch_add(1, Ordering::Relaxed);
    format!(".{name}.nocterm-{seq}.part")
}

/// 上传入口：以本地路径的文件名作为远程目标名，按类型分派到文件或目录处理。
async fn do_upload(
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
    // 用 symlink_metadata 判断本地类型，符号链接目录不被误当作目录递归。
    let metadata = std::fs::symlink_metadata(&local_path)
        .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
    if metadata.is_dir() {
        let dest = join_remote(&remote_dir, &name);
        upload_tree(&connection.sftp, local_path, dest, &cancel, &transferred).await
    } else {
        upload_file_committed(
            &connection.sftp,
            &local_path,
            &remote_dir,
            &name,
            &cancel,
            &transferred,
        )
        .await
    }
}

/// 文件上传：先流式写入临时文件，未取消再原子提交为目标名；取消或失败清理临时文件。
async fn upload_file_committed(
    sftp: &SftpSession,
    local_path: &Path,
    remote_dir: &str,
    name: &str,
    cancel: &Arc<AtomicBool>,
    transferred: &Arc<AtomicU64>,
) -> Result<(), SftpError> {
    let dest = join_remote(remote_dir, name);
    let temp = join_remote(remote_dir, &temp_upload_name(name));
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
    commit_remote(sftp, &temp, &dest).await
}

/// 原子提交：目标已存在则先备份再改名，失败回滚备份；目标不存在则直接改名。
async fn commit_remote(sftp: &SftpSession, temp: &str, dest: &str) -> Result<(), SftpError> {
    let exists = sftp
        .try_exists(dest.to_string())
        .await
        .map_err(map_sftp_error)?;
    if !exists {
        return sftp
            .rename(temp.to_string(), dest.to_string())
            .await
            .map_err(map_sftp_error);
    }
    // 同名目标是目录时必须拒绝：备份逻辑会把整个目录改名成 `.nocterm-backup`，
    // 提交成功后又按"删除备份文件"清理——删不掉目录，于是远端凭空多出一个改了名的
    // 目录，用户的目录树被一次上传搬走了。上传单个文件不该有这种副作用。
    if sftp
        .metadata(dest.to_string())
        .await
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
    {
        return Err(SftpError::new(
            "invalidPath",
            format!("远端已存在同名目录 {dest}，请改用其它文件名或先删除该目录"),
        ));
    }
    let backup = format!("{dest}.nocterm-backup");
    // 清理可能残留的旧备份，避免改名到已存在的备份名而失败。
    let _ = sftp.remove_file(backup.clone()).await;
    sftp.rename(dest.to_string(), backup.clone())
        .await
        .map_err(map_sftp_error)?;
    match sftp.rename(temp.to_string(), dest.to_string()).await {
        Ok(()) => {
            // 提交成功后删除备份；删除失败不影响正确性，忽略即可。
            let _ = sftp.remove_file(backup).await;
            Ok(())
        }
        Err(error) => {
            // 回滚：把备份改回目标名，并清理未成功落盘的临时文件。
            let _ = sftp.rename(backup, dest.to_string()).await;
            let _ = sftp.remove_file(temp.to_string()).await;
            Err(map_sftp_error(error))
        }
    }
}

/// 流式上传单个文件：本地按块读取（同步）→ 远程异步写入，逐块累计进度并检查取消。
async fn upload_file_raw(
    sftp: &SftpSession,
    local_path: &Path,
    remote: &str,
    cancel: &Arc<AtomicBool>,
    transferred: &Arc<AtomicU64>,
) -> Result<(), SftpError> {
    let mut local = std::fs::File::open(local_path)
        .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
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
        let read = std::io::Read::read(&mut local, &mut buffer)
            .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
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
        let read = std::fs::read_dir(&local_dir)
            .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
        for entry in read {
            if cancel.load(Ordering::Relaxed) {
                return Err(SftpError::cancelled());
            }
            let entry =
                entry.map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
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
            let child_local = entry.path();
            let child_remote = join_remote(&remote_dir, &name);
            let file_type = entry
                .file_type()
                .map_err(|error| SftpError::new("localReadFailed", error.to_string()))?;
            if file_type.is_dir() {
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
async fn do_download(
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

/// 流式下载单个文件到本地暂存路径：远程异步读取 → 本地同步写入，逐块累计进度与取消。
async fn download_file_raw(
    sftp: &SftpSession,
    remote: &str,
    staged_local: &Path,
    cancel: &Arc<AtomicBool>,
    transferred: &Arc<AtomicU64>,
) -> Result<(), SftpError> {
    // 确保暂存文件的父目录存在，避免因中间目录缺失而写入失败。
    if let Some(parent) = staged_local.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
    }
    let mut remote_file = sftp
        .open(remote.to_string())
        .await
        .map_err(map_sftp_error)?;
    let mut local = std::fs::File::create(staged_local)
        .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
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
        std::io::Write::write_all(&mut local, &buffer[..read])
            .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
        transferred.fetch_add(read as u64, Ordering::Relaxed);
    }
    std::io::Write::flush(&mut local)
        .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
    Ok(())
}

/// 校验远端目录条目名能否安全地拼进本地路径。
///
/// 远端 `read_dir` 返回的名字是**不可信输入**：`PathBuf::join` 见到 `..` 会向上跳级，
/// 在 Windows 上还会把 `\` 和盘符前缀当成路径语法，异常或恶意的服务端因此能让递归
/// 下载写到目标目录之外。OpenSSH 的 `sftp` 与 WinSCP 同样在客户端侧拦这类条目名。
///
/// 判定刻意交给 `std::path` 的组件解析而不是手写分隔符黑名单：`\` 在 Windows 上是
/// 分隔符、在 Unix 上只是普通文件名字符，按各自平台的真实语义判断才不会在 macOS 上
/// 误拒合法文件名。
fn ensure_safe_child_name(name: &str) -> Result<(), SftpError> {
    let reject = |reason: &str| {
        Err(SftpError::new(
            "invalidPath",
            format!("远端条目名 {name} {reason}，已拒绝下载以免写出目标目录"),
        ))
    };
    // NUL 无法进入任何平台的文件名，提前拦下比让 OS 报一句晦涩的 io 错误清楚。
    if name.is_empty() || name.contains('\0') {
        return reject("为空或含有 NUL 字节");
    }
    let mut components = Path::new(name).components();
    // 合法条目名解析后必须恰好是一个普通组件：`..` 会解析成 ParentDir、`.` 解析成
    // CurDir、带分隔符或盘符的名字会解析出两个以上组件或 Prefix。
    if !matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return reject("含有路径分隔符、盘符或上跳片段");
    }
    // Windows 的冒号是备用数据流语法：下载 `notes.txt:secret` 会静默写进
    // `notes.txt` 的隐藏流，资源管理器里看不到任何新文件。Unix 上冒号只是普通字符，
    // 因此这条检查按平台生效而不是一律拒绝。
    #[cfg(windows)]
    if name.contains(':') {
        return reject("含有冒号，Windows 会把它当作备用数据流");
    }
    Ok(())
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
        std::fs::create_dir_all(&local_dir)
            .map_err(|error| SftpError::new("localWriteFailed", error.to_string()))?;
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
            ensure_safe_child_name(&name)?;
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

#[cfg(test)]
mod tests {
    use super::{ensure_safe_child_name, join_remote};

    #[test]
    fn joins_remote_paths_without_duplicating_the_separator() {
        assert_eq!(join_remote("/home/demo", "a.txt"), "/home/demo/a.txt");
        assert_eq!(join_remote("/", "a.txt"), "/a.txt");
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
