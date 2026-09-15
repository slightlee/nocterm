//! Claude Code headless 启动参数与任务级 MCP 配置。

use super::{PreparedHeadlessLaunch, ProviderAdapter, ProviderLaunch, ProviderLaunchPlan};

pub(super) static ADAPTER: &dyn ProviderAdapter = &ClaudeCodeAdapter;

struct ClaudeCodeAdapter;

impl ProviderAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn command(&self) -> &'static str {
        "claude"
    }

    fn prepare_launch(&self, launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        let mut args = vec![
            "-p".into(),
            "--output-format".into(),
            "stream-json".into(),
            "--verbose".into(),
            "--include-partial-messages".into(),
            "--input-format".into(),
            "text".into(),
            "--no-session-persistence".into(),
            "--permission-mode".into(),
            "manual".into(),
            "--tools".into(),
            "".into(),
            "--strict-mcp-config".into(),
        ];
        let mcp_config = launch.bridge.map_or_else(
            || serde_json::json!({"mcpServers":{}}).to_string(),
            |bridge| crate::commands::ai_bridge::mcp_config(bridge.executable, bridge.endpoint),
        );
        args.extend(["--mcp-config".into(), mcp_config]);
        Ok(ProviderLaunchPlan::Headless(PreparedHeadlessLaunch::new(
            args,
            Some(launch.prompt.to_string()),
            Vec::new(),
            None,
            Vec::new(),
        )))
    }
}
