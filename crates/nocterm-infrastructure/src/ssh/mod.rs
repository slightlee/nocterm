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

/// SSH 基础设施错误保留底层上下文，由 Application 边界转换为稳定错误码。
#[derive(Debug)]
pub struct SshError(String);

impl std::fmt::Display for SshError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for SshError {}
