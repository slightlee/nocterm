//! Grok ACP 的隔离运行环境与模型配置投影。
//! 用户配置是不可信输入：只继承当前模型连接信息，MCP、插件、Hook、规则和权限全部丢弃。

use std::{
    ffi::{OsStr, OsString},
    fs::{self, DirBuilder, File, OpenOptions, TryLockError},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

use super::{
    ProviderAdapter, ProviderLaunch, ProviderLaunchPlan, ProviderToolTransport, cleanup_paths,
    terminal_task_instructions,
};

mod config;

use config::create_runtime_config;
pub(crate) use config::read_grok_model_metadata;

pub(super) static ADAPTER: &dyn ProviderAdapter = &GrokAdapter;

const AGENT_PROFILE_HEADER: &str = r#"---
name: nocterm-terminal
description: Nocterm terminal assistant with task-scoped MCP access only.
prompt_mode: full
model: inherit
permission_mode: default
agents_md: false
tools: [search_tool, use_tool]
---

Use only the connected Nocterm MCP server for terminal-related operations. Do not claim that
terminal access is unavailable unless the relevant Nocterm tool returned an error. Keep MCP and
internal tool names out of user-facing responses and describe actions in natural language. State
the target and execution environment only from tool results. Grok requires MCP schema discovery:
make one precise search_tool call for all capabilities needed by the turn, then use_tool; never
repeat discovery, browse unrelated capabilities, or continue after the requested facts are known.
"#;

fn agent_profile() -> String {
    format!("{AGENT_PROFILE_HEADER}\n{}\n", terminal_task_instructions())
}

const RUNTIME_ENVIRONMENT_ALLOWLIST: [&str; 18] = [
    "PATH",
    "PATHEXT",
    "SYSTEMROOT",
    "WINDIR",
    "COMSPEC",
    "TEMP",
    "TMP",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "no_proxy",
];
const MODEL_ENVIRONMENT_ALLOWLIST: [&str; 7] = [
    "GROK_DEFAULT_MODEL",
    "GROK_MODELS_BASE_URL",
    "GROK_MODELS_LIST_URL",
    "GROK_XAI_API_BASE_URL",
    "GROK_CLI_CHAT_PROXY_BASE_URL",
    "XAI_API_KEY",
    "GROK_CODE_XAI_API_KEY",
];
const SECRET_ENVIRONMENT_VARIABLES: [&str; 6] = [
    "XAI_API_KEY",
    "GROK_CODE_XAI_API_KEY",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "GROK_CLI_CHAT_PROXY_BASE_URL",
];
const PERSISTENT_STATE_DIRECTORY: &str = "ai-provider-state";
const GROK_STATE_DIRECTORY: &str = "grok";
const CONVERSATION_STATE_DIRECTORY: &str = "conversations";

struct GrokAdapter;

impl ProviderAdapter for GrokAdapter {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn command(&self) -> &'static str {
        "grok"
    }

    fn tool_transport(&self) -> ProviderToolTransport {
        ProviderToolTransport::Acp
    }

    fn prepare_launch(&self, _launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        Ok(ProviderLaunchPlan::Persistent)
    }
}

/// 探测进程使用临时目录，聊天进程使用持久目录；清理路径显式记录所有权，避免
/// 正常退出误删跨重启恢复所需的 Grok 会话文件。
pub(crate) struct PreparedGrokAcpLaunch {
    pub args: Vec<String>,
    pub environment: Vec<(OsString, OsString)>,
    pub current_directory: PathBuf,
    secret_environment_keys: Vec<OsString>,
    cleanup: Mutex<Option<GrokLaunchCleanup>>,
}

/// 锁与认证投影拥有同一生命周期。清理先移除投影再释放 OS 锁，旧对象的
/// stdout 收尾或 Drop 不能再次删除新进程在相同路径创建的文件。
struct GrokLaunchCleanup {
    paths: Vec<PathBuf>,
    parent: Option<PathBuf>,
    _conversation_lock: Option<File>,
}

impl Drop for PreparedGrokAcpLaunch {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl PreparedGrokAcpLaunch {
    /// 能力探测与聊天共用隔离环境，防止探测加载聊天运行时已禁用的用户扩展。
    pub fn command(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        crate::commands::ai_process::hide_console_window(&mut command);
        command
            .args(&self.args)
            .env_clear()
            .envs(self.environment.iter().map(|(key, value)| (key, value)))
            .current_dir(&self.current_directory);
        // Windows CLI 包装器和 npm shebang 依赖与可执行文件一致的 PATH。
        if let Some(path) = super::provider_environment_path(executable) {
            command.env("PATH", path);
        }
        super::apply_local_network_bypass(&mut command);
        command
    }

    /// Provider 输出属于不可信边界，任何任务级密钥在进入 IPC 前都必须被替换。
    pub fn redact(&self, text: impl Into<String>) -> String {
        let mut redacted = text.into();
        for (key, value) in &self.environment {
            if self.secret_environment_keys.contains(key)
                && let Some(secret) = value.to_str().filter(|value| !value.is_empty())
            {
                redacted = redacted.replace(secret, "[REDACTED]");
            }
        }
        redacted
    }

    /// Provider 已退出时立即移除会话文件；Drop 再次调用也必须保持幂等。
    pub fn cleanup(&self) {
        // 即使其他清理线程仍在删除文件，也必须等它释放锁后才能返回。
        let mut owned = self
            .cleanup
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(cleanup) = owned.take() {
            cleanup_paths(&cleanup.paths);
            if let Some(parent) = &cleanup.parent {
                let _ = fs::remove_dir(parent);
            }
        }
    }
}

pub(crate) fn prepare_grok_acp_launch(
    timestamp: u128,
    sequence: u64,
    model: Option<&str>,
) -> Result<PreparedGrokAcpLaunch, String> {
    let (runtime_directory, cleanup_parent) = create_runtime_directory(timestamp, sequence)?;
    let prepared = prepare_isolated_launch(
        &runtime_directory,
        vec![runtime_directory.clone()],
        cleanup_parent.clone(),
        model,
        None,
    );
    if prepared.is_err() {
        cleanup_paths(std::slice::from_ref(&runtime_directory));
        if let Some(parent) = cleanup_parent {
            let _ = fs::remove_dir(parent);
        }
    }
    prepared
}

/// 聊天会话按 Nocterm 对话 ID 使用稳定的隔离 HOME/cwd。Provider 原生文件只在该
/// 子目录内持久化，不能读取用户 Grok 的插件、MCP、Hook、Skill 或记忆目录。
pub(crate) fn prepare_grok_conversation_launch(
    app_data_directory: &Path,
    conversation_id: &str,
    model: Option<&str>,
) -> Result<PreparedGrokAcpLaunch, String> {
    let runtime_directory = conversation_runtime_directory(app_data_directory, conversation_id)?;
    let ownership = try_lock_conversation(app_data_directory, conversation_id)?
        .ok_or_else(|| "Grok 会话正在由另一个进程使用，请稍后重试".to_string())?;
    create_private_directory_all(&runtime_directory, "Grok 持久会话目录")?;
    prepare_isolated_launch(&runtime_directory, Vec::new(), None, model, Some(ownership))
}

/// 重置或删除对话只删除白名单 ID 对应的单个目录；应用数据根目录和其他会话不受影响。
pub(crate) fn cleanup_grok_conversation(
    app_data_directory: &Path,
    conversation_id: &str,
) -> Result<bool, String> {
    let runtime_directory = conversation_runtime_directory(app_data_directory, conversation_id)?;
    if !runtime_directory.exists() {
        return Ok(false);
    }
    let _ownership = try_lock_conversation(app_data_directory, conversation_id)?
        .ok_or_else(|| "Grok 会话仍被另一个进程占用，无法清理".to_string())?;
    match fs::remove_dir_all(&runtime_directory) {
        Ok(()) => {
            remove_empty_state_parents(app_data_directory, &runtime_directory);
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("清理 Grok 持久会话目录失败：{error}")),
    }
}

fn prepare_isolated_launch(
    runtime_directory: &Path,
    mut cleanup_paths: Vec<PathBuf>,
    cleanup_parent: Option<PathBuf>,
    model: Option<&str>,
    conversation_lock: Option<File>,
) -> Result<PreparedGrokAcpLaunch, String> {
    let model_environment = create_runtime_config(runtime_directory, model)?;
    let (mut environment, mut secret_environment_keys) =
        create_isolated_environment(model_environment);
    // 显式选择优先于继承的 GROK_DEFAULT_MODEL，仅影响本次隔离子进程。
    if let Some(id) = model {
        set_environment(&mut environment, "GROK_DEFAULT_MODEL".into(), id.into());
    }
    let profile = runtime_directory.join("agent.md");
    replace_private_file(&profile, agent_profile().as_bytes(), "Grok Agent Profile")?;
    // 会话文件可以跨进程保留，认证投影只能活到当前 Provider 进程结束。
    // 将其作为独立清理对象，避免持久目录把 OAuth 凭据一并长期保留。
    if let Some(authentication_path) = link_authentication(runtime_directory)? {
        cleanup_paths.push(authentication_path);
    }
    for (key, value) in [
        (
            OsString::from("GROK_HOME"),
            runtime_directory.as_os_str().to_owned(),
        ),
        (
            OsString::from("HOME"),
            runtime_directory.as_os_str().to_owned(),
        ),
        (
            OsString::from("USERPROFILE"),
            runtime_directory.as_os_str().to_owned(),
        ),
        (OsString::from("GROK_MEMORY"), OsString::from("0")),
        (
            OsString::from("GROK_DISABLE_AUTOUPDATER"),
            OsString::from("1"),
        ),
        (
            OsString::from("GROK_CLAUDE_MCPS_ENABLED"),
            OsString::from("false"),
        ),
        (
            OsString::from("GROK_CURSOR_MCPS_ENABLED"),
            OsString::from("false"),
        ),
    ] {
        set_environment(&mut environment, key, value);
    }
    // 内联值和配置引用都属于凭据，即使 Provider 将其写入 stderr 也必须脱敏。
    secret_environment_keys.extend(
        environment
            .iter()
            .filter(|(key, _)| key.to_string_lossy().starts_with("NOCTERM_GROK_SECRET_"))
            .map(|(key, _)| key.clone()),
    );
    secret_environment_keys.sort();
    secret_environment_keys.dedup();
    Ok(PreparedGrokAcpLaunch {
        args: vec![
            "agent".into(),
            "--no-leader".into(),
            "--agent-profile".into(),
            profile.to_string_lossy().into_owned(),
            "stdio".into(),
        ],
        environment,
        current_directory: runtime_directory.to_path_buf(),
        secret_environment_keys,
        cleanup: Mutex::new(Some(GrokLaunchCleanup {
            paths: cleanup_paths,
            parent: cleanup_parent,
            _conversation_lock: conversation_lock,
        })),
    })
}

fn conversation_runtime_directory(
    app_data_directory: &Path,
    conversation_id: &str,
) -> Result<PathBuf, String> {
    if conversation_id.is_empty()
        || conversation_id.len() > 64
        || !conversation_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err("Grok 持久会话标识无效".to_string());
    }
    Ok(app_data_directory
        .join(PERSISTENT_STATE_DIRECTORY)
        .join(GROK_STATE_DIRECTORY)
        .join(CONVERSATION_STATE_DIRECTORY)
        .join(conversation_id))
}

fn create_private_directory_all(path: &Path, label: &str) -> Result<(), String> {
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .recursive(true)
        .create(path)
        .map_err(|error| format!("创建 {label}失败：{error}"))
}

/// 固定锁文件位于会话目录之外，不能在释放时删除，否则 Unix 上会形成两个 inode
/// 各自持锁的竞态。OS 在宿主崩溃后释放锁，不依赖 PID 存活猜测或新的生产依赖。
fn try_lock_conversation(app_data: &Path, conversation_id: &str) -> Result<Option<File>, String> {
    conversation_runtime_directory(app_data, conversation_id)?;
    let locks = app_data
        .join(PERSISTENT_STATE_DIRECTORY)
        .join(GROK_STATE_DIRECTORY)
        .join("locks");
    create_private_directory_all(&locks, "Grok 会话锁目录")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(locks.join(format!("{conversation_id}.lock")))
        .map_err(|error| format!("打开 Grok 会话锁失败：{error}"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(error) => Err(format!("锁定 Grok 会话目录失败：{error}")),
    }
}

fn remove_empty_state_parents(app_data_directory: &Path, runtime_directory: &Path) {
    let conversations = runtime_directory.parent();
    let grok = conversations.and_then(Path::parent);
    let provider_state = grok.and_then(Path::parent);
    for path in [conversations, grok, provider_state].into_iter().flatten() {
        if path == app_data_directory {
            break;
        }
        if fs::remove_dir(path).is_err() {
            break;
        }
    }
}

/// Grok 的环境变量也是配置入口；只复制运行和模型连接所需项，禁止外部命令及配置覆盖。
fn create_isolated_environment(
    model_environment: Vec<(OsString, OsString)>,
) -> (Vec<(OsString, OsString)>, Vec<OsString>) {
    let mut environment = Vec::new();
    for name in RUNTIME_ENVIRONMENT_ALLOWLIST
        .into_iter()
        .chain(MODEL_ENVIRONMENT_ALLOWLIST)
    {
        if let Some(value) = std::env::var_os(name) {
            set_environment(&mut environment, OsString::from(name), value);
        }
    }
    let mut secret_environment_keys = environment
        .iter()
        .filter(|(key, _)| SECRET_ENVIRONMENT_VARIABLES.iter().any(|name| key == name))
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    for (key, value) in model_environment {
        set_environment(&mut environment, key.clone(), value);
        secret_environment_keys.push(key);
    }
    (environment, secret_environment_keys)
}

fn set_environment(environment: &mut Vec<(OsString, OsString)>, key: OsString, value: OsString) {
    if let Some((_, current)) = environment.iter_mut().find(|(name, _)| *name == key) {
        *current = value;
    } else {
        environment.push((key, value));
    }
}

fn create_runtime_directory(
    timestamp: u128,
    sequence: u64,
) -> Result<(PathBuf, Option<PathBuf>), String> {
    #[cfg(windows)]
    let (root, cleanup_parent) = if let Some(home) = original_home()
        && home.join("auth.json").is_file()
    {
        let root = home.join(".nocterm-runtime");
        fs::create_dir_all(&root)
            .map_err(|error| format!("创建 Grok 隔离目录根路径失败：{error}"))?;
        (root.clone(), Some(root))
    } else {
        (std::env::temp_dir(), None)
    };
    #[cfg(not(windows))]
    let (root, cleanup_parent) = (std::env::temp_dir(), None);
    let path = root.join(format!(
        "nocterm-grok-runtime-{}-{timestamp}-{sequence}",
        std::process::id()
    ));
    #[cfg(unix)]
    let mut builder = DirBuilder::new();
    #[cfg(not(unix))]
    let builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&path)
        .map_err(|error| format!("创建 Grok 隔离目录失败：{error}"))?;
    Ok((path, cleanup_parent))
}

fn push_secret(environment: &mut Vec<(OsString, OsString)>, secret: &str) -> String {
    for index in 1_u64.. {
        let variable = format!("NOCTERM_GROK_SECRET_{index}");
        if environment
            .iter()
            .all(|(key, _)| key != OsStr::new(&variable))
        {
            environment.push((OsString::from(&variable), OsString::from(secret)));
            return variable;
        }
    }
    unreachable!("u64 environment variable sequence exhausted")
}

/// OAuth 凭据只投影到当前 Provider 生命周期。Unix 使用符号链接；Windows 优先
/// 使用硬链接保持 token 刷新一致，跨卷或不支持硬链接时使用受限副本并在退出时删除。
fn link_authentication(runtime_directory: &Path) -> Result<Option<PathBuf>, String> {
    let Some(source_home) = original_home() else {
        return Ok(None);
    };
    let source = source_home.join("auth.json");
    if !source.is_file() {
        return Ok(None);
    }
    let destination = runtime_directory.join("auth.json");
    remove_owned_file(&destination, "旧 Grok 登录凭据链接")?;
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(&source, &destination);
    #[cfg(windows)]
    let result = fs::hard_link(&source, &destination).or_else(|_| {
        // GROK_HOME 可位于其他磁盘，Windows 硬链接不能跨卷；副本仍受私有目录 ACL
        // 和进程级清理约束，且启动时会清扫异常退出遗留。
        fs::copy(&source, &destination).map(|_| ())
    });
    match result {
        Ok(()) => Ok(Some(destination)),
        Err(error) => {
            // 复制失败可能留下截断文件，失败路径同样不能保留凭据片段。
            let _ = fs::remove_file(&destination);
            Err(format!("隔离 Grok 登录凭据失败：{error}"))
        }
    }
}

/// 应用异常退出时 Drop 不会运行；下次启动只清理没有存活 owner 的认证投影。
/// 只处理固定层级下的精确文件名，不扫描或修改用户 `~/.grok`。
pub(crate) fn cleanup_stale_grok_authentication_links(
    app_data_directory: &Path,
) -> Result<(), String> {
    let conversations = app_data_directory
        .join(PERSISTENT_STATE_DIRECTORY)
        .join(GROK_STATE_DIRECTORY)
        .join(CONVERSATION_STATE_DIRECTORY);
    let entries = match fs::read_dir(&conversations) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取 Grok 持久会话目录失败：{error}")),
    };
    for entry in entries {
        let entry = entry.map_err(|error| format!("读取 Grok 持久会话目录失败：{error}"))?;
        if !entry
            .file_type()
            .map_err(|error| format!("读取 Grok 持久会话类型失败：{error}"))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name();
        let Some(conversation_id) = name.to_str() else {
            continue;
        };
        if conversation_runtime_directory(app_data_directory, conversation_id).is_err() {
            continue;
        }
        let Some(_ownership) = try_lock_conversation(app_data_directory, conversation_id)? else {
            continue;
        };
        remove_owned_file(&entry.path().join("auth.json"), "Grok 遗留登录凭据")?;
    }
    Ok(())
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

/// 持久目录可能由上一次进程留下生成文件；先移除已知文件再以 create_new 重建，
/// Provider 创建的未知会话文件不会被扫描、改写或作为配置继承。
pub(super) fn replace_private_file(
    path: &Path,
    contents: &[u8],
    label: &str,
) -> Result<(), String> {
    remove_owned_file(path, label)?;
    write_private_file(path, contents, label)
}

fn remove_owned_file(path: &Path, label: &str) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("移除 {label}失败：{error}")),
    }
}

#[cfg(test)]
mod tests;
