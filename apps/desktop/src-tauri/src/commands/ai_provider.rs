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
/// macOS 从 Finder/Dock 启动的 GUI 进程只继承 launchd 的最小 PATH，用户通过
/// npm、Homebrew、cargo 等安装的 Provider CLI 会探测不到，因此进程 PATH 之外
/// 还要补充一组用户级 CLI 目录；补充目录同样只做文件系统检查。
fn find_provider_executable(command: &str) -> Option<PathBuf> {
    let search_path = combined_search_path(std::env::var_os("PATH"), user_home_directory())?;
    find_provider_executable_in(command, Some(search_path), std::env::var_os("PATHEXT"))
}

/// 用户的家目录是补充 CLI 目录的唯一来源；Windows 使用 USERPROFILE 表达同一概念。
fn user_home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 供 Provider 子进程注入的 PATH：与探测使用同一份合成路径，保证"探测到什么
/// 就用什么启动"。Provider 脚本普遍使用 `#!/usr/bin/env node` 一类 shebang，
/// GUI 启动时的最小 PATH 会让它们即使被发现也无法执行。
pub(crate) fn provider_environment_path() -> Option<OsString> {
    combined_search_path(std::env::var_os("PATH"), user_home_directory())
}

/// 合成探测与子进程共用的搜索路径：进程 PATH 优先，保证开发环境行为不变，
/// 其后是补充目录；合成失败时回退到原始进程 PATH，不因单个非法路径项而失效。
fn combined_search_path(process_path: Option<OsString>, home: Option<PathBuf>) -> Option<OsString> {
    let mut directories = process_path
        .as_ref()
        .map(|value| std::env::split_paths(value).collect::<Vec<_>>())
        .unwrap_or_default();
    directories.extend(supplementary_search_directories(home.as_deref()));
    match std::env::join_paths(&directories) {
        Ok(joined) => Some(joined),
        // join_paths 只在路径项含平台分隔符时失败，此时仍可退回进程 PATH 探测。
        Err(_) => process_path,
    }
}

/// 用户级 CLI 的常见安装位置。列表是固定的代码来源，不读取任何外部配置，
/// 因此不会引入不可信搜索路径；不存在的目录由上层 is_file 检查自然跳过。
/// nvm-windows 把 node 符号链接写入系统 PATH，GUI 进程天然可见，无需在此覆盖。
fn supplementary_search_directories(home: Option<&Path>) -> Vec<PathBuf> {
    let Some(home) = home else {
        return Vec::new();
    };
    let mut directories = vec![
        home.join(".local/bin"),
        home.join("bin"),
        home.join(".cargo/bin"),
        home.join(".grok/bin"),
        home.join(".volta/bin"),
        home.join(".asdf/shims"),
        home.join("Library/pnpm"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
    ];
    // nvm 把 node 全局 CLI 放在版本化目录中，无法静态枚举，只能扫描已有版本；
    // 排序保证结果确定性，避免探测结果随目录枚举顺序漂移。
    if let Ok(entries) = fs::read_dir(home.join(".nvm/versions/node")) {
        let mut version_bins: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path().join("bin"))
            .collect();
        version_bins.sort();
        directories.extend(version_bins);
    }
    directories
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
