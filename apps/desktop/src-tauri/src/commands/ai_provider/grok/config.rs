//! Grok 模型配置的最小安全投影。
//! 这里只继承当前默认模型的连接参数；可执行认证和其他用户配置不会进入隔离会话。

use std::{ffi::OsString, fs, path::Path};

use toml::{Table, Value};

use super::{original_home, push_secret, write_private_file};

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
) -> Result<Vec<(OsString, OsString)>, String> {
    let source = original_home().map(|home| home.join("config.toml"));
    let source_contents = source
        .as_deref()
        .filter(|path| path.is_file())
        .map(fs::read_to_string)
        .transpose()
        .map_err(|error| format!("读取 Grok 模型配置失败：{error}"))?;
    let environment_default = std::env::var("GROK_DEFAULT_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let (contents, environment) =
        project_model_config(source_contents.as_deref(), environment_default.as_deref())?;
    write_private_file(
        &runtime_directory.join("config.toml"),
        contents.as_bytes(),
        "Grok 隔离配置",
    )?;
    Ok(environment)
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
    const ALLOWED_ENDPOINTS: [&str; 5] = [
        "cli_chat_proxy_base_url",
        "models_base_url",
        "models_list_url",
        "models_endpoint",
        "xai_api_base_url",
    ];
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
mod tests {
    use super::project_model_config;

    #[test]
    fn projects_only_the_default_model_and_moves_inline_secrets_to_environment() {
        let source = r#"
            [models]
            default = "private-model"
            allowed_models = ["private-model", "unused-model"]
            extra_headers = { "X-Global" = "global-secret" }

            [model.private-model]
            model = "upstream-model"
            base_url = "https://example.test/v1"
            api_key = "model-secret"
            extra_headers = { "X-Tenant" = "tenant-secret" }

            [model.unused-model]
            api_key = "must-not-be-copied"

            [mcp_servers.external]
            command = "unsafe-command"
        "#;

        let (projected, environment) = project_model_config(Some(source), None).unwrap();

        assert!(projected.contains("private-model"));
        assert!(!projected.contains("unused-model"));
        assert!(!projected.contains("mcp_servers"));
        assert!(!projected.contains("model-secret"));
        assert!(!projected.contains("tenant-secret"));
        assert!(!projected.contains("global-secret"));
        assert!(!projected.contains("allowed_models"));
        assert_eq!(environment.len(), 3);
        assert!(environment.iter().any(|(_, value)| value == "model-secret"));
    }

    #[test]
    fn projects_only_the_provider_referenced_by_the_default_model() {
        let source = r#"
            [models]
            default = "private-model"

            [model.private-model]
            model = "upstream-model"
            model_provider = "private-provider"

            [model_providers.private-provider]
            base_url = "https://example.test/v1"
            api_key = "provider-secret"

            [model_providers.unused-provider]
            api_key = "unused-secret"
        "#;

        let (projected, environment) = project_model_config(Some(source), None).unwrap();

        assert!(projected.contains("private-provider"));
        assert!(!projected.contains("unused-provider"));
        assert!(!projected.contains("provider-secret"));
        assert_eq!(environment.len(), 1);
    }

    #[test]
    fn invalid_or_command_based_auth_configuration_fails_closed() {
        assert!(project_model_config(Some("not = [valid"), None).is_err());
        let external_auth = r#"
            [models]
            default = "private-model"
            [model.private-model]
            auth_provider = "credential-command"
        "#;
        assert!(project_model_config(Some(external_auth), None).is_err());
    }

    #[test]
    fn preserves_existing_environment_references_and_redacts_inline_values() {
        let source = r#"
            [models]
            default = "private-model"
            [model.private-model]
            api_key = "${EXISTING_API_KEY}"
            extra_headers = { "X-Existing" = "${EXISTING_HEADER}", "X-Inline" = "inline-secret" }
        "#;
        let (projected, environment) = project_model_config(Some(source), None).unwrap();
        assert!(projected.contains("env_key = \"EXISTING_API_KEY\""));
        assert!(projected.contains("X-Existing = \"EXISTING_HEADER\""));
        assert!(projected.contains("X-Inline = \"NOCTERM_GROK_SECRET_1\""));
        assert!(projected.contains("env_http_headers"));
        assert!(!projected.contains("inline-secret"));
        assert_eq!(environment.len(), 1);
    }

    #[test]
    fn header_projection_preserves_empty_literals_and_existing_environment_mappings() {
        let source = r#"
            [models]
            default = "private-model"
            [model.private-model]
            extra_headers = { "X-Empty" = "", "X-Override" = "new-secret" }
            env_http_headers = { "X-Existing" = "EXISTING_ENV", "X-Override" = "OLD_ENV" }
        "#;

        let (projected, environment) = project_model_config(Some(source), None).unwrap();

        assert!(projected.contains("X-Empty = \"\""));
        assert!(projected.contains("X-Existing = \"EXISTING_ENV\""));
        assert!(projected.contains("X-Override = \"NOCTERM_GROK_SECRET_1\""));
        assert!(!projected.contains("OLD_ENV"));
        assert_eq!(environment.len(), 1);
    }

    #[test]
    fn environment_default_projects_the_effective_custom_model() {
        let source = r#"
            [models]
            default = "config-model"

            [model.config-model]
            api_key = "unused-secret"

            [model.environment-model]
            api_key = "selected-secret"
        "#;

        let (projected, environment) =
            project_model_config(Some(source), Some("environment-model")).unwrap();

        assert!(projected.contains("default = \"environment-model\""));
        assert!(projected.contains("[model.environment-model]"));
        assert!(!projected.contains("config-model"));
        assert_eq!(environment.len(), 1);
        assert_eq!(environment[0].1, "selected-secret");
    }

    #[test]
    fn projects_only_inference_endpoints_required_by_the_selected_model() {
        let source = r#"
            [models]
            default = "grok-4.6"
            [endpoints]
            models_base_url = "https://models.example.test/v1"
            models_list_url = "https://models.example.test/catalog"
            feedback_base_url = "https://must-not-be-copied.test"
            trace_upload_credentials = "must-not-be-copied"
        "#;

        let (projected, _) = project_model_config(Some(source), None).unwrap();

        assert!(projected.contains("models_base_url"));
        assert!(projected.contains("models_list_url"));
        assert!(!projected.contains("feedback_base_url"));
        assert!(!projected.contains("trace_upload_credentials"));
    }
}
