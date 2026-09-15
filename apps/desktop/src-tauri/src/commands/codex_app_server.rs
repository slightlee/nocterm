//! Codex app-server 会话注册表与复用策略。

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use tauri::AppHandle;

use crate::state::{AiCommandPolicy, AiGatewayState};

mod protocol;
mod session;

use session::CodexAppServer;

/// app-server 正常应快速确认 interrupt；超时仍未收尾时必须终止进程，避免失控 turn
/// 永久占用对话并让前端只能等待空闲超时。
const CODEX_INTERRUPT_GRACE_TIMEOUT: Duration = Duration::from_secs(5);

/// Codex app-server 与一个 Nocterm 对话一一对应。目标变化时必须重建，防止旧 MCP token
/// 被用于另一个终端；同一对话的连续 turn 则复用进程、配置和 Codex thread。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodexSessionIdentity {
    pub connection_id: Option<i64>,
    pub target_session_id: Option<String>,
    pub working_directory: Option<String>,
}

/// 启动一次 Codex turn 所需的完整上下文；调用方无需依赖长参数列表的位置约定。
pub struct CodexTurnRequest {
    pub conversation_id: String,
    pub session_id: String,
    pub identity: CodexSessionIdentity,
    pub initial_prompt: String,
    pub continuation_prompt: String,
    pub bridge: Option<(String, String)>,
    pub bridge_executable: String,
    pub provider_executable: PathBuf,
    pub command_policy: AiCommandPolicy,
}

#[derive(Default)]
pub struct CodexAppServerManager {
    sessions: Mutex<HashMap<String, Arc<CodexAppServer>>>,
}

impl CodexAppServerManager {
    pub fn start_turn(
        &self,
        app: AppHandle,
        request: CodexTurnRequest,
        gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String> {
        let CodexTurnRequest {
            conversation_id,
            session_id,
            identity,
            initial_prompt,
            continuation_prompt,
            bridge,
            bridge_executable,
            provider_executable,
            command_policy,
        } = request;
        let (server, is_new) = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "Codex 会话状态不可用".to_string())?;
            let reusable = sessions
                .get(&conversation_id)
                .filter(|server| server.is_alive())
                .filter(|server| server.matches_identity(&identity))
                .cloned();
            if let Some(server) = reusable {
                (server, false)
            } else {
                if let Some(previous) = sessions.remove(&conversation_id) {
                    previous.shutdown();
                }
                let server = CodexAppServer::spawn(
                    app.clone(),
                    identity,
                    bridge,
                    bridge_executable,
                    &provider_executable,
                    gateway,
                )?;
                sessions.insert(conversation_id.clone(), Arc::clone(&server));
                (server, true)
            }
        };
        let prompt = if is_new {
            initial_prompt
        } else {
            continuation_prompt
        };
        if let Err(error) = server.start_turn(app, session_id, prompt, command_policy) {
            if is_new || !server.is_alive() {
                // 首轮失败或传输已损坏的复用进程都不能继续留在注册表中。
                let removed = {
                    let mut sessions = self
                        .sessions
                        .lock()
                        .map_err(|_| "Codex 会话状态不可用".to_string())?;
                    sessions
                        .get(&conversation_id)
                        .is_some_and(|current| Arc::ptr_eq(current, &server))
                        .then(|| sessions.remove(&conversation_id))
                        .flatten()
                };
                if let Some(removed) = removed {
                    removed.shutdown();
                }
            }
            return Err(error);
        }
        Ok(is_new)
    }

    /// 优先使用协议中断，保留已加载的 app-server 供下一轮复用。
    pub fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        let servers = self
            .sessions
            .lock()
            .map_err(|_| "Codex 会话状态不可用".to_string())?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for server in servers {
            if server.stop_turn(session_id)? {
                let interrupted_server = Arc::clone(&server);
                let interrupted_session_id = session_id.to_string();
                thread::spawn(move || {
                    thread::sleep(CODEX_INTERRUPT_GRACE_TIMEOUT);
                    interrupted_server.shutdown_if_turn_active(&interrupted_session_id);
                });
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// 清空、删除或切换 Provider 时同步销毁后台会话和会话级 Bridge token。
    pub fn reset_conversation(&self, conversation_id: &str) -> bool {
        let server = self
            .sessions
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(conversation_id));
        if let Some(server) = server {
            server.shutdown();
            true
        } else {
            false
        }
    }
}

impl Drop for CodexAppServerManager {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for (_, server) in sessions.drain() {
                server.shutdown();
            }
        }
    }
}
