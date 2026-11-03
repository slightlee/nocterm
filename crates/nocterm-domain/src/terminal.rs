use std::{
    io::Read,
    sync::{atomic::AtomicBool, mpsc},
};

use crate::connection::ConnectionProfile;

/// 已打开的终端句柄及其只读输出流。
///
/// `shell_integration` 只对 SSH bootstrap 成功的终端有值；本地终端恒为 `None`。
/// 集成失败的降级原因记录在 `integration_fallback_reason`，属于诊断事实而非错误：
/// 调用方不得因此 fail `open()`，只用于控制面观测与 Stage 4 策略路由。
pub struct OpenedTerminal {
    pub id: String,
    pub reader: Box<dyn Read + Send>,
    pub shell_integration: Option<SshShellIntegrationPlan>,
    pub integration_fallback_reason: Option<&'static str>,
}

/// SSH bootstrap 成功后注入的 shell integration 计划。
///
/// nonce 是完成信号的关联标识（非密码学认证）；shell 只包含已具备 bootstrap
/// 与协议测试的 kind。真实服务器验收状态由测试矩阵记录，不能由枚举成员推断。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshShellIntegrationPlan {
    pub nonce: String,
    pub shell: SshShellKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SshShellKind {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl SshShellKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Fish => "fish",
            Self::PowerShell => "powershell",
        }
    }
}

/// 独立 SSH exec 通道除输出外还必须返回协议级退出状态，供 Agent 区分成功与失败。
pub struct OpenedSshExecution {
    pub id: String,
    pub reader: Box<dyn Read + Send>,
    pub completion: mpsc::Receiver<SshExecutionCompletion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SshExecutionCompletion {
    /// 遵循 SSH `exit-status` 请求；服务端未发送时保持 None，调用方不得猜测成功。
    pub exit_code: Option<u32>,
}

/// SSH 终端建连失败的结构化错误：`code` 为前端稳定错误码词表，`message`
/// 为面向诊断的中文提示。与 SFTP 建连失败共用同一套分类词表（authFailed /
/// timeout / hostKeyFailed / connectionRefused / hostUnreachable /
/// hostResolveFailed），让认证失败、超时与主机密钥变更在前端可区分提示。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTerminalError {
    pub code: &'static str,
    pub message: String,
}

impl SshTerminalError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SshTerminalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// SSH 终端能力的领域边界，隐藏 PTY、子进程和平台差异。
pub trait SshTerminalPort: Send + Sync {
    fn open(
        &self,
        profile: &ConnectionProfile,
        cols: u16,
        rows: u16,
        password: Option<&str>,
        private_key: Option<&str>,
    ) -> Result<OpenedTerminal, SshTerminalError>;

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String>;

    fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), String>;

    fn close(&self, terminal_id: &str) -> Result<(), String>;

    fn close_connection(&self, connection_id: i64) -> Result<(), String>;

    /// 在当前连接已有认证会话上开启独立的远程命令通道。
    ///
    /// 返回 `Ok(None)` 表示该连接当前没有活跃终端；`Ok(Some(_))` 则保证命令复用了
    /// 已认证 SSH 连接，但不会写入用户可见的 Shell PTY。取消标记在通道建立阶段也生效，
    /// 防止调用方已经停止后远端命令才延迟启动。
    fn exec_existing(
        &self,
        _connection_id: i64,
        _command: &str,
        _cancellation: &AtomicBool,
    ) -> Result<Option<OpenedSshExecution>, String> {
        Ok(None)
    }
}

/// 本地终端能力与 SSH 终端使用相同 PTY 生命周期，但不依赖连接资料和凭据。
pub trait LocalTerminalPort: Send + Sync {
    fn open(&self, cols: u16, rows: u16) -> Result<OpenedTerminal, String>;

    fn write(&self, terminal_id: &str, data: &str) -> Result<(), String>;

    /// 为当前终端实际使用的 Shell 生成“保存上一命令退出码并输出标记”的命令。
    fn completion_command(&self, terminal_id: &str, marker: &str) -> Result<String, String>;

    fn resize(&self, terminal_id: &str, cols: u16, rows: u16) -> Result<(), String>;

    fn close(&self, terminal_id: &str) -> Result<(), String>;
}
