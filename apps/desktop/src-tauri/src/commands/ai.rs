use tauri::{AppHandle, State};

use crate::{
    commands::{
        ai_provider::provider_adapters, ai_session::start_session,
        ai_validation::validate_conversation_id,
    },
    dto::ai::{AiProviderStatus, AiSessionStartRequest, AiSessionStartResponse},
    state::AppState,
};

#[tauri::command]
pub fn ai_provider_status() -> Vec<AiProviderStatus> {
    provider_adapters()
        .into_iter()
        .map(|adapter| AiProviderStatus {
            id: adapter.id().into(),
            command: adapter.command().into(),
            available: adapter.executable().is_some(),
        })
        .collect()
}

#[tauri::command(async)]
pub fn ai_session_start(
    app: AppHandle,
    state: State<'_, AppState>,
    request: AiSessionStartRequest,
) -> Result<AiSessionStartResponse, String> {
    start_session(app, state.inner(), request)
}

#[tauri::command]
pub fn ai_session_stop(state: State<'_, AppState>, session_id: String) -> Result<(), String> {
    let mut handled = false;
    let mut errors = Vec::new();

    // 每一步都尽力执行：某个状态锁异常不能阻止后续 Provider 和子进程回收。
    match state.ai_gateway().deactivate_session(&session_id) {
        Ok(deactivated) => handled |= deactivated,
        Err(error) => errors.push(error),
    }
    match state.ai_provider_runtimes().stop_turn(&session_id) {
        Ok(stopped) => handled |= stopped,
        Err(error) => errors.push(error),
    }
    match state.ai_processes().remove(&session_id) {
        Ok(Some(mut process)) => {
            handled = true;
            // 即使 kill 失败或进程刚好退出，Bridge token 也必须立即失效。
            let kill_result = process.child.kill();
            if let Some(token) = process.bridge_token.as_deref() {
                state.ai_gateway().revoke(token);
            }
            if let Err(error) = kill_result {
                errors.push(format!("停止 AI Provider 进程失败：{error}"));
            }
        }
        Ok(None) => {}
        Err(error) => errors.push(error),
    }

    if !errors.is_empty() {
        return Err(format!("停止 AI 会话失败：{}", errors.join("；")));
    }
    if !handled {
        return Err("AI 会话不存在".to_string());
    }
    Ok(())
}

#[tauri::command]
pub fn ai_conversation_reset(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let conversation_id = validate_conversation_id(&conversation_id)?;
    state
        .ai_provider_runtimes()
        .reset_conversation(&conversation_id)?;
    Ok(())
}

#[tauri::command]
pub fn ai_tool_approval_resolve(
    state: State<'_, AppState>,
    approval_id: String,
    session_id: String,
    approved: bool,
) -> Result<(), String> {
    if state
        .ai_gateway()
        .resolve_approval(&approval_id, &session_id, approved)?
    {
        Ok(())
    } else {
        Err("AI 命令确认不存在或已过期".to_string())
    }
}
