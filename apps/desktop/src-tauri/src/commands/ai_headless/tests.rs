use std::sync::{Mutex, atomic::AtomicBool};

use super::output::{
    AiOutputEmitter, HeadlessOutputBudget, HeadlessOutputContext, OutputReservation, emit_lines,
    validate_readiness_event,
};
use crate::commands::ai_provider::HeadlessReadinessRequirement;
use crate::commands::ai_stream::{MAX_PROVIDER_TURN_OUTPUT_BYTES, redact_token};
use crate::dto::ai::AiOutputEvent;
use crate::state::{AiCommandPolicy, AiGatewayBinding, AiGatewayState, AiTarget};

#[derive(Default)]
struct TestOutputEmitter {
    events: Mutex<Vec<AiOutputEvent>>,
}

impl AiOutputEmitter for TestOutputEmitter {
    fn emit_output(&self, event: AiOutputEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[test]
fn bridge_tokens_never_leave_headless_output() {
    assert_eq!(
        redact_token("provider token=secret-123".into(), Some("secret-123")),
        "provider token=[REDACTED]"
    );
    assert_eq!(redact_token("plain".into(), None), "plain");
}

#[test]
fn stdout_and_stderr_share_one_headless_output_budget() {
    let budget = HeadlessOutputBudget::default();
    assert!(matches!(
        budget.reserve(MAX_PROVIDER_TURN_OUTPUT_BYTES - 1),
        OutputReservation::Accepted
    ));
    assert!(matches!(budget.reserve(1), OutputReservation::Accepted));
    assert!(matches!(
        budget.reserve(1),
        OutputReservation::FirstRejection
    ));
    assert!(matches!(budget.reserve(1), OutputReservation::Rejected));
}

#[test]
fn readiness_requires_the_expected_provider_tool_prefix() {
    let requirement = HeadlessReadinessRequirement {
        event_type: "system",
        event_subtype: "init",
        tool_prefix: "mcp__nocterm__",
        failure_message: "missing nocterm tools",
        require_gateway_call: true,
        missing_gateway_call_message: "missing nocterm call",
    };

    assert_eq!(
        validate_readiness_event(
            r#"{"type":"system","subtype":"init","tools":["mcp__nocterm__session_context"]}"#,
            &requirement,
        ),
        Some(Ok(()))
    );
    assert_eq!(
        validate_readiness_event(
            r#"{"type":"system","subtype":"init","tools":["Bash"]}"#,
            &requirement,
        ),
        Some(Err("missing nocterm tools".into()))
    );
    assert_eq!(
        validate_readiness_event(r#"{"type":"assistant"}"#, &requirement),
        None
    );
}

#[test]
fn terminal_bound_output_is_not_emitted_without_a_real_gateway_call() {
    let emitter = TestOutputEmitter::default();
    let gateway = AiGatewayState::default();
    let token = "test-bridge-token";
    gateway
        .bind(
            token.into(),
            AiGatewayBinding {
                target: AiTarget::Ssh { connection_id: 7 },
                provider: "claude-code".into(),
                session_id: "ai-test".into(),
                command_policy: AiCommandPolicy::AutoSafe,
            },
        )
        .unwrap();
    gateway
        .activate_session(token, "ai-test".into(), AiCommandPolicy::AutoSafe)
        .unwrap();
    let budget = HeadlessOutputBudget::default();
    let failed = AtomicBool::new(false);
    let context = HeadlessOutputContext {
        emitter: &emitter,
        session_id: "ai-test",
        connection_id: Some(7),
        bridge_token: Some(token),
        gateway: &gateway,
        budget: &budget,
        failed: &failed,
    };
    let requirement = HeadlessReadinessRequirement {
        event_type: "system",
        event_subtype: "init",
        tool_prefix: "mcp__nocterm__",
        failure_message: "missing nocterm tools",
        require_gateway_call: true,
        missing_gateway_call_message: "missing nocterm call",
    };
    let output = concat!(
        "{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"mcp__nocterm__session_context\"]}\n",
        "{\"type\":\"assistant\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Mings-Mac.local\"}]}}\n"
    );

    assert!(!emit_lines(
        "stdout",
        output.as_bytes(),
        &context,
        Some(&requirement)
    ));
    let events = emitter.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].data.contains("missing nocterm call"));
    assert!(!events[0].data.contains("Mings-Mac.local"));
}
