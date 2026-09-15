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
    // 先关闭 turn 级授权，确保停止请求与 Provider 中断并发时不会再启动新工具调用。
    state.ai_gateway().deactivate_session(&session_id)?;
    if state.ai_codex_servers().stop_turn(&session_id)? {
        return Ok(());
    }
    let mut process = state
        .ai_processes()
        .remove(&session_id)
        .map_err(|error| format!("停止 AI 会话失败：{error}"))?
        .ok_or_else(|| "AI 会话不存在".to_string())?;
    // 先取得 kill 结果，再无条件撤销 Bridge；即使进程刚好自然退出，token 也不能继续有效。
    let kill_result = process.child.kill();
    if let Some(token) = process.bridge_token.as_deref() {
        state.ai_gateway().revoke(token);
    }
    kill_result.map_err(|error| format!("停止 AI 会话失败：{error}"))
}

#[tauri::command]
pub fn ai_conversation_reset(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let conversation_id = validate_conversation_id(&conversation_id)?;
    state
        .ai_codex_servers()
        .reset_conversation(&conversation_id);
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
