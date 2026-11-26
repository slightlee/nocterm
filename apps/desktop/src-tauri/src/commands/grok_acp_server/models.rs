//! 读取 Grok ACP v1 模型能力，构造会话级强度设置请求。

use serde_json::{Value, json};

use crate::commands::ai_provider::ProviderSessionIdentity;

pub(in crate::commands) const EFFORTS: &[&str] =
    &["none", "minimal", "low", "medium", "high", "xhigh", "max"];
pub(super) const EFFORT_UNAVAILABLE: &str = "AI_GROK_REASONING_EFFORT_UNAVAILABLE";
pub(super) const EFFORT_APPLY_FAILED: &str = "AI_GROK_REASONING_EFFORT_APPLY_FAILED";
pub(super) const EFFORT_REQUEST_ID: u64 = 3;

/// 初始化结果通过 Grok 扩展广播模型目录。缺少扩展时继续使用原 CLI 目录。
pub(in crate::commands) fn initialized_models(result: &Value) -> &[Value] {
    if result["protocolVersion"] != 1 {
        return &[];
    }
    model_list(result.pointer("/_meta/modelState"))
}

fn model_list(state: Option<&Value>) -> &[Value] {
    state
        .and_then(|state| state["availableModels"].as_array())
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// 只公开客户端能校验的原生档位。当前值可能来自用户配置，只有明确标记才算模型默认值。
pub(in crate::commands) fn reasoning_options(model: &Value) -> (Vec<String>, Option<String>) {
    let Some(meta) = model
        .get("_meta")
        .filter(|meta| meta["supportsReasoningEffort"] != false)
    else {
        return (Vec::new(), None);
    };
    let Some(options) = meta["reasoningEfforts"].as_array() else {
        return (Vec::new(), None);
    };
    let mut offered = Vec::new();
    let mut defaults = Vec::new();
    for entry in options.iter().take(32) {
        let Some(value) = [entry["value"].as_str(), entry["id"].as_str()]
            .into_iter()
            .flatten()
            .find(|value| EFFORTS.contains(value))
        else {
            continue;
        };
        offered.push(value);
        if entry["default"] == true || entry["isDefault"] == true {
            defaults.push(value);
        }
    }
    let default = meta["reasoningEffort"]
        .as_str()
        .filter(|value| defaults.contains(value))
        .or_else(|| defaults.first().copied());
    (
        EFFORTS
            .iter()
            .filter(|value| offered.contains(value))
            .map(|value| (*value).to_string())
            .collect(),
        default.map(str::to_string),
    )
}

/// session/new 的同模型信息优先，初始化目录只在会话没有报告该模型时补齐。
/// 明确选了档位但模型已不再提供它时，停止启动，避免用另一档位静默发送。
fn effort_request(
    initialized: &Value,
    session: &Value,
    session_id: &str,
    model_id: &str,
    effort: &str,
) -> Result<Value, String> {
    let model = model_list(session.pointer("/result/models"))
        .iter()
        .chain(initialized_models(&initialized["result"]))
        .find(|model| model["modelId"] == model_id)
        .ok_or(EFFORT_UNAVAILABLE)?;
    let (offered, _) = reasoning_options(model);
    if !offered.iter().any(|value| value == effort) {
        return Err(EFFORT_UNAVAILABLE.into());
    }
    Ok(json!({
        "jsonrpc": "2.0",
        "id": EFFORT_REQUEST_ID,
        "method": "session/set_model",
        "params": {
            "sessionId": session_id,
            "modelId": model_id,
            "_meta": {"reasoningEffort": effort}
        }
    }))
}

/// 只有显式档位才发出额外请求。RPC 失败时在启动边界返回稳定错误，不透传原始响应。
pub(super) fn apply_effort(
    identity: &ProviderSessionIdentity,
    initialized: &Value,
    session: &Value,
    session_id: &str,
    exchange: impl FnOnce(&Value) -> Result<Value, String>,
) -> Result<(), String> {
    let Some(effort) = identity.reasoning_effort.as_deref() else {
        return Ok(());
    };
    let model = identity.model.as_deref().ok_or(EFFORT_UNAVAILABLE)?;
    let request = effort_request(initialized, session, session_id, model, effort)?;
    let response = exchange(&request).map_err(|_| EFFORT_APPLY_FAILED)?;
    if response["id"] != EFFORT_REQUEST_ID
        || !response["result"].is_object()
        || response.get("error").is_some()
    {
        return Err(EFFORT_APPLY_FAILED.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, effort: &str) -> Value {
        json!({"modelId":id,"_meta":{"reasoningEfforts":[{"id":effort}]}})
    }

    #[test]
    fn explicit_effort_uses_grok_v1_metadata_for_the_selected_model() {
        let initialized = json!({"result":{"protocolVersion":1,"_meta":{"modelState":{
            "availableModels":[model("grok-a", "low"), model("grok-b", "high")]
        }}}});
        let session = json!({"result":{"models":{"availableModels":[model("grok-a", "medium")]}}});
        assert_eq!(
            effort_request(
                &initialized,
                &session,
                "current-session",
                "grok-a",
                "medium"
            )
            .unwrap(),
            json!({"jsonrpc":"2.0","id":3,"method":"session/set_model","params":{
                "sessionId":"current-session","modelId":"grok-a","_meta":{"reasoningEffort":"medium"}
            }})
        );
        // 会话的档位覆盖目录里同一模型的旧能力，不能沿用其他模型的档位。
        for effort in ["low", "high"] {
            assert_eq!(
                effort_request(&initialized, &session, "current-session", "grok-a", effort),
                Err(EFFORT_UNAVAILABLE.into())
            );
        }
        assert!(
            effort_request(&initialized, &session, "current-session", "grok-b", "high").is_ok()
        );
        assert_eq!(
            effort_request(&initialized, &session, "current-session", "unknown", "high"),
            Err(EFFORT_UNAVAILABLE.into())
        );
    }

    #[test]
    fn grok_bootstrap_only_sends_explicit_effort_and_requires_an_acknowledgment() {
        let mut identity = ProviderSessionIdentity {
            model: Some("grok-a".into()),
            reasoning_effort: None,
            fast_mode: None,
            connection_id: None,
            target_session_id: Some("local:one".into()),
            working_directory: None,
        };
        let initialized = json!({"result":{"protocolVersion":1}});
        let session = json!({"result":{"models":{"availableModels":[model("grok-a", "low")]}}});
        assert!(
            apply_effort(&identity, &initialized, &session, "test", |_| panic!(
                "不应发送"
            ))
            .is_ok()
        );
        identity.reasoning_effort = Some("high".into());
        assert_eq!(
            apply_effort(&identity, &initialized, &session, "test", |_| panic!(
                "不应发送"
            )),
            Err(EFFORT_UNAVAILABLE.into())
        );
        identity.reasoning_effort = Some("low".into());
        for reply in [
            json!({"id":3,"error":{"code":-32602,"message":"rejected"}}),
            json!({"id":3,"result":null}),
            json!({"id":99,"result":{}}),
        ] {
            assert_eq!(
                apply_effort(&identity, &initialized, &session, "test", |_| Ok(reply)),
                Err(EFFORT_APPLY_FAILED.into())
            );
        }
        assert_eq!(
            apply_effort(&identity, &initialized, &session, "test", |_| Err(
                "timeout".into()
            )),
            Err(EFFORT_APPLY_FAILED.into())
        );
        assert!(
            apply_effort(&identity, &initialized, &session, "test", |request| {
                assert_eq!(request["params"]["_meta"]["reasoningEffort"], "low");
                Ok(json!({"id":3,"result":{}}))
            })
            .is_ok()
        );
    }
}
