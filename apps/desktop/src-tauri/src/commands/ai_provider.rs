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

pub struct ProviderBridge<'a> {
    pub executable: &'a str,
    pub endpoint: &'a str,
}

/// 通用启动上下文只描述 Nocterm 能力，不暴露任何厂商专用参数或配置格式。
pub struct ProviderLaunch<'a> {
    pub prompt: &'a str,
    pub bridge: Option<ProviderBridge<'a>>,
    pub timestamp: u128,
    pub sequence: u64,
}

/// 启动结果持有任务级临时文件；若进程启动失败，Drop 会立即清理。
pub struct PreparedHeadlessLaunch {
    pub args: Vec<String>,
    pub environment: Vec<(OsString, OsString)>,
    pub current_directory: Option<PathBuf>,
    stdin_payload: Option<String>,
    cleanup_paths: Vec<PathBuf>,
}

impl PreparedHeadlessLaunch {
    pub(super) fn new(
        args: Vec<String>,
        stdin_payload: Option<String>,
        environment: Vec<(OsString, OsString)>,
        current_directory: Option<PathBuf>,
        cleanup_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            args,
            environment,
            current_directory,
            stdin_payload,
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
}

impl Drop for PreparedHeadlessLaunch {
    fn drop(&mut self) {
        cleanup_paths(&self.cleanup_paths);
    }
}

/// 使用枚举表达互斥的 Provider 生命周期，避免“执行模式”和可选启动参数不一致。
pub enum ProviderLaunchPlan {
    CodexAppServer,
    Headless(PreparedHeadlessLaunch),
}

/// Adapter 隔离厂商 CLI 差异，不把 Provider 私有协议带入 Domain/Application。
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn command(&self) -> &'static str;
    fn executable(&self) -> Option<PathBuf> {
        find_provider_executable(self.command())
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
mod tests {
    use std::{fs, path::Path};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::{
        ProviderBridge, ProviderLaunch, ProviderLaunchPlan, find_provider_executable_in,
        provider_adapter,
    };

    fn launch<'a>(prompt: &'a str, bridge: Option<ProviderBridge<'a>>) -> ProviderLaunch<'a> {
        ProviderLaunch {
            prompt,
            bridge,
            timestamp: 42,
            sequence: 7,
        }
    }

    #[test]
    fn registry_exposes_all_provider_commands() {
        assert_eq!(
            provider_adapter("codex").map(|item| item.command()),
            Some("codex")
        );
        assert_eq!(
            provider_adapter("claude-code").map(|item| item.command()),
            Some("claude")
        );
        assert_eq!(
            provider_adapter("grok").map(|item| item.command()),
            Some("grok")
        );
        assert!(provider_adapter("unknown").is_none());
    }

    #[test]
    fn executable_discovery_uses_the_supplied_search_path_without_a_probe_process() {
        let current = std::env::current_exe().expect("test executable");
        let directory = current.parent().expect("test executable directory");
        let command = current
            .file_name()
            .and_then(|name| name.to_str())
            .expect("utf8 test executable name");
        let search_path = std::env::join_paths([Path::new(directory)]).expect("search path");

        assert_eq!(
            find_provider_executable_in(command, Some(search_path), None),
            Some(current)
        );
    }

    #[test]
    fn adapters_keep_machine_output_and_disable_owned_shells_for_bridge_tasks() {
        let bridge = || ProviderBridge {
            executable: "/tmp/nocterm",
            endpoint: "127.0.0.1:4567",
        };
        let ProviderLaunchPlan::Headless(claude) = provider_adapter("claude-code")
            .unwrap()
            .prepare_launch(launch("inspect", Some(bridge())))
            .unwrap()
        else {
            panic!("Claude Code should use a headless launch");
        };
        assert!(claude.args.contains(&"stream-json".to_string()));
        assert!(!claude.args.iter().any(|argument| argument == "inspect"));
        assert_eq!(claude.stdin_payload.as_deref(), Some("inspect"));
        assert!(
            claude
                .args
                .contains(&"--no-session-persistence".to_string())
        );
        assert!(claude.args.contains(&"--strict-mcp-config".to_string()));
        assert!(claude.args.windows(2).any(|pair| pair == ["--tools", ""]));

        let ProviderLaunchPlan::Headless(grok) = provider_adapter("grok")
            .unwrap()
            .prepare_launch(launch("inspect", Some(bridge())))
            .unwrap()
        else {
            panic!("Grok should use a headless launch");
        };
        assert!(grok.args.contains(&"streaming-json".to_string()));
        assert!(!grok.args.iter().any(|argument| argument == "inspect"));
        assert!(grok.args.windows(2).any(|pair| pair == ["--tools", ""]));
        let runtime = grok.cleanup_paths.first().unwrap();
        assert_eq!(grok.current_directory.as_deref(), Some(runtime.as_path()));
        assert!(
            grok.environment
                .iter()
                .any(|(key, value)| { key == "GROK_HOME" && value == runtime.as_os_str() })
        );
        let prompt = runtime.join("prompt.txt");
        assert_eq!(fs::read_to_string(&prompt).unwrap(), "inspect");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&prompt).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let contents = fs::read_to_string(runtime.join("agent.md")).unwrap();
        assert!(contents.contains("mcpServers:"));
        assert!(contents.contains("127.0.0.1:4567"));
        assert!(!contents.contains("NOCTERM_MCP_TOKEN"));
        let config = fs::read_to_string(runtime.join("config.toml")).unwrap();
        assert!(config.contains("mcps = false"));
    }

    #[test]
    fn codex_uses_its_persistent_protocol_adapter() {
        let prepared = provider_adapter("codex")
            .unwrap()
            .prepare_launch(launch("inspect", None))
            .unwrap();
        assert!(matches!(prepared, ProviderLaunchPlan::CodexAppServer));
    }
}
