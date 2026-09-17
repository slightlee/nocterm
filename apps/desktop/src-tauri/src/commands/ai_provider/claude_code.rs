//! Claude Code headless 启动参数与任务级 MCP 配置。

use std::{fs::DirBuilder, path::PathBuf};

use super::{
    HeadlessReadinessRequirement, PreparedHeadlessLaunch, ProviderAdapter, ProviderLaunch,
    ProviderLaunchPlan, terminal_session_instructions,
};

pub(super) static ADAPTER: &dyn ProviderAdapter = &ClaudeCodeAdapter;

struct ClaudeCodeAdapter;

// Claude Code 运行在 headless 模式时没有可用的交互式审批回调。
// 只预批准当前 Nocterm MCP Server，实际目标、权限和审计仍由 Gateway 决定。
const NOCTERM_MCP_ALLOWED_TOOLS: &str = "mcp__nocterm__*";
const NOCTERM_MCP_TOOL_PREFIX: &str = "mcp__nocterm__";

impl ProviderAdapter for ClaudeCodeAdapter {
    fn id(&self) -> &'static str {
        "claude-code"
    }

    fn command(&self) -> &'static str {
        "claude"
    }

    fn prepare_launch(&self, launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        let has_bridge = launch.bridge.is_some();
        // Claude 会把 cwd 等宿主信息加入模型上下文。终端任务必须在空目录运行，
        // 否则模型可能把 Nocterm 进程所在仓库误认为用户当前选择的终端。
        let runtime_directory = has_bridge
            .then(create_private_runtime_directory)
            .transpose()?;
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
            "dontAsk".into(),
            "--tools".into(),
            "".into(),
            "--strict-mcp-config".into(),
        ];
        if has_bridge {
            args.extend(["--allowed-tools".into(), NOCTERM_MCP_ALLOWED_TOOLS.into()]);
        }
        if let Some(instructions) = has_bridge
            .then(|| terminal_session_instructions(launch.identity))
            .flatten()
        {
            // 默认 Claude Code 提示面向本机编码工作区，会把 Provider 主机信息带入回答。
            // 绑定终端时以 Nocterm 契约替换它，MCP 工具描述仍由协议独立提供。
            args.extend(["--system-prompt".into(), instructions.into()]);
        }
        let mcp_config = launch.bridge.map_or_else(
            || serde_json::json!({"mcpServers":{}}).to_string(),
            |bridge| crate::commands::ai_bridge::mcp_config(bridge.executable, bridge.endpoint),
        );
        args.extend(["--mcp-config".into(), mcp_config]);
        let cleanup_paths = runtime_directory.iter().cloned().collect();
        let prepared = PreparedHeadlessLaunch::new(
            args,
            Some(launch.prompt.to_string()),
            Vec::new(),
            runtime_directory,
            has_bridge.then_some(HeadlessReadinessRequirement {
                event_type: "system",
                event_subtype: "init",
                tool_prefix: NOCTERM_MCP_TOOL_PREFIX,
                failure_message: "Claude Code 未加载当前终端能力，请检查 Provider 版本与 MCP 配置",
                require_gateway_call: true,
                missing_gateway_call_message:
                    "Claude Code 未通过当前终端能力取得结果，已丢弃未经验证的回答",
            }),
            cleanup_paths,
        );
        Ok(ProviderLaunchPlan::Headless(Box::new(prepared)))
    }
}

/// 使用不可预测名称和仅当前用户可访问的目录隔离项目指令与宿主 cwd。
fn create_private_runtime_directory() -> Result<PathBuf, String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|_| "无法生成 Claude Code 隔离目录名称".to_string())?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut suffix = String::with_capacity(random.len() * 2);
    for byte in random {
        suffix.push(HEX[usize::from(byte >> 4)] as char);
        suffix.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    let path = std::env::temp_dir().join(format!("nocterm-claude-runtime-{suffix}"));
    let mut builder = DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&path)
        .map_err(|error| format!("创建 Claude Code 隔离目录失败：{error}"))?;
    Ok(path)
}
