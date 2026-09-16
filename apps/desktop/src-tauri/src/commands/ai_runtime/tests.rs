use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tauri::AppHandle;

use super::{
    PersistentProviderLaunch, PersistentProviderRuntime, PersistentProviderRuntimeRegistry,
};
use crate::state::AiGatewayState;

struct TestRuntime {
    id: &'static str,
    handled_session: Option<&'static str>,
    stop_calls: Arc<AtomicUsize>,
    reset_calls: Arc<AtomicUsize>,
    stop_error: Option<&'static str>,
    reset_error: Option<&'static str>,
}

impl PersistentProviderRuntime for TestRuntime {
    fn id(&self) -> &'static str {
        self.id
    }

    fn start_turn(
        &self,
        _app: AppHandle,
        _launch: PersistentProviderLaunch,
        _gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String> {
        unreachable!("runtime dispatch tests do not start a Tauri process")
    }

    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        self.stop_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self.stop_error {
            return Err(error.into());
        }
        Ok(self.handled_session == Some(session_id))
    }

    fn reset_conversation(&self, _conversation_id: &str) -> Result<bool, String> {
        self.reset_calls.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self.reset_error {
            return Err(error.into());
        }
        Ok(self.handled_session.is_some())
    }
}

fn runtime(
    id: &'static str,
    handled_session: Option<&'static str>,
    stop_calls: Arc<AtomicUsize>,
    reset_calls: Arc<AtomicUsize>,
) -> Box<dyn PersistentProviderRuntime> {
    Box::new(TestRuntime {
        id,
        handled_session,
        stop_calls,
        reset_calls,
        stop_error: None,
        reset_error: None,
    })
}

fn failing_runtime(
    id: &'static str,
    stop_calls: Arc<AtomicUsize>,
    reset_calls: Arc<AtomicUsize>,
) -> Box<dyn PersistentProviderRuntime> {
    Box::new(TestRuntime {
        id,
        handled_session: None,
        stop_calls,
        reset_calls,
        stop_error: Some("stop state unavailable"),
        reset_error: Some("reset state unavailable"),
    })
}

#[test]
fn default_registry_contains_every_persistent_provider() {
    let registry = PersistentProviderRuntimeRegistry::default();

    assert!(registry.runtime("codex").is_ok());
    assert!(registry.runtime("grok").is_ok());
    assert!(registry.runtime("claude-code").is_err());
}

#[test]
fn stop_routes_until_the_owning_runtime_handles_the_session() {
    let first_calls = Arc::new(AtomicUsize::new(0));
    let owner_calls = Arc::new(AtomicUsize::new(0));
    let skipped_calls = Arc::new(AtomicUsize::new(0));
    let registry = PersistentProviderRuntimeRegistry::new(vec![
        runtime(
            "first",
            None,
            Arc::clone(&first_calls),
            Arc::new(AtomicUsize::new(0)),
        ),
        runtime(
            "owner",
            Some("turn-one"),
            Arc::clone(&owner_calls),
            Arc::new(AtomicUsize::new(0)),
        ),
        runtime(
            "skipped",
            Some("turn-one"),
            Arc::clone(&skipped_calls),
            Arc::new(AtomicUsize::new(0)),
        ),
    ]);

    assert_eq!(registry.stop_turn("turn-one"), Ok(true));
    assert_eq!(first_calls.load(Ordering::Relaxed), 1);
    assert_eq!(owner_calls.load(Ordering::Relaxed), 1);
    // 清理路径会继续扫描其余 Runtime，防止异常状态下残留重复 session。
    assert_eq!(skipped_calls.load(Ordering::Relaxed), 1);
}

#[test]
fn reset_visits_every_runtime_even_after_one_reports_a_match() {
    let first_resets = Arc::new(AtomicUsize::new(0));
    let second_resets = Arc::new(AtomicUsize::new(0));
    let registry = PersistentProviderRuntimeRegistry::new(vec![
        runtime(
            "first",
            Some("turn-one"),
            Arc::new(AtomicUsize::new(0)),
            Arc::clone(&first_resets),
        ),
        runtime(
            "second",
            None,
            Arc::new(AtomicUsize::new(0)),
            Arc::clone(&second_resets),
        ),
    ]);

    assert_eq!(registry.reset_conversation("conversation-one"), Ok(true));
    assert_eq!(first_resets.load(Ordering::Relaxed), 1);
    assert_eq!(second_resets.load(Ordering::Relaxed), 1);
}

#[test]
fn lifecycle_sweeps_continue_after_a_runtime_error() {
    let failed_stops = Arc::new(AtomicUsize::new(0));
    let failed_resets = Arc::new(AtomicUsize::new(0));
    let owner_stops = Arc::new(AtomicUsize::new(0));
    let owner_resets = Arc::new(AtomicUsize::new(0));
    let registry = PersistentProviderRuntimeRegistry::new(vec![
        failing_runtime(
            "broken",
            Arc::clone(&failed_stops),
            Arc::clone(&failed_resets),
        ),
        runtime(
            "owner",
            Some("turn-one"),
            Arc::clone(&owner_stops),
            Arc::clone(&owner_resets),
        ),
    ]);

    let stop_error = registry
        .stop_turn("turn-one")
        .expect_err("report partial failure");
    let reset_error = registry
        .reset_conversation("conversation-one")
        .expect_err("report partial failure");

    assert!(stop_error.contains("broken: stop state unavailable"));
    assert!(reset_error.contains("broken: reset state unavailable"));
    assert_eq!(failed_stops.load(Ordering::Relaxed), 1);
    assert_eq!(failed_resets.load(Ordering::Relaxed), 1);
    assert_eq!(owner_stops.load(Ordering::Relaxed), 1);
    assert_eq!(owner_resets.load(Ordering::Relaxed), 1);
}
