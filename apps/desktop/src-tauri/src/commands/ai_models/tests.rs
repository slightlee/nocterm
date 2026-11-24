use super::*;
use crate::dto::ai::AiModelUnavailableReason;

#[test]
fn configured_grok_models_fill_missing_ids_without_overwriting_probed_capabilities() {
    let mut models = Vec::new();
    append_grok_models(
        &mut models,
        &json!({"protocolVersion":1,"_meta":{"modelState":{
            "availableModels":[{"modelId":"first","name":"CLI First","_meta":{
                "reasoningEfforts":[{"id":"high","default":true}]
            }}]
        }}}),
    )
    .unwrap();
    append_configured_grok_models(
        &mut models,
        vec![
            ("first".into(), Some("Local First".into())),
            ("second-alias".into(), Some("Second".into())),
            ("second-alias".into(), None),
            ("https://invalid.example".into(), None),
            ("unnamed".into(), Some("invalid\nname".into())),
        ],
    )
    .unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["first", "second-alias", "unnamed"]
    );
    assert_eq!(models[0].name, "CLI First");
    assert_eq!(models[0].reasoning_efforts, ["high"]);
    assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(models[1].name, "Second");
    assert!(models[1].reasoning_efforts.is_empty());
    assert!(models[1].default_reasoning_effort.is_none());
    assert_eq!(models[2].name, "unnamed");
}

#[test]
fn reasoning_capabilities_are_provider_specific_and_bounded() {
    let mut models = vec![];
    append_codex_models(&mut models, &json!({"data":[{"model":"test", "supportedReasoningEfforts":[
        {"reasoningEffort":"high"}, {"reasoningEffort":"low"}, {"reasoningEffort":"high"}, {"reasoningEffort":"ultra"}, {"reasoningEffort":"invented"}
    ], "defaultReasoningEffort":"low"}]})).unwrap();
    assert_eq!(models[0].reasoning_efforts, ["low", "high", "ultra"]);
    assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("low"));
    append_claude_models(&mut models, &json!({"models":[
        {"value":"opus","supportsEffort":true,"supportedEffortLevels":["max","high","ultra","ultrathink"]},
        {"value":"haiku","supportsEffort":false,"supportedEffortLevels":["high"]}
    ]})).unwrap();
    assert_eq!(models[1].reasoning_efforts, ["high", "max"]);
    assert!(models[1].default_reasoning_effort.is_none());
    assert!(models[2].reasoning_efforts.is_empty());
}

#[test]
fn builtin_effort_catalog_does_not_invent_levels_for_older_models_or_cli() {
    let mut models = vec![];
    claude_catalog::append_versions(&mut models, Some("2.1.284")).unwrap();
    let levels = |id: &str| {
        &models
            .iter()
            .find(|m| m.id == id)
            .unwrap()
            .reasoning_efforts
    };
    assert!(levels("claude-haiku-4-5").is_empty());
    assert_eq!(levels("claude-sonnet-4-6"), &["low", "medium", "high"]);
    assert!(!levels("claude-opus-4-7").iter().any(|s| s == "xhigh"));
    let mut older = vec![];
    claude_catalog::append_versions(&mut older, Some("2.1.100")).unwrap();
    assert!(older.iter().all(|m| m.reasoning_efforts.is_empty()));
    assert!(older.iter().all(|m| m.default_reasoning_effort.is_none()));
    assert!(
        models
            .iter()
            .find(|m| m.id == "claude-fable-5-2")
            .unwrap()
            .default_reasoning_effort
            .is_none()
    );
}

#[test]
fn claude_catalog_includes_documented_compatible_defaults() {
    let mut models = vec![];
    claude_catalog::append_versions(&mut models, Some("2.1.330")).unwrap();
    for id in [
        "claude-fable-5-2",
        "claude-opus-5-5",
        "claude-sonnet-5-5",
        "claude-haiku-5-5",
    ] {
        let model = models.iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            model.default_reasoning_effort.as_deref(),
            Some("medium"),
            "{id}"
        );
    }
    let opus = models.iter().find(|m| m.id == "claude-opus-5").unwrap();
    assert_eq!(opus.default_reasoning_effort.as_deref(), Some("high"));
    let opus = models.iter().find(|m| m.id == "claude-opus-4-7").unwrap();
    assert!(opus.default_reasoning_effort.is_none());
    assert!(models.iter().all(|model| {
        model
            .default_reasoning_effort
            .as_ref()
            .is_none_or(|effort| model.reasoning_efforts.contains(effort))
    }));
}

#[test]
fn codex_preserves_display_order_and_skips_hidden() {
    let mut models = vec![];
    append_codex_models(
        &mut models,
        &json!({"data":[
            {"model":"gpt-b","displayName":"GPT B"},
            {"model":"hidden","hidden":true}, {"id":"gpt-a"}, {"model":"gpt-b"}
        ]}),
    )
    .unwrap();
    assert_eq!(
        models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        ["gpt-b", "gpt-a"]
    );
    assert_eq!(models[0].name, "GPT B");
}

#[test]
fn claude_keeps_aliases_even_when_they_resolve_to_same_model() {
    let mut models = vec![];
    append_claude_models(
        &mut models,
        &json!({"models":[
            {"value":"default"},
            {"value":"opus","displayName":"Mapped Model","resolvedModel":"mapped-model"},
            {"value":"sonnet","displayName":"Mapped Model","resolvedModel":"mapped-model"},
            {"value":"claude-opus-5-5","displayName":"Local Name"}
        ]}),
    )
    .unwrap();
    claude_catalog::append_versions(&mut models, Some("2.1.284")).unwrap();
    assert_eq!(models[0].id, "opus");
    assert_eq!(models[1].id, "sonnet");
    assert_eq!(models[0].resolved_model.as_deref(), Some("mapped-model"));
    assert_eq!(
        models.iter().filter(|m| m.id == "claude-opus-5-5").count(),
        1
    );
    assert_eq!(models[2].name, "Local Name");
    assert!(models[2].default_reasoning_effort.is_none());
    assert!(
        models
            .iter()
            .find(|m| m.id == "claude-sonnet-5-5")
            .unwrap()
            .unavailable_reason
            .is_none()
    );
    assert_eq!(
        models
            .iter()
            .find(|m| m.id == "claude-fable-5-2")
            .unwrap()
            .unavailable_reason,
        Some(AiModelUnavailableReason::CliUpdateRequired)
    );
}

#[test]
fn catalog_requires_numeric_versions_and_marks_unknown_compatibility() {
    assert!(claude_catalog::parse_version("2.1.99") < claude_catalog::parse_version("2.1.284"));
    for invalid in ["2.1", "2.1.284-beta", "2.1.284.0", "+2.1.284"] {
        assert!(claude_catalog::parse_version(invalid).is_none());
    }
    let mut models = vec![];
    claude_catalog::append_versions(&mut models, None).unwrap();
    // 与核对过的上游 current/legacy 分类一致，历史项仍保留完整调用信息。
    assert_eq!(
        models
            .iter()
            .filter(|m| !m.is_legacy)
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        [
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
            "claude-fable-5-2"
        ]
    );
    assert_eq!(models.iter().filter(|m| m.is_legacy).count(), 10);
    let ids = models
        .iter()
        .map(|m| &m.id)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), models.len());
    for model in models {
        assert!(valid_model_id(&model.id));
        if let Some(minimum) = model.minimum_cli_version {
            assert!(claude_catalog::parse_version(&minimum).is_some());
            assert_eq!(
                model.unavailable_reason,
                Some(AiModelUnavailableReason::CliVersionUnknown)
            );
        } else {
            assert!(model.unavailable_reason.is_none());
        }
    }
}

#[test]
fn metadata_is_bounded_and_never_accepts_an_endpoint_as_model() {
    let mut models = vec![];
    add(&mut models, Some("https://example.invalid"), None, None).unwrap();
    assert!(models.is_empty());
    add(
        &mut models,
        Some("model"),
        Some("unsafe\nname"),
        Some("invalid value"),
    )
    .unwrap();
    assert_eq!(models[0].name, "model");
    assert!(models[0].resolved_model.is_none());
    assert!(append_claude_models(&mut models, &json!({"models":null})).is_err());
}

#[test]
fn grok_initialize_exports_only_supported_options_and_explicit_defaults() {
    let mut models = Vec::new();
    append_grok_models(
        &mut models,
        &json!({"protocolVersion":1,"_meta":{"modelState":{"availableModels":[
            {"modelId":"grok-test","name":"Grok Test","_meta":{
                "reasoningEffort":"high","reasoningEfforts":[
                    {"value":"high"},{"value":"low","default":true},{"id":"high"},
                    {"value":"unknown"},{"value":"rejected","id":"medium"},{"id":"ultra"}
                ]
            }},
            {"modelId":"only-current","_meta":{"reasoningEffort":"max","reasoningEfforts":[{"id":"max"}]}},
            {"modelId":"no-effort","_meta":{"supportsReasoningEffort":false,"reasoningEfforts":[{"id":"low"}]}},
            {"modelId":"grok-test","name":"Duplicate"},
            {"modelId":"https://example.invalid"}
        ]}}}),
    )
    .unwrap();
    assert_eq!(models.len(), 3);
    assert_eq!(models[0].name, "Grok Test");
    assert_eq!(models[0].reasoning_efforts, ["low", "medium", "high"]);
    assert_eq!(models[0].default_reasoning_effort.as_deref(), Some("low"));
    assert_eq!(models[1].reasoning_efforts, ["max"]);
    assert!(models[1].default_reasoning_effort.is_none());
    assert!(models[2].reasoning_efforts.is_empty());
}

#[test]
fn grok_missing_model_state_keeps_cli_fallback_available() {
    for result in [
        json!({"protocolVersion":1}),
        json!({"protocolVersion":1,"_meta":{"modelState":{"availableModels":[]}}}),
        json!({"protocolVersion":2,"_meta":{"modelState":{"availableModels":[{"modelId":"test"}]}}}),
    ] {
        let mut models = Vec::new();
        append_grok_models(&mut models, &result).unwrap();
        assert!(models.is_empty());
    }
}

/// 手动验证本机协议，只输出受限目录元数据，不发送推理消息或打印账号信息。
#[test]
#[ignore = "requires installed local provider CLIs"]
fn local_catalog_probe() {
    for provider in ["codex", "claude-code", "grok"] {
        let models = discover_models(provider).expect(provider);
        println!("{provider}: {}", serde_json::to_string(&models).unwrap());
        assert!(!models.is_empty());
    }
}
