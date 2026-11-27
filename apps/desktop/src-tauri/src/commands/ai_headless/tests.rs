use std::{
    io::Cursor,
    sync::{Mutex, atomic::AtomicBool},
};

use super::output::{
    AiOutputEmitter, HeadlessOutputBudget, HeadlessOutputContext, OutputReservation, emit_lines,
    validate_readiness_event,
};
use super::{ClaudeResumeProbe, MAX_CLAUDE_PROBE_BYTES, classify_claude_probe, read_probe_output};
use crate::commands::ai_provider::HeadlessReadinessRequirement;
use crate::commands::ai_stream::{MAX_PROVIDER_TURN_OUTPUT_BYTES, redact_token};
use crate::dto::ai::AiOutputEvent;
use crate::state::{AiCommandPolicy, AiGatewayBinding, AiGatewayState, AiTarget};

/// 合成子进程不读取 CLI 配置或凭据，仅验证 OS 管道和取消语义。
#[test]
fn probe_subprocess_fixture() {
    use std::{
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    let Ok(mode) = std::env::var("NOCTERM_TEST_PROBE_MODE") else {
        return;
    };
    if mode == "inherited-pipe" {
        let mut descendant = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "commands::ai_headless::tests::probe_subprocess_fixture",
                "--nocapture",
            ])
            .env("NOCTERM_TEST_PROBE_MODE", "hold-pipe")
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        // 父进程刻意先退出；后代短时持有继承的管道，收到合成信号后自行回收。
        let _ = descendant.try_wait();
    } else if mode == "hold-pipe" {
        let release =
            std::path::PathBuf::from(std::env::var_os("NOCTERM_TEST_PROBE_RELEASE").unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        while !release.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
    } else if mode == "wait" {
        thread::sleep(Duration::from_secs(20));
    }
}

fn probe_fixture_command(mode: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    crate::commands::ai_process::hide_console_window(&mut command);
    command
        .args([
            "--exact",
            "commands::ai_headless::tests::probe_subprocess_fixture",
            "--nocapture",
        ])
        .env("NOCTERM_TEST_PROBE_MODE", mode)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    command
}

#[test]
fn resume_probe_cancellation_kills_the_probe_and_blocks_later_registration() {
    use std::{
        sync::Arc,
        thread,
        time::{Duration, Instant},
    };
    let manager = Arc::new(crate::commands::ai_process::AiProcessManager::default());
    let startup = manager.begin_start("probe", "conversation").unwrap();
    let stopping = Arc::clone(&manager);
    let stop = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        assert!(stopping.cancel_startup("probe").unwrap());
    });
    let started = Instant::now();
    let error = super::run_resume_probe(
        &mut probe_fixture_command("wait"),
        startup.cancellation(),
        Duration::from_secs(10),
    )
    .unwrap_err();
    stop.join().unwrap();
    assert!(error.contains("取消"));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(startup.check().is_err());
}

#[test]
fn resume_probe_deadline_includes_inherited_output_pipes() {
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let release = std::env::temp_dir().join(format!(
        "nocterm-probe-release-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut command = probe_fixture_command("inherited-pipe");
    command.env("NOCTERM_TEST_PROBE_RELEASE", &release);
    let started = Instant::now();
    let result = super::run_resume_probe(
        &mut command,
        &AtomicBool::new(false),
        Duration::from_secs(2),
    );
    // 即使断言失败也先释放合成后代，不遗留长期运行的测试进程。
    std::fs::write(&release, b"release").unwrap();
    let error = result.unwrap_err();
    assert!(
        error.contains("读取 Claude Code 探测输出") && error.contains("超时"),
        "{error}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

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
                target: AiTarget::Ssh {
                    connection_id: 7,
                    workspace_session_id: "remote:7:test".into(),
                    terminal_id: "ssh-test".into(),
                    terminal_generation: 1,
                },
                provider: "claude-code".into(),
                conversation_id: "conversation-test".into(),
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
        Some(&requirement),
        None,
    ));
    let events = emitter.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].data.contains("missing nocterm call"));
    assert!(!events[0].data.contains("Mings-Mac.local"));
}

#[test]
fn native_session_is_persisted_only_after_a_valid_readiness_event() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let emitter = TestOutputEmitter::default();
    let gateway = AiGatewayState::default();
    let budget = HeadlessOutputBudget::default();
    let failed = AtomicBool::new(false);
    let context = HeadlessOutputContext {
        emitter: &emitter,
        session_id: "ai-test",
        connection_id: None,
        bridge_token: None,
        gateway: &gateway,
        budget: &budget,
        failed: &failed,
    };
    let requirement = HeadlessReadinessRequirement {
        event_type: "system",
        event_subtype: "init",
        tool_prefix: "mcp__nocterm__",
        failure_message: "missing nocterm tools",
        require_gateway_call: false,
        missing_gateway_call_message: "unused",
    };
    let saves = AtomicUsize::new(0);
    let persist = || {
        saves.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };

    assert!(emit_lines(
        "stdout",
        b"{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"mcp__nocterm__session_context\"]}\n"
            .as_slice(),
        &context,
        Some(&requirement),
        Some(&persist),
    ));
    assert_eq!(saves.load(Ordering::Relaxed), 1);

    let rejected_saves = AtomicUsize::new(0);
    let rejected_persist = || {
        rejected_saves.fetch_add(1, Ordering::Relaxed);
        Ok(())
    };
    assert!(!emit_lines(
        "stdout",
        b"{\"type\":\"system\",\"subtype\":\"init\",\"tools\":[\"Bash\"]}\n".as_slice(),
        &context,
        Some(&requirement),
        Some(&rejected_persist),
    ));
    assert_eq!(rejected_saves.load(Ordering::Relaxed), 0);
}

#[test]
fn claude_text_fallback_requires_the_exact_zero_turn_missing_session_result() {
    let session_id = "123e4567-e89b-42d3-a456-426614174000";
    let missing = format!(
        "{{\"type\":\"result\",\"subtype\":\"error_during_execution\",\"num_turns\":0,\"session_id\":\"{session_id}\",\"errors\":[\"No conversation found with session ID: {session_id}\"]}}\n"
    );
    assert_eq!(
        classify_claude_probe(&missing, "", session_id, false),
        Ok(ClaudeResumeProbe::Missing)
    );

    let possibly_executed = missing.replace("\"num_turns\":0", "\"num_turns\":1");
    assert!(classify_claude_probe(&possibly_executed, "", session_id, false).is_err());
    let other_session = missing.replace(session_id, "223e4567-e89b-42d3-a456-426614174000");
    assert!(classify_claude_probe(&other_session, "", session_id, false).is_err());
    assert!(classify_claude_probe(&missing, "", session_id, true).is_err());
}

#[test]
fn claude_probe_accepts_silent_success_and_output_bound_to_the_expected_session() {
    let session_id = "123e4567-e89b-42d3-a456-426614174000";
    assert_eq!(
        classify_claude_probe("", "", session_id, true),
        Ok(ClaudeResumeProbe::Available)
    );
    assert!(classify_claude_probe("", "", session_id, false).is_err());

    let loaded = format!(
        "{{\"type\":\"system\",\"subtype\":\"session_title_changed\",\"session_id\":\"{session_id}\"}}\n"
    );
    assert_eq!(
        classify_claude_probe(&loaded, "", session_id, true),
        Ok(ClaudeResumeProbe::Available)
    );
    assert!(classify_claude_probe(&loaded, "", session_id, false).is_err());
    assert!(classify_claude_probe("not-json", "unknown error", session_id, true).is_err());
}

#[test]
fn claude_probe_rejects_oversized_and_invalid_utf8_output() {
    let oversized = vec![b'a'; MAX_CLAUDE_PROBE_BYTES as usize + 1];
    assert!(read_probe_output(Some(Cursor::new(oversized))).is_err());
    assert!(read_probe_output(Some(Cursor::new(vec![0xff]))).is_err());
}
