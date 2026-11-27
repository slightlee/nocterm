use std::{error::Error, fmt};

/// Provider 原生会话的复用身份。只保存恢复所需的控制面字段，禁止写入 Prompt、
/// 命令、输出、凭据或 Gateway token。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiNativeSessionIdentity {
    pub provider: String,
    pub model: Option<String>,
    pub reasoning_effort: Option<String>,
    pub fast_mode: Option<bool>,
    pub connection_id: Option<i64>,
    pub target_session_id: Option<String>,
}

impl AiNativeSessionIdentity {
    /// SSH 对话归属于连接，旧记录中的随机 Tab ID 不参与跨重启上下文恢复。
    /// 本地终端仍要求同一目标。实际命令执行必须另外绑定本轮终端和授权。
    pub fn compatible_with(&self, other: &Self) -> bool {
        self.provider == other.provider
            && self.model == other.model
            && self.reasoning_effort == other.reasoning_effort
            && self.fast_mode == other.fast_mode
            && self.connection_id == other.connection_id
            && (self.connection_id.is_some() || self.target_session_id == other.target_session_id)
    }
}

/// 一个 Nocterm 对话最多绑定一个当前有效的 Provider 原生会话。身份变化时由应用层
/// 创建新会话并整体替换本记录，旧 ID 不再参与恢复。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiNativeSessionRecord {
    pub conversation_id: String,
    pub native_session_id: String,
    pub identity: AiNativeSessionIdentity,
    pub updated_at_ms: i64,
}

/// 原生会话绑定属于后端权威状态，前端快照不得读取或覆盖该 Port。
pub trait AiNativeSessionRepository: Send + Sync {
    fn get(
        &self,
        conversation_id: &str,
    ) -> Result<Option<AiNativeSessionRecord>, AiNativeSessionRepositoryError>;
    fn upsert(&self, record: &AiNativeSessionRecord) -> Result<(), AiNativeSessionRepositoryError>;
    fn delete(&self, conversation_id: &str) -> Result<(), AiNativeSessionRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiNativeSessionRepositoryError {
    message: String,
}

impl AiNativeSessionRepositoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AiNativeSessionRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiNativeSessionRepositoryError {}
