//! 持续 AI Provider 的对话级会话注册、启动互斥与统一取消策略。

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use crate::{commands::ai_provider::ProviderSessionIdentity, state::AiCommandPolicy};

const TURN_CANCEL_GRACE_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// 初始化协议按短间隔轮询取消标记，停止按钮无需等待完整握手超时。
pub fn receive_startup_line(
    lines: &mpsc::Receiver<Result<String, String>>,
    deadline: std::time::Instant,
    cancellation: &AtomicBool,
    action: &str,
    disconnected: &str,
) -> Result<String, String> {
    loop {
        if cancellation.load(Ordering::Acquire) {
            return Err(format!("{action}已取消"));
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(format!("{action}超时"));
        }
        match lines.recv_timeout(remaining.min(STARTUP_CANCEL_POLL_INTERVAL)) {
            Ok(result) => return result,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(disconnected.to_string()),
        }
    }
}

/// Provider 协议只实现自身差异；缓存、并发和资源移除由注册表统一负责。
pub trait PersistentProviderSession: Send + Sync + 'static {
    /// 生命周期注册表只转发上下文；具体 Provider 决定是否需要 Tauri AppHandle。
    type TurnContext;

    fn matches_identity(&self, identity: &ProviderSessionIdentity) -> bool;
    fn is_alive(&self) -> bool;
    fn start_turn(
        &self,
        context: Self::TurnContext,
        session_id: String,
        prompt: String,
        command_policy: AiCommandPolicy,
    ) -> Result<(), String>;
    fn stop_turn(&self, session_id: &str) -> Result<bool, String>;
    fn shutdown(&self);
    fn shutdown_if_turn_active(&self, session_id: &str);
}

/// 公共 turn 数据不包含协议参数，避免厂商私有字段进入统一生命周期层。
pub struct PersistentTurnRequest {
    pub conversation_id: String,
    pub session_id: String,
    pub identity: ProviderSessionIdentity,
    pub initial_prompt: String,
    pub continuation_prompt: String,
    pub command_policy: AiCommandPolicy,
}

/// 子进程退出时按 generation 移除缓存，迟到通知不能删除后续重建的新会话。
#[derive(Clone)]
pub struct SessionTermination {
    callback: Arc<dyn Fn() + Send + Sync>,
}

impl SessionTermination {
    pub fn notify(&self) {
        (self.callback)();
    }
}

enum SessionSlot<S> {
    Starting {
        generation: u64,
        session_id: String,
        cancellation: Arc<AtomicBool>,
    },
    Ready {
        generation: u64,
        server: Arc<S>,
    },
}

struct RegistryInner<S> {
    provider_label: &'static str,
    sessions: Mutex<HashMap<String, SessionSlot<S>>>,
    next_generation: AtomicU64,
}

pub struct PersistentSessionRegistry<S: PersistentProviderSession> {
    inner: Arc<RegistryInner<S>>,
}

impl<S: PersistentProviderSession> PersistentSessionRegistry<S> {
    pub fn new(provider_label: &'static str) -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                provider_label,
                sessions: Mutex::new(HashMap::new()),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    /// 全局锁只保护槽位转换；协议握手和进程关闭都在锁外执行。
    pub fn start_turn<F>(
        &self,
        context: S::TurnContext,
        request: PersistentTurnRequest,
        spawn: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(Arc<AtomicBool>, SessionTermination) -> Result<Arc<S>, String>,
    {
        let PersistentTurnRequest {
            conversation_id,
            session_id,
            identity,
            initial_prompt,
            continuation_prompt,
            command_policy,
        } = request;
        let generation = self.inner.next_generation.fetch_add(1, Ordering::Relaxed);
        let startup_cancellation = Arc::new(AtomicBool::new(false));
        let mut previous = None;
        let mut reusable = None;

        {
            let mut sessions = self.inner.sessions.lock().map_err(|_| self.state_error())?;
            match sessions.remove(&conversation_id) {
                Some(SessionSlot::Ready {
                    generation: existing_generation,
                    server,
                }) if server.is_alive() && server.matches_identity(&identity) => {
                    reusable = Some(Arc::clone(&server));
                    sessions.insert(
                        conversation_id.clone(),
                        SessionSlot::Ready {
                            generation: existing_generation,
                            server,
                        },
                    );
                }
                Some(SessionSlot::Ready { server, .. }) => {
                    previous = Some(server);
                    sessions.insert(
                        conversation_id.clone(),
                        SessionSlot::Starting {
                            generation,
                            session_id: session_id.clone(),
                            cancellation: Arc::clone(&startup_cancellation),
                        },
                    );
                }
                Some(slot @ SessionSlot::Starting { .. }) => {
                    sessions.insert(conversation_id.clone(), slot);
                    return Err(format!(
                        "{} 会话正在启动，请稍后重试",
                        self.inner.provider_label
                    ));
                }
                None => {
                    sessions.insert(
                        conversation_id.clone(),
                        SessionSlot::Starting {
                            generation,
                            session_id: session_id.clone(),
                            cancellation: Arc::clone(&startup_cancellation),
                        },
                    );
                }
            }
        }

        if let Some(server) = previous {
            server.shutdown();
        }

        let (server, is_new) = if let Some(server) = reusable {
            (server, false)
        } else {
            // 目标切换会先关闭旧进程；若用户在关闭期间停止，不应再启动一个注定被取消的新进程。
            if startup_cancellation.load(Ordering::Acquire) {
                self.remove_starting(&conversation_id, generation);
                return Err(format!("{} 会话启动已取消", self.inner.provider_label));
            }
            let termination = self.termination(conversation_id.clone(), generation);
            let server = match spawn(Arc::clone(&startup_cancellation), termination) {
                Ok(server) => server,
                Err(error) => {
                    self.remove_starting(&conversation_id, generation);
                    return Err(error);
                }
            };
            if startup_cancellation.load(Ordering::Acquire) {
                server.shutdown();
                self.remove_starting(&conversation_id, generation);
                return Err(format!("{} 会话启动已取消", self.inner.provider_label));
            }
            let registered = {
                let mut sessions = self.inner.sessions.lock().map_err(|_| self.state_error())?;
                let matches_startup = matches!(
                    sessions.get(&conversation_id),
                    Some(SessionSlot::Starting { generation: current, .. }) if *current == generation
                );
                if matches_startup {
                    sessions.insert(
                        conversation_id.clone(),
                        SessionSlot::Ready {
                            generation,
                            server: Arc::clone(&server),
                        },
                    );
                }
                matches_startup
            };
            if !registered {
                server.shutdown();
                return Err(format!("{} 会话启动已取消", self.inner.provider_label));
            }
            (server, true)
        };

        let prompt = if is_new {
            initial_prompt
        } else {
            continuation_prompt
        };
        if let Err(error) = server.start_turn(context, session_id, prompt, command_policy) {
            if is_new || !server.is_alive() {
                self.remove_ready(&conversation_id, &server);
                server.shutdown();
            }
            return Err(error);
        }
        Ok(is_new)
    }

    /// 启动阶段只有取消标记；进程就绪后再委托协议实现发送 cancel/interrupt。
    pub fn stop_turn(&self, session_id: &str) -> Result<bool, String> {
        let servers = {
            let sessions = self.inner.sessions.lock().map_err(|_| self.state_error())?;
            for slot in sessions.values() {
                if let SessionSlot::Starting {
                    session_id: starting_session,
                    cancellation,
                    ..
                } = slot
                    && starting_session == session_id
                {
                    cancellation.store(true, Ordering::Release);
                    return Ok(true);
                }
            }
            sessions
                .values()
                .filter_map(|slot| match slot {
                    SessionSlot::Ready { server, .. } => Some(Arc::clone(server)),
                    SessionSlot::Starting { .. } => None,
                })
                .collect::<Vec<_>>()
        };
        for server in servers {
            if server.stop_turn(session_id)? {
                let interrupted_server = Arc::clone(&server);
                let interrupted_session_id = session_id.to_string();
                thread::spawn(move || {
                    thread::sleep(TURN_CANCEL_GRACE_TIMEOUT);
                    interrupted_server.shutdown_if_turn_active(&interrupted_session_id);
                });
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn reset_conversation(&self, conversation_id: &str) -> Result<bool, String> {
        let slot = self
            .inner
            .sessions
            .lock()
            .map_err(|_| self.state_error())?
            .remove(conversation_id);
        match slot {
            Some(SessionSlot::Starting { cancellation, .. }) => {
                cancellation.store(true, Ordering::Release);
                Ok(true)
            }
            Some(SessionSlot::Ready { server, .. }) => {
                server.shutdown();
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn termination(&self, conversation_id: String, generation: u64) -> SessionTermination {
        let inner: Weak<RegistryInner<S>> = Arc::downgrade(&self.inner);
        SessionTermination {
            callback: Arc::new(move || {
                let Some(inner) = inner.upgrade() else {
                    return;
                };
                if let Ok(mut sessions) = inner.sessions.lock() {
                    let current_generation =
                        sessions.get(&conversation_id).map(|slot| match slot {
                            SessionSlot::Starting { generation, .. }
                            | SessionSlot::Ready { generation, .. } => *generation,
                        });
                    if current_generation == Some(generation) {
                        sessions.remove(&conversation_id);
                    }
                }
            }),
        }
    }

    fn remove_starting(&self, conversation_id: &str, generation: u64) {
        if let Ok(mut sessions) = self.inner.sessions.lock()
            && matches!(
                sessions.get(conversation_id),
                Some(SessionSlot::Starting { generation: current, .. }) if *current == generation
            )
        {
            sessions.remove(conversation_id);
        }
    }

    fn remove_ready(&self, conversation_id: &str, server: &Arc<S>) {
        if let Ok(mut sessions) = self.inner.sessions.lock()
            && matches!(
                sessions.get(conversation_id),
                Some(SessionSlot::Ready { server: current, .. }) if Arc::ptr_eq(current, server)
            )
        {
            sessions.remove(conversation_id);
        }
    }

    fn state_error(&self) -> String {
        format!("{} 会话状态不可用", self.inner.provider_label)
    }
}

impl<S: PersistentProviderSession> Drop for PersistentSessionRegistry<S> {
    fn drop(&mut self) {
        let slots = self
            .inner
            .sessions
            .lock()
            .map(|mut sessions| sessions.drain().map(|(_, slot)| slot).collect::<Vec<_>>())
            .unwrap_or_default();
        // shutdown 会触发 termination callback，必须先释放注册表锁。
        for slot in slots {
            match slot {
                SessionSlot::Starting { cancellation, .. } => {
                    cancellation.store(true, Ordering::Release);
                }
                SessionSlot::Ready { server, .. } => server.shutdown(),
            }
        }
    }
}

#[cfg(test)]
#[path = "ai_persistent/tests.rs"]
mod tests;
