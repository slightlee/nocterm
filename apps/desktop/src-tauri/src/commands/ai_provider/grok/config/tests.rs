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
