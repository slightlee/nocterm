//! AI Provider 启动适配边界。
//! 每个厂商负责自己的命令、MCP 注入和临时资源；终端目标与审批仍由 Nocterm 管理。

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

mod claude_code;
mod codex;
mod grok;
mod terminal_contract;

pub(crate) use grok::{PreparedGrokAcpLaunch, prepare_grok_acp_launch};
pub(crate) use terminal_contract::{terminal_session_instructions, terminal_task_instructions};

pub struct ProviderBridge<'a> {
    pub executable: &'a str,
    pub endpoint: &'a str,
}

/// Provider 只选择 MCP 传输；工具目录、授权和执行仍由公共 Gateway 定义。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderToolTransport {
    Stdio,
    Acp,
}

/// 通用启动上下文只描述 Nocterm 能力，不暴露任何厂商专用参数或配置格式。
pub struct ProviderLaunch<'a> {
    pub prompt: &'a str,
    pub bridge: Option<ProviderBridge<'a>>,
    pub identity: &'a ProviderSessionIdentity,
}

/// 启动结果持有任务级临时文件；若进程启动失败，Drop 会立即清理。
pub struct PreparedHeadlessLaunch {
    pub args: Vec<String>,
    pub environment: Vec<(OsString, OsString)>,
    pub current_directory: Option<PathBuf>,
    stdin_payload: Option<String>,
    readiness_requirement: Option<HeadlessReadinessRequirement>,
    cleanup_paths: Vec<PathBuf>,
}

/// Headless Provider 必须在首个初始化事件中证明任务级工具已经可用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlessReadinessRequirement {
    pub event_type: &'static str,
    pub event_subtype: &'static str,
    pub tool_prefix: &'static str,
    pub failure_message: &'static str,
    /// 绑定终端时至少要有一次请求真正进入 Gateway，不能接受模型直接猜测的结果。
    pub require_gateway_call: bool,
    pub missing_gateway_call_message: &'static str,
}

impl PreparedHeadlessLaunch {
    pub(super) fn new(
        args: Vec<String>,
        stdin_payload: Option<String>,
        environment: Vec<(OsString, OsString)>,
        current_directory: Option<PathBuf>,
        readiness_requirement: Option<HeadlessReadinessRequirement>,
        cleanup_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            args,
            environment,
            current_directory,
            stdin_payload,
            readiness_requirement,
            cleanup_paths,
        }
    }

    /// Prompt 等敏感输入只能经 stdin 或受限临时文件传递，不能出现在进程参数中。
    pub fn take_stdin_payload(&mut self) -> Option<String> {
        self.stdin_payload.take()
    }

    /// Provider 成功启动后把清理所有权转交给 AiProcess。
    pub fn take_cleanup_paths(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.cleanup_paths)
    }

    pub fn take_readiness_requirement(&mut self) -> Option<HeadlessReadinessRequirement> {
        self.readiness_requirement.take()
    }
}

impl Drop for PreparedHeadlessLaunch {
    fn drop(&mut self) {
        cleanup_paths(&self.cleanup_paths);
    }
}

/// 使用枚举表达互斥的 Provider 生命周期，避免“执行模式”和可选启动参数不一致。
pub enum ProviderLaunchPlan {
    Persistent,
    Headless(Box<PreparedHeadlessLaunch>),
}

/// 持久 Provider 的复用身份由宿主确定，不能从模型消息或 Provider 会话状态推断。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderSessionIdentity {
    pub connection_id: Option<i64>,
    pub target_session_id: Option<String>,
    pub working_directory: Option<String>,
}

/// Adapter 隔离厂商 CLI 差异，不把 Provider 私有协议带入 Domain/Application。
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn command(&self) -> &'static str;
    fn executable(&self) -> Option<PathBuf> {
        find_provider_executable(self.command())
    }
    fn tool_transport(&self) -> ProviderToolTransport {
        ProviderToolTransport::Stdio
    }
    fn prepare_launch(&self, launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String>;
}

/// 直接解析当前进程的 PATH/PATHEXT，避免为状态探测启动 `which`/`where` 子进程。
fn find_provider_executable(command: &str) -> Option<PathBuf> {
    find_provider_executable_in(
        command,
        std::env::var_os("PATH"),
        std::env::var_os("PATHEXT"),
    )
}

fn find_provider_executable_in(
    command: &str,
    search_path: Option<OsString>,
    path_extensions: Option<OsString>,
) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 {
        return command_path.is_file().then(|| command_path.to_path_buf());
    }
    let extensions = path_extensions
        .as_deref()
        .and_then(|value| value.to_str())
        .map(|value| {
            value
                .split(';')
                .filter(|extension| !extension.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    std::env::split_paths(&search_path?).find_map(|directory| {
        let direct = directory.join(command);
        if direct.is_file() {
            return Some(direct);
        }
        extensions.iter().find_map(|extension| {
            let candidate = directory.join(format!("{command}{extension}"));
            candidate.is_file().then_some(candidate)
        })
    })
}

/// 只删除当前启动计划创建的精确路径；符号链接按文件移除，不跟随到认证源。
pub fn cleanup_paths(paths: &[PathBuf]) {
    for path in paths {
        let metadata = fs::symlink_metadata(path);
        if metadata.is_ok_and(|metadata| metadata.file_type().is_dir()) {
            let _ = fs::remove_dir_all(path);
        } else {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn provider_adapters() -> [&'static dyn ProviderAdapter; 3] {
    [codex::ADAPTER, claude_code::ADAPTER, grok::ADAPTER]
}

pub fn provider_adapter(id: &str) -> Option<&'static dyn ProviderAdapter> {
    provider_adapters()
        .into_iter()
        .find(|adapter| adapter.id() == id)
}

#[cfg(test)]
mod tests;
