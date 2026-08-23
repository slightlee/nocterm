//! 远程文件管理命令层：远程侧全部改由进程内 `SftpManager`（russh-sftp）完成，
//! 与 SSH 终端共用同一 russh 后端及认证/主机密钥策略；本地侧沿用标准库文件操作。
//!
//! 设计要点：
//! - 远程目录浏览、创建、重命名、删除直接映射到 `SftpManager` 的同步接口；
//! - 传输采用「命令线程装配 + 后台线程执行 + 轮询原子计数上报进度」模型：后台线程
//!   再派生工作线程运行阻塞式 `upload`/`download`，主线程按节流读取累计字节发进度事件；
//! - 上传落远程临时文件后由基础设施层原子提交；下载落本地暂存路径后由命令层
//!   `replace_local_path` 原子替换（保留其备份/回滚语义与既有测试）；
//! - 错误统一归一化为携带前端稳定 `code` 的 `__NOCTERM_SFTP_ERROR__` 哨兵消息。

use std::{
    collections::HashMap,
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

impl SftpTransferState {
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
fn read_credentials(
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

fn modified(path: &Path) -> Option<u64> {
    path.metadata()
        .ok()?
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|v| v.as_secs())
}

#[tauri::command(async)]
pub fn list_local_dir(path: Option<String>) -> Result<LocalDirectoryListing, ErrorResponse> {
    let dir = resolve_local(path)?;
    if !dir.is_dir() {
        return Err(error("SFTP_LOCAL_PATH_INVALID", "目标路径不是目录", false));
    }
    let mut entries = Vec::new();
    for item in
        fs::read_dir(&dir).map_err(|_| error("SFTP_LOCAL_READ_FAILED", "读取本地目录失败", true))?
    {
        let item = item.map_err(|_| error("SFTP_LOCAL_READ_FAILED", "读取本地目录项失败", true))?;
        let p = item.path();
        let metadata = item
            .metadata()
            .map_err(|_| error("SFTP_LOCAL_READ_FAILED", "读取本地文件信息失败", true))?;
        entries.push(LocalFileEntry {
            name: item.file_name().to_string_lossy().into_owned(),
            path: p.to_string_lossy().into_owned(),
            is_dir: metadata.is_dir(),
            size: metadata.is_file().then_some(metadata.len()),
            modified_at: modified(&p),
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
    use std::fs;

    use super::{join_remote, now_id, parent_remote, replace_local_path, temp_name, valid_name};

    #[test]
    fn resolves_remote_parent_and_join_at_root() {
        assert_eq!(join_remote("/", "docs"), "/docs");
        assert_eq!(parent_remote("/var/data/"), Some("/var".to_string()));
        assert_eq!(parent_remote("/"), None);
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
