use std::{
    io::{Cursor, Read},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use nocterm_application::terminal::LocalTerminalService;
use nocterm_domain::terminal::{LocalTerminalPort, OpenedTerminal};
use nocterm_infrastructure::terminal::LocalTerminalManager;

use super::{
    LocalInterruption, drain_until_local_completion, execute_local_sync, interrupt_local_execution,
};
use crate::state::{LocalTerminalRegistry, local_completion_status};

#[test]
fn interrupted_local_execution_waits_for_a_chunked_completion_marker() {
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send("output\r\n__NOCTERM_LOCAL_AI_".into()).unwrap();
    sender.send("DONE_cancel__130\r\n➜ ~ ".into()).unwrap();

    assert!(drain_until_local_completion(
        &receiver,
        "__NOCTERM_LOCAL_AI_DONE_cancel__",
        Instant::now() + Duration::from_secs(1),
    ));
}

#[test]
fn local_completion_status_distinguishes_command_echo_from_result() {
    let marker = "__NOCTERM_LOCAL_AI_DONE_1__";
    assert_eq!(
        local_completion_status(&format!("➜ ~ echo {marker}%ERRORLEVEL%\r"), marker),
        None
    );
    assert_eq!(
        local_completion_status(
            &format!("➜ ~ echo {marker}%ERRORLEVEL%\r{marker}17\r➜ ~ "),
            marker
        )
        .map(|(_, _, status)| status),
        Some(17)
    );
    assert_eq!(
        local_completion_status(&format!("{marker}0"), marker),
        Some((0, 1, 0))
    );
    assert_eq!(
        local_completion_status(&format!("{marker}1x"), marker),
        None
    );
    assert_eq!(
        local_completion_status(&format!("{marker}-1073741510\r"), marker),
        Some((0, 11, -1_073_741_510))
    );
}

struct InterruptTerminal {
    fail_write: bool,
}

impl LocalTerminalPort for InterruptTerminal {
    fn open(&self, _cols: u16, _rows: u16) -> Result<OpenedTerminal, String> {
        Ok(OpenedTerminal {
            id: "local-test".to_string(),
            reader: Box::new(Cursor::new(Vec::new())),
        })
    }

    fn write(&self, _terminal_id: &str, _data: &str) -> Result<(), String> {
        if self.fail_write {
            return Err("interrupt write failed".to_string());
        }
        Ok(())
    }

    fn completion_command(&self, _terminal_id: &str, marker: &str) -> Result<String, String> {
        Ok(format!("echo {marker}0"))
    }

    fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
        Ok(())
    }

    fn close(&self, _terminal_id: &str) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn interruption_releases_the_terminal_only_after_observing_its_marker() {
    let service = LocalTerminalService::new(Arc::new(InterruptTerminal { fail_write: false }));
    let registry = LocalTerminalRegistry::default();
    let marker = "__NOCTERM_LOCAL_AI_DONE_recovered__";
    registry
        .begin_output_marker_filter("local-test", marker)
        .unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    sender.send(format!("output\r\n{marker}130\r\n")).unwrap();

    assert_eq!(
        interrupt_local_execution(
            &service,
            &registry,
            "local-test",
            &receiver,
            marker,
            Instant::now() + Duration::from_millis(100),
        ),
        LocalInterruption::Recovered
    );
    assert!(
        registry
            .begin_output_marker_filter("local-test", "next-marker")
            .is_ok()
    );
}

#[test]
fn interruption_keeps_the_terminal_reserved_while_recovery_is_pending() {
    let service = LocalTerminalService::new(Arc::new(InterruptTerminal { fail_write: false }));
    let registry = LocalTerminalRegistry::default();
    let marker = "__NOCTERM_LOCAL_AI_DONE_pending__";
    registry
        .begin_output_marker_filter("local-test", marker)
        .unwrap();
    let (_sender, receiver) = std::sync::mpsc::channel::<String>();

    assert_eq!(
        interrupt_local_execution(
            &service,
            &registry,
            "local-test",
            &receiver,
            marker,
            Instant::now() + Duration::from_millis(10),
        ),
        LocalInterruption::RecoveryPending
    );
    assert!(
        registry
            .begin_output_marker_filter("local-test", "next-marker")
            .unwrap_err()
            .contains("已有 AI 命令")
    );

    registry.visible_output("local-test", &format!("{marker}130\r\nPS> "));
    assert!(
        registry
            .begin_output_marker_filter("local-test", "next-marker")
            .is_ok()
    );
}

#[test]
fn failed_interrupt_write_does_not_release_the_terminal() {
    let service = LocalTerminalService::new(Arc::new(InterruptTerminal { fail_write: true }));
    let registry = LocalTerminalRegistry::default();
    let marker = "__NOCTERM_LOCAL_AI_DONE_write_failed__";
    registry
        .begin_output_marker_filter("local-test", marker)
        .unwrap();
    let (_sender, receiver) = std::sync::mpsc::channel::<String>();

    let result = interrupt_local_execution(
        &service,
        &registry,
        "local-test",
        &receiver,
        marker,
        Instant::now() + Duration::from_millis(10),
    );
    assert!(matches!(
        result,
        LocalInterruption::WriteFailed(error) if error.contains("interrupt write failed")
    ));
    assert!(
        registry
            .begin_output_marker_filter("local-test", "next-marker")
            .unwrap_err()
            .contains("已有 AI 命令")
    );
}

#[test]
fn local_approved_execution_uses_the_existing_visible_pty() {
    let service = LocalTerminalService::new(Arc::new(LocalTerminalManager::default()));
    let opened = service.open(40, 24).expect("open local terminal");
    let terminal_id = opened.id;
    let registry = Arc::new(LocalTerminalRegistry::default());
    registry.bind("local:test".into(), terminal_id.clone());
    let reader_registry = Arc::clone(&registry);
    let reader_terminal_id = terminal_id.clone();
    let visible_output = Arc::new(Mutex::new(String::new()));
    let reader_visible_output = Arc::clone(&visible_output);
    let mut reader = opened.reader;
    thread::spawn(move || {
        let mut decoder = crate::commands::terminal_text::Utf8Stream::default();
        let mut buffer = [0_u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    let data = decoder.push(&buffer[..length]);
                    if !data.is_empty() {
                        let visible = reader_registry.visible_output(&reader_terminal_id, &data);
                        reader_registry.publish(&reader_terminal_id, &data);
                        reader_visible_output.lock().unwrap().push_str(&visible);
                    }
                }
            }
        }
    });
    let output = execute_local_sync(
        &service,
        &registry,
        "local:test",
        &terminal_id,
        "pwd",
        &AtomicBool::new(false),
        // 默认 Shell 会加载用户配置；并行门禁下保留足够的冷启动余量。
        Instant::now() + Duration::from_secs(15),
    )
    .expect("execute approved local command");
    assert!(!output.output.is_empty());
    assert_eq!(output.exit_code, 0);

    #[cfg(not(windows))]
    let failing_command = "sh -c 'exit 7'";
    #[cfg(windows)]
    let failing_command = "cmd /c exit 7";
    let failed = execute_local_sync(
        &service,
        &registry,
        "local:test",
        &terminal_id,
        failing_command,
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("collect failed local command status");
    assert_eq!(failed.exit_code, 7);
    let terminal_output = visible_output.lock().unwrap();
    assert!(!terminal_output.contains("__NOCTERM_"));
    assert!(!terminal_output.contains("printf '\\n"));
    drop(terminal_output);

    #[cfg(not(windows))]
    let long_command = "sleep 10";
    #[cfg(windows)]
    let long_command = "Start-Sleep -Seconds 10";
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancelled = Arc::clone(&cancellation);
    let execution_service = service.clone();
    let execution_registry = Arc::clone(&registry);
    let execution_terminal_id = terminal_id.clone();
    let execution = thread::spawn(move || {
        execute_local_sync(
            &execution_service,
            &execution_registry,
            "local:test",
            &execution_terminal_id,
            long_command,
            &cancelled,
            Instant::now() + Duration::from_secs(15),
        )
    });
    thread::sleep(Duration::from_millis(250));
    cancellation.store(true, Ordering::Release);
    let cancellation_error = execution
        .join()
        .expect("join cancelled local command")
        .expect_err("cancel local command");
    assert!(cancellation_error.contains("已停止"));
    // Shell 若已返回 marker 可立即复用；否则必须快速拒绝，不能把新命令写进忙碌的 PTY。
    let after_cancel = execute_local_sync(
        &service,
        &registry,
        "local:test",
        &terminal_id,
        "pwd",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(5),
    );
    if cancellation_error.contains("仍在恢复") || cancellation_error.contains("发送失败") {
        assert!(
            after_cancel
                .expect_err("reject command while terminal recovery is pending")
                .contains("已有 AI 命令")
        );
    } else {
        assert_eq!(
            after_cancel
                .expect("execute local command after completed cancellation")
                .exit_code,
            0
        );
    }
    let terminal_output = visible_output.lock().unwrap();
    assert!(!terminal_output.contains("__NOCTERM_"));
    assert!(!terminal_output.contains("printf '\\n"));
    drop(terminal_output);
    service.close(&terminal_id).expect("close local terminal");
}
