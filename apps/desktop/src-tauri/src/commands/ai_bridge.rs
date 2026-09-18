//! Provider 进程使用的 Nocterm MCP Bridge 与公共请求分发器。
//! stdio/TCP 和 Grok MCP-over-ACP 只负责传输；鉴权、审批、审计与终端执行统一在
//! Tauri 进程内由 [`McpRequestDispatcher`] 进入同一个 Tool Gateway。

mod gateway;
mod stdio;
mod transport;

pub(crate) use gateway::McpRequestDispatcher;

pub use gateway::start_gateway;
pub use stdio::{mcp_config, run_stdio};

pub(super) const MAX_MESSAGE_BYTES: usize = 64 * 1024;
pub(super) const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
