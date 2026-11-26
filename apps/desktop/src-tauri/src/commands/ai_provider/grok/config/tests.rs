use super::project_model_config;

#[test]
fn catalog_metadata_includes_non_default_aliases_without_connection_details() {
    let source = r#"
        [models]
        default = "first"
        [model.first]
        name = "First"
        model = "upstream-first"
        [model.second]
        name = "Second"
        model = "upstream-second"
        api_key = "not-for-the-catalog"
        base_url = "https://private.invalid"
        auth_provider = "must-not-run"
        [model.unnamed]
        model = "upstream-unnamed"
        [model_providers.hidden]
        api_key = "provider-secret"
    "#;
    let metadata = super::configured_catalog_metadata_with_inputs(Some(source), None, false)
        .unwrap()
        .models;
    assert_eq!(
        metadata,
        vec![
            ("first".into(), Some("First".into())),
            ("second".into(), Some("Second".into())),
            ("unnamed".into(), None),
        ]
    );
    // 目录枚举不放宽启动边界：会话仍只投影所选模型，外部认证仍明确拒绝。
    let (runtime, _) = project_model_config(Some(source), None).unwrap();
    assert!(!runtime.contains("[model.second]"));
    assert!(project_model_config(Some(source), Some("second")).is_err());
}

#[test]
fn catalog_metadata_handles_missing_and_invalid_configuration_explicitly() {
    for source in [None, Some("[models]\ndefault = 'built-in'")] {
        assert!(
            super::configured_catalog_metadata_with_inputs(source, None, false)
                .unwrap()
                .models
                .is_empty()
        );
    }
    for source in [
        "invalid = [",
        "model = 'invalid'",
        "[model]\nentry = 'invalid'",
    ] {
        assert!(super::configured_catalog_metadata_with_inputs(Some(source), None, false).is_err());
    }
}

#[test]
fn catalog_metadata_classifies_only_the_effective_default_route() {
    let direct = r#"
        [models]
        default = "custom"
        [model.custom]
        base_url = "https://gateway.example.test/v1"
        [model.other]
    "#;
    assert!(
        super::configured_catalog_metadata_with_inputs(Some(direct), None, false)
            .unwrap()
            .uses_custom_endpoint
    );

    let provider = r#"
        [models]
        default = "custom"
        [model.custom]
        model_provider = "gateway"
        [model_providers.gateway]
        base_url = "https://gateway.example.test/v1"
    "#;
    assert!(
        super::configured_catalog_metadata_with_inputs(Some(provider), None, false)
            .unwrap()
            .uses_custom_endpoint
    );

    let non_default = r#"
        [models]
        default = "native"
        [model.native]
        [model.custom]
        base_url = "https://gateway.example.test/v1"
    "#;
    assert!(
        !super::configured_catalog_metadata_with_inputs(Some(non_default), None, false)
            .unwrap()
            .uses_custom_endpoint
    );
    assert!(
        super::configured_catalog_metadata_with_inputs(Some(non_default), Some("custom"), false)
            .unwrap()
            .uses_custom_endpoint
    );

    let global = r#"
        [models]
        default = "native"
        [endpoints]
        xai_api_base_url = "https://gateway.example.test/v1"
    "#;
    assert!(
        super::configured_catalog_metadata_with_inputs(Some(global), None, false)
            .unwrap()
            .uses_custom_endpoint
    );
}

#[test]
fn endpoint_classification_requires_a_valid_non_official_http_route() {
    for endpoint in [
        "https://api.x.ai/v1",
        "https://cli-chat-proxy.grok.com/v1",
        "not-a-url",
        "",
    ] {
        assert!(!super::is_custom_grok_endpoint(endpoint), "{endpoint}");
    }
    for endpoint in [
        "http://127.0.0.1:3000/v1",
        "https://gateway.example.test/v1",
    ] {
        assert!(super::is_custom_grok_endpoint(endpoint), "{endpoint}");
    }

    let official = r#"
        [models]
        default = "native"
        [model.native]
        base_url = "https://api.x.ai/v1"
    "#;
    assert!(
        !super::configured_catalog_metadata_with_inputs(Some(official), None, false)
            .unwrap()
            .uses_custom_endpoint
    );
}

#[test]
fn explicit_selection_projects_only_the_requested_model() {
    let source = r#"
        [models]
        default = "first"
        [model.first]
        model = "first-upstream"
        [model.second]
        model = "second-upstream"
    "#;
    let (projected, _) = project_model_config(Some(source), Some("second")).unwrap();
    let config: toml::Value = toml::from_str(&projected).unwrap();
    assert_eq!(config["models"]["default"].as_str(), Some("second"));
    assert!(config["model"].get("first").is_none());
    assert_eq!(
        config["model"]["second"]["model"].as_str(),
        Some("second-upstream")
    );
}

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
