//! Provider 无关的终端会话行为契约。
//! 厂商适配层只负责选择协议字段，不能各自维护一套回答和目标语义。

use super::ProviderSessionIdentity;

const TERMINAL_SESSION_INSTRUCTIONS: &str = concat!(
    "You are Nocterm's terminal assistant. The provider process is not the terminal target: its ",
    "host, working directory, environment, ",
    "and platform metadata never describe the terminal selected in Nocterm. For any request that ",
    "depends on current terminal state, call the appropriate Nocterm tool before answering and ",
    "treat its structured result as authoritative. Do not merely suggest a command or describe ",
    "how the user could obtain the result. Use only the Nocterm tools provided by this session; ",
    "if a required call fails, report that failure instead of inferring an answer. Use the smallest ",
    "sufficient set of calls and the narrowest available structured tool. Before calling tools, ",
    "identify the exact facts or outcome requested in the current turn. If one tool call can obtain ",
    "all of them without broadening the requested scope or permission requirement, do not split the ",
    "work across overlapping calls. Never combine commands merely to reduce the call count when that ",
    "would require broader approval. session_context describes the ",
    "Nocterm connection binding; use it only when the user asks which target or connection is active. ",
    "Its name and host fields are connection metadata, not the operating system hostname. Do not add exploratory ",
    "commands, directory listings, or broader inspections that are not required by the request; ",
    "once the requested facts are available, stop calling tools. Treat the requested scope literally: ",
    "querying a path or working directory does not authorize listing its contents. Clearly separate ",
    "observed facts from inference; never invent provenance, history, intent, or resource purpose. ",
    "Do not recommend changes, cleanup, or deletion unless the user requested remediation and the ",
    "evidence supports it. For a simple fact lookup, return only one concise label-value entry per ",
    "requested fact, without a preamble, conclusion, connection metadata, or inferred explanation. ",
    "For diagnosis or remediation, distinguish observed evidence from analysis. Answer directly and ",
    "concisely in the user's language, include only requested facts or information necessary to ",
    "explain a failure, and do not expose internal tool or protocol names."
);

/// 持久 Provider 的私有配置复用同一契约，避免厂商适配器复制行为规则。
pub(crate) fn terminal_task_instructions() -> &'static str {
    TERMINAL_SESSION_INSTRUCTIONS
}

/// 根据宿主绑定的目标生成同一份会话规则；未绑定终端时不干预普通对话。
pub(crate) fn terminal_session_instructions(
    identity: &ProviderSessionIdentity,
) -> Option<&'static str> {
    if identity.connection_id.is_none() && identity.target_session_id.is_none() {
        return None;
    }
    Some(terminal_task_instructions())
}

#[cfg(test)]
mod tests {
    use super::{terminal_session_instructions, terminal_task_instructions};
    use crate::commands::ai_provider::ProviderSessionIdentity;

    #[test]
    fn contract_is_identical_for_every_terminal_target() {
        let ssh = terminal_session_instructions(&ProviderSessionIdentity {
            connection_id: Some(7),
            target_session_id: None,
            working_directory: None,
        })
        .unwrap();
        let local = terminal_session_instructions(&ProviderSessionIdentity {
            connection_id: None,
            target_session_id: Some("local-one".into()),
            working_directory: None,
        })
        .unwrap();

        assert!(ssh.contains("directly and concisely"));
        assert!(ssh.contains("call the appropriate Nocterm tool before answering"));
        assert!(ssh.contains("provider process is not the terminal target"));
        assert!(ssh.contains("Do not merely suggest a command"));
        assert!(ssh.contains("smallest sufficient set of calls"));
        assert!(ssh.contains("without broadening the requested scope or permission requirement"));
        assert!(ssh.contains("Never combine commands merely to reduce the call count"));
        assert!(
            ssh.contains("use it only when the user asks which target or connection is active")
        );
        assert!(ssh.contains("not the operating system hostname"));
        assert!(ssh.contains("once the requested facts are available, stop calling tools"));
        assert!(ssh.contains("does not authorize listing its contents"));
        assert!(ssh.contains("never invent provenance, history, intent, or resource purpose"));
        assert!(ssh.contains("only one concise label-value entry per"));
        assert!(ssh.contains("distinguish observed evidence from analysis"));
        assert_eq!(ssh, terminal_task_instructions());
        assert_eq!(ssh, local);
    }

    #[test]
    fn contract_does_not_affect_sessions_without_a_terminal_target() {
        assert!(
            terminal_session_instructions(&ProviderSessionIdentity {
                connection_id: None,
                target_session_id: None,
                working_directory: None,
            })
            .is_none()
        );
    }
}
