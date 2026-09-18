use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    PersistentProviderSession, PersistentSessionRegistry, PersistentTurnRequest, SessionSlot,
    receive_startup_line,
};
use crate::{commands::ai_provider::ProviderSessionIdentity, state::AiCommandPolicy};

struct TestSession {
    identity: ProviderSessionIdentity,
    alive: AtomicBool,
    turns: Mutex<Vec<(String, String)>>,
    shutdown_probe: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl TestSession {
    fn new() -> Self {
        Self::with_identity(identity("default"))
    }

    fn with_identity(identity: ProviderSessionIdentity) -> Self {
        Self {
            identity,
            alive: AtomicBool::new(true),
            turns: Mutex::new(Vec::new()),
            shutdown_probe: Mutex::new(None),
        }
    }
}

impl PersistentProviderSession for TestSession {
    type TurnContext = ();

    fn matches_identity(&self, identity: &ProviderSessionIdentity) -> bool {
        self.identity == *identity
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn start_turn(
        &self,
        _context: Self::TurnContext,
        session_id: String,
        prompt: String,
        _command_policy: AiCommandPolicy,
    ) -> Result<(), String> {
        self.turns.lock().unwrap().push((session_id, prompt));
        Ok(())
    }

    fn stop_turn(&self, _session_id: &str) -> Result<bool, String> {
        Ok(false)
    }

    fn shutdown(&self) {
        self.alive.store(false, Ordering::Release);
        if let Some(probe) = self
            .shutdown_probe
            .lock()
            .expect("shutdown probe lock")
            .as_ref()
        {
            probe();
        }
    }

    fn shutdown_if_turn_active(&self, _session_id: &str) {}
}

fn identity(target: &str) -> ProviderSessionIdentity {
    ProviderSessionIdentity {
        connection_id: None,
        target_session_id: Some(target.to_string()),
        working_directory: Some("/tmp".into()),
    }
}

fn request(
    conversation_id: &str,
    session_id: &str,
    identity: ProviderSessionIdentity,
) -> PersistentTurnRequest {
    PersistentTurnRequest {
        conversation_id: conversation_id.into(),
        session_id: session_id.into(),
        identity,
        initial_prompt: "initial prompt".into(),
        continuation_prompt: "continuation prompt".into(),
        command_policy: AiCommandPolicy::AutoSafe,
    }
}

#[test]
fn startup_receive_observes_asynchronous_cancellation_before_protocol_timeout() {
    let (_sender, receiver) = mpsc::channel();
    let cancellation = Arc::new(AtomicBool::new(false));
    let cancellation_signal = Arc::clone(&cancellation);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(25));
        cancellation_signal.store(true, Ordering::Release);
    });

    let started = Instant::now();
    let error = receive_startup_line(
        &receiver,
        started + Duration::from_secs(10),
        &cancellation,
        "初始化 Provider",
        "Provider 提前退出",
    )
    .expect_err("cancel startup");

    assert!(error.contains("已取消"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn first_turn_spawns_and_the_next_turn_reuses_the_same_session() {
    let registry = PersistentSessionRegistry::<TestSession>::new("Test");
    let session = Arc::new(TestSession::with_identity(identity("local-one")));

    let first = registry
        .start_turn(
            (),
            request("conversation", "turn-one", identity("local-one")),
            |_, _| Ok(Arc::clone(&session)),
        )
        .unwrap();
    let second = registry
        .start_turn(
            (),
            request("conversation", "turn-two", identity("local-one")),
            |_, _| panic!("a reusable session must not spawn again"),
        )
        .unwrap();

    assert!(first);
    assert!(!second);
    assert_eq!(
        *session.turns.lock().unwrap(),
        [
            ("turn-one".into(), "initial prompt".into()),
            ("turn-two".into(), "continuation prompt".into()),
        ]
    );
}

#[test]
fn identity_change_shuts_down_the_old_session_and_spawns_a_replacement() {
    let registry = PersistentSessionRegistry::<TestSession>::new("Test");
    let old = Arc::new(TestSession::with_identity(identity("local-one")));
    let replacement = Arc::new(TestSession::with_identity(identity("local-two")));

    registry
        .start_turn(
            (),
            request("conversation", "turn-one", identity("local-one")),
            |_, _| Ok(Arc::clone(&old)),
        )
        .unwrap();
    let replaced = registry
        .start_turn(
            (),
            request("conversation", "turn-two", identity("local-two")),
            |_, _| Ok(Arc::clone(&replacement)),
        )
        .unwrap();

    assert!(replaced);
    assert!(!old.is_alive());
    assert!(replacement.is_alive());
}

#[test]
fn concurrent_cold_start_for_the_same_conversation_is_rejected() {
    let registry = Arc::new(PersistentSessionRegistry::<TestSession>::new("Test"));
    let worker_registry = Arc::clone(&registry);
    let (spawned_sender, spawned_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        worker_registry.start_turn(
            (),
            request("conversation", "turn-one", identity("local-one")),
            |_, _| {
                spawned_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                Ok(Arc::new(TestSession::with_identity(identity("local-one"))))
            },
        )
    });
    spawned_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    let error = registry
        .start_turn(
            (),
            request("conversation", "turn-two", identity("local-one")),
            |_, _| panic!("a second cold start must not run its spawn closure"),
        )
        .expect_err("reject concurrent startup");
    release_sender.send(()).unwrap();

    assert!(error.contains("正在启动"));
    assert_eq!(worker.join().unwrap(), Ok(true));
}

#[test]
fn stopping_during_spawn_discards_and_shuts_down_the_late_session() {
    let registry = Arc::new(PersistentSessionRegistry::<TestSession>::new("Test"));
    let late_session = Arc::new(TestSession::with_identity(identity("local-one")));
    let worker_session = Arc::clone(&late_session);
    let worker_registry = Arc::clone(&registry);
    let (spawned_sender, spawned_receiver) = mpsc::channel();
    let (release_sender, release_receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        worker_registry.start_turn(
            (),
            request("conversation", "turn-one", identity("local-one")),
            |_, _| {
                spawned_sender.send(()).unwrap();
                release_receiver.recv().unwrap();
                Ok(worker_session)
            },
        )
    });
    spawned_receiver
        .recv_timeout(Duration::from_secs(1))
        .unwrap();

    assert_eq!(registry.stop_turn("turn-one"), Ok(true));
    release_sender.send(()).unwrap();
    let error = worker.join().unwrap().expect_err("startup was cancelled");

    assert!(error.contains("启动已取消"));
    assert!(!late_session.is_alive());
    assert!(registry.inner.sessions.lock().unwrap().is_empty());
}

#[test]
fn stopping_a_starting_session_sets_its_cancellation_signal() {
    let registry = PersistentSessionRegistry::<TestSession>::new("Test");
    let cancellation = Arc::new(AtomicBool::new(false));
    registry.inner.sessions.lock().unwrap().insert(
        "conversation".into(),
        SessionSlot::Starting {
            generation: 1,
            session_id: "turn-one".into(),
            cancellation: Arc::clone(&cancellation),
        },
    );

    assert_eq!(registry.stop_turn("turn-one"), Ok(true));
    assert!(cancellation.load(Ordering::Acquire));
}

#[test]
fn stale_termination_does_not_remove_a_replacement_session() {
    let registry = PersistentSessionRegistry::<TestSession>::new("Test");
    let stale_termination = registry.termination("conversation".into(), 1);
    registry.inner.sessions.lock().unwrap().insert(
        "conversation".into(),
        SessionSlot::Ready {
            generation: 2,
            server: Arc::new(TestSession::new()),
        },
    );

    stale_termination.notify();
    assert!(
        registry
            .inner
            .sessions
            .lock()
            .unwrap()
            .contains_key("conversation")
    );

    registry.termination("conversation".into(), 2).notify();
    assert!(
        !registry
            .inner
            .sessions
            .lock()
            .unwrap()
            .contains_key("conversation")
    );
}

#[test]
fn registry_drop_invokes_shutdown_without_holding_the_session_map_lock() {
    let registry = PersistentSessionRegistry::<TestSession>::new("Test");
    let session = Arc::new(TestSession::new());
    let inner = Arc::downgrade(&registry.inner);
    let shutdown_observed = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&shutdown_observed);
    *session.shutdown_probe.lock().unwrap() = Some(Arc::new(move || {
        let inner = inner
            .upgrade()
            .expect("registry inner remains alive during drop");
        assert!(inner.sessions.try_lock().is_ok());
        observed.store(true, Ordering::Release);
    }));
    registry.inner.sessions.lock().unwrap().insert(
        "conversation".into(),
        SessionSlot::Ready {
            generation: 1,
            server: session,
        },
    );

    drop(registry);

    assert!(shutdown_observed.load(Ordering::Acquire));
}
