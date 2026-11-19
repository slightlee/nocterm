//! 远程文件管理命令层：远程侧全部改由进程内 `SftpManager`（russh-sftp）完成，
//! 与 SSH 终端共用同一 russh 后端及认证/主机密钥策略；本地侧沿用标准库文件操作。
//!
//! 设计要点：
//! - 远程目录浏览、创建、重命名、删除直接映射到 `SftpManager` 的同步接口；
//! - 传输采用「命令线程装配 + 后台线程执行 + 轮询原子计数上报进度」模型：后台线程
//!   再派生工作线程运行阻塞式 `upload`/`download`，主线程按节流读取累计字节发进度事件；
//! - 上传落远程临时文件后由基础设施层原子提交；下载落本地暂存路径后由命令层
//!   `replace_local_path` 原子替换（保留其备份/回滚语义与既有测试）；
//! - 在线编辑 IPC 与用例位于 `file_editor`；本模块只向其提供凭据解析和传输生命周期守卫，
//!   文本规则与本地/远程提交分别由 Application 与 Infrastructure 负责；
//! - 错误统一归一化为携带前端稳定 `code` 的 `__NOCTERM_SFTP_ERROR__` 哨兵消息。

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

use nocterm_domain::connection::{AuthenticationMethod, ConnectionProfile};
use nocterm_infrastructure::ssh::sftp::SftpError;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    commands::credential::{read_secret, resolve_private_key},
    dto::error::ErrorResponse,
    state::AppState,
};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalFileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified_at: Option<u64>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<LocalFileEntry>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified_at: Option<u64>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteDirectoryListing {
    pub path: String,
    pub parent: Option<String>,
    pub entries: Vec<RemoteFileEntry>,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferStart {
    pub task_id: String,
}
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TransferProgress {
    pub task_id: String,
    pub direction: String,
    pub file_name: String,
    pub transferred: u64,
    pub total: u64,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone)]
struct TransferControl {
    connection_id: i64,
    cancel: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct SftpTransferState {
    controls: Arc<Mutex<HashMap<String, TransferControl>>>,
}

const LOCAL_DIRECTORY_READ_TIMEOUT: Duration = Duration::from_secs(8);

/// 本地目录读取无法安全取消底层文件系统调用；超时后保留路径占用，阻止重复点击
/// 为同一受保护目录持续堆积阻塞线程，避免底层读取完成前重复发起请求。
#[derive(Default)]
pub struct SftpLocalDirectoryState {
    active_paths: Arc<Mutex<HashSet<PathBuf>>>,
}

struct LocalDirectoryReadGuard {
    active_paths: Arc<Mutex<HashSet<PathBuf>>>,
    path: PathBuf,
}

impl SftpLocalDirectoryState {
    fn acquire(&self, path: &Path) -> Result<LocalDirectoryReadGuard, ErrorResponse> {
        let mut active_paths = self.active_paths.lock().map_err(|_| {
            error(
                "SFTP_LOCAL_READ_INTERRUPTED",
                "本地目录读取状态不可用",
                true,
            )
        })?;
        if !active_paths.insert(path.to_path_buf()) {
            return Err(error(
                "SFTP_LOCAL_READ_BUSY",
                "该目录仍在等待系统响应，请稍后重试",
                true,
            ));
        }
        Ok(LocalDirectoryReadGuard {
            active_paths: self.active_paths.clone(),
            path: path.to_path_buf(),
        })
    }
}

impl Drop for LocalDirectoryReadGuard {
    fn drop(&mut self) {
        if let Ok(mut active_paths) = self.active_paths.lock() {
            active_paths.remove(&self.path);
        }
    }
}

impl SftpTransferState {
    pub(super) fn has_running(&self, connection_id: i64) -> Result<bool, ErrorResponse> {
        Ok(self
            .controls
            .lock()
            .map_err(|_| error("SFTP_TRANSFER_FAILED", "传输状态不可用", true))?
            .values()
            .any(|control| control.connection_id == connection_id))
    }
    /// 编辑保存先登记再做 stat，连接关闭必须等待整个保存用例，而非仅上传阶段。
    pub(super) fn begin_edit(&self, connection_id: i64) -> Result<SftpEditTask, ErrorResponse> {
        let id = now_id("edit");
        let cancel = Arc::new(AtomicBool::new(false));
        self.controls
            .lock()
            .map_err(|_| error("SFTP_TRANSFER_FAILED", "传输状态不可用", true))?
            .insert(
                id.clone(),
                TransferControl {
                    connection_id,
                    cancel: cancel.clone(),
                },
            );
        Ok(SftpEditTask {
            id,
            cancel,
            controls: self.controls.clone(),
        })
    }

    /// 删除连接前必须先终止并等待其传输退出，避免凭据删除后留下远端临时文件。
    pub fn cancel_connection_and_wait(
        &self,
        connection_id: i64,
        timeout: Duration,
    ) -> Result<(), ErrorResponse> {
        self.cancel_and_wait(Some(connection_id), timeout)
    }

    fn cancel_all_and_wait(&self, timeout: Duration) -> Result<(), ErrorResponse> {
        self.cancel_and_wait(None, timeout)
    }

    fn cancel_and_wait(
        &self,
        connection_id: Option<i64>,
        timeout: Duration,
    ) -> Result<(), ErrorResponse> {
        {
            let controls = self
                .controls
                .lock()
                .map_err(|_| error("SFTP_TRANSFER_FAILED", "传输状态不可用", true))?;
            for control in controls.values() {
                if connection_id.is_none_or(|id| control.connection_id == id) {
                    control.cancel.store(true, Ordering::SeqCst);
                }
            }
        }

        let deadline = std::time::Instant::now() + timeout;
        loop {
            let active = self
                .controls
                .lock()
                .map_err(|_| error("SFTP_TRANSFER_FAILED", "传输状态不可用", true))?
                .values()
                .any(|control| connection_id.is_none_or(|id| control.connection_id == id));
            if !active {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                return Err(error(
                    "SFTP_TRANSFER_CANCEL_TIMEOUT",
                    "仍有文件传输未能及时停止，请稍后重试",
                    true,
                ));
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

/// RAII 保证所有错误路径都注销保存任务，关连接的等待条件不会因提前返回而悬挂。
pub(super) struct SftpEditTask {
    id: String,
    pub cancel: Arc<AtomicBool>,
    controls: Arc<Mutex<HashMap<String, TransferControl>>>,
}
impl Drop for SftpEditTask {
    fn drop(&mut self) {
        if let Ok(mut controls) = self.controls.lock() {
            controls.remove(&self.id);
        }
    }
}

impl Drop for SftpTransferState {
    fn drop(&mut self) {
        if let Ok(controls) = self.controls.lock() {
            for control in controls.values() {
                control.cancel.store(true, Ordering::SeqCst);
            }
        }
    }
}

fn error(code: &'static str, message: impl Into<String>, retryable: bool) -> ErrorResponse {
    nocterm_application::error::AppError::new(code, message, retryable).into()
}

fn now_id(prefix: &str) -> String {
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);
    let sequence = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{sequence}", std::process::id())
}

/// 读取连接资料与对应认证凭据：密码/私钥按需从密钥串取出，Agent 方式无需额外凭据。
/// 返回所有权数据以便移动进后台传输线程，避免跨线程借用命令层状态。
pub(super) fn read_credentials(
    state: &AppState,
    connection_id: i64,
) -> Result<(ConnectionProfile, Option<String>, Option<String>), ErrorResponse> {
    let profile = state
        .connection_service()
        .get(connection_id)
        .map_err(ErrorResponse::from)?;
    let (password, private_key) = match profile.authentication {
        AuthenticationMethod::Password => (Some(resolve_password(state, &profile)?), None),
        AuthenticationMethod::PrivateKey => (None, Some(resolve_private_key(state, &profile)?)),
        AuthenticationMethod::SshAgent => (None, None),
    };
    Ok((profile, password, private_key))
}

/// 文件页面没有可输入的提示符，因此口令来源为「会话内存缓存 > 系统凭据库」：
/// 用户在同一连接的 SSH 终端里现场输入过的口令可直接复用，不必为了打开文件页
/// 而强制把密码写进系统凭据库。两者都没有时给出可执行的提示而非泛化的凭据错误。
///
/// 缓存的口令只在该连接尚有活跃终端会话时存在（见 `state::session_password`），
/// 所以这里不持有租约：SFTP 会话一旦建立便自持连接，终端关闭后已有会话仍可继续用。
fn resolve_password(
    state: &AppState,
    profile: &ConnectionProfile,
) -> Result<String, ErrorResponse> {
    if let Some(secret) = state.session_passwords().get(profile.id) {
        return Ok(secret);
    }
    if profile.credential_status == "bound" {
        return read_secret(state, &profile.id.to_string(), "password");
    }
    Err(error(
        "SFTP_PASSWORD_REQUIRED",
        transfer_error("passwordRequired"),
        true,
    ))
}

/// 将基础设施层 `SftpError` 归一化为前端契约：消息体携带稳定错误码哨兵，交由前端本地化。
fn sftp_err_to_response(err: SftpError, fallback_code: &'static str) -> ErrorResponse {
    error(fallback_code, transfer_error(err.code), true)
}

fn resolve_local(path: Option<String>) -> Result<PathBuf, ErrorResponse> {
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

fn modified(metadata: &fs::Metadata) -> Option<u64> {
    metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|v| v.as_secs())
}

fn local_read_error(err: &std::io::Error) -> ErrorResponse {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        error(
            "SFTP_LOCAL_PERMISSION_DENIED",
            "当前应用没有访问该本地目录的权限",
            true,
        )
    } else {
        error("SFTP_LOCAL_READ_FAILED", "读取本地目录失败", true)
    }
}

fn read_local_directory(dir: PathBuf) -> Result<LocalDirectoryListing, ErrorResponse> {
    let dir_metadata = fs::metadata(&dir).map_err(|err| local_read_error(&err))?;
    if !dir_metadata.is_dir() {
        return Err(error("SFTP_LOCAL_PATH_INVALID", "目标路径不是目录", false));
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(&dir).map_err(|err| local_read_error(&err))? {
        // 单个目录项可能在枚举期间消失或被系统保护；这不应让其余可访问项全部失败。
        let Ok(item) = item else {
            continue;
        };
        let p = item.path();
        let metadata = item.metadata().ok();
        let is_dir = metadata.as_ref().is_some_and(fs::Metadata::is_dir)
            || (metadata.is_none() && item.file_type().is_ok_and(|kind| kind.is_dir()));
        entries.push(LocalFileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: p.to_string_lossy().into_owned(),
            is_dir,
            size: metadata
                .as_ref()
                .filter(|value| value.is_file())
                .map(fs::Metadata::len),
            modified_at: metadata.as_ref().and_then(modified),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(LocalDirectoryListing {
        path: dir.to_string_lossy().into_owned(),
        parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
        entries,
    })
}

async fn run_local_directory_read(
    guard: LocalDirectoryReadGuard,
    dir: PathBuf,
    timeout: Duration,
    read: impl FnOnce(PathBuf) -> Result<LocalDirectoryListing, ErrorResponse> + Send + 'static,
) -> Result<LocalDirectoryListing, ErrorResponse> {
    let (sender, mut receiver) = tauri::async_runtime::channel(1);
    let timeout_sender = sender.clone();
    let (cancel_timeout, wait_for_cancel) = std::sync::mpsc::sync_channel(1);

    tauri::async_runtime::spawn_blocking(move || {
        let result = read(dir);
        // 结果可见之前先释放路径；立即刷新时不应撞上已经完成的读取守卫。
        // 若底层 read 仍阻塞，guard 始终留在该线程，超时不会放开并发读取。
        drop(guard);
        let _ = cancel_timeout.send(());
        let _ = sender.blocking_send(result);
    });
    thread::spawn(move || {
        if wait_for_cancel.recv_timeout(timeout).is_err() {
            let _ = timeout_sender.blocking_send(Err(error(
                "SFTP_LOCAL_READ_TIMEOUT",
                "读取本地目录超时，请稍后重试",
                true,
            )));
        }
    });

    receiver.recv().await.unwrap_or_else(|| {
        Err(error(
            "SFTP_LOCAL_READ_INTERRUPTED",
            "本地目录读取被中断，请重试",
            true,
        ))
    })
}

/// 文件系统权限弹窗或云盘占位符可能让 `read_dir` 长时间阻塞。命令在专用阻塞线程
/// 中执行，并在超时后恢复前端交互；底层调用结束前，同一路径由状态守卫拒绝重复读取。
#[tauri::command]
pub async fn list_local_dir(
    state: State<'_, SftpLocalDirectoryState>,
    path: Option<String>,
) -> Result<LocalDirectoryListing, ErrorResponse> {
    let dir = resolve_local(path)?;
    let guard = state.acquire(&dir)?;
    run_local_directory_read(
        guard,
        dir,
        LOCAL_DIRECTORY_READ_TIMEOUT,
        read_local_directory,
    )
    .await
}

/// 拼接远程路径：根目录下避免出现重复分隔符，其余去除末尾斜杠后以 `/` 连接。
fn join_remote(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}

/// 计算远程路径的父目录；根目录或空路径返回 `None`，用于禁止对根目录做危险操作。
fn parent_remote(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    if path.is_empty() || path == "/" {
        return None;
    }
    let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
    Some(if parent.is_empty() {
        "/".to_string()
    } else {
        parent.to_string()
    })
}

fn valid_name(name: &str) -> Result<(), ErrorResponse> {
    if name.trim().is_empty()
        || matches!(name, "." | "..")
        || name.contains(['/', '\\', '\n', '\r', '\t', '\0'])
    {
        Err(error(
            "SFTP_INVALID_NAME",
            "名称不能包含路径分隔符或换行符",
            false,
        ))
    } else {
        Ok(())
    }
}

#[tauri::command(async)]
pub fn list_remote_dir(
    state: State<'_, AppState>,
    connection_id: i64,
    path: Option<String>,
) -> Result<RemoteDirectoryListing, ErrorResponse> {
    let requested = path
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| ".".to_string());
    if requested.contains(['\n', '\r']) {
        return Err(error("SFTP_INVALID_PATH", "远程路径不能包含换行符", false));
    }
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    let listing = state
        .sftp_manager()
        .list_dir(
            &profile,
            password.as_deref(),
            private_key.as_deref(),
            &requested,
        )
        .map_err(|err| sftp_err_to_response(err, "SFTP_REMOTE_READ_FAILED"))?;
    let mut entries: Vec<RemoteFileEntry> = listing
        .entries
        .into_iter()
        .map(|entry| RemoteFileEntry {
            name: entry.name,
            is_dir: entry.is_dir,
            size: entry.size,
            modified_at: entry.modified_at,
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(RemoteDirectoryListing {
        parent: parent_remote(&listing.path),
        path: listing.path,
        entries,
    })
}

#[tauri::command(async)]
pub fn local_path_exists(path: String) -> bool {
    PathBuf::from(path).exists()
}

#[tauri::command(async)]
pub fn remote_path_exists(
    state: State<'_, AppState>,
    connection_id: i64,
    remote_path: String,
) -> Result<bool, ErrorResponse> {
    if remote_path.contains(['\n', '\r']) {
        return Err(error("SFTP_INVALID_PATH", "远程路径不能包含换行符", false));
    }
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    state
        .sftp_manager()
        .exists(
            &profile,
            password.as_deref(),
            private_key.as_deref(),
            &remote_path,
        )
        .map_err(|err| sftp_err_to_response(err, "SFTP_REMOTE_READ_FAILED"))
}

#[tauri::command(async)]
pub fn create_local_dir(parent: String, name: String) -> Result<(), ErrorResponse> {
    valid_name(&name)?;
    fs::create_dir(PathBuf::from(parent).join(name))
        .map_err(|_| error("SFTP_LOCAL_WRITE_FAILED", "创建本地目录失败", true))
}

#[tauri::command(async)]
pub fn create_remote_dir(
    state: State<'_, AppState>,
    connection_id: i64,
    parent: String,
    name: String,
) -> Result<(), ErrorResponse> {
    valid_name(&name)?;
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    state
        .sftp_manager()
        .create_dir(
            &profile,
            password.as_deref(),
            private_key.as_deref(),
            &join_remote(&parent, &name),
        )
        .map_err(|err| sftp_err_to_response(err, "SFTP_REMOTE_WRITE_FAILED"))
}

#[tauri::command(async)]
pub fn rename_local_path(path: String, new_name: String) -> Result<(), ErrorResponse> {
    valid_name(&new_name)?;
    let src = PathBuf::from(path);
    let target = src
        .parent()
        .ok_or_else(|| error("SFTP_LOCAL_WRITE_FAILED", "无法定位本地父目录", false))?
        .join(new_name);
    fs::rename(src, target)
        .map_err(|_| error("SFTP_LOCAL_WRITE_FAILED", "重命名本地路径失败", true))
}

#[tauri::command(async)]
pub fn rename_remote_path(
    state: State<'_, AppState>,
    connection_id: i64,
    remote_path: String,
    new_name: String,
) -> Result<(), ErrorResponse> {
    valid_name(&new_name)?;
    let parent = parent_remote(&remote_path)
        .ok_or_else(|| error("SFTP_INVALID_PATH", "不能重命名远程根目录", false))?;
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    state
        .sftp_manager()
        .rename(
            &profile,
            password.as_deref(),
            private_key.as_deref(),
            &remote_path,
            &join_remote(&parent, &new_name),
        )
        .map_err(|err| sftp_err_to_response(err, "SFTP_REMOTE_WRITE_FAILED"))
}

#[tauri::command(async)]
pub fn delete_local_path(path: String) -> Result<(), ErrorResponse> {
    let p = PathBuf::from(path);
    if !p.exists() {
        return Err(error("SFTP_LOCAL_PATH_INVALID", "本地路径不存在", false));
    }
    if p.is_dir() {
        fs::remove_dir_all(p)
    } else {
        fs::remove_file(p)
    }
    .map_err(|_| error("SFTP_LOCAL_WRITE_FAILED", "删除本地路径失败", true))
}

#[tauri::command(async)]
pub fn delete_remote_path(
    state: State<'_, AppState>,
    connection_id: i64,
    remote_path: String,
) -> Result<(), ErrorResponse> {
    if parent_remote(&remote_path).is_none() {
        return Err(error("SFTP_INVALID_PATH", "不能删除远程根目录", false));
    }
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    state
        .sftp_manager()
        .remove(
            &profile,
            password.as_deref(),
            private_key.as_deref(),
            &remote_path,
        )
        .map_err(|err| sftp_err_to_response(err, "SFTP_REMOTE_WRITE_FAILED"))
}

#[tauri::command(async)]
pub fn close_sftp_session(
    state: State<'_, AppState>,
    transfers: State<'_, SftpTransferState>,
    connection_id: i64,
) -> Result<(), ErrorResponse> {
    // 标签关闭代表用户明确断开：先让传输安全退出，再关闭复用的 SFTP 会话。
    transfers.cancel_connection_and_wait(connection_id, Duration::from_secs(5))?;
    state.sftp_manager().close_connection(connection_id);
    Ok(())
}

/// 应用退出时统一停止全部传输并关闭所有 SFTP 会话，释放底层 russh 连接。
pub(crate) fn shutdown_sftp(app: &AppHandle) {
    if let Some(transfers) = app.try_state::<SftpTransferState>() {
        let _ = transfers.cancel_all_and_wait(Duration::from_secs(5));
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.sftp_manager().shutdown();
    }
}

fn total_size(path: &Path) -> u64 {
    if let Ok(m) = fs::metadata(path) {
        if m.is_file() {
            return m.len();
        }
        if m.is_dir() {
            return fs::read_dir(path)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|e| total_size(&e.path()))
                .sum();
        }
    }
    0
}

fn temp_name(name: &str, task: &str, suffix: &str) -> String {
    format!(".{name}.nocterm-{task}.{suffix}")
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn remove_path(path: &Path) {
    let is_real_directory = fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_dir() && !metadata.file_type().is_symlink())
        .unwrap_or(false);
    if is_real_directory {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

fn emit(app: &AppHandle, p: TransferProgress) {
    let _ = app.emit("nocterm://sftp-transfer-progress", p);
}

fn transfer_error(code: &str) -> String {
    format!("__NOCTERM_SFTP_ERROR__\t{code}\t")
}

struct TransferEvent<'a> {
    task_id: &'a str,
    direction: &'a str,
    file_name: &'a str,
    transferred: u64,
    total: u64,
    status: &'a str,
    error_code: Option<&'a str>,
}

fn emit_transfer_state(app: &AppHandle, event: TransferEvent<'_>) {
    emit(
        app,
        TransferProgress {
            task_id: event.task_id.to_string(),
            direction: event.direction.to_string(),
            file_name: event.file_name.to_string(),
            transferred: event.transferred,
            total: event.total,
            status: event.status.to_string(),
            error: event.error_code.map(transfer_error),
        },
    );
}

/// 先把旧目标移到任务专属备份，再提交新目标；提交失败时恢复旧目标。
fn replace_local_path(staged: &Path, target: &Path, backup: &Path) -> std::io::Result<()> {
    remove_path(backup);
    let had_target = path_exists(target);
    if had_target {
        fs::rename(target, backup)?;
    }

    if let Err(commit_error) = fs::rename(staged, target) {
        if had_target {
            fs::rename(backup, target).map_err(|rollback_error| {
                std::io::Error::new(
                    rollback_error.kind(),
                    format!(
                        "commit failed: {commit_error}; rollback failed: {rollback_error}; backup remains at {}",
                        backup.display()
                    ),
                )
            })?;
        }
        return Err(commit_error);
    }

    if had_target {
        remove_path(backup);
    }
    Ok(())
}

fn register(
    state: &State<'_, SftpTransferState>,
    id: &str,
    connection_id: i64,
) -> Result<Arc<AtomicBool>, ErrorResponse> {
    let cancel = Arc::new(AtomicBool::new(false));
    let mut controls = state
        .controls
        .lock()
        .map_err(|_| error("SFTP_TRANSFER_FAILED", "传输状态不可用", true))?;
    if controls.contains_key(id) {
        return Err(error(
            "SFTP_TRANSFER_ID_CONFLICT",
            "文件传输任务标识冲突",
            true,
        ));
    }
    controls.insert(
        id.to_string(),
        TransferControl {
            connection_id,
            cancel: cancel.clone(),
        },
    );
    Ok(cancel)
}

fn unregister(map: &Arc<Mutex<HashMap<String, TransferControl>>>, id: &str) {
    if let Ok(mut m) = map.lock() {
        m.remove(id);
    }
}

/// 后台执行阻塞传输并轮询进度：派生工作线程运行 `work`，主线程按 ~40ms 节流读取累计
/// 字节发送 running 事件，直至工作线程结束；返回工作线程结果（成功或稳定错误码）。
fn run_transfer<F>(
    app: &AppHandle,
    task: &str,
    direction: &'static str,
    file_name: &str,
    total: u64,
    transferred: Arc<AtomicU64>,
    work: F,
) -> Result<(), &'static str>
where
    F: FnOnce() -> Result<(), SftpError> + Send + 'static,
{
    let worker = thread::spawn(work);
    while !worker.is_finished() {
        thread::sleep(Duration::from_millis(40));
        emit_transfer_state(
            app,
            TransferEvent {
                task_id: task,
                direction,
                file_name,
                transferred: transferred.load(Ordering::Relaxed),
                total,
                status: "running",
                error_code: None,
            },
        );
    }
    match worker.join() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(err.code),
        Err(_) => Err("unknown"),
    }
}

#[tauri::command]
pub fn cancel_file_transfer(
    state: State<'_, SftpTransferState>,
    task_id: String,
) -> Result<(), ErrorResponse> {
    let controls = state
        .controls
        .lock()
        .map_err(|_| error("SFTP_TRANSFER_FAILED", "传输状态不可用", true))?;
    if let Some(control) = controls.get(&task_id) {
        control.cancel.store(true, Ordering::SeqCst);
    }
    Ok(())
}

// 编辑 IPC 的兼容测试仍从本模块调用，业务实现已迁至专用用例和 Adapter。
#[cfg(test)]
use super::file_editor::{LocalFileBaseline, sftp_read_local_file, sftp_write_local_file};
#[cfg(test)]
const SFTP_EDIT_MAX_BYTES: u64 = nocterm_application::file_edit::EDIT_MAX_BYTES;

#[tauri::command(async)]
pub fn upload_local_to_remote(
    app: AppHandle,
    state: State<'_, SftpTransferState>,
    connection_id: i64,
    local_path: String,
    remote_dir: String,
) -> Result<TransferStart, ErrorResponse> {
    let source = PathBuf::from(local_path);
    if !source.exists() {
        return Err(error("SFTP_LOCAL_PATH_INVALID", "本地文件不存在", false));
    }
    let name = source
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| error("SFTP_LOCAL_PATH_INVALID", "无法识别本地文件名", false))?
        .to_string();
    // 递归 SFTP 传输可精确统计字节，目录与文件都以总字节作为进度分母。
    let total = total_size(&source);
    let app_state = app.state::<AppState>();
    let (profile, password, private_key) = read_credentials(&app_state, connection_id)?;
    let manager = app_state.sftp_manager().clone();
    let task = now_id("upload");
    let task_id = task.clone();
    let flag = register(&state, &task, connection_id)?;
    let controls = state.controls.clone();
    let transferred = Arc::new(AtomicU64::new(0));
    let app2 = app.clone();
    emit_transfer_state(
        &app,
        TransferEvent {
            task_id: &task,
            direction: "upload",
            file_name: &name,
            transferred: 0,
            total,
            status: "running",
            error_code: None,
        },
    );
    thread::spawn(move || {
        let poll_counter = transferred.clone();
        let work_counter = transferred.clone();
        let work_cancel = flag.clone();
        // 阻塞式上传在独立工作线程执行，主线程负责节流上报进度。
        let work = move || {
            manager.upload(
                &profile,
                password.as_deref(),
                private_key.as_deref(),
                &source,
                &remote_dir,
                work_cancel,
                work_counter,
            )
        };
        let code = run_transfer(&app2, &task, "upload", &name, total, poll_counter, work);
        let final_transferred = transferred.load(Ordering::Relaxed);
        let cancelled = flag.load(Ordering::SeqCst);
        let (status, error_code) = match code {
            Ok(()) => ("completed", None),
            Err(c) if cancelled || c == "cancelled" => ("cancelled", None),
            Err(c) => ("error", Some(c)),
        };
        emit_transfer_state(
            &app2,
            TransferEvent {
                task_id: &task,
                direction: "upload",
                file_name: &name,
                transferred: final_transferred,
                total,
                status,
                error_code,
            },
        );
        unregister(&controls, &task);
    });
    Ok(TransferStart { task_id })
}

#[tauri::command(async)]
pub fn download_remote_to_local(
    app: AppHandle,
    state: State<'_, SftpTransferState>,
    connection_id: i64,
    remote_path: String,
    local_dir: String,
    total: Option<u64>,
    is_dir: Option<bool>,
) -> Result<TransferStart, ErrorResponse> {
    let target_dir = PathBuf::from(local_dir);
    if !target_dir.is_dir() {
        return Err(error("SFTP_LOCAL_PATH_INVALID", "本地目标不是目录", false));
    }
    // is_dir 仅用于选择暂存名后缀；实际类型由基础设施层按远程 stat 权威判定。
    let is_dir = is_dir.unwrap_or(false);
    let name = remote_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| error("SFTP_INVALID_PATH", "无法识别远程文件名", false))?
        .to_string();
    valid_name(&name)?;
    let app_state = app.state::<AppState>();
    let (profile, password, private_key) = read_credentials(&app_state, connection_id)?;
    let manager = app_state.sftp_manager().clone();
    let task = now_id("download");
    let task_id = task.clone();
    let staged = target_dir.join(temp_name(
        &name,
        &task,
        if is_dir { "partial" } else { "part" },
    ));
    let target = target_dir.join(&name);
    let backup = target_dir.join(temp_name(&name, &task, "backup"));
    let flag = register(&state, &task, connection_id)?;
    let controls = state.controls.clone();
    let total = total.unwrap_or(0);
    let transferred = Arc::new(AtomicU64::new(0));
    let app2 = app.clone();
    emit_transfer_state(
        &app,
        TransferEvent {
            task_id: &task,
            direction: "download",
            file_name: &name,
            transferred: 0,
            total,
            status: "running",
            error_code: None,
        },
    );
    thread::spawn(move || {
        let poll_counter = transferred.clone();
        let work_counter = transferred.clone();
        let work_cancel = flag.clone();
        let staged_work = staged.clone();
        // 下载先落到暂存路径（文件或镜像目录），成功后由命令层原子替换到目标。
        let work = move || {
            manager.download(
                &profile,
                password.as_deref(),
                private_key.as_deref(),
                &remote_path,
                &staged_work,
                work_cancel,
                work_counter,
            )
        };
        let code = run_transfer(&app2, &task, "download", &name, total, poll_counter, work);
        let final_transferred = transferred.load(Ordering::Relaxed);
        let cancelled = flag.load(Ordering::SeqCst) || matches!(code, Err("cancelled"));
        let (status, error_code) = if cancelled {
            remove_path(&staged);
            ("cancelled", None)
        } else if let Err(c) = code {
            remove_path(&staged);
            ("error", Some(c))
        } else if path_exists(&staged) && replace_local_path(&staged, &target, &backup).is_ok() {
            ("completed", None)
        } else {
            // 提交失败：清理残留暂存，向前端报本地写入失败。
            remove_path(&staged);
            ("error", Some("localWriteFailed"))
        };
        emit_transfer_state(
            &app2,
            TransferEvent {
                task_id: &task,
                direction: "download",
                file_name: &name,
                transferred: final_transferred,
                total,
                status,
                error_code,
            },
        );
        unregister(&controls, &task);
    });
    Ok(TransferStart { task_id })
}

#[cfg(test)]
mod tests {
    use std::{
        fs, thread,
        time::{Duration, Instant},
    };

    use super::{
        LocalDirectoryListing, SftpLocalDirectoryState, join_remote, now_id, parent_remote,
        read_local_directory, replace_local_path, run_local_directory_read, temp_name, valid_name,
    };

    /// 构造仅本组测试使用的临时目录并在结束时清理；任务标识保证唯一。
    fn edit_scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(now_id(&format!("nocterm-edit-test-{tag}")));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("创建测试目录");
        dir
    }

    /// 序列化本地编辑测试的文件元数据，供冲突断言复用。
    fn edit_baseline(path: &std::path::Path) -> super::LocalFileBaseline {
        super::sftp_read_local_file(path.to_string_lossy().into_owned())
            .expect("读取测试基线")
            .baseline
    }

    #[test]
    fn local_directory_read_returns_entries_and_releases_path_guard() {
        let root = std::env::temp_dir().join(now_id("nocterm-sftp-list-test"));
        fs::create_dir_all(root.join("folder")).expect("create test directory");
        fs::write(root.join("file.txt"), "content").expect("write test file");

        let state = SftpLocalDirectoryState::default();
        let first_guard = state.acquire(&root).expect("acquire first path guard");
        assert!(state.acquire(&root).is_err());
        drop(first_guard);
        assert!(state.acquire(&root).is_ok());

        let listing = read_local_directory(root.clone()).expect("read local directory");
        assert_eq!(listing.path, root.to_string_lossy());
        assert_eq!(listing.entries.len(), 2);
        assert!(listing.entries[0].is_dir);
        assert_eq!(listing.entries[1].size, Some(7));

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn local_directory_read_times_out_without_releasing_running_path() {
        let root = std::env::temp_dir().join(now_id("nocterm-sftp-timeout-test"));
        let state = SftpLocalDirectoryState::default();
        let guard = state.acquire(&root).expect("acquire path guard");
        // 读取器用通道门控代替固定睡眠：测试放行前闭包必然仍在运行，超时必然发生；
        // 守卫何时释放通过有界轮询观察，不再假设线程池的调度速度。固定毫秒级
        // 睡眠在繁忙的共享 Runner 上会因调度延迟耗尽裕度，是本用例此前偶发失败的原因。
        let (release_reader, reader_gate) = std::sync::mpsc::channel::<()>();
        let result = tauri::async_runtime::block_on(run_local_directory_read(
            guard,
            root.clone(),
            Duration::from_millis(5),
            move |path| {
                // 阻塞等待测试确认超时路径后再返回；期间守卫始终被持有。
                let _ = reader_gate.recv();
                Ok(LocalDirectoryListing {
                    path: path.to_string_lossy().into_owned(),
                    parent: None,
                    entries: Vec::new(),
                })
            },
        ));

        let error = serde_json::to_value(result.err().expect("directory read should time out"))
            .expect("serialize timeout error");
        assert_eq!(error["code"], "SFTP_LOCAL_READ_TIMEOUT");
        assert!(state.acquire(&root).is_err());
        release_reader.send(()).expect("release the gated reader");
        // 读取器完成后守卫必须释放：轮询观察而非定时断言，等待上限只是防挂死保险。
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match state.acquire(&root) {
                Ok(reacquired) => {
                    drop(reacquired);
                    break;
                }
                Err(_) => {
                    assert!(
                        Instant::now() < deadline,
                        "读取器完成后守卫应在合理时间内释放"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn resolves_remote_parent_and_join_at_root() {
        assert_eq!(join_remote("/", "docs"), "/docs");
        assert_eq!(parent_remote("/var/data/"), Some("/var".to_string()));
        assert_eq!(parent_remote("/"), None);
    }

    #[test]
    fn local_read_returns_identity_path_and_baseline() {
        let root = edit_scratch_dir("read");
        let target = root.join("deploy.sh");
        fs::write(&target, "#!/bin/sh\n").expect("写入脚本");

        let result = super::sftp_read_local_file(target.to_string_lossy().into_owned())
            .expect("读取本地文本");
        assert_eq!(result.content, "#!/bin/sh\n");
        // 身份路径必须规范化为绝对路径，前端以此为标签身份。
        assert!(std::path::Path::new(&result.identity_path).is_absolute());
        assert_eq!(result.baseline.size, 10);

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn local_read_rejects_a_file_above_the_limit() {
        let root = edit_scratch_dir("oversized");
        let target = root.join("huge.log");
        fs::write(
            &target,
            vec![b'x'; (super::SFTP_EDIT_MAX_BYTES + 1) as usize],
        )
        .expect("写入超限文件");

        let failure = serde_json::to_value(
            super::sftp_read_local_file(target.to_string_lossy().into_owned()).unwrap_err(),
        )
        .expect("序列化超限错误");
        assert_eq!(failure["code"], "SFTP_FILE_TOO_LARGE");

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn local_save_rejects_null_bytes_and_preserves_the_original() {
        let root = edit_scratch_dir("nul-save");
        let target = root.join("notes.txt");
        fs::write(&target, "original").expect("写入原文件");
        let baseline = edit_baseline(&target);

        let failure = serde_json::to_value(
            super::sftp_write_local_file(
                target.to_string_lossy().into_owned(),
                "new\0content".to_string(),
                Some(baseline.size),
                Some(baseline.modified_seconds),
                Some(baseline.modified_nanos),
                false,
            )
            .unwrap_err(),
        )
        .expect("序列化二进制错误");
        assert_eq!(failure["code"], "SFTP_FILE_NOT_TEXT");
        assert_eq!(fs::read_to_string(&target).expect("读取原文件"), "original");

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn local_save_conflict_and_overwrite_roundtrip() {
        let root = edit_scratch_dir("save");
        let target = root.join("config.yml");
        fs::write(&target, "port: 8080\n").expect("写入初版");

        let read =
            super::sftp_read_local_file(target.to_string_lossy().into_owned()).expect("读取初版");
        let baseline = &read.baseline;

        // 基线一致时保存成功并返回新基线。
        let saved = super::sftp_write_local_file(
            target.to_string_lossy().into_owned(),
            "port: 9090\n".to_string(),
            Some(baseline.size),
            Some(baseline.modified_seconds),
            Some(baseline.modified_nanos),
            false,
        )
        .expect("基线一致的保存");
        assert_eq!(
            fs::read_to_string(&target).expect("读取保存结果"),
            "port: 9090\n"
        );
        // "port: 9090\n" 为 11 字节；断言保存结果与内容长度一致。
        assert_eq!(saved.size, 11);
        assert!(saved.modified_seconds >= baseline.modified_seconds);

        // 外部把文件改回相同大小（内容不同）后，旧基线保存必须报冲突。
        fs::write(&target, "port: 7777\n").expect("模拟外部修改");
        let conflict = serde_json::to_value(
            super::sftp_write_local_file(
                target.to_string_lossy().into_owned(),
                "port: 9090\n".to_string(),
                Some(saved.size),
                Some(saved.modified_seconds),
                Some(saved.modified_nanos),
                false,
            )
            .unwrap_err(),
        )
        .expect("序列化冲突错误");
        assert_eq!(conflict["code"], "SFTP_LOCAL_FILE_CHANGED");
        // 冲突不得破坏原文件内容。
        assert_eq!(
            fs::read_to_string(&target).expect("冲突后原文件"),
            "port: 7777\n"
        );

        // force 只跳过冲突，正常完成覆盖。
        super::sftp_write_local_file(
            target.to_string_lossy().into_owned(),
            "port: 9090\n".to_string(),
            Some(saved.size),
            Some(saved.modified_seconds),
            Some(saved.modified_nanos),
            true,
        )
        .expect("强制覆盖保存");
        assert_eq!(
            fs::read_to_string(&target).expect("强制覆盖结果"),
            "port: 9090\n"
        );

        // 成功路径不残留任何隐藏暂存/备份文件。
        let leftovers: Vec<_> = fs::read_dir(&root)
            .expect("枚举目录")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "不应残留暂存文件: {leftovers:?}");

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn local_save_rejects_unwritable_target_and_preserves_original() {
        let root = edit_scratch_dir("readonly");
        let target = root.join("locked.txt");
        fs::write(&target, "original").expect("写入原文件");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o444)).expect("设置只读权限");
            let denied = serde_json::to_value(
                super::sftp_write_local_file(
                    target.to_string_lossy().into_owned(),
                    "changed".to_string(),
                    None,
                    None,
                    None,
                    false,
                )
                .unwrap_err(),
            )
            .expect("序列化权限错误");
            assert_eq!(denied["code"], "SFTP_LOCAL_PERMISSION_DENIED");
            assert_eq!(
                fs::read_to_string(&target).expect("只读目标未被改写"),
                "original"
            );
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
                .expect("恢复权限以便清理");
        }
        #[cfg(not(unix))]
        {
            let _ = &target;
        }

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn local_save_preserves_unix_permission_bits() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let root = edit_scratch_dir("perm");
            let target = root.join("script.sh");
            fs::write(&target, "echo hi\n").expect("写入脚本");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .expect("设置可执行权限");

            let baseline = edit_baseline(&target);
            super::sftp_write_local_file(
                target.to_string_lossy().into_owned(),
                "echo bye\n".to_string(),
                Some(baseline.size),
                Some(baseline.modified_seconds),
                Some(baseline.modified_nanos),
                false,
            )
            .expect("保存脚本");

            let mode = fs::metadata(&target)
                .expect("读取保存后元数据")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o755, "保存必须保留权限位");

            fs::remove_dir_all(root).ok();
        }
        #[cfg(not(unix))]
        {
            // Windows 权限模型不同：只读语义由只读拒绝测试覆盖。
        }
    }

    #[test]
    fn local_read_rejects_missing_file_with_stable_code() {
        let root = edit_scratch_dir("missing");
        let missing = serde_json::to_value(
            super::sftp_read_local_file(root.join("nope.txt").to_string_lossy().into_owned())
                .unwrap_err(),
        )
        .expect("序列化缺失错误");
        assert_eq!(missing["code"], "SFTP_LOCAL_PATH_INVALID");
        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn rejects_names_that_can_escape_the_selected_directory() {
        assert!(valid_name("folder").is_ok());
        assert!(valid_name("../folder").is_err());
        assert!(valid_name("..").is_err());
        assert!(valid_name("tab\tname").is_err());
        assert!(valid_name("line\nbreak").is_err());
    }

    #[test]
    fn transfer_temp_names_are_hidden_and_task_scoped() {
        assert_eq!(
            temp_name("report.txt", "upload-1-2", "part"),
            ".report.txt.nocterm-upload-1-2.part"
        );
    }

    #[test]
    fn task_ids_remain_unique_within_the_same_process() {
        assert_ne!(now_id("upload"), now_id("upload"));
    }

    #[test]
    fn failed_local_commit_restores_the_original_target() {
        let root = std::env::temp_dir().join(now_id("nocterm-sftp-commit-test"));
        fs::create_dir(&root).expect("create test directory");
        let target = root.join("target.txt");
        let missing_staged = root.join("missing.part");
        let backup = root.join("target.backup");
        fs::write(&target, "original").expect("write original target");

        assert!(replace_local_path(&missing_staged, &target, &backup).is_err());
        assert_eq!(
            fs::read_to_string(&target).expect("read restored target"),
            "original"
        );
        assert!(!backup.exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn successful_local_commit_replaces_target_and_removes_backup() {
        let root = std::env::temp_dir().join(now_id("nocterm-sftp-commit-test"));
        fs::create_dir(&root).expect("create test directory");
        let target = root.join("target.txt");
        let staged = root.join("target.part");
        let backup = root.join("target.backup");
        fs::write(&target, "original").expect("write original target");
        fs::write(&staged, "replacement").expect("write staged target");

        replace_local_path(&staged, &target, &backup).expect("commit staged target");

        assert_eq!(
            fs::read_to_string(&target).expect("read committed target"),
            "replacement"
        );
        assert!(!staged.exists());
        assert!(!backup.exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }
}
