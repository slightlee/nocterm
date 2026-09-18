//! AI 会话启动编排。
//! 目标解析、Bridge 授权与 Provider 启动计划在此形成一个原子流程。

use std::sync::Arc;

use tauri::AppHandle;

use crate::{
    commands::{
        ai_bridge::McpRequestDispatcher,
        ai_headless::{HeadlessSessionLaunch, start_headless_session},
        ai_provider::{
            ProviderBridge, ProviderLaunch, ProviderLaunchPlan, ProviderSessionIdentity,
            ProviderToolTransport, provider_adapter,
        },
        ai_runtime::PersistentProviderLaunch,
        ai_validation::{ValidatedAiSessionStart, validate_start_request},
    },
    dto::ai::{AiSessionStartRequest, AiSessionStartResponse},
    state::{AiCommandPolicy, AiGatewayBinding, AiTarget, AppState},
};

/// 完成所有准备和授权后启动一种互斥的 Provider 生命周期。
pub(super) fn start_session(
    app: AppHandle,
    state: &AppState,
    request: AiSessionStartRequest,
) -> Result<AiSessionStartResponse, String> {
    let validated = validate_start_request(&request)?;
    let ValidatedAiSessionStart {
        session_id,
        conversation_id,
        working_directory,
        local_session_id,
        command_policy,
    } = validated;
    let connection_id = request.connection_id;
    let target = resolve_target(state, connection_id, local_session_id.as_deref())?;
    let adapter =
        provider_adapter(&request.provider).ok_or_else(|| "不支持的 AI Provider".to_string())?;
    let provider_id = adapter.id();
    let provider_command = adapter.command();
    let provider_executable = adapter
        .executable()
        .ok_or_else(|| format!("未找到本机 AI Provider：{provider_command}"))?;
    let tool_transport = adapter.tool_transport();

    // 只有 stdio Provider 需要启动 Bridge 子进程；ACP 直接进入进程内公共分发器。
    let bridge_executable = if tool_transport == ProviderToolTransport::Stdio {
        std::env::current_exe()
            .map_err(|error| format!("无法定位 Nocterm AI Bridge：{error}"))?
            .to_str()
            .ok_or_else(|| "Nocterm 可执行文件路径不是有效 UTF-8".to_string())?
            .to_string()
    } else {
        String::new()
    };
    let gateway_token = bind_gateway(
        state,
        target.as_ref(),
        provider_id,
        &session_id,
        command_policy,
    )?;
    let bridge_endpoint = match (gateway_token.as_ref(), tool_transport) {
        (Some(token), ProviderToolTransport::Stdio) => match state.ai_gateway().endpoint() {
            Some(endpoint) => Some(endpoint),
            None => {
                state.ai_gateway().revoke(token);
                return Err("Nocterm AI Bridge 尚未启动，请重启应用后重试".to_string());
            }
        },
        (Some(_), ProviderToolTransport::Acp) => None,
        _ => None,
    };
    let bridge = bridge_endpoint
        .as_ref()
        .zip(gateway_token.as_ref())
        .map(|(endpoint, token)| (endpoint.clone(), token.clone()));
    let provider_bridge = bridge.as_ref().map(|(endpoint, _)| ProviderBridge {
        executable: &bridge_executable,
        endpoint,
    });
    let identity = ProviderSessionIdentity {
        connection_id,
        target_session_id: local_session_id.clone(),
        working_directory: working_directory.clone(),
    };
    let launch_plan = match adapter.prepare_launch(ProviderLaunch {
        prompt: &request.prompt,
        bridge: provider_bridge,
        identity: &identity,
    }) {
        Ok(plan) => plan,
        Err(error) => {
            revoke_gateway(state, gateway_token.as_deref());
            return Err(error);
        }
    };

    match launch_plan {
        ProviderLaunchPlan::Persistent => {
            let mcp_dispatcher = Arc::new(McpRequestDispatcher::new(app.clone(), state));
            let started = state.ai_provider_runtimes().start_turn(
                provider_id,
                app,
                PersistentProviderLaunch {
                    conversation_id,
                    session_id: session_id.clone(),
                    identity,
                    initial_prompt: request.prompt,
                    continuation_prompt: request.continuation_prompt,
                    bridge: bridge.clone(),
                    gateway_token: gateway_token.clone(),
                    bridge_executable,
                    provider_executable,
                    command_policy,
                    mcp_dispatcher,
                },
                Arc::clone(state.ai_gateway()),
            );
            match started {
                Ok(is_new) => {
                    // 复用服务器时，新候选 token 未进入子进程，必须立即撤销。
                    if !is_new {
                        revoke_gateway(state, gateway_token.as_deref());
                    }
                    Ok(AiSessionStartResponse { session_id })
                }
                Err(error) => {
                    revoke_gateway(state, gateway_token.as_deref());
                    Err(error)
                }
            }
        }
        ProviderLaunchPlan::Headless(prepared) => {
            if let Some(token) = gateway_token.as_ref()
                && let Err(error) =
                    state
                        .ai_gateway()
                        .activate_session(token, session_id.clone(), command_policy)
            {
                state.ai_gateway().revoke(token);
                return Err(error);
            }
            start_headless_session(
                app,
                state,
                HeadlessSessionLaunch {
                    session_id: session_id.clone(),
                    provider_command,
                    provider_executable,
                    prepared: *prepared,
                    working_directory,
                    connection_id,
                    bridge_token: gateway_token,
                },
            )?;
            Ok(AiSessionStartResponse { session_id })
        }
    }
}

fn resolve_target(
    state: &AppState,
    connection_id: Option<i64>,
    local_session_id: Option<&str>,
) -> Result<Option<AiTarget>, String> {
    if let Some(connection_id) = connection_id {
        state
            .connection_service()
            .get(connection_id)
            .map_err(|_| "目标 SSH 连接不存在或已被删除".to_string())?;
        return Ok(Some(AiTarget::Ssh { connection_id }));
    }
    let Some(session_id) = local_session_id else {
        return Ok(None);
    };
    let terminal_id = state
        .local_terminals()
        .terminal_for(session_id)
        .ok_or_else(|| "当前本地终端尚未就绪或已关闭".to_string())?;
    Ok(Some(AiTarget::Local {
        session_id: session_id.to_string(),
        terminal_id,
    }))
}

fn bind_gateway(
    state: &AppState,
    target: Option<&AiTarget>,
    provider: &str,
    session_id: &str,
    command_policy: AiCommandPolicy,
) -> Result<Option<String>, String> {
    let Some(target) = target else {
        return Ok(None);
    };
    let token = generate_bridge_token()?;
    state.ai_gateway().bind(
        token.clone(),
        AiGatewayBinding {
            target: target.clone(),
            provider: provider.into(),
            session_id: session_id.into(),
            command_policy,
        },
    )?;
    Ok(Some(token))
}

fn revoke_gateway(state: &AppState, token: Option<&str>) {
    if let Some(token) = token {
        state.ai_gateway().revoke(token);
    }
}

/// Bridge token 直接使用操作系统 CSPRNG；随机源不可用时禁止启动带工具的会话。
fn generate_bridge_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| "无法生成安全的 AI Bridge token".to_string())?;
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut token = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        token.push(HEX[usize::from(byte >> 4)] as char);
        token.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    Ok(token)
}

#[cfg(test)]
mod tests {
    use super::generate_bridge_token;

    #[test]
    fn bridge_tokens_are_independent_256_bit_lowercase_hex_values() {
        let first = generate_bridge_token().expect("generate first token");
        let second = generate_bridge_token().expect("generate second token");

        assert_eq!(first.len(), 64);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_ne!(first, second);
    }
}
