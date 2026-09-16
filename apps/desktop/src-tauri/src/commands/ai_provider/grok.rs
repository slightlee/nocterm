//! Grok ACP 的隔离运行环境与模型配置投影。
//! 用户配置是不可信输入：只继承当前模型连接信息，MCP、插件、Hook、规则和权限全部丢弃。

use std::{
    ffi::{OsStr, OsString},
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use super::{ProviderAdapter, ProviderLaunch, ProviderLaunchPlan, cleanup_paths};

mod config;

use config::create_runtime_config;

pub(super) static ADAPTER: &dyn ProviderAdapter = &GrokAdapter;

const AGENT_PROFILE: &str = r#"---
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
internal tool names out of user-facing responses and describe actions in natural language.
"#;

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

struct GrokAdapter;

impl ProviderAdapter for GrokAdapter {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn command(&self) -> &'static str {
        "grok"
    }

    fn prepare_launch(&self, _launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        Ok(ProviderLaunchPlan::Persistent)
    }
}

/// 隔离目录的所有权跟随 ACP 进程；无论启动失败、重置还是宿主退出都会统一清理。
pub(crate) struct PreparedGrokAcpLaunch {
    pub args: Vec<String>,
    pub environment: Vec<(OsString, OsString)>,
    pub current_directory: PathBuf,
    secret_environment_keys: Vec<OsString>,
    cleanup_paths: Vec<PathBuf>,
    cleanup_parent: Option<PathBuf>,
}

impl Drop for PreparedGrokAcpLaunch {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl PreparedGrokAcpLaunch {
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
        cleanup_paths(&self.cleanup_paths);
        if let Some(parent) = &self.cleanup_parent {
            let _ = fs::remove_dir(parent);
        }
    }
}

pub(crate) fn prepare_grok_acp_launch(
    timestamp: u128,
    sequence: u64,
) -> Result<PreparedGrokAcpLaunch, String> {
    let (runtime_directory, cleanup_parent) = create_runtime_directory(timestamp, sequence)?;
    let prepared = prepare_isolated_launch(&runtime_directory, cleanup_parent.clone());
    if prepared.is_err() {
        cleanup_paths(std::slice::from_ref(&runtime_directory));
        if let Some(parent) = cleanup_parent {
            let _ = fs::remove_dir(parent);
        }
    }
    prepared
}

fn prepare_isolated_launch(
    runtime_directory: &Path,
    cleanup_parent: Option<PathBuf>,
) -> Result<PreparedGrokAcpLaunch, String> {
    let model_environment = create_runtime_config(runtime_directory)?;
    let (mut environment, mut secret_environment_keys) =
        create_isolated_environment(model_environment);
    link_authentication(runtime_directory)?;
    let profile = runtime_directory.join("agent.md");
    write_private_file(&profile, AGENT_PROFILE.as_bytes(), "Grok Agent Profile")?;
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
        cleanup_paths: vec![runtime_directory.to_path_buf()],
        cleanup_parent,
    })
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
    let mut builder = DirBuilder::new();
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        AGENT_PROFILE, MODEL_ENVIRONMENT_ALLOWLIST, PreparedGrokAcpLaunch,
        RUNTIME_ENVIRONMENT_ALLOWLIST,
    };

    #[test]
    fn agent_profile_exposes_only_mcp_aggregation_tools() {
        assert!(AGENT_PROFILE.contains("tools: [search_tool, use_tool]"));
        assert!(AGENT_PROFILE.contains("agents_md: false"));
        assert!(!AGENT_PROFILE.contains("run_terminal_cmd"));
        assert!(!AGENT_PROFILE.contains("Read,"));
    }

    #[test]
    fn isolated_launch_hides_generic_user_configuration_roots() {
        let runtime = std::env::temp_dir().join("nocterm-grok-environment-contract");
        let prepared = super::PreparedGrokAcpLaunch {
            args: Vec::new(),
            environment: vec![
                ("GROK_HOME".into(), runtime.as_os_str().to_owned()),
                ("HOME".into(), runtime.as_os_str().to_owned()),
                ("USERPROFILE".into(), runtime.as_os_str().to_owned()),
            ],
            current_directory: runtime.clone(),
            secret_environment_keys: Vec::new(),
            cleanup_paths: Vec::new(),
            cleanup_parent: None,
        };
        for key in ["GROK_HOME", "HOME", "USERPROFILE"] {
            assert!(
                prepared
                    .environment
                    .iter()
                    .any(|(name, value)| name == key && value == runtime.as_os_str())
            );
        }
    }

    #[test]
    fn environment_allowlist_excludes_executable_and_overlay_configuration() {
        let allowed = RUNTIME_ENVIRONMENT_ALLOWLIST
            .into_iter()
            .chain(MODEL_ENVIRONMENT_ALLOWLIST)
            .collect::<Vec<_>>();

        assert!(!allowed.contains(&"GROK_AUTH_PROVIDER_COMMAND"));
        assert!(!allowed.contains(&"GROK_CONFIG"));
        assert!(!allowed.contains(&"GROK_CONFIG_PATH"));
        assert!(!allowed.contains(&"GROK_LOG_FILE"));
        assert!(!allowed.contains(&"GROK_AGENT"));
    }

    #[test]
    fn redacts_extracted_secret_values_from_provider_output() {
        let prepared = PreparedGrokAcpLaunch {
            args: Vec::new(),
            environment: vec![("NOCTERM_GROK_SECRET_1".into(), "inline-secret".into())],
            current_directory: std::path::PathBuf::new(),
            secret_environment_keys: vec!["NOCTERM_GROK_SECRET_1".into()],
            cleanup_paths: Vec::new(),
            cleanup_parent: None,
        };
        assert_eq!(
            prepared.redact("request failed with inline-secret"),
            "request failed with [REDACTED]"
        );
    }

    #[test]
    fn explicit_runtime_cleanup_is_idempotent_with_drop() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let runtime = std::env::temp_dir().join(format!(
            "nocterm-grok-cleanup-contract-{}-{timestamp}",
            std::process::id()
        ));
        std::fs::create_dir(&runtime).unwrap();
        std::fs::write(runtime.join("session-state"), b"test").unwrap();
        let prepared = PreparedGrokAcpLaunch {
            args: Vec::new(),
            environment: Vec::new(),
            current_directory: runtime.clone(),
            secret_environment_keys: Vec::new(),
            cleanup_paths: vec![runtime.clone()],
            cleanup_parent: None,
        };

        prepared.cleanup();
        prepared.cleanup();

        assert!(!runtime.exists());
    }
}
