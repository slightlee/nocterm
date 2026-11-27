//! 串联真实 ACP 加载解析与共享注册表，验证发送前失败后的历史选择。

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use serde_json::{Value, json};

use super::{BootstrapMessages, resolve_session, wait_for_response_message_collect_with_requests};
use crate::{
    commands::{
        ai_persistent::{
            PersistentProviderSession, PersistentSessionRegistry, PersistentTurnRequest,
        },
        ai_provider::ProviderSessionIdentity,
    },
    state::AiCommandPolicy,
};

struct LoadedSession {
    restored_context: bool,
    fail_before_send: bool,
    alive: AtomicBool,
    prompts: Mutex<Vec<String>>,
}

impl LoadedSession {
    fn new(restored_context: bool, fail_before_send: bool) -> Arc<Self> {
        Arc::new(Self {
            restored_context,
            fail_before_send,
            alive: AtomicBool::new(true),
            prompts: Mutex::new(Vec::new()),
        })
    }
}

impl PersistentProviderSession for LoadedSession {
    type TurnContext = ();

    fn matches_identity(&self, other: &ProviderSessionIdentity) -> bool {
        *other == identity()
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn resumed_from_native(&self) -> bool {
        self.restored_context
    }

    fn start_turn(
        &self,
        _context: (),
        _session_id: String,
        prompt: String,
        _policy: AiCommandPolicy,
    ) -> Result<(), String> {
        if self.fail_before_send {
            return Err("synthetic failure before prompt write".into());
        }
        self.prompts.lock().unwrap().push(prompt);
        Ok(())
    }

    fn stop_turn(&self, _session_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn shutdown(&self) {
        self.alive.store(false, Ordering::Release);
    }

    fn shutdown_if_turn_active(&self, _session_id: &str) {}
}

fn identity() -> ProviderSessionIdentity {
    ProviderSessionIdentity {
        model: Some("synthetic".into()),
        reasoning_effort: None,
        fast_mode: None,
        connection_id: None,
        target_session_id: Some("local-one".into()),
        working_directory: None,
    }
}

fn request() -> PersistentTurnRequest {
    PersistentTurnRequest {
        conversation_id: "conversation".into(),
        session_id: "turn".into(),
        identity: identity(),
        initial_prompt: "PRIOR_CONTEXT + CURRENT_QUESTION".into(),
        continuation_prompt: "CURRENT_QUESTION".into(),
        command_policy: AiCommandPolicy::AutoSafe,
    }
}

fn update(kind: &str) -> Value {
    json!({"method":"session/update", "params":{
        "sessionId":"saved", "update":{"sessionUpdate":kind,
        "content":{"type":"text", "text":"synthetic history"}}
    }})
}

fn load(messages: Vec<Value>) -> Result<(String, bool), String> {
    let (tx, rx) = mpsc::channel();
    for message in messages {
        tx.send(Ok(message.to_string())).unwrap();
    }
    drop(tx);
    let mut pending = BootstrapMessages::default();
    let response = wait_for_response_message_collect_with_requests(
        &rx,
        2,
        "加载",
        &mut pending,
        &AtomicBool::new(false),
        Some("saved"),
        |_| Ok(false),
    )?;
    resolve_session(&response, Some("saved"), &pending)
}

#[test]
fn empty_loaded_session_after_failed_first_send_replays_ui_context_once() {
    // 真实 Grok 空会话会发送菜单通知；它和模式、用量等状态都不代表已有对话历史。
    for metadata in [
        vec![],
        vec!["available_commands_update"],
        vec![
            "available_commands_update",
            "current_mode_update",
            "config_option_update",
            "session_info_update",
            "usage_update",
        ],
    ] {
        let registry = PersistentSessionRegistry::<LoadedSession>::new("Grok");
        let failed = LoadedSession::new(false, true);
        assert!(
            registry
                .start_turn((), request(), |_, _| Ok(Arc::clone(&failed)))
                .is_err()
        );
        assert!(!failed.is_alive());
        assert!(failed.prompts.lock().unwrap().is_empty());

        // 绑定已落盘，但首轮未发送；即使连续重试失败也不能丢掉 UI 历史。
        for fail_before_send in [true, false] {
            let mut messages: Vec<_> = metadata.iter().map(|kind| update(kind)).collect();
            messages.push(json!({"id":2,"result":{}}));
            let (id, restored) = load(messages).unwrap();
            assert_eq!(id, "saved", "空会话仍须使用原生 ID");
            let retry = LoadedSession::new(restored, fail_before_send);
            let result = registry.start_turn((), request(), |_, _| Ok(Arc::clone(&retry)));
            if fail_before_send {
                assert!(result.is_err());
                assert!(!retry.is_alive());
                continue;
            }
            assert!(result.unwrap());
            assert!(
                !registry
                    .start_turn((), request(), |_, _| panic!("must reuse"))
                    .unwrap()
            );
            assert_eq!(
                *retry.prompts.lock().unwrap(),
                ["PRIOR_CONTEXT + CURRENT_QUESTION", "CURRENT_QUESTION"]
            );
        }
    }
}

#[test]
fn loaded_history_never_replays_ui_context_again() {
    // 只收到用户输入、尚无回答的中断轮次也已含首轮上下文，不能再次补发。
    for kind in [
        "user_message_chunk",
        "agent_message_chunk",
        "agent_thought_chunk",
        "tool_call",
        "tool_call_update",
        "plan",
    ] {
        let (_, restored) = load(vec![update(kind), json!({"id":2,"result":{}})]).unwrap();
        let registry = PersistentSessionRegistry::<LoadedSession>::new("Grok");
        let session = LoadedSession::new(restored, false);
        registry
            .start_turn((), request(), |_, _| Ok(Arc::clone(&session)))
            .unwrap();
        assert_eq!(
            *session.prompts.lock().unwrap(),
            ["CURRENT_QUESTION"],
            "{kind}"
        );
    }
}

#[test]
fn other_sessions_and_requests_do_not_prove_loaded_history() {
    let mut other = update("user_message_chunk");
    other["params"]["sessionId"] = json!("other");
    let mut request = update("user_message_chunk");
    request["id"] = json!(7);
    assert_eq!(
        load(vec![other, request, json!({"id":2,"result":{}})]).unwrap(),
        ("saved".into(), false)
    );
}

#[test]
fn failed_or_interrupted_load_cannot_be_treated_as_empty_history() {
    for response in [
        json!({"id":2,"error":{"code":-32603,"message":"storage unavailable"}}),
        json!({"id":2,"error":{"code":-32602,"message":"Session does not exist"}}),
        json!({"id":2}),
    ] {
        assert!(load(vec![response.clone()]).is_err());
        assert!(load(vec![update("user_message_chunk"), response]).is_err());
    }
    assert!(load(vec![]).is_err());
    assert!(load(vec![update("user_message_chunk")]).is_err());
}

#[test]
fn unknown_replay_events_cannot_trigger_context_replay() {
    for message in [
        update("unknown_update"),
        json!({
            "method":"session/update", "params":{"sessionId":"saved"}
        }),
    ] {
        assert!(load(vec![message, json!({"id":2,"result":{}})]).is_err());
    }
}
