use std::sync::Arc;

use nocterm_domain::ai_audit::{AiAuditRepository, AiToolAuditEvent};

use crate::error::AppError;

/// 审计服务把基础设施错误收敛为稳定错误，避免 SQL 诊断越过应用边界。
#[derive(Clone)]
pub struct AiAuditService {
    repository: Arc<dyn AiAuditRepository>,
}

impl AiAuditService {
    pub fn new(repository: Arc<dyn AiAuditRepository>) -> Self {
        Self { repository }
    }

    pub fn append(&self, event: &AiToolAuditEvent) -> Result<(), AppError> {
        self.repository.append(event).map_err(|_| {
            AppError::new(
                "AI_AUDIT_WRITE_FAILED",
                "AI 操作审计暂不可用，已阻止继续执行",
                true,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use nocterm_domain::ai_audit::{
        AiAuditApproval, AiAuditOutcome, AiAuditRepositoryError, AiAuditTarget, AiAuditTool,
    };

    use super::*;

    struct FailingRepository;

    impl AiAuditRepository for FailingRepository {
        fn append(&self, _event: &AiToolAuditEvent) -> Result<(), AiAuditRepositoryError> {
            Err(AiAuditRepositoryError::new("disk full: /private/path"))
        }
    }

    #[test]
    fn hides_infrastructure_details_behind_a_stable_error() {
        let service = AiAuditService::new(Arc::new(FailingRepository));
        let error = service
            .append(&AiToolAuditEvent {
                provider: "codex".into(),
                session_id: "ai-one".into(),
                target: AiAuditTarget::Ssh { connection_id: 7 },
                tool: AiAuditTool::GetSystemInfo,
                approval: AiAuditApproval::NotRequired,
                outcome: AiAuditOutcome::Pending,
                duration_ms: None,
                error_code: None,
            })
            .expect_err("audit must fail");

        assert_eq!(error.code, "AI_AUDIT_WRITE_FAILED");
        assert!(!error.message.contains("private/path"));
    }
}
