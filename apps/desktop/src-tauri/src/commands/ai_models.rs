//! 按需查询 CLI 的模型目录。探测只发送初始化/目录请求，绝不提交用户消息。
//! IPC 只返回模型标识与受限元数据；协议诊断、凭据与原始进程输出留在边界内。

use std::{
    io::Write,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::dto::ai::{AiModelCatalog, AiModelOption, AiModelSource, AiProviderAccess};

mod claude_catalog;
#[cfg(test)]
mod tests;

use super::{
    ai_process::hide_console_window,
    ai_provider::{
        apply_local_network_bypass, http_endpoint_host, prepare_grok_acp_launch, provider_adapter,
        provider_environment_path, read_grok_model_metadata,
    },
    ai_stream::read_bounded_lines,
    ai_validation::{valid_model_id, valid_reasoning_effort},
    grok_acp_server::{grok_initialize_request, models as grok_models, next_runtime_id},
};

const UNAVAILABLE: &str = "AI_MODEL_CATALOG_UNAVAILABLE";
const INVALID: &str = "AI_MODEL_CATALOG_INVALID";
const LIMIT: usize = 1024;
const CLAUDE_ROLE_ALIASES: [&str; 4] = ["opus", "fable", "sonnet", "haiku"];

/// 执行一次目录探测。面板负责预取、复用结果与合并同一 Agent 的并发请求。
#[tauri::command]
pub async fn ai_provider_models(provider: String) -> Result<AiModelCatalog, String> {
    tauri::async_runtime::spawn_blocking(move || discover_models(&provider))
        .await
        .map_err(|_| UNAVAILABLE.to_string())?
}

/// 设置页按需读取版本；只返回校验后的版本号，失败不泄露 CLI 原始输出。
#[tauri::command]
pub async fn ai_provider_version(provider: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        probe_cli_version(&mut provider_command(&provider)?)
            .map_err(|_| "AI_PROVIDER_VERSION_UNAVAILABLE".to_string())
    })
    .await
    .map_err(|_| "AI_PROVIDER_VERSION_UNAVAILABLE".to_string())?
}

fn provider_command(provider: &str) -> Result<Command, String> {
    let adapter = provider_adapter(provider).ok_or("AI_PROVIDER_UNKNOWN")?;
    let executable = adapter.executable().ok_or("AI_PROVIDER_NOT_INSTALLED")?;
    let mut command = Command::new(&executable);
    if let Some(path) = provider_environment_path(&executable) {
        command.env("PATH", path);
    }
    hide_console_window(&mut command);
    apply_local_network_bypass(&mut command);
    command.current_dir(std::env::temp_dir());
    Ok(command)
}

/// 发送前以后端实际可执行文件为准复核版本，避免旧缓存、自定义模型或直接 IPC
/// 绕过目录中的不可用标记。版本探测不读取账号，也不发送推理请求。
pub(super) fn ensure_claude_native_session_support(
    executable: &std::path::Path,
    cancellation: &AtomicBool,
) -> Result<(), String> {
    let mut command = Command::new(executable);
    if let Some(path) = provider_environment_path(executable) {
        command.env("PATH", path);
    }
    hide_console_window(&mut command);
    command.current_dir(std::env::temp_dir());
    let version = probe_cli_version_cancellable(&mut command, cancellation).map_err(|error| {
        if cancellation.load(Ordering::Acquire) {
            error
        } else {
            "无法确认 Claude Code 版本，已停止以避免不兼容的会话恢复".to_string()
        }
    })?;
    if claude_catalog::supports_native_session_contract(&version) {
        return Ok(());
    }
    Err(format!(
        "Claude Code 版本过低，原生会话恢复需要 {} 或更高版本",
        claude_catalog::NATIVE_SESSION_MINIMUM
    ))
}

fn discover_models(provider: &str) -> Result<AiModelCatalog, String> {
    let mut command = provider_command(provider)?;
    let executable = std::path::PathBuf::from(command.get_program());
    let mut models = Vec::new();
    let access;
    match provider {
        "codex" => {
            let mut probe = Probe::start(command.args(["app-server", "--stdio"]), false)?;
            probe.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{
                "name":"nocterm","version":env!("NOCTERM_PRODUCT_VERSION")}}}))?;
            probe.response(1)?;
            probe.send(json!({"method":"initialized"}))?;
            probe.send(json!({"id":100,"method":"account/read","params":{}}))?;
            let account = probe.response(100).ok();
            probe
                .send(json!({"id":101,"method":"config/read","params":{"includeLayers":false}}))?;
            let config = probe.response(101).ok();
            access = codex_access(account.as_ref(), config.as_ref());
            let mut cursor = Value::Null;
            for id in 2..66 {
                probe.send(
                    json!({"id":id,"method":"model/list","params":{"cursor":cursor,"limit":100}}),
                )?;
                let result = probe.response(id)?;
                append_codex_models(&mut models, &result)?;
                cursor = result["nextCursor"].clone();
                if cursor.is_null() {
                    break;
                }
                if id == 65 || !cursor.is_string() {
                    return Err(INVALID.into());
                }
            }
        }
        "claude-code" => {
            // 版本探测失败时仍显示本机目录，内置版本项明确标为无法核对兼容性。
            let version = probe_cli_version(&mut command).ok();
            // 只读控制握手：禁用内置工具、插件目录外的项目设置和 MCP 自动加载。
            let mut probe = Probe::start(
                command.args([
                    "-p",
                    "--input-format",
                    "stream-json",
                    "--output-format",
                    "stream-json",
                    "--verbose",
                    "--no-session-persistence",
                    "--tools",
                    "",
                    "--setting-sources",
                    "user",
                    "--strict-mcp-config",
                ]),
                false,
            )?;
            probe.send(json!({"type":"control_request","request_id":"models",
                "request":{"subtype":"initialize"}}))?;
            loop {
                let event = probe.json_line()?;
                let response = &event["response"];
                if event["type"] != "control_response" || response["request_id"] != "models" {
                    continue;
                }
                append_claude_models(&mut models, &response["response"])?;
                access = claude_access(&response["response"]);
                break;
            }
            claude_catalog::append_versions(&mut models, version.as_deref())?;
        }
        "grok" => {
            // 优先使用隔离 ACP 目录。旧版、超时或空目录继续走原本的列表命令。
            let (timestamp, sequence) = next_runtime_id();
            match probe_grok_acp_models(&executable, timestamp, sequence) {
                Ok(discovered) if !discovered.is_empty() => models = discovered,
                _ => {
                    // 旧版列表行以星号或短横线开头，保留原有 CLI 回退。
                    let mut probe = Probe::start(command.arg("models"), true)?;
                    while let Some(line) = probe.next_line()? {
                        let mut columns = line.split_whitespace();
                        if matches!(columns.next(), Some("-" | "*")) {
                            add(&mut models, columns.next(), None, None)?;
                        }
                    }
                    probe.finish()?;
                }
            }
            // 隔离探测只加载默认模型；两种发现路径都补齐其余本机模型的安全元数据。
            let metadata = read_grok_model_metadata().map_err(|_| INVALID)?;
            append_configured_grok_models(&mut models, metadata.models)?;
            // 只报告默认模型的连接方式；Grok 没有可靠的独立只读认证状态。
            access = metadata
                .uses_custom_endpoint
                .then_some(AiProviderAccess::CustomEndpoint);
        }
        _ => return Err("AI_PROVIDER_UNKNOWN".into()),
    }
    Ok(AiModelCatalog { models, access })
}

/// 只读取路由类别，不把 base_url 或账号字段带出进程边界。
fn codex_access(account: Option<&Value>, config: Option<&Value>) -> Option<AiProviderAccess> {
    if codex_has_custom_endpoint(config) {
        return Some(AiProviderAccess::CustomEndpoint);
    }
    let account = account?;
    if !account["account"].is_null() {
        return None;
    }
    (account["requiresOpenaiAuth"].as_bool() == Some(true))
        .then_some(AiProviderAccess::LoginRequired)
}

fn codex_has_custom_endpoint(config: Option<&Value>) -> bool {
    let Some(config) = config.and_then(|value| value.get("config")) else {
        return false;
    };
    let provider = config["model_provider"].as_str().unwrap_or("openai");
    let base_url = if provider == "openai" {
        config["openai_base_url"].as_str()
    } else {
        config["model_providers"][provider]["base_url"].as_str()
    };
    base_url
        .and_then(http_endpoint_host)
        .is_some_and(|host| host != "api.openai.com" && host != "chatgpt.com")
}

fn claude_access(result: &Value) -> Option<AiProviderAccess> {
    let custom_endpoint = configured_claude_base_url()
        .as_deref()
        .is_some_and(is_custom_claude_url);
    classify_claude_access(result, custom_endpoint)
}

fn classify_claude_access(result: &Value, custom_endpoint: bool) -> Option<AiProviderAccess> {
    if custom_endpoint {
        return Some(AiProviderAccess::CustomEndpoint);
    }
    let account = result
        .get("account")
        .filter(|account| account.is_object())?;
    let api_provider = account["apiProvider"].as_str();
    if api_provider.is_some_and(|provider| provider != "firstParty") {
        return Some(AiProviderAccess::ExternalProvider);
    }
    let token_source = account["tokenSource"].as_str();
    let api_key_source = account["apiKeySource"].as_str();
    let authenticated = account["email"].is_string()
        || account["subscriptionType"].is_string()
        || token_source.is_some_and(|source| source != "none")
        || api_key_source.is_some_and(|source| source != "none");
    if authenticated {
        return None;
    }
    (api_provider == Some("firstParty")).then_some(AiProviderAccess::LoginRequired)
}

fn is_custom_claude_url(url: &str) -> bool {
    http_endpoint_host(url).is_some_and(|host| host != "api.anthropic.com")
}

/// Claude Code 会从用户 settings.json 注入环境；只定向读取 Base URL，忽略同表中的凭据。
fn configured_claude_base_url() -> Option<String> {
    if let Ok(value) = std::env::var("ANTHROPIC_BASE_URL")
        && is_custom_claude_url(&value)
    {
        return Some(value);
    }
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .or_else(|| std::env::var_os("HOME"))
                .map(|home| std::path::PathBuf::from(home).join(".claude"))
        })?;
    let path = config_dir.join("settings.json");
    if std::fs::metadata(&path).ok()?.len() > 1024 * 1024 {
        return None;
    }
    let settings: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    settings
        .pointer("/env/ANTHROPIC_BASE_URL")
        .and_then(Value::as_str)
        .filter(|url| is_custom_claude_url(url))
        .map(str::to_string)
}

/// ACP/CLI 已报告的项保持原顺序与能力；本机缺失项复用同一 ID/名称校验与数量上限。
fn append_configured_grok_models(
    models: &mut Vec<AiModelOption>,
    configured: Vec<(String, Option<String>)>,
) -> Result<(), String> {
    for (id, name) in configured {
        add(models, Some(&id), name.as_deref(), None)?;
    }
    Ok(())
}

/// 只做 initialize，探测进程无需创建会话、注册终端工具或发送推理消息。
fn probe_grok_acp_models(
    executable: &std::path::Path,
    timestamp: u128,
    sequence: u64,
) -> Result<Vec<AiModelOption>, String> {
    let prepared = prepare_grok_acp_launch(timestamp, sequence, None).map_err(|_| UNAVAILABLE)?;
    let mut probe = Probe::start(&mut prepared.command(executable), false)?;
    probe.send(grok_initialize_request())?;
    let mut models = Vec::new();
    append_grok_models(&mut models, &probe.response(1)?)?;
    Ok(models)
}

fn append_grok_models(models: &mut Vec<AiModelOption>, result: &Value) -> Result<(), String> {
    for item in grok_models::initialized_models(result) {
        let before = models.len();
        add(
            models,
            item["modelId"].as_str(),
            item["name"].as_str(),
            None,
        )?;
        if models.len() > before {
            let model = &mut models[before];
            (model.reasoning_efforts, model.default_reasoning_effort) =
                grok_models::reasoning_options(item);
        }
    }
    Ok(())
}

fn append_codex_models(models: &mut Vec<AiModelOption>, result: &Value) -> Result<(), String> {
    for item in result["data"].as_array().ok_or(INVALID)? {
        if item["hidden"].as_bool() != Some(true) {
            let before = models.len();
            add(
                models,
                item["model"].as_str().or(item["id"].as_str()),
                item["displayName"].as_str(),
                None,
            )?;
            if models.len() > before {
                let model = models.last_mut().expect("appended model");
                model.reasoning_efforts = read_efforts(
                    "codex",
                    item["supportedReasoningEfforts"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|v| v["reasoningEffort"].as_str()),
                );
                model.default_reasoning_effort = item["defaultReasoningEffort"]
                    .as_str()
                    .filter(|effort| model.reasoning_efforts.iter().any(|value| value == effort))
                    .map(str::to_string);
                model.supports_fast_mode = reported_fast_mode(item);
            }
        }
    }
    Ok(())
}

fn append_claude_models(models: &mut Vec<AiModelOption>, result: &Value) -> Result<(), String> {
    for item in result["models"].as_array().ok_or(INVALID)? {
        if item["value"] != "default" && !is_external_claude_role_mapping(item) {
            let before = models.len();
            add(
                models,
                item["value"].as_str(),
                item["displayName"].as_str(),
                item["resolvedModel"].as_str(),
            )?;
            if models.len() > before && item["supportsEffort"] == true {
                models.last_mut().expect("appended model").reasoning_efforts = read_efforts(
                    "claude-code",
                    item["supportedEffortLevels"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str),
                );
            }
        }
    }
    Ok(())
}

/// 角色别名映射由 Claude Code/CC Switch 负责，不自动投影成 Nocterm 的第三方模型。
/// 用户仍可在设置中显式添加实际可调用的模型 ID；原生 Claude 路由保留用于云平台兼容。
fn is_external_claude_role_mapping(item: &Value) -> bool {
    let Some(alias) = item["value"].as_str() else {
        return false;
    };
    let Some(resolved) = item["resolvedModel"].as_str() else {
        return false;
    };
    is_claude_role_alias(alias) && !is_native_claude_route(resolved)
}

fn is_native_claude_route(model: &str) -> bool {
    let model = model.trim().to_ascii_lowercase();
    is_claude_role_alias(&model)
        || model.starts_with("claude-")
        || model.starts_with("anthropic.claude-")
        || model.contains(".anthropic.claude-")
}

fn is_claude_role_alias(model: &str) -> bool {
    CLAUDE_ROLE_ALIASES
        .iter()
        .any(|alias| model.eq_ignore_ascii_case(alias))
}

fn reported_fast_mode(item: &Value) -> bool {
    [
        "serviceTiers",
        "supportedServiceTiers",
        "additionalSpeedTiers",
    ]
    .into_iter()
    .filter_map(|field| item[field].as_array())
    .flatten()
    .any(|tier| {
        let id = tier.as_str().or_else(|| tier["id"].as_str()).unwrap_or("");
        let name = tier["name"].as_str().unwrap_or("");
        id.eq_ignore_ascii_case("fast")
            || id.eq_ignore_ascii_case("priority")
            || name.eq_ignore_ascii_case("fast")
    })
}

/// 只投影原生档位，未知值不进入菜单；去重并按强度排列，不复制 CLI 的任意描述。
fn read_efforts<'a>(provider: &str, values: impl Iterator<Item = &'a str>) -> Vec<String> {
    let reported = values.take(32).collect::<Vec<_>>();
    [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ]
    .into_iter()
    .filter(|value| valid_reasoning_effort(provider, value) && reported.contains(value))
    .map(str::to_string)
    .collect()
}

/// 保留 Provider 报告顺序，按调用 ID 去重；名称相同的不同别名不能合并。
fn add(
    models: &mut Vec<AiModelOption>,
    candidate: Option<&str>,
    name: Option<&str>,
    resolved: Option<&str>,
) -> Result<(), String> {
    if let Some(id) = candidate.filter(|id| valid_model_id(id)) {
        if models.iter().any(|model| model.id == id) {
            return Ok(());
        }
        let name = name
            .filter(|s| !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(char::is_control))
            .unwrap_or(id);
        models.push(AiModelOption {
            id: id.into(),
            name: name.into(),
            source: AiModelSource::Cli,
            is_legacy: false,
            reasoning_efforts: Vec::new(),
            default_reasoning_effort: None,
            supports_fast_mode: false,
            resolved_model: resolved.filter(|s| valid_model_id(s)).map(str::to_string),
            minimum_cli_version: None,
            unavailable_reason: None,
        });
    }
    if models.len() > LIMIT {
        return Err(INVALID.into());
    }
    Ok(())
}

/// 使用已发现的同一可执行文件与环境；不读取账号文件，也不从初始化中的账号信息推断版本。
fn probe_cli_version(command: &mut Command) -> Result<String, String> {
    probe_cli_version_cancellable(command, &AtomicBool::new(false))
}

/// 聊天启动的版本探测复用取消信号；设置页的独立版本读取保持原截止时间。
fn probe_cli_version_cancellable(
    command: &mut Command,
    cancellation: &AtomicBool,
) -> Result<String, String> {
    if cancellation.load(Ordering::Acquire) {
        return Err("AI 会话启动已取消".into());
    }
    // Command 的参数会累积，版本探测用独立实例，原命令仍留给初始化协议。
    let mut version_command = Command::new(command.get_program());
    for (key, value) in command.get_envs() {
        if let Some(value) = value {
            version_command.env(key, value);
        }
    }
    if let Some(cwd) = command.get_current_dir() {
        version_command.current_dir(cwd);
    }
    hide_console_window(&mut version_command);
    let mut probe = Probe::start(version_command.arg("--version"), false)?;
    let line = probe.next_line_cancellable(cancellation)?.ok_or(INVALID)?;
    probe.finish_cancellable(cancellation)?;
    let version = parse_cli_version(&line).ok_or(INVALID)?;
    Ok(version.to_string())
}

/// 三种 CLI 使用纯版本号或“产品名 版本号”；仅投影受限 token，忽略后面的说明。
fn parse_cli_version(line: &str) -> Option<&str> {
    line.split_whitespace().take(2).find_map(|token| {
        let version = token.strip_prefix('v').unwrap_or(token);
        if version.len() > 80
            || !version
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
        {
            return None;
        }
        let core = version.split(['-', '+']).next()?;
        claude_catalog::parse_version(core).map(|_| version)
    })
}

/// 有界行通道和整轮截止时间限制损坏协议与无响应 CLI 的资源占用。
struct Probe {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<Result<String, String>>,
    deadline: Instant,
}

impl Probe {
    fn start(command: &mut Command, include_stderr: bool) -> Result<Self, String> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(if include_stderr {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .spawn()
            .map_err(|_| UNAVAILABLE)?;
        let stdout = child.stdout.take().expect("piped stdout");
        let stdin = child.stdin.take();
        let (tx, lines) = mpsc::sync_channel(8);
        // Grok 各版本会把清单写入不同流；RPC Provider 只允许解析 stdout。
        if let Some(stderr) = child.stderr.take() {
            read_probe_stream(stderr, tx.clone());
        }
        read_probe_stream(stdout, tx);
        Ok(Self {
            child,
            stdin,
            lines,
            deadline: Instant::now() + Duration::from_secs(12),
        })
    }

    fn send(&mut self, message: Value) -> Result<(), String> {
        writeln!(self.stdin.as_mut().ok_or(UNAVAILABLE)?, "{message}")
            .map_err(|_| UNAVAILABLE.into())
    }

    fn next_line(&self) -> Result<Option<String>, String> {
        self.next_line_cancellable(&AtomicBool::new(false))
    }

    fn next_line_cancellable(&self, cancellation: &AtomicBool) -> Result<Option<String>, String> {
        loop {
            if cancellation.load(Ordering::Acquire) {
                return Err("AI 会话启动已取消".into());
            }
            let time = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or("AI_MODEL_CATALOG_TIMEOUT")?;
            match self.lines.recv_timeout(time.min(Duration::from_millis(20))) {
                Ok(line) => return line.map(Some),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
            }
        }
    }

    fn json_line(&self) -> Result<Value, String> {
        serde_json::from_str(&self.next_line()?.ok_or(UNAVAILABLE)?).map_err(|_| INVALID.into())
    }

    fn response(&self, id: u32) -> Result<Value, String> {
        loop {
            let mut message = self.json_line()?;
            if message["id"] == id {
                return message
                    .get_mut("result")
                    .map(Value::take)
                    .ok_or_else(|| UNAVAILABLE.into());
            }
        }
    }

    /// stdout EOF 不保证进程已退出，短命目录命令仍需确认退出码并受截止时间约束。
    fn finish(&mut self) -> Result<(), String> {
        self.finish_cancellable(&AtomicBool::new(false))
    }

    fn finish_cancellable(&mut self, cancellation: &AtomicBool) -> Result<(), String> {
        loop {
            if cancellation.load(Ordering::Acquire) {
                return Err("AI 会话启动已取消".into());
            }
            if let Some(status) = self.child.try_wait().map_err(|_| UNAVAILABLE)? {
                return if status.success() {
                    Ok(())
                } else {
                    Err(UNAVAILABLE.into())
                };
            }
            if Instant::now() >= self.deadline {
                return Err("AI_MODEL_CATALOG_TIMEOUT".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

fn read_probe_stream(
    stream: impl std::io::Read + Send + 'static,
    tx: mpsc::SyncSender<Result<String, String>>,
) {
    thread::spawn(move || {
        if read_bounded_lines(stream, |line| tx.send(Ok(line)).is_ok()).is_err() {
            let _ = tx.send(Err(INVALID.into()));
        }
    });
}

impl Drop for Probe {
    fn drop(&mut self) {
        // 先关闭协议输入，给 Windows CLI 包装器和真正的子进程机会沿 EOF 一起退出。
        drop(self.stdin.take());
        for _ in 0..10 {
            if self.child.try_wait().is_ok_and(|status| status.is_some()) {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
