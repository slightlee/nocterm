use std::{error::Error, fmt};

/// 消息角色使用封闭枚举，数据库层以 CHECK 约束兜底，拒绝任意第三方写入的角色值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiMessageRole {
    User,
    Assistant,
}

impl AiMessageRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, AiConversationRepositoryError> {
        match raw {
            "user" => Ok(Self::User),
            "assistant" => Ok(Self::Assistant),
            _ => Err(AiConversationRepositoryError::new(format!(
                "unknown ai message role: {raw}"
            ))),
        }
    }
}

/// 单条已定稿的对话消息。parts 以调用方序列化好的 JSON 存储，
/// 本层不解析其内部结构，展示契约仍由前端维护。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiMessageRecord {
    pub id: String,
    pub role: AiMessageRole,
    pub content: String,
    pub parts_json: Option<String>,
    pub created_at_ms: i64,
}

/// 一次完整的会话快照：upsert 语义以本结构整体覆盖旧记录，
/// 消息列表随会话一起重建，保证前端状态与数据库内容一致。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiConversationRecord {
    pub id: String,
    pub title: String,
    pub provider: String,
    /// 会话可能从未设置命令策略；空值表示沿用后端默认策略。
    pub command_policy: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub messages: Vec<AiMessageRecord>,
}

/// 会话历史属于用户可再生的展示数据：读失败允许前端退化为空历史，
/// 写失败必须让调用方感知，因此 Port 不提供吞错的默认实现。
pub trait AiConversationRepository: Send + Sync {
    fn list(&self) -> Result<Vec<AiConversationRecord>, AiConversationRepositoryError>;
    fn upsert(
        &self,
        conversation: &AiConversationRecord,
    ) -> Result<(), AiConversationRepositoryError>;
    fn delete(&self, id: &str) -> Result<(), AiConversationRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiConversationRepositoryError {
    message: String,
}

impl AiConversationRepositoryError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for AiConversationRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for AiConversationRepositoryError {}

#[cfg(test)]
mod tests {
    use super::{AiConversationRepositoryError, AiMessageRole};

    #[test]
    fn message_roles_round_trip_through_the_storage_form() {
        assert_eq!(AiMessageRole::User.as_str(), "user");
        assert_eq!(AiMessageRole::Assistant.as_str(), "assistant");
        assert_eq!(AiMessageRole::parse("user"), Ok(AiMessageRole::User));
        assert_eq!(
            AiMessageRole::parse("assistant"),
            Ok(AiMessageRole::Assistant)
        );
        assert!(AiMessageRole::parse("system").is_err());
    }

    #[test]
    fn repository_errors_render_their_message() {
        let error = AiConversationRepositoryError::new("disk full");
        assert_eq!(error.to_string(), "disk full");
    }
}
