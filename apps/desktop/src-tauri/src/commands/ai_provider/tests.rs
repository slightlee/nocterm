use std::path::{Path, PathBuf};

use super::{
    ProviderBridge, ProviderLaunch, ProviderLaunchPlan, ProviderSessionIdentity,
    ProviderToolTransport, combined_search_path, find_provider_executable_in, provider_adapter,
    supplementary_search_directories,
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

/// 构造隔离的模拟家目录；测试结束后整体删除，不污染真实用户环境。
struct FakeHome {
    root: PathBuf,
}

impl FakeHome {
    fn create(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "nocterm-ai-provider-test-{}-{label}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create fake home root");
        Self { root }
    }

    fn touch(&self, relative: &str) -> PathBuf {
        let path = self.root.join(relative);
        std::fs::create_dir_all(path.parent().expect("parent directory"))
            .expect("create parent directory");
        std::fs::write(&path, b"#!/bin/sh\n").expect("write fake executable");
        path
    }
}

impl Drop for FakeHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn supplementary_directories_cover_user_cli_locations_without_home_fallback() {
    let home = FakeHome::create("supplementary");
    home.touch(".local/bin/claude");
    home.touch(".nvm/versions/node/v22.17.1/bin/codex");
    home.touch(".nvm/versions/node/v20.19.0/bin/codex");

    let directories = supplementary_search_directories(Some(&home.root));
    // 用户目录在前，固定系统目录居中，nvm 版本目录按名称排序在后，保证确定性。
    assert_eq!(
        directories[..7],
        [
            home.root.join(".local/bin"),
            home.root.join("bin"),
            home.root.join(".cargo/bin"),
            home.root.join(".grok/bin"),
            home.root.join(".volta/bin"),
            home.root.join(".asdf/shims"),
            home.root.join("Library/pnpm"),
        ]
    );
    assert!(directories.contains(&PathBuf::from("/usr/local/bin")));
    assert!(directories.contains(&PathBuf::from("/opt/homebrew/bin")));
    let nvm_bins: Vec<_> = directories
        .iter()
        .filter(|directory| directory.starts_with(home.root.join(".nvm")))
        .cloned()
        .collect();
    assert_eq!(
        nvm_bins,
        vec![
            home.root.join(".nvm/versions/node/v20.19.0/bin"),
            home.root.join(".nvm/versions/node/v22.17.1/bin"),
        ]
    );
    assert!(supplementary_search_directories(None).is_empty());
}

#[test]
fn discovery_finds_clis_installed_outside_the_gui_path() {
    let home = FakeHome::create("discovery");
    let claude = home.touch(".local/bin/claude");
    let codex = home.touch(".nvm/versions/node/v22.17.1/bin/codex");

    // GUI 进程可能没有 PATH；补充目录必须独立完成发现。
    let search_path = combined_search_path(None, Some(home.root.clone())).expect("search path");
    assert_eq!(
        find_provider_executable_in("claude", Some(search_path.clone()), None),
        Some(claude)
    );
    assert_eq!(
        find_provider_executable_in("codex", Some(search_path), None),
        Some(codex)
    );
}

#[test]
fn combined_search_path_keeps_the_process_path_first() {
    let home = FakeHome::create("combined");
    let process_directory = std::env::temp_dir().join(format!(
        "nocterm-ai-provider-test-{}-combined-path",
        std::process::id()
    ));
    std::fs::create_dir_all(&process_directory).expect("create process path directory");
    let process_path = std::env::join_paths([&process_directory]).expect("join process path");

    let combined = combined_search_path(Some(process_path.clone()), Some(home.root.clone()))
        .expect("combined search path");
    let directories: Vec<_> = std::env::split_paths(&combined).collect();
    assert_eq!(directories.first(), Some(&process_directory));
    assert!(directories.contains(&home.root.join(".local/bin")));

    std::fs::remove_dir_all(&process_directory).expect("cleanup process path directory");
}
