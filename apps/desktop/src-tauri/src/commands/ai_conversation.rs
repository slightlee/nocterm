//! AI 会话历史的读写命令。命令保持轻薄：校验、服务调用、DTO 转换。

use tauri::State;

use crate::{
    commands::ai_validation::{
        validate_conversation_id, validate_message_content, validate_message_id,
        validate_provider_id, validate_timestamp_ms,
    },
    dto::ai::{AiConversationDto, AiMessageDto},
    state::AppState,
};
use nocterm_domain::ai_conversation::{AiConversationRecord, AiMessageRecord, AiMessageRole};

/// 会话级命令策略白名单：与数据库 CHECK 约束和现有策略枚举保持一致。
const ALLOWED_COMMAND_POLICIES: [&str; 4] =
    ["deny_all", "confirm_each", "auto_safe", "full_access"];

fn validate_conversation(conversation: &AiConversationDto) -> Result<AiConversationRecord, String> {
    let id = validate_conversation_id(&conversation.id)?;
    let title = conversation.title.trim();
    if title.is_empty() || title.len() > 200 {
        return Err("AI 会话标题无效".to_string());
    }
    let provider = validate_provider_id(&conversation.provider)?;
    let command_policy = match conversation.command_policy.as_deref() {
        None | Some("") => None,
        Some(raw) if ALLOWED_COMMAND_POLICIES.contains(&raw) => Some(raw.to_string()),
        Some(_) => return Err("终端命令执行策略无效".to_string()),
    };
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
        command_policy,
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
        command_policy: record.command_policy.clone(),
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
            command_policy: Some("auto_safe".into()),
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
