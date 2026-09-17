//! AI 工具调用共享的终端执行契约。
//! SSH 独立 exec 与本地可见 PTY 使用不同完成机制，由各自模块封装。

mod local;
mod ssh;

pub(crate) use local::execute_local_sync;
pub(crate) use ssh::{execute_ssh_inspection_sync, execute_ssh_sync};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TerminalCommandResult {
    pub output: String,
    /// SSH 取协议级状态，本地终端取当前 Shell 紧随命令返回的状态。
    pub exit_code: i64,
}
