use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionStartRequest {
    /// 前端在调用 IPC 前生成，使监听器能在任何输出或退出事件到达前绑定本轮任务。
    pub client_session_id: String,
    /// UI 对话 ID 用于复用 Codex app-server；它不作为终端授权身份。
    pub conversation_id: String,
    pub provider: String,
    /// 首次创建 Provider 会话时包含界面已有历史，用于应用重启后的上下文恢复。
    pub prompt: String,
    /// 已复用 Provider 会话只发送当前问题，避免重复注入历史消息。
    pub continuation_prompt: String,
    pub working_directory: Option<String>,
    /// AI 面板位于 SSH 连接上下文中；后端据此校验目标，不信任 Prompt 中的主机名。
    pub connection_id: Option<i64>,
    /// 本地终端上下文使用 UI 会话标识绑定当前 PTY；远程任务仍使用 connection_id。
    pub target_session_id: Option<String>,
    /// 会话级终端权限：仅分析、每次确认、变更前确认或完全访问。
    pub command_policy: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderStatus {
    pub id: String,
    pub command: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionStartResponse {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiOutputEvent {
    pub session_id: String,
    pub connection_id: Option<i64>,
    pub stream: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiExitEvent {
    pub session_id: String,
    pub connection_id: Option<i64>,
    pub code: Option<i32>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiToolApprovalEvent {
    pub approval_id: String,
    pub session_id: String,
    pub target_kind: String,
    pub target_label: String,
    pub command: String,
    /// 前端以此清理因休眠、丢事件或后端超时而残留的确认卡片。
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiToolApprovalClosedEvent {
    pub approval_id: String,
    pub session_id: String,
    pub resolution: String,
}
