//! 文件编辑 IPC 只装配目标、权限和 DTO；内容规则及保存事务分别由用例和 Adapter 负责。
use super::sftp::{SftpTransferState, read_credentials};
use crate::platform::file_editor_close::{CloseIntent, FileEditorCloseState};
use crate::{dto::error::ErrorResponse, state::AppState};
use nocterm_application::{
    error::AppError,
    file_edit::{FileContent, FileEditService},
};
use nocterm_domain::file_edit::FileVersion;
use nocterm_infrastructure::file_edit::{LocalTextFiles, RemoteTextFiles};
use std::sync::{Arc, atomic::AtomicBool};
use tauri::State;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn file_editor_enable_close_guard(state: State<'_, FileEditorCloseState>) {
    // 前端只在监听成功后启用；启动阶段不能等待尚未存在的确认方。
    state.enable();
}

/// 迟到回复不能关闭新的窗口；系统动作失败会恢复保护，不能永久授权后续退出。
#[tauri::command]
pub fn file_editor_resolve_close(
    app: AppHandle,
    state: State<'_, FileEditorCloseState>,
    request_id: u64,
    allow: bool,
) -> Result<(), ErrorResponse> {
    if let Some(intent) = state.resolve(request_id, allow) {
        match intent {
            CloseIntent::Exit => app.exit(0),
            CloseIntent::Window(label) => {
                if let Some(window) = app.get_webview_window(&label) {
                    if window.destroy().is_err() {
                        state.restore();
                        return Err(AppError::new(
                            "FILE_EDITOR_CLOSE_FAILED",
                            "关闭窗口失败，请重试",
                            true,
                        )
                        .into());
                    }
                } else {
                    state.restore();
                }
            }
        }
    }
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteFileContent {
    content: String,
    size: Option<u64>,
    modified_at: Option<u64>,
    target_revision: String,
}
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteSaveResult {
    size: Option<u64>,
    modified_at: Option<u64>,
    target_revision: String,
}
#[derive(serde::Serialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFileBaseline {
    pub size: u64,
    pub modified_seconds: u64,
    pub modified_nanos: u32,
}
#[derive(serde::Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct LocalFileContent {
    pub content: String,
    pub identity_path: String,
    pub baseline: LocalFileBaseline,
}

impl From<FileVersion> for LocalFileBaseline {
    // 本地 Adapter 提供全部字段，DTO 保持秒/纳秒独立的既有 IPC 形态。
    fn from(value: FileVersion) -> Self {
        Self {
            size: value.size.unwrap_or(0),
            modified_seconds: value.modified_seconds.unwrap_or(0),
            modified_nanos: value.modified_nanos.unwrap_or(0),
        }
    }
}
impl From<FileContent> for LocalFileContent {
    fn from(value: FileContent) -> Self {
        Self {
            content: value.content,
            identity_path: value.identity_path,
            baseline: value.version.into(),
        }
    }
}

fn unavailable() -> ErrorResponse {
    AppError::new(
        "SFTP_REMOTE_TARGET_CHANGED",
        "连接状态不可用，请重新打开文件",
        true,
    )
    .into()
}

#[tauri::command(async)]
/// 后端签发目标版本，不接收前端重复传入的主机和认证资料。
pub fn sftp_read_remote_file(
    state: State<'_, AppState>,
    connection_id: i64,
    remote_path: String,
) -> Result<RemoteFileContent, ErrorResponse> {
    let target = state
        .file_targets()
        .connection(connection_id)
        .map_err(ErrorResponse::from)?;
    let mut target = target.lock().map_err(|_| unavailable())?;
    // 锁覆盖资料读取与文件快照，不能在读取中途换服务器。
    // 返回的文件内容和目标版本属于同一份 profile。
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    let revision = target.observe(&profile);
    let service = FileEditService(RemoteTextFiles {
        manager: state.sftp_manager().clone(),
        profile,
        password,
        private_key,
        cancel: Arc::new(AtomicBool::new(false)),
    });
    let value = service.read(&remote_path).map_err(ErrorResponse::from)?;
    Ok(RemoteFileContent {
        content: value.content,
        size: value.version.size,
        modified_at: value.version.modified_seconds,
        target_revision: revision,
    })
}

/// 同一连接的配置修改与保存共用目标锁；force 不能跳过打开时的目标版本。
#[allow(clippy::too_many_arguments)]
#[tauri::command(async)]
pub fn sftp_write_remote_file(
    state: State<'_, AppState>,
    transfers: State<'_, SftpTransferState>,
    connection_id: i64,
    remote_path: String,
    content: String,
    baseline_size: Option<u64>,
    baseline_modified_at: Option<u64>,
    target_revision: String,
    force: bool,
) -> Result<RemoteSaveResult, ErrorResponse> {
    let target = state
        .file_targets()
        .connection(connection_id)
        .map_err(ErrorResponse::from)?;
    let mut target = target.lock().map_err(|_| unavailable())?;
    let profile = state
        .connection_service()
        .get(connection_id)
        .map_err(ErrorResponse::from)?;
    let revision = target
        .require_revision(&profile, &target_revision)
        .map_err(ErrorResponse::from)?;
    // 错误版本在 stat 与上传之前拒绝，不触发远程写入。
    // 任务守卫覆盖完整用例，所有提前返回与错误传播都由 Drop 注销。
    let task = transfers.begin_edit(connection_id)?;
    let (profile, password, private_key) = read_credentials(&state, connection_id)?;
    let service = FileEditService(RemoteTextFiles {
        manager: state.sftp_manager().clone(),
        profile,
        password,
        private_key,
        cancel: task.cancel.clone(),
    });
    let value = service
        .write(
            &remote_path,
            &content,
            FileVersion {
                size: baseline_size,
                modified_seconds: baseline_modified_at,
                modified_nanos: None,
            },
            force,
        )
        .map_err(ErrorResponse::from)?;
    Ok(RemoteSaveResult {
        // 基线来自实际提交的暂存文件，不能在提交后重新 stat 目标。
        size: value.size,
        modified_at: value.modified_seconds,
        target_revision: revision,
    })
}

#[tauri::command(async)]
/// Host 只转换结果，本地权限保留与回滚由 Adapter 执行。
pub fn sftp_read_local_file(path: String) -> Result<LocalFileContent, ErrorResponse> {
    FileEditService(LocalTextFiles)
        .read(&path)
        .map(LocalFileContent::from)
        .map_err(ErrorResponse::from)
}

#[tauri::command(async)]
/// 保持扁平参数兼容；用例先验证文本，再进入文件系统事务。
pub fn sftp_write_local_file(
    path: String,
    content: String,
    baseline_size: Option<u64>,
    baseline_modified_seconds: Option<u64>,
    baseline_modified_nanos: Option<u32>,
    force: bool,
) -> Result<LocalFileBaseline, ErrorResponse> {
    FileEditService(LocalTextFiles)
        .write(
            &path,
            &content,
            FileVersion {
                size: baseline_size,
                modified_seconds: baseline_modified_seconds,
                modified_nanos: baseline_modified_nanos,
            },
            force,
        )
        .map(LocalFileBaseline::from)
        .map_err(ErrorResponse::from)
}
