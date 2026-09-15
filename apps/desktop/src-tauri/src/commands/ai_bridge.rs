//! Provider 进程使用的最小 MCP stdio Bridge。
//! stdio 子进程只负责协议适配，真正的终端操作仍在 Tauri 进程内完成。

mod gateway;
mod stdio;
mod transport;

pub use gateway::start_gateway;
pub use stdio::{mcp_config, run_stdio};

pub(super) const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
