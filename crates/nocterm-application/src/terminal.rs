use std::sync::Arc;

use nocterm_domain::terminal::{LocalTerminalPort, SshTerminalPort};
use nocterm_domain::{connection::ConnectionProfile, terminal::OpenedTerminal};

use crate::error::AppError;

/// 终端用例只编排领域 Port，不暴露具体 PTY 或 OpenSSH 实现。
#[derive(Clone)]
pub struct TerminalService {
    backend: Arc<dyn SshTerminalPort>,
}

impl TerminalService {
    pub fn new(backend: Arc<dyn SshTerminalPort>) -> Self {
        Self { backend }
    }

    pub fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        private_key: Option<&str>,
    ) -> Result<OpenedTerminal, AppError> {
        self.backend
            .open(profile, cols, rows, private_key)
            .map_err(terminal_error)
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), AppError> {
        self.backend
            .write(terminal_id, data)
            .map_err(terminal_error)
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), AppError> {
        self.backend
            .resize(terminal_id, cols, rows)
            .map_err(terminal_error)
    }

    pub fn close(&self, terminal_id: &str) -> Result<(), AppError> {
        self.backend.close(terminal_id).map_err(terminal_error)
    }

    pub fn close_connection(&self, connection_id: i64) -> Result<(), AppError> {
        self.backend
            .close_connection(connection_id)
            .map_err(terminal_error)
    }
}

fn terminal_error(error: String) -> AppError {
    let _ = error;
    AppError::new(
        "SSH_TERMINAL_FAILED",
        "SSH 终端操作失败，请检查连接配置和网络后重试",
        true,
    )
}

/// 本地终端用例独立于 SSH 连接资料，确保本地 Shell 不经过连接仓储和凭据链路。
#[derive(Clone)]
pub struct LocalTerminalService {
    backend: Arc<dyn LocalTerminalPort>,
}

impl LocalTerminalService {
    pub fn new(backend: Arc<dyn LocalTerminalPort>) -> Self {
        Self { backend }
    }

    pub fn open(&self, cols: u16, rows: u16) -> Result<OpenedTerminal, AppError> {
        self.backend.open(cols, rows).map_err(local_terminal_error)
    }

    pub fn write(&self, terminal_id: &str, data: &str) -> Result<(), AppError> {
        self.backend
            .write(terminal_id, data)
            .map_err(local_terminal_error)
    }

    pub fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), AppError> {
        self.backend
            .resize(terminal_id, cols, rows)
            .map_err(local_terminal_error)
    }

    pub fn close(&self, terminal_id: &str) -> Result<(), AppError> {
        self.backend
            .close(terminal_id)
            .map_err(local_terminal_error)
    }
}

fn local_terminal_error(error: String) -> AppError {
    AppError::new(
        "LOCAL_TERMINAL_FAILED",
        format!("本地终端操作失败：{error}"),
        true,
    )
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use nocterm_domain::{
        connection::{AuthenticationMethod, ConnectionProfile},
        terminal::{LocalTerminalPort, OpenedTerminal, SshTerminalPort},
    };

    use super::*;

    #[derive(Default)]
    struct FakeTerminal {
        fail_open: bool,
    }

    impl SshTerminalPort for FakeTerminal {
        fn open(
            &self,
            _profile: &ConnectionProfile,
            _cols: u16,
            _rows: u16,
            _private_key: Option<&str>,
        ) -> Result<OpenedTerminal, String> {
            if self.fail_open {
                return Err("backend unavailable".to_string());
            }
            Ok(OpenedTerminal {
                id: "terminal-1".to_string(),
                reader: Box::new(Cursor::new(Vec::new())),
            })
        }

        fn write(&self, _terminal_id: &str, _data: &str) -> Result<(), String> {
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

    impl LocalTerminalPort for FakeTerminal {
        fn open(&self, _cols: u16, _rows: u16) -> Result<OpenedTerminal, String> {
            if self.fail_open {
                return Err("backend unavailable".to_string());
            }
            Ok(OpenedTerminal {
                id: "local-1".to_string(),
                reader: Box::new(Cursor::new(Vec::new())),
            })
        }

        fn write(&self, _terminal_id: &str, _data: &str) -> Result<(), String> {
            Ok(())
        }

        fn resize(&self, _terminal_id: &str, _cols: u16, _rows: u16) -> Result<(), String> {
            Ok(())
        }

        fn close(&self, _terminal_id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn profile() -> ConnectionProfile {
        ConnectionProfile {
            id: 1,
            name: "Test".to_string(),
            host: "localhost".to_string(),
            port: 22,
            username: "tester".to_string(),
            authentication: AuthenticationMethod::SshAgent,
            created_at: 1,
            updated_at: 1,
            group_id: None,
            remark: None,
            remote_initial_path: None,
            icon: None,
            sort_order: None,
            group_name: None,
            credential_kind: None,
            credential_status: "missing".to_string(),
            sync_mode: "local_only".to_string(),
            execution_target: "remote_terminal".to_string(),
        }
    }

    #[test]
    fn delegates_terminal_lifecycle_and_normalizes_backend_errors() {
        let service = TerminalService::new(Arc::new(FakeTerminal::default()));
        let opened = service
            .open(&profile(), 80, 24, None)
            .expect("open terminal");
        assert_eq!(opened.id, "terminal-1");
        service.write(&opened.id, "echo ok").expect("write");
        service.resize(&opened.id, 100, 30).expect("resize");
        service.close(&opened.id).expect("close");

        let failing = TerminalService::new(Arc::new(FakeTerminal { fail_open: true }));
        let error = match failing.open(&profile(), 80, 24, None) {
            Ok(_) => panic!("open must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, "SSH_TERMINAL_FAILED");
        assert_eq!(
            error.message,
            "SSH 终端操作失败，请检查连接配置和网络后重试"
        );
    }

    #[test]
    fn delegates_local_terminal_lifecycle_with_a_distinct_error_code() {
        let service = LocalTerminalService::new(Arc::new(FakeTerminal::default()));
        let opened = service.open(80, 24).expect("open local terminal");
        assert_eq!(opened.id, "local-1");
        service.write(&opened.id, "echo ok").expect("write");
        service.resize(&opened.id, 100, 30).expect("resize");
        service.close(&opened.id).expect("close");

        let failing = LocalTerminalService::new(Arc::new(FakeTerminal { fail_open: true }));
        let error = match failing.open(80, 24) {
            Ok(_) => panic!("open must fail"),
            Err(error) => error,
        };
        assert_eq!(error.code, "LOCAL_TERMINAL_FAILED");
    }
}
