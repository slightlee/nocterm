//! Grok ACP 会话注册表、Runtime 接入与按 UI 对话复用策略。

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::AppHandle;

use crate::{
    commands::{
        ai_persistent::{PersistentSessionRegistry, PersistentTurnRequest},
        ai_runtime::{PersistentProviderLaunch, PersistentProviderRuntime},
    },
    state::AiGatewayState,
};

pub(super) mod models;
mod protocol;
mod session;

pub(super) use protocol::initialize_request as grok_initialize_request;
use session::{GrokAcpServer, GrokAcpServerLaunch};

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

/// 目录探测和聊天进程共用序列，私有目录不会相互覆盖。
pub(super) fn next_runtime_id() -> (u128, u64) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    (timestamp, NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed))
}

pub struct GrokAcpServerManager {
    sessions: PersistentSessionRegistry<GrokAcpServer>,
}

impl Default for GrokAcpServerManager {
    fn default() -> Self {
        Self {
            sessions: PersistentSessionRegistry::new("Grok"),
        }
    }
}

impl PersistentProviderRuntime for GrokAcpServerManager {
    fn id(&self) -> &'static str {
        "grok"
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
            bridge: _,
            gateway_token,
            bridge_executable: _,
            provider_executable,
            command_policy,
            mcp_dispatcher,
        } = launch;
        let (timestamp, sequence) = next_runtime_id();
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
                GrokAcpServer::spawn(
                    app,
                    GrokAcpServerLaunch {
                        identity,
                        bridge_token: gateway_token,
                        provider_executable,
                        gateway,
                        mcp_dispatcher,
                        timestamp,
                        sequence,
                    },
                    startup_cancellation,
                    termination,
                )
            },
        )
    }

    /// ACP cancel 只停止当前 turn；超时未完成时回收进程，避免残留失控工具调用。
    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        self.sessions.stop_turn(session_id)
    }

    fn reset_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        self.sessions.reset_conversation(conversation_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::commands::ai_provider::ProviderSessionIdentity;

    #[test]
    fn persistent_identity_changes_when_the_terminal_target_changes() {
        let local = ProviderSessionIdentity {
            model: None,
            reasoning_effort: None,
            fast_mode: None,
            connection_id: None,
            target_session_id: Some("local:one".into()),
            working_directory: Some("/tmp".into()),
        };
        let remote = ProviderSessionIdentity {
            model: None,
            reasoning_effort: None,
            fast_mode: None,
            connection_id: Some(7),
            target_session_id: None,
            working_directory: Some("/tmp".into()),
        };
        assert_ne!(local, remote);
        assert_eq!(local, local.clone());
    }
}
