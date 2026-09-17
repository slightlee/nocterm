//! 持续 Provider 的统一运行入口。
//! 注册表只按 Provider ID 分发生命周期，不解释 JSON-RPC、ACP 等厂商协议。

use std::{path::PathBuf, sync::Arc};

use tauri::AppHandle;

use crate::{
    commands::{
        ai_bridge::McpRequestDispatcher, ai_provider::ProviderSessionIdentity,
        codex_app_server::CodexAppServerManager, grok_acp_server::GrokAcpServerManager,
    },
    state::{AiCommandPolicy, AiGatewayState},
};

/// 持续 Provider 共用的启动数据；厂商握手参数由各自 Runtime 在内部生成。
pub struct PersistentProviderLaunch {
    pub conversation_id: String,
    pub session_id: String,
    pub identity: ProviderSessionIdentity,
    pub initial_prompt: String,
    pub continuation_prompt: String,
    pub bridge: Option<(String, String)>,
    /// 与传输无关的 Gateway 授权；ACP Provider 不需要回环端点。
    pub gateway_token: Option<String>,
    pub bridge_executable: String,
    pub provider_executable: PathBuf,
    pub command_policy: AiCommandPolicy,
    /// Grok 的官方 MCP-over-ACP 直接复用公共 Gateway；其他 Provider 可忽略此宿主能力。
    pub mcp_dispatcher: Arc<McpRequestDispatcher>,
}

/// Runtime 适配持续 Provider 的宿主生命周期；协议消息仍属于具体 Provider。
pub trait PersistentProviderRuntime: Send + Sync {
    fn id(&self) -> &'static str;
    fn start_turn(
        &self,
        app: AppHandle,
        launch: PersistentProviderLaunch,
        gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String>;
    fn stop_turn(&self, session_id: &str) -> Result<bool, String>;
    fn reset_conversation(&self, conversation_id: &str) -> Result<bool, String>;
}

/// 所有持续 Provider 的单一分发点，避免 Tauri Command 和 AppState 感知厂商类型。
pub struct PersistentProviderRuntimeRegistry {
    runtimes: Vec<Box<dyn PersistentProviderRuntime>>,
}

impl Default for PersistentProviderRuntimeRegistry {
    fn default() -> Self {
        Self::new(vec![
            Box::new(CodexAppServerManager::default()),
            Box::new(GrokAcpServerManager::default()),
        ])
    }
}

impl PersistentProviderRuntimeRegistry {
    fn new(runtimes: Vec<Box<dyn PersistentProviderRuntime>>) -> Self {
        debug_assert!(
            runtimes
                .iter()
                .enumerate()
                .all(|(index, runtime)| runtimes[index + 1..]
                    .iter()
                    .all(|other| other.id() != runtime.id())),
            "persistent Provider runtime IDs must be unique"
        );
        Self { runtimes }
    }

    pub fn start_turn(
        &self,
        provider_id: &str,
        app: AppHandle,
        launch: PersistentProviderLaunch,
        gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String> {
        self.runtime(provider_id)?.start_turn(app, launch, gateway)
    }

    pub fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        let mut stopped = false;
        let mut errors = Vec::new();
        // 即使一个 Runtime 状态损坏，也要继续尝试停止真正持有该 session 的其他 Provider。
        for runtime in &self.runtimes {
            match runtime.stop_turn(session_id) {
                Ok(handled) => stopped |= handled,
                Err(error) => errors.push(format!("{}: {error}", runtime.id())),
            }
        }
        finish_lifecycle_sweep("停止持续 AI 会话失败", stopped, errors)
    }

    pub fn reset_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        let mut reset = false;
        let mut errors = Vec::new();
        // 异常状态下同一对话可能同时残留多个 Provider，清理必须遍历全部 Runtime。
        for runtime in &self.runtimes {
            match runtime.reset_conversation(conversation_id) {
                Ok(handled) => reset |= handled,
                Err(error) => errors.push(format!("{}: {error}", runtime.id())),
            }
        }
        finish_lifecycle_sweep("重置持续 AI 会话失败", reset, errors)
    }

    fn runtime(&self, provider_id: &str) -> Result<&dyn PersistentProviderRuntime, String> {
        self.runtimes
            .iter()
            .find(|runtime| runtime.id() == provider_id)
            .map(|runtime| runtime.as_ref())
            .ok_or_else(|| format!("Provider {provider_id} 不支持持续会话"))
    }
}

fn finish_lifecycle_sweep(
    action: &str,
    handled: bool,
    errors: Vec<String>,
) -> Result<bool, String> {
    if errors.is_empty() {
        Ok(handled)
    } else {
        Err(format!("{action}：{}", errors.join("；")))
    }
}

#[cfg(test)]
#[path = "ai_runtime/tests.rs"]
mod tests;
