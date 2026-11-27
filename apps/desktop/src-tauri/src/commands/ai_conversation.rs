//! AI 会话历史的读写命令。命令保持轻薄：校验、服务调用、DTO 转换。

use tauri::State;

use crate::{
    commands::ai_validation::{
        valid_model_id, valid_reasoning_effort, validate_conversation_id, validate_message_content,
        validate_message_id, validate_provider_id, validate_timestamp_ms,
    },
    dto::ai::{AiConversationDto, AiMessageDto},
    state::AppState,
};
use nocterm_domain::ai_conversation::{
    AiConversationRecord, AiConversationTargetRecord, AiMessageRecord, AiMessageRole,
};

/// 会话级命令策略白名单：与数据库 CHECK 约束和现有策略枚举保持一致。
const ALLOWED_COMMAND_POLICIES: [&str; 4] =
    ["deny_all", "confirm_each", "auto_safe", "full_access"];

fn validate_target(
    conversation: &AiConversationDto,
) -> Result<Option<AiConversationTargetRecord>, String> {
    match (
        conversation.target_kind.as_deref(),
        conversation.target_id.as_deref(),
        conversation.target_label.as_deref(),
    ) {
        (None, None, None) => Ok(None),
        (Some(kind), Some(id), Some(label)) => {
            if !matches!(kind, "local" | "ssh") {
                return Err("AI 会话目标类型无效".to_string());
            }
            let id = id.trim();
            let label = label.trim();
            if id.is_empty() || id.len() > 128 || id.contains('\0') {
                return Err("AI 会话目标标识无效".to_string());
            }
            if label.is_empty() || label.len() > 200 || label.contains('\0') {
                return Err("AI 会话目标名称无效".to_string());
            }
            Ok(Some(AiConversationTargetRecord {
                kind: kind.to_string(),
                id: id.to_string(),
                label: label.to_string(),
            }))
        }
        _ => Err("AI 会话目标信息不完整".to_string()),
    }
}

fn validate_conversation(conversation: &AiConversationDto) -> Result<AiConversationRecord, String> {
    let id = validate_conversation_id(&conversation.id)?;
    let title = conversation.title.trim();
    if title.is_empty() || title.len() > 200 {
        return Err("AI 会话标题无效".to_string());
    }
    let provider = validate_provider_id(&conversation.provider)?;
    // 历史选项沿用发送边界；速度偏好可保留在暂不支持速度的模型上，由能力快照决定是否发送。
    if conversation
        .model
        .as_deref()
        .is_some_and(|model| !valid_model_id(model))
    {
        return Err("AI_MODEL_ID_INVALID".into());
    }
    if let Some(effort) = &conversation.reasoning_effort {
        if !valid_reasoning_effort(&provider, effort) {
            return Err("AI_REASONING_EFFORT_INVALID".into());
        }
        if conversation.model.is_none() {
            return Err("AI_REASONING_MODEL_REQUIRED".into());
        }
    }
    if conversation.model.is_none()
        && (conversation.fast_mode.is_some() || conversation.supports_fast_mode.is_some())
    {
        return Err("AI_SPEED_MODEL_REQUIRED".into());
    }
    let command_policy = match conversation.command_policy.as_deref() {
        None | Some("") => None,
        Some(raw) if ALLOWED_COMMAND_POLICIES.contains(&raw) => Some(raw.to_string()),
        Some(_) => return Err("终端命令执行策略无效".to_string()),
    };
    let target = validate_target(conversation)?;
    validate_timestamp_ms(conversation.created_at_ms, "会话创建时间")?;
    validate_timestamp_ms(conversation.updated_at_ms, "会话更新时间")?;
    if conversation.messages.len() > 1_000 {
        return Err("AI 会话消息数量超出上限".to_string());
    }

    let messages = conversation
        .messages
        .iter()
        .map(|message| {
            let message_id = validate_message_id(&message.id)?;
            let role =
                AiMessageRole::parse(&message.role).map_err(|_| "AI 消息角色无效".to_string())?;
            validate_message_content(&message.content)?;
            if let Some(parts_json) = message.parts_json.as_deref() {
                // parts 只作有界 JSON 存放，不在后端解析其展示结构。
                if parts_json.len() > 256 * 1024 {
                    return Err("AI 消息活动记录超出 256 KiB 上限".to_string());
                }
                if parts_json.contains('\0') {
                    return Err("AI 消息活动记录包含不允许的空字符".to_string());
                }
            }
            validate_timestamp_ms(message.created_at_ms, "消息创建时间")?;
            Ok(AiMessageRecord {
                id: message_id,
                role,
                content: message.content.clone(),
                parts_json: message.parts_json.clone(),
                created_at_ms: message.created_at_ms,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(AiConversationRecord {
        id,
        title: title.to_string(),
        provider,
        model: conversation.model.clone(),
        reasoning_effort: conversation.reasoning_effort.clone(),
        fast_mode: conversation.fast_mode,
        supports_fast_mode: conversation.supports_fast_mode,
        command_policy,
        target,
        created_at_ms: conversation.created_at_ms,
        updated_at_ms: conversation.updated_at_ms,
        messages,
    })
}

fn conversation_to_dto(record: &AiConversationRecord) -> AiConversationDto {
    AiConversationDto {
        id: record.id.clone(),
        title: record.title.clone(),
        provider: record.provider.clone(),
        model: record.model.clone(),
        reasoning_effort: record.reasoning_effort.clone(),
        fast_mode: record.fast_mode,
        supports_fast_mode: record.supports_fast_mode,
        command_policy: record.command_policy.clone(),
        target_kind: record.target.as_ref().map(|target| target.kind.clone()),
        target_id: record.target.as_ref().map(|target| target.id.clone()),
        target_label: record.target.as_ref().map(|target| target.label.clone()),
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
        messages: record
            .messages
            .iter()
            .map(|message| AiMessageDto {
                id: message.id.clone(),
                role: message.role.as_str().to_string(),
                content: message.content.clone(),
                parts_json: message.parts_json.clone(),
                created_at_ms: message.created_at_ms,
            })
            .collect(),
    }
}

#[tauri::command(async)]
pub fn ai_conversation_list(state: State<'_, AppState>) -> Result<Vec<AiConversationDto>, String> {
    let records = state
        .ai_conversation_service()
        .list()
        .map_err(|error| error.message)?;
    Ok(records.iter().map(conversation_to_dto).collect())
}

#[tauri::command(async)]
pub fn ai_conversation_save(
    state: State<'_, AppState>,
    conversation: AiConversationDto,
) -> Result<(), String> {
    let validated = validate_conversation(&conversation)?;
    state
        .ai_conversation_service()
        .save(&validated)
        .map_err(|error| error.message)
}

#[tauri::command(async)]
pub fn ai_conversation_delete(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<(), String> {
    let conversation_id = validate_conversation_id(&conversation_id)?;
    // 关闭占用覆盖进程回收和删除事务，既拒绝旧响应回写，也拒绝清理途中的新启动。
    let _closing = state
        .ai_native_session_service()
        .begin_close(&conversation_id)
        .map_err(|error| error.message)?;
    super::ai::close_headless_conversation(&state, &conversation_id)?;
    // 清理失败保留数据库记录供重试；Repository 在删除对话的事务中清理原生绑定。
    state
        .ai_provider_runtimes()
        .reset_conversation(&conversation_id)?;
    state
        .ai_conversation_service()
        .delete(&conversation_id)
        .map_err(|error| error.message)
}

#[cfg(test)]
mod tests {
    use crate::dto::ai::{AiConversationDto, AiMessageDto};

    use super::validate_conversation;

    fn conversation() -> AiConversationDto {
        AiConversationDto {
            id: "123e4567-e89b-12d3-a456-426614174000".into(),
            title: "排查磁盘".into(),
            provider: "codex".into(),
            model: Some("test-model".into()),
            reasoning_effort: Some("high".into()),
            fast_mode: Some(true),
            supports_fast_mode: Some(true),
            command_policy: Some("auto_safe".into()),
            target_kind: Some("ssh".into()),
            target_id: Some("7".into()),
            target_label: Some("Production".into()),
            created_at_ms: 1_000,
            updated_at_ms: 2_000,
            messages: vec![AiMessageDto {
                id: "msg-one".into(),
                role: "user".into(),
                content: "查看磁盘占用".into(),
                parts_json: None,
                created_at_ms: 1_500,
            }],
        }
    }

    #[test]
    fn persisted_options_restore_the_native_identity_after_reopening_database() {
        use nocterm_application::{
            ai_conversation::AiConversationService, ai_native_session::AiNativeSessionService,
        };
        use nocterm_domain::ai_native_session::{AiNativeSessionIdentity, AiNativeSessionRecord};
        use nocterm_infrastructure::persistence::SqliteConnectionRepository;
        use std::sync::Arc;

        let path = std::env::temp_dir().join(format!(
            "nocterm-options-{}-{}.db",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let dto = conversation();
        let expected = validate_conversation(&dto).unwrap();
        let identity = AiNativeSessionIdentity {
            provider: dto.provider.clone(),
            model: dto.model.clone(),
            reasoning_effort: dto.reasoning_effort.clone(),
            fast_mode: dto.fast_mode,
            connection_id: Some(7),
            target_session_id: None,
        };
        {
            let repository = Arc::new(SqliteConnectionRepository::open(&path).unwrap());
            AiConversationService::new(repository.clone())
                .save(&expected)
                .unwrap();
            let native = AiNativeSessionService::new(repository);
            let session = native.prepare_session(&dto.id, &identity).unwrap();
            native
                .save(
                    &session.lease,
                    &AiNativeSessionRecord {
                        conversation_id: dto.id.clone(),
                        native_session_id: "existing-thread".into(),
                        identity: identity.clone(),
                        updated_at_ms: 2,
                    },
                )
                .unwrap();
        }
        {
            let repository = Arc::new(SqliteConnectionRepository::open(&path).unwrap());
            let records = AiConversationService::new(repository.clone())
                .list()
                .unwrap();
            let restored = super::conversation_to_dto(&records[0]);
            assert_eq!(validate_conversation(&restored).unwrap(), expected);
            let restored_identity = AiNativeSessionIdentity {
                provider: restored.provider,
                model: restored.model,
                reasoning_effort: restored.reasoning_effort,
                fast_mode: if restored.supports_fast_mode == Some(true) {
                    restored.fast_mode
                } else {
                    None
                },
                connection_id: Some(7),
                target_session_id: None,
            };
            let native = AiNativeSessionService::new(repository);
            assert_eq!(
                native
                    .prepare_session(&dto.id, &restored_identity)
                    .unwrap()
                    .native_session_id
                    .as_deref(),
                Some("existing-thread")
            );
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn accepts_legacy_missing_options_and_rejects_invalid_model_choices() {
        let mut legacy = serde_json::to_value(conversation()).unwrap();
        for key in ["model", "reasoningEffort", "fastMode", "supportsFastMode"] {
            legacy.as_object_mut().unwrap().remove(key);
        }
        let legacy: AiConversationDto = serde_json::from_value(legacy).unwrap();
        assert!(validate_conversation(&legacy).unwrap().model.is_none());
        let mut invalid = conversation();
        invalid.model = Some("--untrusted-option".into());
        assert_eq!(
            validate_conversation(&invalid).unwrap_err(),
            "AI_MODEL_ID_INVALID"
        );
        invalid = conversation();
        invalid.reasoning_effort = Some("invented".into());
        assert_eq!(
            validate_conversation(&invalid).unwrap_err(),
            "AI_REASONING_EFFORT_INVALID"
        );
    }

    #[test]
    fn accepts_a_well_formed_conversation_snapshot() {
        assert!(validate_conversation(&conversation()).is_ok());
    }

    #[test]
    fn rejects_unknown_providers_roles_and_bad_timestamps() {
        let mut invalid = conversation();
        invalid.provider = "unknown-provider".into();
        assert!(validate_conversation(&invalid).is_err());

        let mut invalid = conversation();
        invalid.messages[0].role = "system".into();
        assert!(validate_conversation(&invalid).is_err());

        let mut invalid = conversation();
        invalid.updated_at_ms = 0;
        assert!(validate_conversation(&invalid).is_err());

        let mut invalid = conversation();
        invalid.command_policy = Some("full_access_everything".into());
        assert!(validate_conversation(&invalid).is_err());

        let mut invalid = conversation();
        invalid.target_id = None;
        assert!(validate_conversation(&invalid).is_err());

        let mut invalid = conversation();
        invalid.target_kind = Some("container".into());
        assert!(validate_conversation(&invalid).is_err());
    }

    #[test]
    fn accepts_legacy_conversations_without_a_target() {
        let mut legacy = conversation();
        legacy.target_kind = None;
        legacy.target_id = None;
        legacy.target_label = None;

        assert!(validate_conversation(&legacy).is_ok());
    }

    #[test]
    fn rejects_oversized_content_and_unbounded_message_counts() {
        let mut invalid = conversation();
        invalid.messages[0].content = "x".repeat(256 * 1024 + 1);
        assert!(validate_conversation(&invalid).is_err());

        let mut invalid = conversation();
        invalid.messages = Vec::new();
        // 空消息列表是合法的：新建会话尚未产生任何对话。
        assert!(validate_conversation(&invalid).is_ok());
    }
}
