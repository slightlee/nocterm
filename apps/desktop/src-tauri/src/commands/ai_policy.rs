//! AI 终端命令的权限决策与输入输出边界。
//! 结构化只读工具与严格受限的简单命令共用单调递增的会话级权限模型。

use std::time::Duration;

use crate::state::AiCommandPolicy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiOperationClass {
    ReadOnly,
    Unrestricted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AiExecutionDecision {
    Deny,
    Confirm,
    Execute,
}

pub(crate) const AI_OUTPUT_LIMIT_BYTES: usize = 128 * 1024;
pub(crate) const AI_APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const AI_EXECUTION_TIMEOUT: Duration = Duration::from_secs(60);
/// 一次工具调用包含审批和执行两个阶段；Bridge 必须比总预算多留协议收尾余量。
pub(crate) const AI_TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const AI_BRIDGE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(135);

/// 将 UI 传入的策略解析为受限枚举；缺省采用智能安全模式，未知值直接失败。
pub(crate) fn parse_command_policy(raw: Option<&str>) -> Result<AiCommandPolicy, String> {
    match raw.unwrap_or("auto_safe") {
        "deny_all" => Ok(AiCommandPolicy::DenyAll),
        "auto_safe" => Ok(AiCommandPolicy::AutoSafe),
        "confirm_each" => Ok(AiCommandPolicy::ConfirmEach),
        "full_access" => Ok(AiCommandPolicy::FullAccess),
        _ => Err("终端命令执行策略无效".to_string()),
    }
}

/// 权限级别保持单调递增：仅分析 < 每次确认 < 变更前确认 < 完全访问。
pub(crate) fn execution_decision(
    policy: AiCommandPolicy,
    operation: AiOperationClass,
) -> AiExecutionDecision {
    match policy {
        AiCommandPolicy::DenyAll => AiExecutionDecision::Deny,
        AiCommandPolicy::ConfirmEach => AiExecutionDecision::Confirm,
        AiCommandPolicy::AutoSafe => match operation {
            AiOperationClass::ReadOnly => AiExecutionDecision::Execute,
            AiOperationClass::Unrestricted => AiExecutionDecision::Confirm,
        },
        AiCommandPolicy::FullAccess => AiExecutionDecision::Execute,
    }
}

/// 判断命令是否属于可自动执行的最小只读集合。
/// 这里故意不解析 Shell 语法，而是拒绝所有组合符号和参数，确保策略 fail-closed。
pub(crate) fn is_safe_readonly_command(raw: &str) -> bool {
    let command = raw.trim();
    if command.is_empty()
        || command.contains(['\0', '\n', '\r', '\\'])
        || command
            .chars()
            .any(|ch| matches!(ch, ';' | '|' | '&' | '>' | '<' | '$' | '`'))
    {
        return false;
    }
    let mut parts = command.split_whitespace();
    let Some(name) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    matches!(
        name,
        "pwd" | "ls" | "id" | "whoami" | "uname" | "hostname" | "date" | "uptime"
    )
}

pub(crate) fn validate_approved_command(raw: &str) -> Result<String, String> {
    let command = raw.trim();
    if command.is_empty() || command.len() > 4096 {
        return Err("终端命令不能为空且不能超过 4096 个字符".to_string());
    }
    if command.contains('\0') {
        return Err("终端命令包含不允许的空字符".to_string());
    }
    Ok(command.to_string())
}

/// 按字节限制输出但始终停在 UTF-8 字符边界，避免 `String::truncate` 在多字节字符中间 panic。
pub(crate) fn truncate_utf8(value: &mut String, max_bytes: usize) -> bool {
    if value.len() <= max_bytes {
        return false;
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    true
}

#[cfg(test)]
mod tests {
    use crate::state::AiCommandPolicy;

    use super::{
        AiExecutionDecision, AiOperationClass, execution_decision, is_safe_readonly_command,
        parse_command_policy, truncate_utf8, validate_approved_command,
    };

    #[test]
    fn output_limit_preserves_utf8_boundaries() {
        let mut output = "你好世界".to_string();
        assert!(truncate_utf8(&mut output, 4));
        assert_eq!(output, "你");
        assert!(output.is_char_boundary(output.len()));
    }

    #[test]
    fn approved_commands_only_enforce_transport_boundaries() {
        assert_eq!(validate_approved_command("  id  ").unwrap(), "id");
        assert!(validate_approved_command("").is_err());
        assert!(validate_approved_command("echo \0 secret").is_err());
        assert!(validate_approved_command(&"x".repeat(4097)).is_err());
    }

    #[test]
    fn parses_command_policy_fail_closed() {
        assert_eq!(
            parse_command_policy(None).unwrap(),
            AiCommandPolicy::AutoSafe
        );
        assert_eq!(
            parse_command_policy(Some("confirm_each")).unwrap(),
            AiCommandPolicy::ConfirmEach
        );
        assert_eq!(
            parse_command_policy(Some("deny_all")).unwrap(),
            AiCommandPolicy::DenyAll
        );
        assert_eq!(
            parse_command_policy(Some("full_access")).unwrap(),
            AiCommandPolicy::FullAccess
        );
        assert!(parse_command_policy(Some("always_allow")).is_err());
    }

    #[test]
    fn only_allows_single_safe_readonly_commands() {
        assert!(is_safe_readonly_command(" pwd "));
        assert!(is_safe_readonly_command("hostname"));
        // 即使每个子命令单独安全，Shell 组合语法仍必须进入审批路径。
        assert!(!is_safe_readonly_command("hostname; pwd"));
        assert!(!is_safe_readonly_command("pwd; rm -rf /"));
        assert!(!is_safe_readonly_command("ls | cat"));
        assert!(!is_safe_readonly_command("uname -n"));
        assert!(!is_safe_readonly_command("docker ps"));
    }

    #[test]
    fn auto_safe_confirms_compound_readonly_commands() {
        let operation = if is_safe_readonly_command("hostname; pwd") {
            AiOperationClass::ReadOnly
        } else {
            AiOperationClass::Unrestricted
        };

        assert_eq!(
            execution_decision(AiCommandPolicy::AutoSafe, operation),
            AiExecutionDecision::Confirm
        );
    }

    #[test]
    fn permission_levels_increase_monotonically() {
        let policies = [
            AiCommandPolicy::DenyAll,
            AiCommandPolicy::ConfirmEach,
            AiCommandPolicy::AutoSafe,
            AiCommandPolicy::FullAccess,
        ];
        let readonly =
            policies.map(|policy| execution_decision(policy, AiOperationClass::ReadOnly));
        let unrestricted =
            policies.map(|policy| execution_decision(policy, AiOperationClass::Unrestricted));

        assert_eq!(
            readonly,
            [
                AiExecutionDecision::Deny,
                AiExecutionDecision::Confirm,
                AiExecutionDecision::Execute,
                AiExecutionDecision::Execute,
            ]
        );
        assert_eq!(
            unrestricted,
            [
                AiExecutionDecision::Deny,
                AiExecutionDecision::Confirm,
                AiExecutionDecision::Confirm,
                AiExecutionDecision::Execute,
            ]
        );
    }
}
