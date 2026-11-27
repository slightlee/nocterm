use std::time::{SystemTime, UNIX_EPOCH};

use super::{
    AGENT_PROFILE_HEADER, MODEL_ENVIRONMENT_ALLOWLIST, PreparedGrokAcpLaunch,
    RUNTIME_ENVIRONMENT_ALLOWLIST, agent_profile, cleanup_grok_conversation,
    cleanup_stale_grok_authentication_links, prepare_grok_conversation_launch,
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
        cleanup: std::sync::Mutex::new(None),
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
        cleanup: std::sync::Mutex::new(None),
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
        cleanup: std::sync::Mutex::new(Some(super::GrokLaunchCleanup {
            paths: vec![runtime.clone()],
            parent: None,
            _conversation_lock: None,
        })),
    };

    prepared.cleanup();
    prepared.cleanup();

    assert!(!runtime.exists());
}

#[test]
fn conversation_runtime_survives_process_cleanup_until_the_conversation_is_reset() {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let app_data = std::env::temp_dir().join(format!(
        "nocterm-grok-persistent-contract-{}-{timestamp}",
        std::process::id()
    ));
    let first = prepare_grok_conversation_launch(&app_data, "conversation-one", None)
        .expect("prepare persistent Grok launch");
    let runtime = first.current_directory.clone();
    std::fs::write(runtime.join("provider-session-state"), b"retained").unwrap();

    first.cleanup();
    drop(first);

    assert_eq!(
        std::fs::read(runtime.join("provider-session-state")).unwrap(),
        b"retained"
    );
    assert!(
        cleanup_grok_conversation(&app_data, "conversation-one").expect("reset conversation state")
    );
    assert!(!runtime.exists());
    assert!(
        !cleanup_grok_conversation(&app_data, "conversation-one")
            .expect("idempotent conversation cleanup")
    );
}

#[test]
fn startup_cleanup_removes_only_stale_authentication_projections() {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let app_data = std::env::temp_dir().join(format!(
        "nocterm-grok-auth-cleanup-{}-{timestamp}",
        std::process::id()
    ));
    let runtime = app_data
        .join("ai-provider-state")
        .join("grok")
        .join("conversations")
        .join("conversation-one");
    std::fs::create_dir_all(&runtime).unwrap();
    std::fs::write(runtime.join("auth.json"), b"stale-secret").unwrap();
    std::fs::write(runtime.join("provider-session-state"), b"retained").unwrap();

    cleanup_stale_grok_authentication_links(&app_data).expect("clean stale authentication");

    assert!(!runtime.join("auth.json").exists());
    assert_eq!(
        std::fs::read(runtime.join("provider-session-state")).unwrap(),
        b"retained"
    );
    std::fs::remove_dir_all(app_data).unwrap();
}

#[test]
fn conversation_cleanup_is_scoped_to_one_validated_directory() {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let app_data = std::env::temp_dir().join(format!(
        "nocterm-grok-cleanup-scope-{}-{timestamp}",
        std::process::id()
    ));
    let first = prepare_grok_conversation_launch(&app_data, "conversation-one", None)
        .expect("prepare first conversation");
    let second = prepare_grok_conversation_launch(&app_data, "conversation-two", None)
        .expect("prepare second conversation");
    let second_runtime = second.current_directory.clone();

    first.cleanup();
    cleanup_grok_conversation(&app_data, "conversation-one").expect("remove first conversation");

    assert!(!first.current_directory.exists());
    assert!(second_runtime.exists());
    assert!(cleanup_grok_conversation(&app_data, "../outside").is_err());
    second.cleanup();
    cleanup_grok_conversation(&app_data, "conversation-two").expect("remove second conversation");
}

fn private_test_directory(label: &str) -> std::path::PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "nocterm-grok-{label}-{}-{timestamp}",
        std::process::id()
    ))
}

/// 使用合成认证内容验证所有权，测试不读取或复制用户凭据。
fn owned_launch(app_data: &std::path::Path) -> PreparedGrokAcpLaunch {
    let ownership = super::try_lock_conversation(app_data, "conversation")
        .unwrap()
        .unwrap();
    let runtime = super::conversation_runtime_directory(app_data, "conversation").unwrap();
    std::fs::create_dir_all(&runtime).unwrap();
    let auth = runtime.join("auth.json");
    std::fs::write(&auth, b"test-credential").unwrap();
    PreparedGrokAcpLaunch {
        args: Vec::new(),
        environment: Vec::new(),
        current_directory: runtime,
        secret_environment_keys: Vec::new(),
        cleanup: std::sync::Mutex::new(Some(super::GrokLaunchCleanup {
            paths: vec![auth],
            parent: None,
            _conversation_lock: Some(ownership),
        })),
    }
}

#[test]
fn delayed_old_cleanup_cannot_remove_the_next_launch_authentication() {
    let app_data = private_test_directory("owner-replacement");
    let first = owned_launch(&app_data);
    first.cleanup();
    let next = owned_launch(&app_data);
    first.cleanup();
    drop(first);
    assert_eq!(
        std::fs::read(next.current_directory.join("auth.json")).unwrap(),
        b"test-credential"
    );
    drop(next);
    std::fs::remove_dir_all(app_data).unwrap();
}

#[test]
fn startup_and_reset_preserve_authentication_owned_by_a_live_process() {
    let app_data = private_test_directory("live-owner");
    let active = owned_launch(&app_data);
    let auth = active.current_directory.join("auth.json");
    cleanup_stale_grok_authentication_links(&app_data).unwrap();
    assert!(auth.exists());
    assert!(cleanup_grok_conversation(&app_data, "conversation").is_err());
    assert!(auth.exists());
    assert!(
        super::try_lock_conversation(&app_data, "conversation")
            .unwrap()
            .is_none()
    );
    active.cleanup();
    std::fs::write(&auth, b"orphaned-test-credential").unwrap();
    cleanup_stale_grok_authentication_links(&app_data).unwrap();
    assert!(!auth.exists());
    drop(active);
    std::fs::remove_dir_all(app_data).unwrap();
}

#[test]
fn conversation_lock_excludes_another_host_process() {
    let app_data = private_test_directory("cross-process-lock");
    let active = owned_launch(&app_data);
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "commands::ai_provider::grok::tests::child_checks_conversation_lock",
            "--nocapture",
        ])
        .env("NOCTERM_TEST_GROK_LOCK_ROOT", &app_data)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(active.current_directory.join("auth.json").exists());
    drop(active);
    std::fs::remove_dir_all(app_data).unwrap();
}

#[test]
fn child_checks_conversation_lock() {
    let Some(root) = std::env::var_os("NOCTERM_TEST_GROK_LOCK_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    assert!(
        super::try_lock_conversation(&root, "conversation")
            .unwrap()
            .is_none()
    );
    cleanup_stale_grok_authentication_links(&root).unwrap();
    assert!(
        super::conversation_runtime_directory(&root, "conversation")
            .unwrap()
            .join("auth.json")
            .exists()
    );
}
