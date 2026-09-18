use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    AGENT_PROFILE_HEADER, MODEL_ENVIRONMENT_ALLOWLIST, PreparedGrokAcpLaunch,
    RUNTIME_ENVIRONMENT_ALLOWLIST, agent_profile,
};

#[test]
fn agent_profile_exposes_only_mcp_aggregation_tools() {
    let profile = agent_profile();
    assert!(AGENT_PROFILE_HEADER.contains("tools: [search_tool, use_tool]"));
    assert!(profile.contains("agents_md: false"));
    assert!(profile.contains("smallest sufficient set of calls"));
    assert!(profile.contains("make one precise search_tool call"));
    assert!(profile.contains("never\nrepeat discovery"));
    assert!(!profile.contains("run_terminal_cmd"));
}

#[test]
fn isolated_launch_hides_generic_user_configuration_roots() {
    let runtime = std::env::temp_dir().join("nocterm-grok-environment-contract");
    let prepared = PreparedGrokAcpLaunch {
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
