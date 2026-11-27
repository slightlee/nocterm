//! Grok 模型配置的最小安全投影。
//! 这里只继承当前默认模型的连接参数；可执行认证和其他用户配置不会进入隔离会话。

use std::{ffi::OsString, fs, path::Path};

use toml::{Table, Value};

use super::{original_home, push_secret, replace_private_file};
use crate::commands::ai_provider::http_endpoint_host;

const ALLOWED_ENDPOINTS: [&str; 5] = [
    "cli_chat_proxy_base_url",
    "models_base_url",
    "models_list_url",
    "models_endpoint",
    "xai_api_base_url",
];
const ENDPOINT_ENVIRONMENT: [&str; 4] = [
    "GROK_MODELS_BASE_URL",
    "GROK_MODELS_LIST_URL",
    "GROK_XAI_API_BASE_URL",
    "GROK_CLI_CHAT_PROXY_BASE_URL",
];

const ISOLATED_CONFIG: &str = r#"
disable_web_search = true

[cli]
auto_update = false
session_registry = false
use_leader = false

[features]
backend_tools = false
campaigns = false
codebase_indexing = false
feedback = false
lsp_tools = false
managed_config = false
mcp_recursive_config_watch = false
remote_fetch = false
telemetry = false

[session]
load_envrc = false

[memory]
enabled = false

[memory_v2]
enabled = false

[compat.cursor]
skills = false
rules = false
agents = false
mcps = false
hooks = false
sessions = false

[compat.claude]
skills = false
rules = false
agents = false
mcps = false
hooks = false
sessions = false

[compat.codex]
hooks = false
skills = false
sessions = false
"#;

/// 内联凭据只进入子进程环境；生成的 TOML 不保存模型密钥或认证 Header。
pub(super) fn create_runtime_config(
    runtime_directory: &Path,
    selected_model: Option<&str>,
) -> Result<Vec<(OsString, OsString)>, String> {
    let source_contents = read_source_config()?;
    let environment_default = std::env::var("GROK_DEFAULT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let (contents, environment) = project_model_config(
        source_contents.as_deref(),
        selected_model.or(environment_default.as_deref()),
    )?;
    replace_private_file(
        &runtime_directory.join("config.toml"),
        contents.as_bytes(),
        "Grok 隔离配置",
    )?;
    Ok(environment)
}

fn read_source_config() -> Result<Option<String>, String> {
    let source = original_home().map(|home| home.join("config.toml"));
    source
        .as_deref()
        .filter(|path| path.is_file())
        .map(fs::read_to_string)
        .transpose()
        .map_err(|error| format!("读取 Grok 模型配置失败：{error}"))
}

pub(crate) struct GrokCatalogMetadata {
    pub models: Vec<(String, Option<String>)>,
    pub uses_custom_endpoint: bool,
}

/// 目录只带出模型标识、名称和端点分类；URL、凭据与可执行认证留在配置边界内。
pub(crate) fn read_grok_model_metadata() -> Result<GrokCatalogMetadata, String> {
    let source = read_source_config()?;
    let environment_default = std::env::var("GROK_DEFAULT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    configured_catalog_metadata_with_inputs(
        source.as_deref(),
        environment_default.as_deref(),
        endpoint_environment_is_configured(),
    )
}

/// 不把上游 model 当作调用 ID；端点摘要只描述当前有效默认模型的连接方式。
fn configured_catalog_metadata_with_inputs(
    source: Option<&str>,
    environment_default: Option<&str>,
    environment_endpoint: bool,
) -> Result<GrokCatalogMetadata, String> {
    let Some(source) = source else {
        return Ok(GrokCatalogMetadata {
            models: Vec::new(),
            uses_custom_endpoint: environment_endpoint,
        });
    };
    let config: Value = toml::from_str(source)
        .map_err(|_| "Grok config.toml 格式无效，请先修正本机 Grok 配置".to_string())?;
    let default_model = environment_default.or_else(|| {
        config
            .get("models")
            .and_then(Value::as_table)
            .and_then(|models| models.get("default"))
            .and_then(Value::as_str)
    });
    let uses_custom_endpoint =
        environment_endpoint || configured_route_uses_custom_endpoint(&config, default_model);
    let Some(models) = config.get("model") else {
        return Ok(GrokCatalogMetadata {
            models: Vec::new(),
            uses_custom_endpoint,
        });
    };
    let models = models.as_table().ok_or("Grok model 必须是 TOML 表")?;
    let models = models
        .iter()
        .map(|(id, model)| {
            let model = model.as_table().ok_or("Grok 模型配置必须是 TOML 表")?;
            Ok((
                id.clone(),
                model
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(GrokCatalogMetadata {
        models,
        uses_custom_endpoint,
    })
}

fn configured_route_uses_custom_endpoint(config: &Value, default_model: Option<&str>) -> bool {
    let global_endpoint = config
        .get("endpoints")
        .and_then(Value::as_table)
        .is_some_and(|endpoints| {
            ALLOWED_ENDPOINTS
                .iter()
                .any(|key| custom_endpoint_value(endpoints.get(*key)))
        });
    if global_endpoint {
        return true;
    }
    let Some(model) = default_model
        .and_then(|id| config.get("model")?.as_table()?.get(id))
        .and_then(Value::as_table)
    else {
        return false;
    };
    if custom_endpoint_value(model.get("base_url")) {
        return true;
    }
    model
        .get("model_provider")
        .and_then(Value::as_str)
        .and_then(|provider| config.get("model_providers")?.as_table()?.get(provider))
        .and_then(Value::as_table)
        .is_some_and(|provider| custom_endpoint_value(provider.get("base_url")))
}

fn endpoint_environment_is_configured() -> bool {
    ENDPOINT_ENVIRONMENT.iter().any(|name| {
        std::env::var(name)
            .ok()
            .is_some_and(|value| is_custom_grok_endpoint(&value))
    })
}

fn custom_endpoint_value(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(is_custom_grok_endpoint)
}

/// 显式填写官方默认地址仍属于官方路由；无效字符串不作为接入方式展示。
fn is_custom_grok_endpoint(value: &str) -> bool {
    http_endpoint_host(value)
        .is_some_and(|host| host != "api.x.ai" && host != "cli-chat-proxy.grok.com")
}

fn project_model_config(
    source: Option<&str>,
    environment_default: Option<&str>,
) -> Result<(String, Vec<(OsString, OsString)>), String> {
    let mut projected = toml::from_str::<Value>(ISOLATED_CONFIG)
        .map_err(|_| "Nocterm 内置 Grok 隔离配置无效".to_string())?
        .as_table()
        .cloned()
        .ok_or_else(|| "Nocterm 内置 Grok 隔离配置格式无效".to_string())?;
    let Some(source) = source else {
        return serialize_projected_config(projected, Vec::new());
    };
    let source = toml::from_str::<Value>(source)
        .map_err(|_| "Grok config.toml 格式无效，请先修正本机 Grok 配置".to_string())?;
    let source = source
        .as_table()
        .ok_or_else(|| "Grok config.toml 顶层必须是 TOML 表".to_string())?;
    let mut environment = Vec::new();

    project_endpoints(source, &mut projected)?;

    if let Some(models) = source.get("models").and_then(Value::as_table) {
        let mut models = models.clone();
        models.remove("allowed_models");
        if let Some(environment_default) = environment_default {
            models.insert(
                "default".into(),
                Value::String(environment_default.to_string()),
            );
        }
        extract_table_secrets(&mut models, &mut environment)?;
        capture_environment_references(&models, &mut environment);
        projected.insert("models".into(), Value::Table(models));
    }

    let default_model = environment_default.or_else(|| {
        source
            .get("models")
            .and_then(Value::as_table)
            .and_then(|models| models.get("default"))
            .and_then(Value::as_str)
    });
    if let Some(default_model) = default_model
        && let Some(model) = source
            .get("model")
            .and_then(Value::as_table)
            .and_then(|models| models.get(default_model))
            .and_then(Value::as_table)
    {
        let mut model = model.clone();
        reject_external_auth(&model, "模型")?;
        let provider = model
            .get("model_provider")
            .and_then(Value::as_str)
            .map(str::to_string);
        extract_table_secrets(&mut model, &mut environment)?;
        capture_environment_references(&model, &mut environment);
        projected.insert(
            "model".into(),
            Value::Table(Table::from_iter([(
                default_model.to_string(),
                Value::Table(model),
            )])),
        );
        if let Some(provider) = provider {
            let provider_config = source
                .get("model_providers")
                .and_then(Value::as_table)
                .and_then(|providers| providers.get(&provider))
                .and_then(Value::as_table)
                .ok_or_else(|| format!("Grok 模型引用的 model_provider 不存在：{provider}"))?;
            let mut provider_config = provider_config.clone();
            reject_external_auth(&provider_config, "模型 Provider")?;
            extract_table_secrets(&mut provider_config, &mut environment)?;
            capture_environment_references(&provider_config, &mut environment);
            projected.insert(
                "model_providers".into(),
                Value::Table(Table::from_iter([(
                    provider,
                    Value::Table(provider_config),
                )])),
            );
        }
    }
    serialize_projected_config(projected, environment)
}

/// 只继承模型请求实际使用的端点，反馈、遥测、托管配置和部署凭据继续保持关闭。
fn project_endpoints(source: &Table, projected: &mut Table) -> Result<(), String> {
    let Some(endpoints) = source.get("endpoints").and_then(Value::as_table) else {
        return Ok(());
    };
    let mut selected = Table::new();
    for key in ALLOWED_ENDPOINTS {
        if let Some(value) = endpoints.get(key) {
            if !value.is_str() {
                return Err(format!("Grok endpoints.{key} 必须是字符串"));
            }
            selected.insert(key.to_string(), value.clone());
        }
    }
    if !selected.is_empty() {
        projected.insert("endpoints".into(), Value::Table(selected));
    }
    Ok(())
}

fn serialize_projected_config(
    projected: Table,
    environment: Vec<(OsString, OsString)>,
) -> Result<(String, Vec<(OsString, OsString)>), String> {
    toml::to_string(&Value::Table(projected))
        .map(|contents| (contents, environment))
        .map_err(|error| format!("生成 Grok 隔离配置失败：{error}"))
}

/// 外部认证 Provider 会执行用户命令，无法作为只读配置继承，必须 fail closed。
fn reject_external_auth(table: &Table, label: &str) -> Result<(), String> {
    if table.contains_key("auth_provider") {
        return Err(format!(
            "{label}使用外部 auth_provider，Nocterm 无法在不执行用户命令的前提下安全继承"
        ));
    }
    Ok(())
}

fn extract_table_secrets(
    table: &mut Table,
    environment: &mut Vec<(OsString, OsString)>,
) -> Result<(), String> {
    if let Some(secret) = table.remove("api_key") {
        let secret = secret
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "Grok 模型 api_key 必须是非空字符串".to_string())?;
        let variable = environment_reference_name(secret)
            .map(str::to_string)
            .unwrap_or_else(|| push_secret(environment, secret));
        table.insert("env_key".into(), Value::String(variable));
    }
    if let Some(headers) = table.remove("extra_headers") {
        let headers = headers
            .as_table()
            .ok_or_else(|| "Grok 模型 extra_headers 必须是 TOML 表".to_string())?;
        let mut environment_headers = table
            .remove("env_http_headers")
            .map(|headers| {
                headers
                    .as_table()
                    .cloned()
                    .ok_or_else(|| "Grok 模型 env_http_headers 必须是 TOML 表".to_string())
            })
            .transpose()?
            .unwrap_or_default();
        let mut literal_headers = Table::new();
        for (name, value) in headers {
            let secret = value
                .as_str()
                .ok_or_else(|| "Grok 模型 extra_headers 的值必须是字符串".to_string())?;
            if secret.is_empty() {
                literal_headers.insert(name.clone(), Value::String(String::new()));
                continue;
            }
            let variable = environment_reference_name(secret)
                .map(str::to_string)
                .unwrap_or_else(|| push_secret(environment, secret));
            // Grok 不对 extra_headers 做 ${VAR} 插值；动态值必须走官方 env_http_headers。
            environment_headers.insert(name.clone(), Value::String(variable));
        }
        if !literal_headers.is_empty() {
            table.insert("extra_headers".into(), Value::Table(literal_headers));
        }
        if !environment_headers.is_empty() {
            table.insert("env_http_headers".into(), Value::Table(environment_headers));
        }
    }
    Ok(())
}

/// `env_key` 和 `env_http_headers` 只声明变量名；隔离进程必须显式复制被引用的值。
fn capture_environment_references(table: &Table, environment: &mut Vec<(OsString, OsString)>) {
    if let Some(env_key) = table.get("env_key") {
        match env_key {
            Value::String(name) => push_current_environment(environment, name),
            Value::Array(names) => {
                for name in names.iter().filter_map(Value::as_str) {
                    push_current_environment(environment, name);
                }
            }
            _ => {}
        }
    }
    if let Some(headers) = table.get("env_http_headers").and_then(Value::as_table) {
        for name in headers.values().filter_map(Value::as_str) {
            push_current_environment(environment, name);
        }
    }
}

fn push_current_environment(environment: &mut Vec<(OsString, OsString)>, name: &str) {
    if name.is_empty() || environment.iter().any(|(current, _)| current == name) {
        return;
    }
    if let Some(value) = std::env::var_os(name) {
        environment.push((OsString::from(name), value));
    }
}

fn environment_reference_name(value: &str) -> Option<&str> {
    let name = value.strip_prefix("${")?.strip_suffix('}')?;
    (!name.is_empty()
        && !name.contains('=')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_'))
    .then_some(name)
}

#[cfg(test)]
mod tests;
