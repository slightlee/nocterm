//! Codex app-server 会话注册表、Runtime 接入与复用策略。

use std::sync::Arc;

use tauri::AppHandle;

use crate::{
    commands::{
        ai_persistent::{PersistentSessionRegistry, PersistentTurnRequest},
        ai_provider::ProviderSessionIdentity,
        ai_runtime::{PersistentProviderLaunch, PersistentProviderRuntime},
    },
    state::AiGatewayState,
};

mod protocol;
mod session;

use session::{CodexAppServer, CodexAppServerLaunch};

/// Codex app-server 与一个 Nocterm 对话一一对应。目标变化时必须重建，防止旧 MCP token
/// 被用于另一个终端；同一对话的连续 turn 则复用进程、配置和 Codex thread。
pub type CodexSessionIdentity = ProviderSessionIdentity;

pub struct CodexAppServerManager {
    sessions: PersistentSessionRegistry<CodexAppServer>,
}

impl Default for CodexAppServerManager {
    fn default() -> Self {
        Self {
            sessions: PersistentSessionRegistry::new("Codex"),
        }
    }
}

impl PersistentProviderRuntime for CodexAppServerManager {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn start_turn(
        &self,
        app: AppHandle,
        launch: PersistentProviderLaunch,
        gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String> {
        let PersistentProviderLaunch {
            conversation_id,
            session_id,
            identity,
            initial_prompt,
            continuation_prompt,
            bridge,
            gateway_token: _,
            bridge_executable,
            provider_executable,
            command_policy,
            mcp_dispatcher: _,
        } = launch;
        self.sessions.start_turn(
            app.clone(),
            PersistentTurnRequest {
                conversation_id,
                session_id,
                identity: identity.clone(),
                initial_prompt,
                continuation_prompt,
                command_policy,
            },
            move |startup_cancellation, termination| {
                CodexAppServer::spawn(
                    app,
                    CodexAppServerLaunch {
                        identity,
                        bridge,
                        bridge_executable,
                        provider_executable,
                        gateway,
                    },
                    startup_cancellation,
                    termination,
                )
            },
        )
    }

    /// 优先使用协议中断，保留已加载的 app-server 供下一轮复用。
    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        self.sessions.stop_turn(session_id)
    }

    /// 清空、删除或切换 Provider 时同步销毁后台会话和会话级 Bridge token。
    fn reset_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        self.sessions.reset_conversation(conversation_id)
    }
}
