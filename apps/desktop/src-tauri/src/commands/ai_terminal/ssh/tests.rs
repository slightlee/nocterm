use std::{
    io::{self, Cursor, Read},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

use nocterm_application::terminal::TerminalService;
use nocterm_infrastructure::ssh::SshTerminalManager;

use super::{collect_ssh_output, require_successful_ssh_result};
use crate::commands::ai_terminal::TerminalCommandResult;

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
fn inspection_output_requires_a_valid_success_status() {
    assert_eq!(
        require_successful_ssh_result(TerminalCommandResult {
            output: "server-a".into(),
            exit_code: 0,
        })
        .unwrap(),
        "server-a"
    );
    assert!(
        require_successful_ssh_result(TerminalCommandResult {
            output: "permission denied".into(),
            exit_code: 1,
        })
        .unwrap_err()
        .contains("退出码 1")
    );
}
