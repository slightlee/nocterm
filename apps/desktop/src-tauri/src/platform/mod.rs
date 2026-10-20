//! 平台适配层：`cfg(target_os)` 仅允许出现在此目录（架构约定）。

#[cfg(target_os = "windows")]
pub mod windows_icon;
