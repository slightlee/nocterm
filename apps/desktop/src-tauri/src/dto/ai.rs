use nocterm_domain::ai_execution::{AiExecutionRecord, AiExecutionTarget};
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

/// 执行卡只接收脱敏控制面事实；完整命令、参数和输出不属于该 DTO。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AiToolExecutionEvent {
    pub execution_id: String,
    pub sequence: u64,
    pub provider_session_id: String,
    pub conversation_id: String,
    pub target_kind: String,
    pub workspace_session_id: Option<String>,
    pub terminal_id: Option<String>,
    pub terminal_generation: Option<u64>,
    pub connection_id: Option<i64>,
    pub tool: String,
    pub phase: String,
    pub approval_state: String,
    pub execution_mode: String,
    pub fallback_reason: Option<String>,
    pub result_state: String,
    pub dispatch_receipt: String,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub error_code: Option<String>,
    pub updated_at_ms: i64,
}

impl From<&AiExecutionRecord> for AiToolExecutionEvent {
    fn from(record: &AiExecutionRecord) -> Self {
        let (workspace_session_id, terminal_id, terminal_generation, connection_id) =
            match &record.target {
                AiExecutionTarget::Local {
                    workspace_session_id,
                    terminal_id,
                } => (
                    Some(workspace_session_id.clone()),
                    Some(terminal_id.clone()),
                    None,
                    None,
                ),
                AiExecutionTarget::Ssh {
                    workspace_session_id,
                    terminal_id,
                    terminal_generation,
                    connection_id,
                } => (
                    Some(workspace_session_id.clone()),
                    Some(terminal_id.clone()),
                    Some(*terminal_generation),
                    Some(*connection_id),
                ),
            };
        Self {
            execution_id: record.execution_id.clone(),
            sequence: record.sequence,
            provider_session_id: record.request_key.provider_session_id.clone(),
            conversation_id: record.conversation_id.clone(),
            target_kind: record.target.kind().to_string(),
            workspace_session_id,
            terminal_id,
            terminal_generation,
            connection_id,
            tool: record.request_key.tool.as_str().to_string(),
            phase: record.phase.as_str().to_string(),
            approval_state: record.approval.as_str().to_string(),
            execution_mode: record.mode.as_str().to_string(),
            fallback_reason: record.fallback_reason.clone(),
            result_state: record.result.as_str().to_string(),
            dispatch_receipt: record.dispatch_receipt.as_str().to_string(),
            exit_code: record.exit_code,
            duration_ms: record.duration_ms,
            error_code: record.error_code.clone(),
            updated_at_ms: record.updated_at_ms,
        }
    }
}

/// 持久化层的单条消息快照；parts 由前端序列化为 JSON 字符串传递。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessageDto {
    pub id: String,
    pub role: String,
    pub content: String,
    pub parts_json: Option<String>,
    pub created_at_ms: i64,
}

/// 一次会话快照的 IPC 契约；保存请求与查询响应共用同一结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiConversationDto {
    pub id: String,
    pub title: String,
    pub provider: String,
    pub command_policy: Option<String>,
    pub target_kind: Option<String>,
    pub target_id: Option<String>,
    pub target_label: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub messages: Vec<AiMessageDto>,
}
