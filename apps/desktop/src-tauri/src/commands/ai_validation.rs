//! AI IPC 输入校验。
//! 该边界只接受有界、无歧义的会话标识、目标选择和文本参数。

use crate::{
    commands::ai_policy::parse_command_policy, dto::ai::AiSessionStartRequest,
    state::AiCommandPolicy,
};

const MAX_AI_PROMPT_BYTES: usize = 64 * 1024;
const MAX_WORKING_DIRECTORY_BYTES: usize = 4096;

pub(super) struct ValidatedAiSessionStart {
    pub session_id: String,
    pub conversation_id: String,
    pub working_directory: Option<String>,
    pub local_session_id: Option<String>,
    pub command_policy: AiCommandPolicy,
}

/// 在创建进程或绑定 Bridge token 前完成全部无副作用校验。
pub(super) fn validate_start_request(
    request: &AiSessionStartRequest,
) -> Result<ValidatedAiSessionStart, String> {
    let session_id = validate_client_session_id(&request.client_session_id)?;
    let conversation_id = validate_conversation_id(&request.conversation_id)?;
    validate_prompt(&request.prompt, "AI 请求")?;
    validate_prompt(&request.continuation_prompt, "AI 本轮请求")?;
    let working_directory = validate_working_directory(request.working_directory.as_deref())?;
    let command_policy = parse_command_policy(request.command_policy.as_deref())?;
    let local_session_id =
        validate_target_selectors(request.connection_id, request.target_session_id.as_deref())?;
    Ok(ValidatedAiSessionStart {
        session_id,
        conversation_id,
        working_directory,
        local_session_id,
        command_policy,
    })
}

fn validate_client_session_id(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if !(value.starts_with("ai-")
        && value.len() <= 64
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-'))
    {
        return Err("AI 会话标识格式无效".to_string());
    }
    Ok(value.to_string())
}

pub(super) fn validate_conversation_id(raw: &str) -> Result<String, String> {
    let value = raw.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return Err("AI 对话标识格式无效".to_string());
    }
    Ok(value.to_string())
}

fn validate_prompt(raw: &str, label: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err(format!("{label}不能为空"));
    }
    if raw.len() > MAX_AI_PROMPT_BYTES {
        return Err(format!("{label}不能超过 64 KiB"));
    }
    if raw.contains('\0') {
        return Err(format!("{label}包含不允许的空字符"));
    }
    Ok(())
}

fn validate_working_directory(raw: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() > MAX_WORKING_DIRECTORY_BYTES || value.contains('\0') {
        return Err("AI 工作目录无效或超过 4096 字节".to_string());
    }
    Ok(Some(value.to_string()))
}

/// 终端目标必须是 SSH、本地或无目标三者之一；歧义目标在 IPC 边界直接拒绝。
fn validate_target_selectors(
    connection_id: Option<i64>,
    target_session_id: Option<&str>,
) -> Result<Option<String>, String> {
    if connection_id.is_some_and(|value| value <= 0) {
        return Err("目标 SSH 连接标识无效".to_string());
    }
    let local_session_id = target_session_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if local_session_id.as_deref().is_some_and(|value| {
        !value.starts_with("local:")
            || value.len() > 128
            || !value.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, ':' | '-' | '_')
            })
    }) {
        return Err("目标本地终端会话标识无效".to_string());
    }
    if connection_id.is_some() && local_session_id.is_some() {
        return Err("AI 会话不能同时绑定本地终端和 SSH 连接".to_string());
    }
    if connection_id.is_none() && local_session_id.is_none() {
        return Err("请先连接本地终端或远程服务器，再使用 AI 助手".to_string());
    }
    Ok(local_session_id)
}

#[cfg(test)]
mod tests {
    use crate::{dto::ai::AiSessionStartRequest, state::AiCommandPolicy};

    use super::{
        validate_client_session_id, validate_conversation_id, validate_prompt,
        validate_start_request, validate_target_selectors, validate_working_directory,
    };

    #[test]
    fn accepts_only_bounded_client_session_ids() {
        assert!(validate_client_session_id("ai-123e4567-e89b-12d3-a456-426614174000").is_ok());
        assert!(validate_client_session_id("other-session").is_err());
        assert!(validate_client_session_id("ai-bad/session").is_err());
        assert!(validate_client_session_id(&format!("ai-{}", "x".repeat(70))).is_err());
    }

    #[test]
    fn accepts_only_bounded_conversation_ids() {
        assert!(validate_conversation_id("123e4567-e89b-12d3-a456-426614174000").is_ok());
        assert!(validate_conversation_id("").is_err());
        assert!(validate_conversation_id("bad/session").is_err());
        assert!(validate_conversation_id(&"x".repeat(65)).is_err());
    }

    #[test]
    fn terminal_target_selectors_are_unambiguous_and_positive() {
        assert!(validate_target_selectors(None, None).is_err());
        assert_eq!(
            validate_target_selectors(None, Some(" local:one ")).unwrap(),
            Some("local:one".to_string())
        );
        assert_eq!(validate_target_selectors(Some(7), None).unwrap(), None);
        assert!(validate_target_selectors(Some(0), None).is_err());
        assert!(validate_target_selectors(Some(-1), None).is_err());
        assert!(validate_target_selectors(Some(7), Some("local:one")).is_err());
        assert!(validate_target_selectors(None, Some("remote:one")).is_err());
        assert!(validate_target_selectors(None, Some("local:bad/path")).is_err());
        assert!(
            validate_target_selectors(None, Some(&format!("local:{}", "x".repeat(130)))).is_err()
        );
    }

    #[test]
    fn prompt_and_working_directory_inputs_are_bounded() {
        assert!(validate_prompt("inspect", "request").is_ok());
        assert!(validate_prompt("", "request").is_err());
        assert!(validate_prompt("bad\0prompt", "request").is_err());
        assert!(validate_prompt(&"x".repeat(64 * 1024 + 1), "request").is_err());
        assert_eq!(
            validate_working_directory(Some(" /tmp/project ")).unwrap(),
            Some("/tmp/project".into())
        );
        assert!(validate_working_directory(Some("bad\0path")).is_err());
    }

    #[test]
    fn start_request_validation_applies_the_fail_closed_policy_default() {
        let request = AiSessionStartRequest {
            client_session_id: "ai-one".into(),
            conversation_id: "conversation-one".into(),
            provider: "codex".into(),
            prompt: "inspect".into(),
            continuation_prompt: "inspect".into(),
            working_directory: None,
            connection_id: None,
            target_session_id: Some("local:one".into()),
            command_policy: None,
        };

        let validated = validate_start_request(&request).unwrap();

        assert_eq!(validated.session_id, "ai-one");
        assert_eq!(validated.conversation_id, "conversation-one");
        assert_eq!(validated.command_policy, AiCommandPolicy::AutoSafe);
    }
}
