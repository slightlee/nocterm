use std::io::Read;

use crate::connection::ConnectionProfile;

/// 已打开的终端句柄及其只读输出流。
pub struct OpenedTerminal {
    pub id: String,
    pub reader: Box<dyn Read + Send>,
}

/// SSH 终端能力的领域边界，隐藏 PTY、子进程和平台差异。
pub trait SshTerminalPort: Send + Sync {
    fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        private_key: Option<&str>,
    ) -> Result<OpenedTerminal, String>;

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String>;

    fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), String>;

    fn close(&self, terminal_id: &str) -> Result<(), String>;

    fn close_connection(&self, connection_id: i64) -> Result<(), String>;
}

/// 本地终端能力与 SSH 终端使用相同 PTY 生命周期，但不依赖连接资料和凭据。
pub trait LocalTerminalPort: Send + Sync {
    fn open(&self, cols: u16, rows: u16) -> Result<OpenedTerminal, String>;

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String>;

    fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), String>;

    fn close(&self, terminal_id: &str) -> Result<(), String>;
}
