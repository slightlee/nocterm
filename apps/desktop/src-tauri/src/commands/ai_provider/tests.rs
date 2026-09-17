use std::path::Path;

use super::{
    ProviderBridge, ProviderLaunch, ProviderLaunchPlan, ProviderSessionIdentity,
    ProviderToolTransport, find_provider_executable_in, provider_adapter,
};

fn identity() -> ProviderSessionIdentity {
    ProviderSessionIdentity {
        connection_id: Some(7),
        target_session_id: None,
        working_directory: None,
    }
}

fn launch<'a>(
    prompt: &'a str,
    bridge: Option<ProviderBridge<'a>>,
    identity: &'a ProviderSessionIdentity,
) -> ProviderLaunch<'a> {
    ProviderLaunch {
        prompt,
        bridge,
        identity,
    }
}

#[test]
fn registry_exposes_all_provider_commands() {
    assert_eq!(
        provider_adapter("codex").map(|item| item.command()),
        Some("codex")
    );
    assert_eq!(
        provider_adapter("claude-code").map(|item| item.command()),
        Some("claude")
    );
    assert_eq!(
        provider_adapter("grok").map(|item| item.command()),
        Some("grok")
    );
    assert_eq!(
        provider_adapter("codex").map(|item| item.tool_transport()),
        Some(ProviderToolTransport::Stdio)
    );
    assert_eq!(
        provider_adapter("claude-code").map(|item| item.tool_transport()),
        Some(ProviderToolTransport::Stdio)
    );
    assert_eq!(
        provider_adapter("grok").map(|item| item.tool_transport()),
        Some(ProviderToolTransport::Acp)
    );
    assert!(provider_adapter("unknown").is_none());
}

#[test]
fn executable_discovery_uses_the_supplied_search_path_without_a_probe_process() {
    let current = std::env::current_exe().expect("test executable");
    let directory = current.parent().expect("test executable directory");
    let command = current
        .file_name()
        .and_then(|name| name.to_str())
        .expect("utf8 test executable name");
    let search_path = std::env::join_paths([Path::new(directory)]).expect("search path");

    assert_eq!(
        find_provider_executable_in(command, Some(search_path), None),
        Some(current)
    );
}

#[test]
fn adapters_keep_machine_output_and_disable_owned_shells_for_bridge_tasks() {
    let bridge = || ProviderBridge {
        executable: "/tmp/nocterm",
        endpoint: "127.0.0.1:4567",
    };
    let identity = identity();
    let ProviderLaunchPlan::Headless(claude) = provider_adapter("claude-code")
        .unwrap()
        .prepare_launch(launch("inspect", Some(bridge()), &identity))
        .unwrap()
    else {
        panic!("Claude Code should use a headless launch");
    };
    assert!(claude.args.contains(&"stream-json".to_string()));
    assert!(!claude.args.iter().any(|argument| argument == "inspect"));
    assert_eq!(claude.stdin_payload.as_deref(), Some("inspect"));
    assert!(
        claude
            .args
            .contains(&"--no-session-persistence".to_string())
    );
    assert!(claude.args.contains(&"--strict-mcp-config".to_string()));
    assert!(claude.args.windows(2).any(|pair| pair == ["--tools", ""]));
    assert!(
        claude
            .args
            .windows(2)
            .any(|pair| { pair == ["--permission-mode", "dontAsk"] })
    );
    assert!(
        claude
            .args
            .windows(2)
            .any(|pair| { pair == ["--allowed-tools", "mcp__nocterm__*"] })
    );
    assert!(!claude.args.iter().any(|argument| argument == "manual"));
    assert_eq!(
        claude
            .readiness_requirement
            .as_ref()
            .map(|requirement| requirement.tool_prefix),
        Some("mcp__nocterm__")
    );
    assert!(
        claude
            .readiness_requirement
            .as_ref()
            .is_some_and(|requirement| requirement.require_gateway_call)
    );
    let claude_runtime = claude
        .current_directory
        .clone()
        .expect("terminal-bound Claude launch should have an isolated cwd");
    assert!(claude_runtime.is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode = std::fs::metadata(&claude_runtime)
            .expect("Claude runtime metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
    }
    assert_ne!(
        claude_runtime,
        std::env::current_dir().expect("current test directory")
    );
    assert!(
        claude
            .args
            .windows(2)
            .any(|pair| pair[0] == "--system-prompt"
                && pair[1].contains("call the appropriate Nocterm tool before answering")
                && pair[1].contains("provider process is not the terminal target"))
    );
    assert!(!claude.args.contains(&"--append-system-prompt".to_string()));

    let ProviderLaunchPlan::Headless(claude_without_bridge) = provider_adapter("claude-code")
        .unwrap()
        .prepare_launch(launch("inspect", None, &identity))
        .unwrap()
    else {
        panic!("Claude Code should use a headless launch");
    };
    assert!(
        !claude_without_bridge
            .args
            .iter()
            .any(|argument| argument == "mcp__nocterm__*")
    );
    assert!(
        !claude_without_bridge
            .args
            .contains(&"--system-prompt".to_string())
    );
    assert!(claude_without_bridge.readiness_requirement.is_none());
    assert!(claude_without_bridge.current_directory.is_none());

    drop(claude);
    assert!(!claude_runtime.exists());

    let grok = provider_adapter("grok")
        .unwrap()
        .prepare_launch(launch("inspect", Some(bridge()), &identity))
        .unwrap();
    assert!(matches!(grok, ProviderLaunchPlan::Persistent));
}

#[test]
fn codex_uses_its_persistent_protocol_adapter() {
    let identity = identity();
    let prepared = provider_adapter("codex")
        .unwrap()
        .prepare_launch(launch("inspect", None, &identity))
        .unwrap();
    assert!(matches!(prepared, ProviderLaunchPlan::Persistent));
}
