//! 按需查询 CLI 的模型目录。探测只发送初始化/目录请求，绝不提交用户消息。
//! IPC 只返回模型标识与受限元数据；协议诊断、凭据与原始进程输出留在边界内。

use std::{
    io::Write,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use crate::dto::ai::{AiModelOption, AiModelSource};

mod claude_catalog;
#[cfg(test)]
mod tests;

use super::{
    ai_process::hide_console_window,
    ai_provider::{
        apply_local_network_bypass, prepare_grok_acp_launch, provider_adapter,
        provider_environment_path, read_grok_model_metadata,
    },
    ai_stream::read_bounded_lines,
    ai_validation::{valid_model_id, valid_reasoning_effort},
    grok_acp_server::{grok_initialize_request, models as grok_models, next_runtime_id},
};

const UNAVAILABLE: &str = "AI_MODEL_CATALOG_UNAVAILABLE";
const INVALID: &str = "AI_MODEL_CATALOG_INVALID";
const LIMIT: usize = 1024;

/// 执行一次目录探测。面板负责预取、复用结果与合并同一 Agent 的并发请求。
#[tauri::command]
pub async fn ai_provider_models(provider: String) -> Result<Vec<AiModelOption>, String> {
    tauri::async_runtime::spawn_blocking(move || discover_models(&provider))
        .await
        .map_err(|_| UNAVAILABLE.to_string())?
}

fn discover_models(provider: &str) -> Result<Vec<AiModelOption>, String> {
    let adapter = provider_adapter(provider).ok_or("AI_PROVIDER_UNKNOWN")?;
    let executable = adapter.executable().ok_or("AI_PROVIDER_NOT_INSTALLED")?;
    let mut command = Command::new(&executable);
    if let Some(path) = provider_environment_path(&executable) {
        command.env("PATH", path);
    }
    hide_console_window(&mut command);
    apply_local_network_bypass(&mut command);
    command.current_dir(std::env::temp_dir());
    let mut models = Vec::new();
    match adapter.id() {
        "codex" => {
            let mut probe = Probe::start(command.args(["app-server", "--stdio"]), false)?;
            probe.send(json!({"id":1,"method":"initialize","params":{"clientInfo":{
                "name":"nocterm","version":env!("NOCTERM_PRODUCT_VERSION")}}}))?;
            probe.response(1)?;
            probe.send(json!({"method":"initialized"}))?;
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
            append_configured_grok_models(
                &mut models,
                read_grok_model_metadata().map_err(|_| INVALID)?,
            )?;
        }
        _ => return Err("AI_PROVIDER_UNKNOWN".into()),
    }
    Ok(models)
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
            }
        }
    }
    Ok(())
}

fn append_claude_models(models: &mut Vec<AiModelOption>, result: &Value) -> Result<(), String> {
    for item in result["models"].as_array().ok_or(INVALID)? {
        if item["value"] != "default" {
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
    let line = probe.next_line()?.ok_or(INVALID)?;
    probe.finish()?;
    let version = line.split_whitespace().next().ok_or(INVALID)?;
    claude_catalog::parse_version(version).ok_or(INVALID)?;
    Ok(version.to_string())
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
        let time = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or("AI_MODEL_CATALOG_TIMEOUT")?;
        match self.lines.recv_timeout(time) {
            Ok(line) => line.map(Some),
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(None),
            Err(mpsc::RecvTimeoutError::Timeout) => Err("AI_MODEL_CATALOG_TIMEOUT".into()),
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
        loop {
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
