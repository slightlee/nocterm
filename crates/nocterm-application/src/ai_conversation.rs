use std::sync::Arc;

use nocterm_domain::ai_conversation::{
    AiConversationRecord, AiConversationRepository, AiConversationRepositoryError,
};

use crate::error::AppError;

/// 会话历史服务把仓储错误收敛为稳定错误码，SQL 诊断不越过应用边界。
/// 历史属于展示数据：读失败可降级，写失败必须反馈给用户。
#[derive(Clone)]
pub struct AiConversationService {
    repository: Arc<dyn AiConversationRepository>,
}

impl AiConversationService {
    pub fn new(repository: Arc<dyn AiConversationRepository>) -> Self {
        Self { repository }
    }

    pub fn list(&self) -> Result<Vec<AiConversationRecord>, AppError> {
        self.repository.list().map_err(|error| {
            ai_conversation_error(error, "AI_CONVERSATION_LOAD_FAILED", "读取 AI 会话历史失败")
        })
    }

    pub fn save(&self, conversation: &AiConversationRecord) -> Result<(), AppError> {
        self.repository.upsert(conversation).map_err(|error| {
            ai_conversation_error(error, "AI_CONVERSATION_SAVE_FAILED", "保存 AI 会话历史失败")
        })
    }

    pub fn delete(&self, id: &str) -> Result<(), AppError> {
        self.repository.delete(id).map_err(|error| {
            ai_conversation_error(
                error,
                "AI_CONVERSATION_DELETE_FAILED",
                "删除 AI 会话历史失败",
            )
        })
    }
}

fn ai_conversation_error(
    error: AiConversationRepositoryError,
    code: &'static str,
    message: &'static str,
) -> AppError {
    // 基础设施细节只进日志语义的调试信息，面向 UI 的 message 保持稳定。
    let _ = error;
    AppError::new(code, message, true)
}

#[cfg(test)]
mod tests {
    use nocterm_domain::ai_conversation::{AiMessageRecord, AiMessageRole};

    use super::*;

    struct FailingRepository;

    impl AiConversationRepository for FailingRepository {
        fn list(&self) -> Result<Vec<AiConversationRecord>, AiConversationRepositoryError> {
            Err(AiConversationRepositoryError::new(
                "disk error: /private/var",
            ))
        }

        fn upsert(
            &self,
            _conversation: &AiConversationRecord,
        ) -> Result<(), AiConversationRepositoryError> {
            Err(AiConversationRepositoryError::new(
                "disk error: /private/var",
            ))
        }

        fn delete(&self, _id: &str) -> Result<(), AiConversationRepositoryError> {
            Err(AiConversationRepositoryError::new(
                "disk error: /private/var",
            ))
        }
    }

    fn conversation() -> AiConversationRecord {
        AiConversationRecord {
            id: "conv-one".into(),
            title: "排查磁盘".into(),
            provider: "codex".into(),
            command_policy: Some("auto_safe".into()),
            created_at_ms: 1_000,
            updated_at_ms: 2_000,
            messages: vec![AiMessageRecord {
                id: "msg-one".into(),
                role: AiMessageRole::User,
                content: "查看磁盘占用".into(),
                parts_json: None,
                created_at_ms: 1_500,
            }],
        }
    }

    #[test]
    fn load_failures_map_to_a_stable_error_code() {
        let service = AiConversationService::new(Arc::new(FailingRepository));
        let error = service.list().expect_err("list must fail");
        assert_eq!(error.code, "AI_CONVERSATION_LOAD_FAILED");
        assert!(!error.message.contains("/private/var"));
    }

    #[test]
    fn save_failures_map_to_a_stable_error_code() {
        let service = AiConversationService::new(Arc::new(FailingRepository));
        let error = service.save(&conversation()).expect_err("save must fail");
        assert_eq!(error.code, "AI_CONVERSATION_SAVE_FAILED");
    }

    #[test]
    fn delete_failures_map_to_a_stable_error_code() {
        let service = AiConversationService::new(Arc::new(FailingRepository));
        let error = service.delete("conv-one").expect_err("delete must fail");
        assert_eq!(error.code, "AI_CONVERSATION_DELETE_FAILED");
    }
}
