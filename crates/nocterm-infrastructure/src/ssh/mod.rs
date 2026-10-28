//! 进程内 SSH 终端后端：使用纯 Rust 的 russh 直接实现 SSH 协议，
//! 规避 Windows OpenSSH 无法自动注入密码、缺少 ControlMaster 多路复用的限制。
//!
//! 模块结构：
//! - `connection`：共享连接/认证原语、主机密钥 TOFU 校验、分阶段建连与错误分类；
//! - `terminal`：会话管理器、终端/exec 生命周期、取消注册表与 `SshTerminalPort` 实现；
//! - `sftp`：SFTP 实现，复用 `connection` 的建连与认证策略。

mod connection;
pub mod sftp;
mod terminal;

pub use terminal::SshTerminalManager;

/// SSH 基础设施错误：`code` 为前端稳定错误码（与领域 `SshTerminalError`
/// 及 SFTP 建连分类共用词表），`message` 保留底层上下文供诊断；
/// Application 边界据此转换为稳定错误码，不再压平成一句话。
#[derive(Debug)]
pub struct SshError {
    pub code: &'static str,
    pub message: String,
}

impl SshError {
    /// 常规错误：无既定分类（会话生命周期、锁损坏等），归为未知。
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: "unknown",
            message: message.into(),
        }
    }

    /// 已分类错误：建连阶段（解析/拨号/握手/认证）与凭据装配的精确提示。
    pub fn coded(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SshError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for SshError {}
