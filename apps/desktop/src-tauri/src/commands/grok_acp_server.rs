//! Grok ACP 会话注册表、Runtime 接入与按 UI 对话复用策略。

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::AppHandle;

use crate::{
    commands::{
        ai_persistent::{PersistentSessionRegistry, PersistentTurnRequest},
        ai_provider::cleanup_grok_conversation,
        ai_runtime::{PersistentProviderLaunch, PersistentProviderRuntime},
    },
    state::AiGatewayState,
};

pub(super) mod models;
mod protocol;
mod session;

pub(super) use protocol::initialize_request as grok_initialize_request;
use session::{GrokAcpServer, GrokAcpServerLaunch};

static NEXT_RUNTIME_ID: AtomicU64 = AtomicU64::new(1);

/// 目录探测和聊天进程共用序列，私有目录不会相互覆盖。
pub(super) fn next_runtime_id() -> (u128, u64) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    (timestamp, NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed))
}

pub struct GrokAcpServerManager {
    sessions: PersistentSessionRegistry<GrokAcpServer>,
    app_data_directory: PathBuf,
    conversation_locks: Mutex<HashMap<String, Weak<Mutex<()>>>>,
}

impl Default for GrokAcpServerManager {
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("nocterm-grok-default-state"))
    }
}

impl GrokAcpServerManager {
    pub fn new(app_data_directory: impl AsRef<Path>) -> Self {
        Self {
            sessions: PersistentSessionRegistry::new("Grok"),
            app_data_directory: app_data_directory.as_ref().to_path_buf(),
            conversation_locks: Mutex::new(HashMap::new()),
        }
    }

    /// 仅目录创建/回收串行化；取消信号必须在等待此锁之前交给注册表。
    /// 不同对话持有不同锁，后台任务仍可并行启动。
    fn conversation_lock(&self, conversation_id: &str) -> Result<Arc<Mutex<()>>, String> {
        let mut locks = self
            .conversation_locks
            .lock()
            .map_err(|_| "Grok 会话目录锁状态不可用".to_string())?;
        locks.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = locks.get(conversation_id).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(conversation_id.to_string(), Arc::downgrade(&lock));
        Ok(lock)
    }
}

impl PersistentProviderRuntime for GrokAcpServerManager {
    fn id(&self) -> &'static str {
        "grok"
    }

    fn start_turn(
        &self,
        app: AppHandle,
        launch: PersistentProviderLaunch,
        gateway: Arc<AiGatewayState>,
    ) -> Result<bool, String> {
        let PersistentProviderLaunch {
            conversation_id,
            session_id,
            identity,
            native_session_id,
            native_session_identity,
            native_session_lease,
            native_session_service,
            initial_prompt,
            continuation_prompt,
            bridge: _,
            gateway_token,
            bridge_executable: _,
            provider_executable,
            command_policy,
            mcp_dispatcher,
        } = launch;
        let conversation_lock = self.conversation_lock(&conversation_id)?;
        // 锁在 Starting 登记之后取得，但一直保留到注册表接管或回收迟到服务器之后。
        let mut conversation_guard = None;
        let guard = &mut conversation_guard;
        let lock = &conversation_lock;
        let native_conversation_id = conversation_id.clone();
        let app_data_directory = self.app_data_directory.clone();
        self.sessions.start_turn(
            app.clone(),
            PersistentTurnRequest {
                conversation_id,
                session_id,
                identity: identity.clone(),
                initial_prompt,
                continuation_prompt,
                command_policy,
            },
            move |startup_cancellation, termination| {
                // 注册表先登记 Starting，再进入目录临界区，reset 随时能取消握手。
                *guard = Some(
                    lock.lock()
                        .map_err(|_| "Grok 会话目录锁状态不可用".to_string())?,
                );
                if startup_cancellation.load(Ordering::Acquire) {
                    return Err("Grok 会话启动已取消".into());
                }
                native_session_service
                    .ensure_current(&native_session_lease)
                    .map_err(|error| error.message)?;
                GrokAcpServer::spawn(
                    app,
                    GrokAcpServerLaunch {
                        identity,
                        conversation_id: native_conversation_id,
                        native_session_id,
                        native_session_identity,
                        native_session_lease,
                        native_session_service,
                        app_data_directory,
                        bridge_token: gateway_token,
                        provider_executable,
                        gateway,
                        mcp_dispatcher,
                    },
                    startup_cancellation,
                    termination,
                )
            },
        )
    }

    /// ACP cancel 只停止当前 turn；超时未完成时回收进程，避免残留失控工具调用。
    fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        self.sessions.stop_turn(session_id)
    }

    fn reset_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        let conversation_lock = self.conversation_lock(conversation_id)?;
        // 先撤销 Starting，握手按取消标记退出并释放目录锁；不能先等完整握手超时。
        let reset = self.sessions.reset_conversation(conversation_id)?;
        let _conversation_guard = conversation_lock
            .lock()
            .map_err(|_| "Grok 会话目录锁状态不可用".to_string())?;
        let removed = cleanup_grok_conversation(&self.app_data_directory, conversation_id)?;
        Ok(reset || removed)
    }

    fn release_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        let conversation_lock = self.conversation_lock(conversation_id)?;
        let released = self.sessions.reset_conversation(conversation_id)?;
        let _guard = conversation_lock
            .lock()
            .map_err(|_| "Grok 会话目录锁状态不可用".to_string())?;
        // 会话历史由 Grok 留在稳定目录，只有显式 reset/delete 才删除。
        Ok(released)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::ai_provider::ProviderSessionIdentity;

    #[test]
    fn reset_cancels_startup_before_waiting_for_directory_ownership() {
        use std::{
            thread,
            time::{Duration, Instant},
        };
        let (timestamp, sequence) = next_runtime_id();
        let root = std::env::temp_dir().join(format!("nocterm-grok-reset-{timestamp}-{sequence}"));
        let directory = root.join("ai-provider-state/grok/conversations/conversation-one");
        std::fs::create_dir_all(&directory).unwrap();
        let history = directory.join("session.json");
        std::fs::write(&history, b"synthetic history").unwrap();
        let manager = Arc::new(GrokAcpServerManager::new(&root));
        let lock = manager.conversation_lock("conversation-one").unwrap();
        let guard = lock.lock().unwrap();
        let cancellation = manager
            .sessions
            .pending_startup_for_test("conversation-one");
        let resetting = Arc::clone(&manager);
        let worker = thread::spawn(move || resetting.reset_conversation("conversation-one"));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cancellation.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        let cancelled_before_unlock = cancellation.load(Ordering::Acquire);
        let preserved_until_unlock = history.exists();
        drop(guard);
        assert!(worker.join().unwrap().unwrap());
        assert!(
            cancelled_before_unlock,
            "reset must cancel while startup still owns the directory"
        );
        assert!(preserved_until_unlock);
        assert!(!directory.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn navigation_preserves_native_files_but_explicit_reset_removes_them() {
        // 使用合成历史文件验证真实 Runtime 的目录语义，不启动 CLI 或读取用户凭据。
        let (timestamp, sequence) = next_runtime_id();
        let root =
            std::env::temp_dir().join(format!("nocterm-grok-release-{timestamp}-{sequence}"));
        let directory = root.join("ai-provider-state/grok/conversations/conversation-one");
        std::fs::create_dir_all(&directory).unwrap();
        let history = directory.join("session.json");
        std::fs::write(&history, b"synthetic history").unwrap();
        let manager = GrokAcpServerManager::new(&root);

        assert!(!manager.release_conversation("conversation-one").unwrap());
        assert_eq!(std::fs::read(&history).unwrap(), b"synthetic history");
        assert!(manager.reset_conversation("conversation-one").unwrap());
        assert!(!directory.exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn persistent_identity_changes_when_the_terminal_target_changes() {
        let local = ProviderSessionIdentity {
            model: None,
            reasoning_effort: None,
            fast_mode: None,
            connection_id: None,
            target_session_id: Some("local:one".into()),
            working_directory: Some("/tmp".into()),
        };
        let remote = ProviderSessionIdentity {
            model: None,
            reasoning_effort: None,
            fast_mode: None,
            connection_id: Some(7),
            target_session_id: None,
            working_directory: Some("/tmp".into()),
        };
        assert_ne!(local, remote);
        assert_eq!(local, local.clone());
    }
}
