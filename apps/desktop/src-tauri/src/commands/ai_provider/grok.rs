//! Grok 的隔离启动环境。
//! 该 CLI 缺少任务级 MCP 配置参数，必须显式隔离用户配置、Hook 和会话状态。

use std::{
    ffi::OsString,
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use super::{
    PreparedHeadlessLaunch, ProviderAdapter, ProviderLaunch, ProviderLaunchPlan, cleanup_paths,
};

pub(super) static ADAPTER: &dyn ProviderAdapter = &GrokAdapter;

struct GrokAdapter;

impl ProviderAdapter for GrokAdapter {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn command(&self) -> &'static str {
        "grok"
    }

    fn prepare_launch(&self, launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        let runtime_directory = create_runtime_directory(launch.timestamp, launch.sequence)?;
        let prepared = prepare_isolated_launch(&runtime_directory, launch);
        if prepared.is_err() {
            cleanup_paths(std::slice::from_ref(&runtime_directory));
        }
        prepared
    }
}

/// HOME 与 cwd 都切到任务级目录，确保用户配置不能旁路 Nocterm 的工具边界。
fn prepare_isolated_launch(
    runtime_directory: &Path,
    launch: ProviderLaunch<'_>,
) -> Result<ProviderLaunchPlan, String> {
    create_runtime_config(runtime_directory)?;
    link_authentication(runtime_directory)?;
    let prompt_file = create_prompt_file(runtime_directory, launch.prompt)?;
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
        let profile = create_agent_profile(runtime_directory, bridge.executable, bridge.endpoint)?;
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

/// Prompt 经任务级 0600 文件传入，避免敏感内容进入进程参数。
fn create_prompt_file(runtime_directory: &Path, prompt: &str) -> Result<PathBuf, String> {
    let path = runtime_directory.join("prompt.txt");
    write_private_file(&path, prompt.as_bytes(), "Grok 任务输入")?;
    Ok(path)
}

/// Agent Profile 不含 token；子进程只能从环境读取当前任务的 Bridge token。
fn create_agent_profile(
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

fn create_runtime_directory(timestamp: u128, sequence: u64) -> Result<PathBuf, String> {
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

/// 禁用兼容层，防止从 ~/.claude、~/.cursor 载入额外 MCP、Hook 或指令。
fn create_runtime_config(runtime_directory: &Path) -> Result<(), String> {
    const CONFIG: &str = "[cli]\nauto_update = false\n\n[features]\ntelemetry = false\nfeedback = false\n\n[compat.cursor]\nskills = false\nrules = false\nagents = false\nmcps = false\nhooks = false\nsessions = false\n\n[compat.claude]\nskills = false\nrules = false\nagents = false\nmcps = false\nhooks = false\nsessions = false\n\n[compat.codex]\nsessions = false\n";
    write_private_file(
        &runtime_directory.join("config.toml"),
        CONFIG.as_bytes(),
        "Grok 隔离配置",
    )
}

/// OAuth 凭据不复制：Unix 使用符号链接，Windows 使用同卷硬链接。
fn link_authentication(runtime_directory: &Path) -> Result<(), String> {
    let Some(source_home) = original_home() else {
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

fn original_home() -> Option<PathBuf> {
    std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .map(|home| home.join(".grok"))
        })
}

/// create_new 防止链接替换；Unix 显式使用 0600，Windows 继承临时目录 ACL。
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
