//! AI Provider 启动适配边界。
//! 每个厂商负责自己的命令、MCP 注入和临时资源；终端目标与审批仍由 Nocterm 管理。

use std::{
    ffi::OsString,
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

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
    fn new(
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

struct CodexAdapter;
struct ClaudeCodeAdapter;
struct GrokAdapter;

impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn command(&self) -> &'static str {
        "codex"
    }
    fn prepare_launch(&self, _launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        Ok(ProviderLaunchPlan::CodexAppServer)
    }
}

impl ProviderAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        "claude-code"
    }
    fn command(&self) -> &'static str {
        "claude"
    }
    fn prepare_launch(&self, launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        let mut args = vec![
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "--input-format".into(),
            "text".into(),
            "--no-session-persistence".into(),
            "--permission-mode".into(),
            "manual".into(),
            "--tools".into(),
            "".into(),
            "--strict-mcp-config".into(),
        ];
        let mcp_config = launch.bridge.map_or_else(
            || serde_json::json!({"mcpServers":{}}).to_string(),
            |bridge| crate::commands::ai_bridge::mcp_config(bridge.executable, bridge.endpoint),
        );
        args.extend(["--mcp-config".into(), mcp_config]);
        Ok(ProviderLaunchPlan::Headless(PreparedHeadlessLaunch::new(
            args,
            Some(launch.prompt.to_string()),
            Vec::new(),
            None,
            Vec::new(),
        )))
    }
}

impl ProviderAdapter for GrokAdapter {
    fn id(&self) -> &'static str {
        "grok"
    }
    fn command(&self) -> &'static str {
        "grok"
    }
    fn prepare_launch(&self, launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        let runtime_directory = create_grok_runtime_directory(launch.timestamp, launch.sequence)?;
        let prepared = prepare_isolated_grok_launch(&runtime_directory, launch);
        if prepared.is_err() {
            cleanup_paths(std::slice::from_ref(&runtime_directory));
        }
        prepared
    }
}

/// Grok 缺少 Claude `--strict-mcp-config` 等价参数，因此把 HOME 与 cwd 都切到任务级目录。
/// MCP 元工具只能发现 Agent Profile 注入的 Nocterm Server，用户 Hook、MCP 和会话也不会旁路。
fn prepare_isolated_grok_launch(
    runtime_directory: &Path,
    launch: ProviderLaunch<'_>,
) -> Result<ProviderLaunchPlan, String> {
    create_grok_runtime_config(runtime_directory)?;
    link_grok_authentication(runtime_directory)?;
    let prompt_file = create_grok_prompt_file(runtime_directory, launch.prompt)?;
    let mut args = vec![
        "--prompt-file".into(),
        prompt_file.to_string_lossy().into_owned(),
        "--output-format".into(),
        "streaming-json".into(),
        "--no-memory".into(),
        "--no-subagents".into(),
        "--disable-web-search".into(),
        "--no-auto-update".into(),
        "--tools".into(),
        "".into(),
    ];
    if let Some(bridge) = launch.bridge {
        let profile =
            create_grok_agent_profile(runtime_directory, bridge.executable, bridge.endpoint)?;
        args.splice(
            0..0,
            ["--agent".into(), profile.to_string_lossy().into_owned()],
        );
    }
    let environment = vec![
        (
            OsString::from("GROK_HOME"),
            runtime_directory.as_os_str().to_owned(),
        ),
        (
            OsString::from("GROK_CLAUDE_MCPS_ENABLED"),
            OsString::from("false"),
        ),
        (
            OsString::from("GROK_CURSOR_MCPS_ENABLED"),
            OsString::from("false"),
        ),
    ];
    Ok(ProviderLaunchPlan::Headless(PreparedHeadlessLaunch::new(
        args,
        None,
        environment,
        Some(runtime_directory.to_path_buf()),
        vec![runtime_directory.to_path_buf()],
    )))
}

/// Grok headless 暂不支持从 stdin 读取 prompt；使用任务级 0600 文件避免内容进入 argv，
/// 文件所有权在进程退出或启动失败时由 PreparedHeadlessLaunch/AiProcess 统一清理。
fn create_grok_prompt_file(runtime_directory: &Path, prompt: &str) -> Result<PathBuf, String> {
    let path = runtime_directory.join("prompt.txt");
    write_private_file(&path, prompt.as_bytes(), "Grok 任务输入")?;
    Ok(path)
}

/// Grok 没有单次 `--mcp-config` 参数，因此使用不含 token 的任务级 Agent Profile。
fn create_grok_agent_profile(
    runtime_directory: &Path,
    executable: &str,
    endpoint: &str,
) -> Result<PathBuf, String> {
    let path = runtime_directory.join("agent.md");
    let executable = serde_json::to_string(executable).map_err(|error| error.to_string())?;
    let endpoint = serde_json::to_string(endpoint).map_err(|error| error.to_string())?;
    let contents = format!(
        "---\nname: nocterm-task\ndescription: Nocterm task-scoped terminal agent.\nprompt_mode: full\nmodel: inherit\npermission_mode: default\nagents_md: true\nmcpServers:\n  - name: nocterm\n    command: {executable}\n    args: [\"mcp-stdio\", \"--endpoint\", {endpoint}]\n---\n\nUse the Nocterm MCP tools for all terminal operations.\n"
    );
    write_private_file(&path, contents.as_bytes(), "Grok 任务配置")?;
    Ok(path)
}

fn create_grok_runtime_directory(timestamp: u128, sequence: u64) -> Result<PathBuf, String> {
    let path = std::env::temp_dir().join(format!(
        "nocterm-grok-runtime-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&path)
        .map_err(|error| format!("创建 Grok 隔离目录失败：{error}"))?;
    Ok(path)
}

/// 显式关闭兼容层，防止 Grok 从 ~/.claude、~/.cursor 载入 MCP、Hook 或指令。
fn create_grok_runtime_config(runtime_directory: &Path) -> Result<(), String> {
    const CONFIG: &str = "[cli]\nauto_update = false\n\n[features]\ntelemetry = false\nfeedback = false\n\n[compat.cursor]\nskills = false\nrules = false\nagents = false\nmcps = false\nhooks = false\nsessions = false\n\n[compat.claude]\nskills = false\nrules = false\nagents = false\nmcps = false\nhooks = false\nsessions = false\n\n[compat.codex]\nsessions = false\n";
    write_private_file(
        &runtime_directory.join("config.toml"),
        CONFIG.as_bytes(),
        "Grok 隔离配置",
    )
}

/// OAuth 凭据不复制到临时文件：Unix 使用符号链接，Windows 使用同卷硬链接。
/// 使用 XAI_API_KEY 的用户不需要该文件。
fn link_grok_authentication(runtime_directory: &Path) -> Result<(), String> {
    let Some(source_home) = original_grok_home() else {
        return Ok(());
    };
    let source = source_home.join("auth.json");
    if !source.is_file() {
        return Ok(());
    }
    let destination = runtime_directory.join("auth.json");
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&source, &destination);
    #[cfg(windows)]
    let result = fs::hard_link(&source, &destination);
    result.map_err(|error| format!("隔离 Grok 登录凭据失败：{error}"))
}

fn original_grok_home() -> Option<PathBuf> {
    std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .map(|home| home.join(".grok"))
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

/// create_new 防止链接替换；Unix 显式使用 0600，Windows 继承当前用户临时目录 ACL。
fn write_private_file(path: &Path, contents: &[u8], label: &str) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = options
        .open(path)
        .and_then(|mut file| file.write_all(contents));
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(format!("创建 {label}失败：{error}"));
    }
    Ok(())
}

static CODEX: CodexAdapter = CodexAdapter;
static CLAUDE_CODE: ClaudeCodeAdapter = ClaudeCodeAdapter;
static GROK: GrokAdapter = GrokAdapter;

pub fn provider_adapters() -> [&'static dyn ProviderAdapter; 3] {
    [&CODEX, &CLAUDE_CODE, &GROK]
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
