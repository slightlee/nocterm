use std::{
    io::{self, Cursor, Read},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant},
};

use nocterm_application::terminal::TerminalService;
use nocterm_domain::{
    connection::ConnectionProfile,
    terminal::{OpenedTerminal, SshTerminalError, SshTerminalPort},
};
use nocterm_infrastructure::ssh::SshTerminalManager;

use super::{collect_ssh_output, execute_ssh_visible_sync, marker_or_release_gate};
use crate::state::{Osc633Event, SshTerminalRegistry};
use nocterm_domain::ssh_visible_protocol::{
    visible_result as marker_visible_output, wrapped_command as marker_wrapped_command,
};

struct VisibleSshPort {
    registry: Arc<SshTerminalRegistry>,
    writes: Mutex<Vec<String>>,
}

struct MarkerRecoveryPort {
    registry: Arc<SshTerminalRegistry>,
    visible: Mutex<Vec<String>>,
}

struct InterruptRecoveryPort {
    registry: Arc<SshTerminalRegistry>,
    writes: Mutex<Vec<String>>,
}

struct LargeCompletionPort {
    registry: Arc<SshTerminalRegistry>,
    writes: Arc<Mutex<Vec<String>>>,
}

impl SshTerminalPort for LargeCompletionPort {
    fn open(
        &self,
        _profile: &ConnectionProfile,
        _cols: u16,
        _rows: u16,
        _password: Option<&str>,
        _private_key: Option<&str>,
    ) -> Result<OpenedTerminal, SshTerminalError> {
        Err(SshTerminalError::new("test", "not used"))
    }

    fn write(&self, _terminal_id: &str, data: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push(data.to_string());
        if data != "\u{3}" {
            self.registry
                .publish("ssh-visible", &"x".repeat(128 * 1024 + 1));
            let registry = Arc::clone(&self.registry);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(10));
                registry.record_integration_event(
                    "ssh-visible",
                    &Osc633Event::CommandFinished {
                        code: 0,
                        nonce: Some("nonce-visible".into()),
                    },
                );
            });
        }
        Ok(())
    }

    fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn close(&self, _terminal_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn close_connection(&self, _connection_id: i64) -> Result<(), String> {
        Ok(())
    }
}

impl SshTerminalPort for InterruptRecoveryPort {
    fn open(
        &self,
        _profile: &ConnectionProfile,
        _cols: u16,
        _rows: u16,
        _password: Option<&str>,
        _private_key: Option<&str>,
    ) -> Result<OpenedTerminal, SshTerminalError> {
        Err(SshTerminalError::new("test", "not used"))
    }

    fn write(&self, _terminal_id: &str, data: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push(data.to_string());
        if data == "\u{3}" {
            self.registry.record_integration_event(
                "ssh-visible",
                &Osc633Event::CommandFinished {
                    code: 130,
                    nonce: Some("nonce-visible".into()),
                },
            );
        }
        Ok(())
    }

    fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn close(&self, _terminal_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn close_connection(&self, _connection_id: i64) -> Result<(), String> {
        Ok(())
    }
}

impl SshTerminalPort for MarkerRecoveryPort {
    fn open(
        &self,
        _profile: &ConnectionProfile,
        _cols: u16,
        _rows: u16,
        _password: Option<&str>,
        _private_key: Option<&str>,
    ) -> Result<OpenedTerminal, SshTerminalError> {
        Err(SshTerminalError::new("test", "not used"))
    }

    fn write(&self, _terminal_id: &str, data: &str) -> Result<(), String> {
        let prefix = "__NOCTERM_SSH_AI_DONE_";
        let marker_start = data
            .find(prefix)
            .ok_or_else(|| "missing marker".to_string())?;
        let marker_tail = &data[marker_start + prefix.len()..];
        let marker_end = marker_tail
            .find("__")
            .map(|end| marker_start + prefix.len() + end + 2)
            .ok_or_else(|| "incomplete marker".to_string())?;
        let marker = &data[marker_start..marker_end];
        let raw = format!("{data}recovered output\r\n{marker}23__\r\nprompt$ ");
        self.visible
            .lock()
            .unwrap()
            .push(self.registry.visible_output("ssh-marker", &raw));
        self.registry.publish("ssh-marker", &raw);
        Ok(())
    }

    fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn close(&self, _terminal_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn close_connection(&self, _connection_id: i64) -> Result<(), String> {
        Ok(())
    }
}

impl SshTerminalPort for VisibleSshPort {
    fn open(
        &self,
        _profile: &ConnectionProfile,
        _cols: u16,
        _rows: u16,
        _password: Option<&str>,
        _private_key: Option<&str>,
    ) -> Result<OpenedTerminal, SshTerminalError> {
        Err(SshTerminalError::new("test", "not used"))
    }

    fn write(&self, _terminal_id: &str, data: &str) -> Result<(), String> {
        self.writes.lock().unwrap().push(data.to_string());
        self.registry.publish("ssh-visible", "visible output\r\n");
        self.registry.record_integration_event(
            "ssh-visible",
            &Osc633Event::CommandFinished {
                code: 0,
                nonce: Some("nonce-visible".into()),
            },
        );
        Ok(())
    }

    fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn close(&self, _terminal_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn close_connection(&self, _connection_id: i64) -> Result<(), String> {
        Ok(())
    }
}

struct FailingReader;

impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::ConnectionReset, "test reset"))
    }
}

fn opened_ssh_execution(
    id: &str,
    output: &[u8],
    exit_code: Option<u32>,
) -> nocterm_domain::terminal::OpenedSshExecution {
    let (completion_tx, completion) = std::sync::mpsc::channel();
    completion_tx
        .send(nocterm_domain::terminal::SshExecutionCompletion { exit_code })
        .expect("send test completion");
    nocterm_domain::terminal::OpenedSshExecution {
        id: id.into(),
        reader: Box::new(Cursor::new(output.to_vec())),
        completion,
    }
}

#[test]
fn reused_exec_output_completes_on_channel_eof_without_a_shell_marker() {
    let service = TerminalService::new(Arc::new(SshTerminalManager::default()));
    let opened = opened_ssh_execution("ssh-exec-test", b"container-a\n", Some(0));
    let output = collect_ssh_output(
        &service,
        opened,
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(1),
    )
    .expect("read exec output");
    assert_eq!(output.output, "container-a");
    assert_eq!(output.exit_code, Some(0));
}

#[test]
fn cancelled_or_expired_exec_never_returns_buffered_output_as_success() {
    let service = TerminalService::new(Arc::new(SshTerminalManager::default()));
    let opened = || opened_ssh_execution("ssh-exec-cancelled", b"late output\n", Some(0));
    let cancelled = AtomicBool::new(true);

    assert!(
        collect_ssh_output(
            &service,
            opened(),
            &cancelled,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err()
        .contains("已停止")
    );
    assert!(
        collect_ssh_output(&service, opened(), &AtomicBool::new(false), Instant::now())
            .unwrap_err()
            .contains("超时")
    );
}

#[test]
fn ssh_reader_failure_is_not_treated_as_a_successful_eof() {
    let service = TerminalService::new(Arc::new(SshTerminalManager::default()));
    let (completion_sender, completion) = std::sync::mpsc::channel();
    completion_sender
        .send(nocterm_domain::terminal::SshExecutionCompletion { exit_code: Some(0) })
        .unwrap();
    let opened = nocterm_domain::terminal::OpenedSshExecution {
        id: "ssh-exec-failed-read".into(),
        reader: Box::new(FailingReader),
        completion,
    };

    let error = collect_ssh_output(
        &service,
        opened,
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(1),
    )
    .expect_err("read errors must fail the command");

    assert!(error.contains("读取 SSH 命令输出失败"));
    assert!(error.contains("test reset"));
}

#[test]
fn visible_execution_writes_to_the_bound_pty_and_keeps_the_final_output_chunk() {
    let registry = Arc::new(SshTerminalRegistry::default());
    registry
        .bind("remote:7:test", "ssh-visible", 7)
        .expect("bind SSH terminal");
    registry.register_integration("ssh-visible", "nonce-visible");
    registry.record_integration_event(
        "ssh-visible",
        &Osc633Event::PromptStarted {
            nonce: Some("nonce-visible".into()),
        },
    );
    assert!(registry.try_reserve_visible_execution("ssh-visible", "exec-visible"));
    let port = Arc::new(VisibleSshPort {
        registry: Arc::clone(&registry),
        writes: Mutex::new(Vec::new()),
    });
    let service = TerminalService::new(port.clone());

    let result = execute_ssh_visible_sync(
        &service,
        &registry,
        "ssh-visible".into(),
        7,
        "exec-visible",
        "printf ok",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(1),
    )
    .expect("visible execution succeeds");

    assert_eq!(port.writes.lock().unwrap().as_slice(), ["printf ok\n"]);
    assert_eq!(result.output, "visible output");
    assert_eq!(result.exit_code, 0);
    assert!(!registry.input_gated("ssh-visible"));
}

#[test]
fn completed_visible_execution_wins_a_concurrent_cancellation_without_writing_ctrl_c() {
    let registry = Arc::new(SshTerminalRegistry::default());
    registry
        .bind("remote:7:test", "ssh-visible", 7)
        .expect("bind SSH terminal");
    registry.register_integration("ssh-visible", "nonce-visible");
    registry.record_integration_event(
        "ssh-visible",
        &Osc633Event::PromptStarted {
            nonce: Some("nonce-visible".into()),
        },
    );
    assert!(registry.try_reserve_visible_execution("ssh-visible", "exec-visible"));
    let port = Arc::new(VisibleSshPort {
        registry: Arc::clone(&registry),
        writes: Mutex::new(Vec::new()),
    });
    let service = TerminalService::new(port.clone());
    let started = Instant::now();

    let result = execute_ssh_visible_sync(
        &service,
        &registry,
        "ssh-visible".into(),
        7,
        "exec-visible",
        "sleep 10",
        &AtomicBool::new(true),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("completion already proved that the command finished");

    assert_eq!(result.exit_code, 0);
    assert_eq!(result.output, "visible output");
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(port.writes.lock().unwrap().as_slice(), ["sleep 10\n"]);
    assert!(!registry.input_gated("ssh-visible"));
}

#[test]
fn interruption_observes_a_completion_emitted_synchronously_by_ctrl_c() {
    let registry = Arc::new(SshTerminalRegistry::default());
    registry
        .bind("remote:7:test", "ssh-visible", 7)
        .expect("bind SSH terminal");
    registry.register_integration("ssh-visible", "nonce-visible");
    registry.record_integration_event(
        "ssh-visible",
        &Osc633Event::PromptStarted {
            nonce: Some("nonce-visible".into()),
        },
    );
    assert!(registry.try_reserve_visible_execution("ssh-visible", "exec-visible"));
    let port = Arc::new(InterruptRecoveryPort {
        registry: Arc::clone(&registry),
        writes: Mutex::new(Vec::new()),
    });
    let service = TerminalService::new(port.clone());
    let started = Instant::now();

    let error = execute_ssh_visible_sync(
        &service,
        &registry,
        "ssh-visible".into(),
        7,
        "exec-visible",
        "sleep 10",
        &AtomicBool::new(true),
        Instant::now() + Duration::from_secs(5),
    )
    .expect_err("unfinished cancelled execution must fail");

    assert!(error.contains("已停止"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        port.writes.lock().unwrap().as_slice(),
        ["sleep 10\n", "\u{3}"]
    );
    assert!(!registry.input_gated("ssh-visible"));
}

#[test]
fn output_limit_does_not_interrupt_a_prompt_completed_in_the_same_read_cycle() {
    let registry = Arc::new(SshTerminalRegistry::default());
    registry
        .bind("remote:7:test", "ssh-visible", 7)
        .expect("bind SSH terminal");
    registry.register_integration("ssh-visible", "nonce-visible");
    registry.record_integration_event(
        "ssh-visible",
        &Osc633Event::PromptStarted {
            nonce: Some("nonce-visible".into()),
        },
    );
    assert!(registry.try_reserve_visible_execution("ssh-visible", "exec-visible"));
    let writes = Arc::new(Mutex::new(Vec::new()));
    let service = TerminalService::new(Arc::new(LargeCompletionPort {
        registry: Arc::clone(&registry),
        writes: Arc::clone(&writes),
    }));

    let error = execute_ssh_visible_sync(
        &service,
        &registry,
        "ssh-visible".into(),
        7,
        "exec-visible",
        "generate-large-output",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(1),
    )
    .expect_err("oversized output must still fail");

    assert_eq!(error, "可见终端命令输出超过 128 KiB");
    assert_eq!(
        writes.lock().unwrap().as_slice(),
        ["generate-large-output\n"]
    );
    assert!(!registry.input_gated("ssh-visible"));
}

#[test]
fn marker_generation_failure_releases_the_reserved_terminal_gate() {
    let registry = SshTerminalRegistry::default();
    registry
        .bind("remote:7:test", "ssh-visible", 7)
        .expect("bind SSH terminal");
    registry.register_integration("ssh-visible", "nonce-visible");
    registry.record_integration_event(
        "ssh-visible",
        &Osc633Event::PromptStarted {
            nonce: Some("nonce-visible".into()),
        },
    );
    assert!(registry.try_reserve_visible_execution("ssh-visible", "exec-visible"));

    let error = marker_or_release_gate(
        &registry,
        "ssh-visible",
        "exec-visible",
        Result::<String, ()>::Err(()),
    )
    .expect_err("marker generation failure must fail before dispatch");

    assert_eq!(error, "无法生成 SSH 可见执行完成标记");
    assert!(!registry.input_gated("ssh-visible"));
}

#[test]
fn marker_wrapper_preserves_the_command_status_and_hides_internal_protocol_from_results() {
    let marker = "__NOCTERM_SSH_AI_DONE_deadbeef__";
    let wrapped = marker_wrapped_command("printf 'ok'", marker);
    assert!(wrapped.contains("eval 'printf '\\''ok'\\'''"));
    assert!(wrapped.contains(&format!("printf '{marker}%s__\\n'")));
    assert!(wrapped.contains("unset __nocterm_status_deadbeef"));
    assert!(wrapped.contains("return \"$__nocterm_code\""));

    let raw = format!("{wrapped}\r\nok\r\n{marker}17__\r\nprompt$ ");
    assert_eq!(marker_visible_output(&raw, Some(marker)), "ok");

    let no_echo = format!("first line\r\nsecond line\r\n{marker}17__\r\nprompt$ ");
    assert_eq!(
        marker_visible_output(&no_echo, Some(marker)),
        "first line\r\nsecond line"
    );
}

#[test]
fn visible_execution_uses_marker_only_when_the_expected_osc_completion_is_missing() {
    let registry = Arc::new(SshTerminalRegistry::default());
    registry
        .bind("remote:7:marker", "ssh-marker", 7)
        .expect("bind SSH terminal");
    registry.register_integration("ssh-marker", "nonce-marker");
    registry.enable_marker_recovery("ssh-marker");
    registry.record_integration_event(
        "ssh-marker",
        &Osc633Event::PromptStarted {
            nonce: Some("nonce-marker".into()),
        },
    );
    assert!(registry.try_reserve_visible_execution("ssh-marker", "exec-marker"));
    let port = Arc::new(MarkerRecoveryPort {
        registry: Arc::clone(&registry),
        visible: Mutex::new(Vec::new()),
    });
    let service = TerminalService::new(port.clone());

    let result = execute_ssh_visible_sync(
        &service,
        &registry,
        "ssh-marker".into(),
        7,
        "exec-marker",
        "false",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(2),
    )
    .expect("marker recovery succeeds");

    assert_eq!(result.output, "recovered output");
    assert_eq!(result.exit_code, 23);
    assert_eq!(
        port.visible.lock().unwrap().as_slice(),
        ["false\r\nrecovered output\r\nprompt$ "]
    );
    assert!(!registry.input_gated("ssh-marker"));
    // Marker 证明命令完成并释放 gate，但不能伪造慢提示符已经完成渲染。
    assert!(!registry.prompt_ready("ssh-marker"));
    assert!(!registry.try_reserve_visible_execution("ssh-marker", "exec-next"));
}
