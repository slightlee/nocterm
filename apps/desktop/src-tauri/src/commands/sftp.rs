use std::{
    collections::HashMap,
    collections::hash_map::DefaultHasher,
    fs::{self, OpenOptions},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, UNIX_EPOCH},
};

use nocterm_domain::connection::{AuthenticationMethod, ConnectionProfile};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{commands::credential::read_secret, dto::error::ErrorResponse, state::AppState};

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

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}
fn join_remote(dir: &str, name: &str) -> String {
    if dir == "/" {
        format!("/{name}")
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}
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

/// 控制套接字绑定连接配置指纹，资料更新后不能复用旧主机或旧凭据的 SSH Master。
fn ssh_control_path(profile: &ConnectionProfile) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    profile.id.hash(&mut hasher);
    profile.host.hash(&mut hasher);
    profile.port.hash(&mut hasher);
    profile.username.hash(&mut hasher);
    profile.authentication.as_str().hash(&mut hasher);
    profile.updated_at.hash(&mut hasher);
    std::env::temp_dir().join(format!("nocterm-sftp-{:x}", hasher.finish()))
}

/// 关闭连接专属的 OpenSSH Master。不存在或已经退出视为成功，保证重复关闭安全。
pub(crate) fn close_sftp_master(state: &AppState, connection_id: i64) -> Result<(), ErrorResponse> {
    let profile = state
        .connection_service()
        .get(connection_id)
        .map_err(ErrorResponse::from)?;
    let control_path = ssh_control_path(&profile);
    if !control_path.exists() {
        return Ok(());
    }
    let output = Command::new("ssh")
        .args([
            "-p",
            &profile.port.to_string(),
            "-S",
            control_path.to_string_lossy().as_ref(),
            "-O",
            "exit",
            &format!("{}@{}", profile.username, profile.host),
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|_| error("SFTP_DISCONNECT_FAILED", "关闭 SFTP 连接失败", true))?;
    if output.status.success() || !control_path.exists() || missing_control_master(&output.stderr) {
        return Ok(());
    }
    Err(error(
        "SFTP_DISCONNECT_FAILED",
        "关闭 SFTP 连接失败，请重试",
        true,
    ))
}

/// 最终退出时统一停止传输并关闭仍存活的 Master，避免 ControlPersist 留下后台进程。
pub(crate) fn shutdown_sftp(app: &AppHandle) {
    if let Some(transfers) = app.try_state::<SftpTransferState>() {
        let _ = transfers.cancel_all_and_wait(Duration::from_secs(5));
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(profiles) = state.connection_service().list() else {
        return;
    };
    for profile in profiles {
        let _ = close_sftp_master(&state, profile.id);
    }
}

fn missing_control_master(stderr: &[u8]) -> bool {
    let detail = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    detail.contains("control socket connect")
        && (detail.contains("no such file") || detail.contains("connection refused"))
}

#[tauri::command(async)]
pub fn close_sftp_session(
    state: State<'_, AppState>,
    transfers: State<'_, SftpTransferState>,
    connection_id: i64,
) -> Result<(), ErrorResponse> {
    // 标签关闭代表用户明确断开：先让传输安全退出，再终止复用连接。
    transfers.cancel_connection_and_wait(connection_id, Duration::from_secs(5))?;
    close_sftp_master(&state, connection_id)
}

struct SshProcess {
    command: Command,
    secret_dir: Option<SecretDirectory>,
}

/// 临时凭据目录由所有权管理，任何提前返回或后台任务退出都会触发清理。
/// Drop 无法把清理失败返回给调用者，因此只记录不包含凭据内容的诊断信息。
#[derive(Clone)]
struct SecretDirectory(Arc<SecretDirectoryInner>);

struct SecretDirectoryInner {
    path: PathBuf,
}

impl SecretDirectory {
    fn create(id: &str) -> Result<Self, ErrorResponse> {
        let path = std::env::temp_dir().join(format!("nocterm-sftp-{id}"));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|_| error("SFTP_CREDENTIAL_FAILED", "创建临时凭据目录失败", true))?;
        Ok(Self(Arc::new(SecretDirectoryInner { path })))
    }

    fn path(&self) -> &Path {
        &self.0.path
    }
}

impl Drop for SecretDirectoryInner {
    fn drop(&mut self) {
        match fs::remove_dir_all(&self.path) {
            Err(cleanup_error) if cleanup_error.kind() != std::io::ErrorKind::NotFound => {
                eprintln!("failed to remove temporary SFTP credential directory: {cleanup_error}");
            }
            _ => {}
        }
    }
}

/// Unix 在创建文件时直接设置权限，避免先写入再 chmod 产生短暂的宽权限窗口。
fn write_secret_file(path: &Path, content: &[u8], executable: bool) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(if executable { 0o700 } else { 0o600 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    options.open(path)?.write_all(content)
}

fn ssh_process(
    state: &AppState,
    connection_id: i64,
    id: &str,
) -> Result<SshProcess, ErrorResponse> {
    let profile = state
        .connection_service()
        .get(connection_id)
        .map_err(ErrorResponse::from)?;
    let mut command = Command::new("ssh");
    let control_path = format!("ControlPath={}", ssh_control_path(&profile).display());
    command.args([
        "-p",
        &profile.port.to_string(),
        "-o",
        "ConnectTimeout=8",
        "-o",
        "ServerAliveInterval=30",
        "-o",
        "ServerAliveCountMax=3",
        "-o",
        // SFTP 没有交互式指纹确认界面；未知主机必须先通过 SSH 终端完成确认。
        "StrictHostKeyChecking=yes",
        "-o",
        "ControlMaster=auto",
        "-o",
        "ControlPersist=120",
        "-o",
        &control_path,
    ]);
    let mut secret_dir = None;
    match profile.authentication {
        AuthenticationMethod::PrivateKey => {
            let secret = read_secret(state, &connection_id.to_string(), "private_key")?;
            let dir = SecretDirectory::create(id)?;
            let key = dir.path().join("identity");
            if write_secret_file(&key, secret.as_bytes(), false).is_err() {
                return Err(error("SFTP_CREDENTIAL_FAILED", "写入临时私钥失败", true));
            }
            command.args(["-i", key.to_string_lossy().as_ref(), "-o", "BatchMode=yes"]);
            secret_dir = Some(dir);
        }
        AuthenticationMethod::SshAgent => {
            command.arg("-o").arg("BatchMode=yes");
        }
        AuthenticationMethod::Password => {
            let password = read_secret(state, &connection_id.to_string(), "password")?;
            #[cfg(unix)]
            {
                let dir = SecretDirectory::create(id)?;
                let pass = dir.path().join("password");
                let askpass = dir.path().join("askpass.sh");
                if write_secret_file(&pass, password.as_bytes(), false).is_err()
                    || write_secret_file(
                        &askpass,
                        b"#!/bin/sh\ncat \"$NOCTERM_SFTP_PASSWORD_FILE\"\n",
                        true,
                    )
                    .is_err()
                {
                    return Err(error("SFTP_CREDENTIAL_FAILED", "写入临时凭据失败", true));
                }
                command
                    .env("SSH_ASKPASS", &askpass)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env("NOCTERM_SFTP_PASSWORD_FILE", &pass)
                    .env("DISPLAY", "nocterm")
                    .stdin(Stdio::null());
                command.args([
                    "-o",
                    "PreferredAuthentications=password,keyboard-interactive",
                    "-o",
                    "PubkeyAuthentication=no",
                    "-o",
                    "NumberOfPasswordPrompts=1",
                ]);
                secret_dir = Some(dir);
            }
            #[cfg(windows)]
            {
                let dir = SecretDirectory::create(id)?;
                let pass = dir.path().join("password");
                let askpass = dir.path().join("askpass.cmd");
                if write_secret_file(&pass, password.as_bytes(), false).is_err()
                    || write_secret_file(
                        &askpass,
                        b"@echo off\r\ntype \"%NOCTERM_SFTP_PASSWORD_FILE%\"\r\n",
                        false,
                    )
                    .is_err()
                {
                    return Err(error("SFTP_CREDENTIAL_FAILED", "写入临时凭据失败", true));
                }
                command
                    .env("SSH_ASKPASS", &askpass)
                    .env("SSH_ASKPASS_REQUIRE", "force")
                    .env("NOCTERM_SFTP_PASSWORD_FILE", &pass)
                    .stdin(Stdio::null());
                command.args([
                    "-o",
                    "PreferredAuthentications=password,keyboard-interactive",
                    "-o",
                    "PubkeyAuthentication=no",
                    "-o",
                    "NumberOfPasswordPrompts=1",
                ]);
                secret_dir = Some(dir);
            }
        }
    }
    command.arg(format!("{}@{}", profile.username, profile.host));
    Ok(SshProcess {
        command,
        secret_dir,
    })
}

fn cleanup(dir: Option<SecretDirectory>) {
    drop(dir);
}

/// 将 OpenSSH 的 stderr 归一化为前端稳定错误码，不把平台文本作为契约暴露。
fn classify_remote_failure(stderr: &[u8], fallback_code: &'static str) -> ErrorResponse {
    let code = classify_remote_code(stderr);
    error(
        fallback_code,
        format!("__NOCTERM_SFTP_ERROR__\t{code}\t"),
        true,
    )
}

fn classify_remote_code(stderr: &[u8]) -> &'static str {
    let detail = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if detail.contains("host key verification failed")
        || detail.contains("remote host identification has changed")
    {
        "hostKeyFailed"
    } else if detail.contains("connection timed out") || detail.contains("operation timed out") {
        "timeout"
    } else if detail.contains("connection refused") {
        "connectionRefused"
    } else if detail.contains("could not resolve hostname")
        || detail.contains("name or service not known")
        || detail.contains("temporary failure in name resolution")
    {
        "hostResolveFailed"
    } else if detail.contains("no route to host") || detail.contains("network is unreachable") {
        "hostUnreachable"
    } else if detail.contains("no such file or directory") {
        "pathMissing"
    } else if detail.contains("permission denied") && detail.contains("cd:") {
        "permissionDenied"
    } else if detail.contains("permission denied")
        || detail.contains("authentication failed")
        || detail.contains("permission denied (publickey")
        || detail.contains("no supported authentication methods")
    {
        "authFailed"
    } else {
        "unknown"
    }
}

/// 远端目录协议使用 NUL 分隔字段；任何截断或非法记录都必须让整个列表失败。
fn parse_remote_listing(stdout: &[u8]) -> Result<(String, Vec<RemoteFileEntry>), ErrorResponse> {
    let mut remote_path = None;
    let mut entries = Vec::new();
    let mut fields = stdout.split(|byte| *byte == 0);
    while let Some(marker) = fields.next() {
        match marker {
            b"__NOCTERM_PWD__" => {
                let path = fields.next().ok_or_else(remote_listing_protocol_error)?;
                if remote_path.is_some() {
                    return Err(remote_listing_protocol_error());
                }
                remote_path = Some(
                    std::str::from_utf8(path)
                        .map_err(|_| remote_listing_protocol_error())?
                        .to_string(),
                );
            }
            b"__NOCTERM_ENTRY__" => {
                let name = fields.next().ok_or_else(remote_listing_protocol_error)?;
                let kind = fields.next().ok_or_else(remote_listing_protocol_error)?;
                let size = fields.next().ok_or_else(remote_listing_protocol_error)?;
                let modified = fields.next().ok_or_else(remote_listing_protocol_error)?;
                let is_dir = match kind {
                    b"d" => true,
                    b"f" => false,
                    _ => return Err(remote_listing_protocol_error()),
                };
                let size = if is_dir {
                    if !size.is_empty() {
                        return Err(remote_listing_protocol_error());
                    }
                    None
                } else {
                    Some(
                        std::str::from_utf8(size)
                            .map_err(|_| remote_listing_protocol_error())?
                            .parse()
                            .map_err(|_| remote_listing_protocol_error())?,
                    )
                };
                let modified_at = std::str::from_utf8(modified)
                    .map_err(|_| remote_listing_protocol_error())?
                    .split('.')
                    .next()
                    .ok_or_else(remote_listing_protocol_error)?
                    .parse()
                    .map_err(|_| remote_listing_protocol_error())?;
                entries.push(RemoteFileEntry {
                    name: std::str::from_utf8(name)
                        .map_err(|_| remote_listing_protocol_error())?
                        .to_string(),
                    is_dir,
                    size,
                    modified_at: Some(modified_at),
                });
            }
            b"" => {}
            _ => return Err(remote_listing_protocol_error()),
        }
    }
    Ok((
        remote_path.ok_or_else(remote_listing_protocol_error)?,
        entries,
    ))
}

fn remote_listing_protocol_error() -> ErrorResponse {
    error(
        "SFTP_REMOTE_PROTOCOL_INVALID",
        "远程目录响应不完整，请重试",
        true,
    )
}

/// GNU find 能在一次遍历中输出全部元数据；其他 Unix 使用安全的逐项回退。
/// 两条路径都使用 NUL 分隔文件名，并忽略当前文件模型无法表示的特殊文件。
fn remote_listing_script(requested: &str) -> String {
    format!(
        "cd -- {} || exit 12; printf '__NOCTERM_PWD__\\000%s\\000' \"$(pwd -P)\" || exit 13; if find . -maxdepth 0 -printf '' >/dev/null 2>&1; then find . -mindepth 1 -maxdepth 1 -type d -printf '__NOCTERM_ENTRY__\\000%f\\000d\\000\\000%T@\\000' && find . -mindepth 1 -maxdepth 1 -type f -printf '__NOCTERM_ENTRY__\\000%f\\000f\\000%s\\000%T@\\000' || exit 13; else for p in ./* ./.[!.]* ./..?*; do [ -e \"$p\" ] || [ -L \"$p\" ] || continue; if [ -d \"$p\" ]; then kind=d; elif [ -f \"$p\" ]; then kind=f; else continue; fi; metadata=$(stat -c '%s %Y' \"$p\" 2>/dev/null || stat -f '%z %m' \"$p\" 2>/dev/null) || exit 13; size=${{metadata%% *}}; modified=${{metadata#* }}; if [ \"$kind\" = d ]; then size=; fi; printf '__NOCTERM_ENTRY__\\000%s\\000%s\\000%s\\000%s\\000' \"${{p#./}}\" \"$kind\" \"$size\" \"$modified\" || exit 13; done; fi",
        shell_quote(requested)
    )
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
    let mut process = ssh_process(&state, connection_id, &now_id("list"))?;
    let script = remote_listing_script(&requested);
    let output = process
        .command
        .arg(script)
        .output()
        .map_err(|_| error("SFTP_REMOTE_READ_FAILED", "读取远程目录失败", true));
    cleanup(process.secret_dir);
    let output = output?;
    if !output.status.success() {
        return Err(classify_remote_failure(
            &output.stderr,
            "SFTP_REMOTE_READ_FAILED",
        ));
    }
    let (remote_path, mut entries) = parse_remote_listing(&output.stdout)?;
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(RemoteDirectoryListing {
        parent: parent_remote(&remote_path),
        path: remote_path,
        entries,
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
    let mut p = ssh_process(&state, connection_id, &now_id("exists"))?;
    let out = p
        .command
        .arg(format!("test -e {}", shell_quote(&remote_path)))
        .output()
        .map_err(|_| error("SFTP_REMOTE_READ_FAILED", "检查远程路径失败", true));
    cleanup(p.secret_dir);
    let out = out?;
    if out.status.success() {
        Ok(true)
    } else if out.stderr.is_empty() {
        // `test -e` 使用退出码 1 表示目标不存在，这不是 SSH 连接错误。
        Ok(false)
    } else {
        Err(classify_remote_failure(
            &out.stderr,
            "SFTP_REMOTE_READ_FAILED",
        ))
    }
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
    let mut p = ssh_process(&state, connection_id, &now_id("mkdir"))?;
    let out = p
        .command
        .arg(format!(
            "mkdir -- {}",
            shell_quote(&join_remote(&parent, &name))
        ))
        .output()
        .map_err(|_| error("SFTP_REMOTE_WRITE_FAILED", "创建远程目录失败", true));
    cleanup(p.secret_dir);
    if out?.status.success() {
        Ok(())
    } else {
        Err(error(
            "SFTP_REMOTE_WRITE_FAILED",
            "创建远程目录失败，请检查权限",
            true,
        ))
    }
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
    let mut p = ssh_process(&state, connection_id, &now_id("rename"))?;
    let out = p
        .command
        .arg(format!(
            "mv -- {} {}",
            shell_quote(&remote_path),
            shell_quote(&join_remote(&parent, &new_name))
        ))
        .output()
        .map_err(|_| error("SFTP_REMOTE_WRITE_FAILED", "重命名远程路径失败", true));
    cleanup(p.secret_dir);
    if out?.status.success() {
        Ok(())
    } else {
        Err(error(
            "SFTP_REMOTE_WRITE_FAILED",
            "重命名远程路径失败，请检查权限",
            true,
        ))
    }
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
    let mut p = ssh_process(&state, connection_id, &now_id("delete"))?;
    let out = p
        .command
        .arg(format!("rm -rf -- {}", shell_quote(&remote_path)))
        .output()
        .map_err(|_| error("SFTP_REMOTE_WRITE_FAILED", "删除远程路径失败", true));
    cleanup(p.secret_dir);
    if out?.status.success() {
        Ok(())
    } else {
        Err(error(
            "SFTP_REMOTE_WRITE_FAILED",
            "删除远程路径失败，请检查权限",
            true,
        ))
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

fn cleanup_remote_temp(app: &AppHandle, connection_id: i64, path: &str, task_id: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Ok(mut process) = ssh_process(&state, connection_id, &format!("{task_id}-cleanup")) else {
        return;
    };
    let _ = process
        .command
        .arg(format!("rm -rf -- {}", shell_quote(path)))
        .output();
    cleanup(process.secret_dir);
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

/// 远端替换同样采用备份与回滚，不能把“上传流结束”误报为“目标已提交”。
fn commit_remote_temp(
    app: &AppHandle,
    connection_id: i64,
    temp: &str,
    target: &str,
    backup: &str,
    task_id: &str,
) -> Result<(), &'static str> {
    let Some(state) = app.try_state::<AppState>() else {
        return Err("unknown");
    };
    let Ok(mut process) = ssh_process(&state, connection_id, &format!("{task_id}-commit")) else {
        return Err("unknown");
    };
    let script = format!(
        "set -e; rm -rf -- {backup}; had=0; if [ -e {target} ] || [ -L {target} ]; then mv -- {target} {backup}; had=1; fi; if mv -- {temp} {target}; then rm -rf -- {backup}; else if [ \"$had\" = 1 ]; then mv -- {backup} {target}; fi; exit 1; fi",
        backup = shell_quote(backup),
        target = shell_quote(target),
        temp = shell_quote(temp),
    );
    let output = process.command.arg(script).output();
    cleanup(process.secret_dir);
    match output {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(classify_remote_code(&output.stderr)),
        Err(_) => Err("unknown"),
    }
}

fn kill_child(child: &Arc<Mutex<Child>>) {
    if let Ok(mut child) = child.lock() {
        let _ = child.kill();
    }
}

fn wait_child(child: &Arc<Mutex<Child>>) -> bool {
    child
        .lock()
        .ok()
        .and_then(|mut child| child.wait().ok())
        .map(|status| status.success())
        .unwrap_or(false)
}

/// 阻塞在管道读写时由独立观察线程终止 SSH，确保取消不依赖下一次循环迭代。
fn watch_cancellation(
    child: Arc<Mutex<Child>>,
    cancel: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !finished.load(Ordering::SeqCst) {
            if cancel.load(Ordering::SeqCst) {
                kill_child(&child);
                return;
            }
            thread::sleep(Duration::from_millis(25));
        }
    })
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
    let is_dir = source.is_dir();
    // tar 流包含头部与填充，目录大小不能作为可靠百分比分母。
    let total = if is_dir { 0 } else { total_size(&source) };
    let name = source
        .file_name()
        .and_then(|v| v.to_str())
        .ok_or_else(|| error("SFTP_LOCAL_PATH_INVALID", "无法识别本地文件名", false))?
        .to_string();
    let task = now_id("upload");
    let task_id = task.clone();
    let (mut reader, mut source_child): (Box<dyn Read + Send>, Option<Child>) = if is_dir {
        let mut child = Command::new("tar")
            .args(["-C", source.to_string_lossy().as_ref(), "-cf", "-", "."])
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|_| error("SFTP_TRANSFER_FAILED", "启动本地归档任务失败", true))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            let _ = child.kill();
            let _ = child.wait();
            error("SFTP_TRANSFER_FAILED", "打开本地归档流失败", true)
        })?;
        (Box::new(stdout), Some(child))
    } else {
        let file = fs::File::open(&source)
            .map_err(|_| error("SFTP_LOCAL_READ_FAILED", "打开本地文件失败", true))?;
        (Box::new(file), None)
    };
    let app2 = app.clone();
    let profile_state = app.state::<AppState>();
    let mut process = match ssh_process(&profile_state, connection_id, &task) {
        Ok(process) => process,
        Err(error) => {
            if let Some(child) = source_child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            return Err(error);
        }
    };
    let temp = join_remote(
        &remote_dir,
        &temp_name(&name, &task, if is_dir { "partial" } else { "part" }),
    );
    let receive = if is_dir {
        format!(
            "set -e; rm -rf -- {0}; mkdir -- {0}; tar -xpf - -C {0}",
            shell_quote(&temp)
        )
    } else {
        format!("set -e; cat > {}", shell_quote(&temp))
    };
    process
        .command
        .arg(receive)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = process.command.spawn().map_err(|_| {
        cleanup(process.secret_dir.clone());
        if let Some(child) = source_child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        error("SFTP_TRANSFER_FAILED", "启动上传任务失败", true)
    })?;
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        cleanup(process.secret_dir);
        if let Some(source_child) = source_child.as_mut() {
            let _ = source_child.kill();
            let _ = source_child.wait();
        }
        return Err(error("SFTP_TRANSFER_FAILED", "打开上传流失败", true));
    };
    let Some(mut stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        cleanup(process.secret_dir);
        if let Some(source_child) = source_child.as_mut() {
            let _ = source_child.kill();
            let _ = source_child.wait();
        }
        return Err(error("SFTP_TRANSFER_FAILED", "打开上传错误流失败", true));
    };
    let flag = match register(&state, &task, connection_id) {
        Ok(flag) => flag,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            cleanup(process.secret_dir);
            if let Some(source_child) = source_child.as_mut() {
                let _ = source_child.kill();
                let _ = source_child.wait();
            }
            return Err(error);
        }
    };
    let controls = state.controls.clone();
    let commit_conn = connection_id;
    let secret = process.secret_dir;
    let target = join_remote(&remote_dir, &name);
    let backup = join_remote(&remote_dir, &temp_name(&name, &task, "backup"));
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
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let cancel_watcher = watch_cancellation(child.clone(), flag.clone(), finished.clone());
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });
        let mut transferred = 0;
        let mut buf = [0u8; 65536];
        let mut local_read_ok = true;
        let mut remote_write_ok = true;
        loop {
            if flag.load(Ordering::SeqCst) {
                break;
            }
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if stdin.write_all(&buf[..n]).is_err() {
                        remote_write_ok = false;
                        break;
                    }
                    transferred += n as u64;
                    emit_transfer_state(
                        &app2,
                        TransferEvent {
                            task_id: &task,
                            direction: "upload",
                            file_name: &name,
                            transferred,
                            total,
                            status: "running",
                            error_code: None,
                        },
                    );
                }
                Err(_) => {
                    local_read_ok = false;
                    break;
                }
            }
        }
        drop(stdin);
        drop(reader);
        let cancelled = flag.load(Ordering::SeqCst);
        if cancelled || !local_read_ok || !remote_write_ok {
            kill_child(&child);
            if let Some(source_child) = source_child.as_mut() {
                let _ = source_child.kill();
            }
        }
        let remote_ok = wait_child(&child);
        let source_ok = source_child
            .as_mut()
            .map(|child| child.wait().map(|status| status.success()).unwrap_or(false))
            .unwrap_or(true);
        finished.store(true, Ordering::SeqCst);
        let _ = cancel_watcher.join();
        let stderr = stderr_reader.join().unwrap_or_default();

        let ready_to_commit =
            !cancelled && local_read_ok && remote_write_ok && remote_ok && source_ok;
        let commit_error = ready_to_commit
            .then(|| commit_remote_temp(&app2, commit_conn, &temp, &target, &backup, &task))
            .and_then(Result::err);
        let committed = ready_to_commit && commit_error.is_none();
        if !committed {
            cleanup_remote_temp(&app2, commit_conn, &temp, &task);
        }

        let status = if cancelled {
            "cancelled"
        } else if committed {
            "completed"
        } else {
            "error"
        };
        let error_code = (!cancelled && !committed).then(|| {
            if !local_read_ok || !source_ok {
                "localReadFailed"
            } else if !remote_ok || !remote_write_ok {
                classify_remote_code(&stderr)
            } else {
                commit_error.unwrap_or("unknown")
            }
        });
        emit_transfer_state(
            &app2,
            TransferEvent {
                task_id: &task,
                direction: "upload",
                file_name: &name,
                transferred,
                total,
                status,
                error_code,
            },
        );
        unregister(&controls, &task);
        cleanup(secret);
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
    let is_dir = is_dir.unwrap_or(false);
    let name = remote_path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| error("SFTP_INVALID_PATH", "无法识别远程文件名", false))?
        .to_string();
    valid_name(&name)?;
    let task = now_id("download");
    let task_id = task.clone();
    let temp = target_dir.join(temp_name(
        &name,
        &task,
        if is_dir { "partial" } else { "part" },
    ));
    let target = target_dir.join(&name);
    let backup = target_dir.join(temp_name(&name, &task, "backup"));
    let (mut writer, mut extract_child): (Box<dyn Write + Send>, Option<Child>) = if is_dir {
        fs::create_dir(&temp)
            .map_err(|_| error("SFTP_LOCAL_WRITE_FAILED", "创建本地临时目录失败", true))?;
        let mut child = Command::new("tar")
            .args(["-xpf", "-", "-C", temp.to_string_lossy().as_ref()])
            .stdin(Stdio::piped())
            .spawn()
            .map_err(|_| {
                remove_path(&temp);
                error("SFTP_TRANSFER_FAILED", "启动本地解压任务失败", true)
            })?;
        let Some(stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            remove_path(&temp);
            return Err(error("SFTP_TRANSFER_FAILED", "打开本地解压流失败", true));
        };
        (Box::new(stdin), Some(child))
    } else {
        let file = fs::File::create(&temp)
            .map_err(|_| error("SFTP_LOCAL_WRITE_FAILED", "创建本地临时文件失败", true))?;
        (Box::new(file), None)
    };
    let app2 = app.clone();
    let profile_state = app.state::<AppState>();
    let mut process = match ssh_process(&profile_state, connection_id, &task) {
        Ok(process) => process,
        Err(error) => {
            if let Some(child) = extract_child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
            remove_path(&temp);
            return Err(error);
        }
    };
    let script = if is_dir {
        let parent = parent_remote(&remote_path).unwrap_or_else(|| "/".into());
        format!(
            "tar -C {} -cf - -- {}",
            shell_quote(&parent),
            shell_quote(&name)
        )
    } else {
        format!("cat -- {}", shell_quote(&remote_path))
    };
    process
        .command
        .arg(script)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = process.command.spawn().map_err(|_| {
        cleanup(process.secret_dir.clone());
        if let Some(child) = extract_child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        remove_path(&temp);
        error("SFTP_TRANSFER_FAILED", "启动下载任务失败", true)
    })?;
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        cleanup(process.secret_dir);
        if let Some(extract_child) = extract_child.as_mut() {
            let _ = extract_child.kill();
            let _ = extract_child.wait();
        }
        remove_path(&temp);
        return Err(error("SFTP_TRANSFER_FAILED", "打开下载流失败", true));
    };
    let Some(mut stderr) = child.stderr.take() else {
        let _ = child.kill();
        let _ = child.wait();
        cleanup(process.secret_dir);
        if let Some(extract_child) = extract_child.as_mut() {
            let _ = extract_child.kill();
            let _ = extract_child.wait();
        }
        remove_path(&temp);
        return Err(error("SFTP_TRANSFER_FAILED", "打开下载错误流失败", true));
    };
    let flag = match register(&state, &task, connection_id) {
        Ok(flag) => flag,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            cleanup(process.secret_dir);
            if let Some(extract_child) = extract_child.as_mut() {
                let _ = extract_child.kill();
                let _ = extract_child.wait();
            }
            remove_path(&temp);
            return Err(error);
        }
    };
    let controls = state.controls.clone();
    let secret = process.secret_dir;
    let total = total.unwrap_or(0);
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
        let child = Arc::new(Mutex::new(child));
        let finished = Arc::new(AtomicBool::new(false));
        let cancel_watcher = watch_cancellation(child.clone(), flag.clone(), finished.clone());
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = stderr.read_to_end(&mut bytes);
            bytes
        });
        let mut transferred = 0;
        let mut buf = [0u8; 65536];
        let mut io_ok = true;
        loop {
            if flag.load(Ordering::SeqCst) {
                break;
            }
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if writer.write_all(&buf[..n]).is_err() {
                        io_ok = false;
                        break;
                    }
                    transferred += n as u64;
                    emit_transfer_state(
                        &app2,
                        TransferEvent {
                            task_id: &task,
                            direction: "download",
                            file_name: &name,
                            transferred,
                            total,
                            status: "running",
                            error_code: None,
                        },
                    );
                }
                Err(_) => {
                    io_ok = false;
                    break;
                }
            }
        }
        drop(writer);
        let cancelled = flag.load(Ordering::SeqCst);
        if cancelled || !io_ok {
            kill_child(&child);
            if let Some(extract_child) = extract_child.as_mut() {
                let _ = extract_child.kill();
            }
        }
        let remote_ok = wait_child(&child);
        let extract_ok = extract_child
            .as_mut()
            .map(|child| child.wait().map(|status| status.success()).unwrap_or(false))
            .unwrap_or(true);
        finished.store(true, Ordering::SeqCst);
        let _ = cancel_watcher.join();
        let stderr = stderr_reader.join().unwrap_or_default();

        let staged = if is_dir {
            temp.join(&name)
        } else {
            temp.clone()
        };
        let committed = !cancelled
            && io_ok
            && remote_ok
            && extract_ok
            && path_exists(&staged)
            && replace_local_path(&staged, &target, &backup).is_ok();
        if is_dir || !committed {
            remove_path(&temp);
        }
        let status = if cancelled {
            "cancelled"
        } else if committed {
            "completed"
        } else {
            "error"
        };
        let error_code = (!cancelled && !committed).then(|| {
            if remote_ok {
                "localWriteFailed"
            } else {
                classify_remote_code(&stderr)
            }
        });
        emit_transfer_state(
            &app2,
            TransferEvent {
                task_id: &task,
                direction: "download",
                file_name: &name,
                transferred,
                total,
                status,
                error_code,
            },
        );
        unregister(&controls, &task);
        cleanup(secret);
    });
    Ok(TransferStart { task_id })
}

#[cfg(test)]
mod tests {
    use std::{fs, process::Command};

    use super::{
        SecretDirectory, classify_remote_code, join_remote, missing_control_master, now_id,
        parent_remote, parse_remote_listing, remote_listing_script, replace_local_path,
        shell_quote, temp_name, valid_name, write_secret_file,
    };

    #[test]
    fn classifies_common_ssh_failures() {
        assert_eq!(classify_remote_code(b"Connection timed out"), "timeout");
        assert_eq!(
            classify_remote_code(b"Connection refused"),
            "connectionRefused"
        );
        assert_eq!(
            classify_remote_code(b"Host key verification failed"),
            "hostKeyFailed"
        );
        assert_eq!(
            classify_remote_code(b"Permission denied (publickey,password)"),
            "authFailed"
        );
    }

    #[test]
    fn treats_an_absent_control_master_as_an_idempotent_disconnect() {
        assert!(missing_control_master(
            b"Control socket connect(/tmp/nocterm): No such file or directory"
        ));
        assert!(!missing_control_master(b"Permission denied"));
    }

    #[test]
    fn quotes_shell_values_without_losing_single_quotes() {
        assert_eq!(shell_quote("a'b"), "'a'\"'\"'b'");
    }

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
    fn secret_directory_drop_removes_sensitive_files() {
        let id = now_id("nocterm-sftp-secret-test");
        let path = {
            let directory = SecretDirectory::create(&id).expect("create secret directory");
            let path = directory.path().to_path_buf();
            write_secret_file(&path.join("secret"), b"sensitive", false)
                .expect("write secret file");
            assert!(path.join("secret").exists());
            path
        };

        assert!(!path.exists());
    }

    #[test]
    fn task_ids_remain_unique_within_the_same_process() {
        assert_ne!(now_id("upload"), now_id("upload"));
    }

    #[test]
    fn parses_remote_names_with_tabs_and_newlines_without_splitting_records() {
        let payload =
            b"__NOCTERM_PWD__\0/srv\0__NOCTERM_ENTRY__\0line\nwith\ttab\0f\0\x31\x32\0\x33\x34\0";
        let (path, entries) = parse_remote_listing(payload).expect("parse complete listing");

        assert_eq!(path, "/srv");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "line\nwith\ttab");
        assert_eq!(entries[0].size, Some(12));
        assert_eq!(entries[0].modified_at, Some(34));
    }

    #[test]
    fn parses_fractional_gnu_find_timestamps_as_epoch_seconds() {
        let payload = b"__NOCTERM_PWD__\0/srv\0__NOCTERM_ENTRY__\0note.txt\0f\0\x35\0\x31\x37\x30\x30.123456789\0";
        let (_, entries) = parse_remote_listing(payload).expect("parse GNU timestamp");

        assert_eq!(entries[0].modified_at, Some(1700));
    }

    #[test]
    fn remote_listing_reads_metadata_without_opening_file_contents() {
        let script = remote_listing_script("/srv/data");

        assert!(!script.contains("wc -c"));
        assert!(script.contains("find . -mindepth 1 -maxdepth 1"));
        assert!(script.contains("&& find . -mindepth 1 -maxdepth 1 -type f"));
        assert!(script.contains("-printf '__NOCTERM_ENTRY__"));
        assert!(script.contains("elif [ -f"));
        assert!(script.contains("else continue"));
    }

    #[test]
    fn rejects_truncated_or_invalid_remote_listing_records() {
        let truncated = b"__NOCTERM_PWD__\x00/srv\x00__NOCTERM_ENTRY__\x00note.txt\x00f\x005\x00";
        let invalid_kind =
            b"__NOCTERM_PWD__\x00/srv\x00__NOCTERM_ENTRY__\x00link\x00l\x000\x001700\x00";
        let missing_path = b"__NOCTERM_ENTRY__\x00note.txt\x00f\x005\x001700\x00";

        assert!(parse_remote_listing(truncated).is_err());
        assert!(parse_remote_listing(invalid_kind).is_err());
        assert!(parse_remote_listing(missing_path).is_err());
    }

    #[test]
    fn remote_listing_script_returns_file_and_directory_metadata() {
        let root = std::env::temp_dir().join(now_id("nocterm-sftp-list-test"));
        fs::create_dir_all(root.join("folder")).expect("create test directory");
        fs::write(root.join("note.txt"), "hello").expect("write test file");

        let output = Command::new("sh")
            .arg("-c")
            .arg(remote_listing_script(
                root.to_str().expect("temporary path is UTF-8"),
            ))
            .output()
            .expect("run listing script");
        assert!(output.status.success());

        let (path, entries) =
            parse_remote_listing(&output.stdout).expect("parse listing command output");
        assert_eq!(
            fs::canonicalize(path).expect("canonicalize listed path"),
            fs::canonicalize(&root).expect("canonicalize test path")
        );
        assert_eq!(entries.len(), 2);
        assert!(
            entries
                .iter()
                .any(|entry| entry.name == "folder" && entry.is_dir)
        );
        assert!(
            entries.iter().any(|entry| {
                entry.name == "note.txt" && !entry.is_dir && entry.size == Some(5)
            })
        );

        fs::remove_dir_all(root).expect("remove test directory");
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
