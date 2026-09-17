use std::{
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use nocterm_application::terminal::LocalTerminalService;
use nocterm_infrastructure::terminal::LocalTerminalManager;

use super::{drain_until_local_completion, execute_local_sync};
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
    assert!(
        execution
            .join()
            .expect("join cancelled local command")
            .expect_err("cancel local command")
            .contains("已停止")
    );
    // 取消收尾后再执行一条命令，证明 PTY 没有被遗留过滤器永久占用。
    let after_cancel = execute_local_sync(
        &service,
        &registry,
        "local:test",
        &terminal_id,
        "pwd",
        &AtomicBool::new(false),
        Instant::now() + Duration::from_secs(5),
    )
    .expect("execute local command after cancellation");
    assert_eq!(after_cancel.exit_code, 0);
    let terminal_output = visible_output.lock().unwrap();
    assert!(!terminal_output.contains("__NOCTERM_"));
    assert!(!terminal_output.contains("printf '\\n"));
    drop(terminal_output);
    service.close(&terminal_id).expect("close local terminal");
}
