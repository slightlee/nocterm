use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use nocterm_domain::ai_native_session::{
    AiNativeSessionIdentity, AiNativeSessionRecord, AiNativeSessionRepository,
    AiNativeSessionRepositoryError,
};

use crate::error::AppError;

/// 原生会话服务只管理绑定事实；何时允许恢复或降级由具体 Provider 协议决定。
#[derive(Clone)]
pub struct AiNativeSessionService {
    repository: Arc<dyn AiNativeSessionRepository>,
    lifecycle: Arc<NativeSessionLifecycle>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiNativeSessionLease {
    conversation_id: String,
    generation: u64,
    // 最后一个租约释放或生命周期所有者显式 finish 后允许下次启动。
    ownership: Arc<()>,
}

/// 关闭操作的排他所有权；必须持有到进程、目录和数据库清理全部完成。
/// 退出作用域（包括错误返回）后释放占用，旧启动租约始终保持失效。
#[derive(Debug)]
pub struct AiNativeSessionClosing {
    lease: AiNativeSessionLease,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAiNativeSession {
    pub native_session_id: Option<String>,
    pub lease: AiNativeSessionLease,
}

#[derive(Default)]
struct NativeSessionLifecycle {
    current_generations: Mutex<HashMap<String, NativeSessionOwner>>,
    next_generation: AtomicU64,
}

struct NativeSessionOwner {
    generation: u64,
    ownership: Weak<()>,
    closing: bool,
}

impl AiNativeSessionService {
    pub fn new(repository: Arc<dyn AiNativeSessionRepository>) -> Self {
        Self {
            repository,
            lifecycle: Arc::new(NativeSessionLifecycle {
                current_generations: Mutex::new(HashMap::new()),
                next_generation: AtomicU64::new(1),
            }),
        }
    }

    /// 读取兼容绑定并登记本次启动代际。读取、失效旧绑定和登记租约在同一临界区内，
    /// reset/delete 因此可以阻止迟到的 Provider 初始化回写已清理的会话。
    pub fn prepare_session(
        &self,
        conversation_id: &str,
        identity: &AiNativeSessionIdentity,
    ) -> Result<PreparedAiNativeSession, AppError> {
        let mut generations = self
            .lifecycle
            .current_generations
            .lock()
            .map_err(|_| lifecycle_error())?;
        generations.retain(|_, owner| owner.ownership.strong_count() > 0);
        if generations.contains_key(conversation_id) {
            return Err(AppError::new(
                "AI_NATIVE_SESSION_BUSY",
                "当前 AI 会话正在启动、处理请求或关闭，请稍后重试",
                true,
            ));
        }
        let record = self.repository.get(conversation_id).map_err(load_error)?;
        let native_session_id = match record {
            Some(record) if record.identity.compatible_with(identity) => {
                Some(record.native_session_id)
            }
            Some(_) => {
                self.repository
                    .delete(conversation_id)
                    .map_err(delete_error)?;
                None
            }
            None => None,
        };
        let generation = self
            .lifecycle
            .next_generation
            .fetch_add(1, Ordering::Relaxed);
        let ownership = Arc::new(());
        generations.insert(
            conversation_id.to_string(),
            NativeSessionOwner {
                generation,
                ownership: Arc::downgrade(&ownership),
                closing: false,
            },
        );
        Ok(PreparedAiNativeSession {
            native_session_id,
            lease: AiNativeSessionLease {
                conversation_id: conversation_id.to_string(),
                generation,
                ownership,
            },
        })
    }

    /// 仅当前启动代际可以写绑定。持锁完成检查和 upsert，确保 reset/delete 不会夹在中间。
    pub fn save(
        &self,
        lease: &AiNativeSessionLease,
        record: &AiNativeSessionRecord,
    ) -> Result<(), AppError> {
        let generations = self
            .lifecycle
            .current_generations
            .lock()
            .map_err(|_| lifecycle_error())?;
        let is_current = lease.conversation_id == record.conversation_id
            && generations
                .get(&lease.conversation_id)
                .map(|owner| owner.generation)
                == Some(lease.generation);
        if !is_current {
            return Err(stale_lease_error());
        }
        self.repository.upsert(record).map_err(save_error)
    }

    /// Provider 明确报告旧 ID 不存在时，只清理当前启动拥有的绑定。保留租约供新
    /// Session 写回；reset 后到达的旧响应既不能删除新绑定，也不能重新取得写入权。
    pub fn clear_missing_binding(&self, lease: &AiNativeSessionLease) -> Result<(), AppError> {
        let generations = self
            .lifecycle
            .current_generations
            .lock()
            .map_err(|_| lifecycle_error())?;
        if generations
            .get(&lease.conversation_id)
            .map(|owner| owner.generation)
            != Some(lease.generation)
        {
            return Err(stale_lease_error());
        }
        self.repository
            .delete(&lease.conversation_id)
            .map_err(delete_error)
    }

    /// 删除绑定不释放关闭占用；调用方仍需持有 guard 到整个关闭操作返回。
    pub fn delete(&self, closing: &AiNativeSessionClosing) -> Result<(), AppError> {
        self.clear_missing_binding(&closing.lease)
    }

    /// 进程完成后撤销该代际，读管道未收到 EOF 的旧线程不能继续占用或写回。
    /// 按代际比较，保证迟到的收尾不会结束 reset 后已经启动的新一轮。
    pub fn finish(&self, lease: &AiNativeSessionLease) -> Result<(), AppError> {
        let mut generations = self
            .lifecycle
            .current_generations
            .lock()
            .map_err(|_| lifecycle_error())?;
        if generations
            .get(&lease.conversation_id)
            .map(|owner| owner.generation)
            == Some(lease.generation)
        {
            generations.remove(&lease.conversation_id);
        }
        Ok(())
    }

    /// 在耗时探测和真正启动之间复核代际，阻止 reset 前已准备好的请求继续启动。
    pub fn ensure_current(&self, lease: &AiNativeSessionLease) -> Result<(), AppError> {
        let generations = self
            .lifecycle
            .current_generations
            .lock()
            .map_err(|_| lifecycle_error())?;
        if generations
            .get(&lease.conversation_id)
            .map(|owner| owner.generation)
            != Some(lease.generation)
        {
            return Err(stale_lease_error());
        }
        Ok(())
    }

    /// 原子失效旧启动并进入关闭态；锁外等待 Provider 退出时，同对话不能启动或再次关闭。
    /// 不持有全局锁执行耗时清理，其他对话仍可独立启动。
    pub fn begin_close(&self, conversation_id: &str) -> Result<AiNativeSessionClosing, AppError> {
        let mut generations = self
            .lifecycle
            .current_generations
            .lock()
            .map_err(|_| lifecycle_error())?;
        generations.retain(|_, owner| owner.ownership.strong_count() > 0);
        if generations
            .get(conversation_id)
            .is_some_and(|owner| owner.closing)
        {
            return Err(AppError::new(
                "AI_NATIVE_SESSION_BUSY",
                "当前 AI 会话正在关闭，请稍后重试",
                true,
            ));
        }
        let generation = self
            .lifecycle
            .next_generation
            .fetch_add(1, Ordering::Relaxed);
        let ownership = Arc::new(());
        generations.insert(
            conversation_id.to_string(),
            NativeSessionOwner {
                generation,
                ownership: Arc::downgrade(&ownership),
                closing: true,
            },
        );
        Ok(AiNativeSessionClosing {
            lease: AiNativeSessionLease {
                conversation_id: conversation_id.into(),
                generation,
                ownership,
            },
        })
    }
}

fn lifecycle_error() -> AppError {
    AppError::new(
        "AI_NATIVE_SESSION_STATE_UNAVAILABLE",
        "AI 原生会话状态不可用",
        true,
    )
}

fn stale_lease_error() -> AppError {
    AppError::new(
        "AI_NATIVE_SESSION_STALE",
        "AI 会话已被重置，已丢弃迟到的原生会话绑定",
        false,
    )
}

fn load_error(_error: AiNativeSessionRepositoryError) -> AppError {
    AppError::new(
        "AI_NATIVE_SESSION_LOAD_FAILED",
        "读取 AI 原生会话绑定失败",
        true,
    )
}

fn save_error(_error: AiNativeSessionRepositoryError) -> AppError {
    AppError::new(
        "AI_NATIVE_SESSION_SAVE_FAILED",
        "保存 AI 原生会话绑定失败",
        true,
    )
}

fn delete_error(_error: AiNativeSessionRepositoryError) -> AppError {
    AppError::new(
        "AI_NATIVE_SESSION_DELETE_FAILED",
        "清理 AI 原生会话绑定失败",
        true,
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemoryRepository(Mutex<Option<AiNativeSessionRecord>>);

    impl AiNativeSessionRepository for MemoryRepository {
        fn get(
            &self,
            _conversation_id: &str,
        ) -> Result<Option<AiNativeSessionRecord>, AiNativeSessionRepositoryError> {
            Ok(self.0.lock().expect("record lock").clone())
        }

        fn upsert(
            &self,
            record: &AiNativeSessionRecord,
        ) -> Result<(), AiNativeSessionRepositoryError> {
            *self.0.lock().expect("record lock") = Some(record.clone());
            Ok(())
        }

        fn delete(&self, _conversation_id: &str) -> Result<(), AiNativeSessionRepositoryError> {
            *self.0.lock().expect("record lock") = None;
            Ok(())
        }
    }

    fn identity(model: &str) -> AiNativeSessionIdentity {
        AiNativeSessionIdentity {
            provider: "codex".into(),
            model: Some(model.into()),
            reasoning_effort: Some("high".into()),
            fast_mode: Some(false),
            connection_id: Some(7),
            target_session_id: Some("remote:7:terminal".into()),
        }
    }

    #[test]
    fn close_excludes_new_starts_until_process_and_binding_cleanup_complete() {
        let repository = Arc::new(MemoryRepository::default());
        let service = AiNativeSessionService::new(repository.clone());
        let old = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        service
            .save(&old.lease, &record(identity("model"), "old"))
            .unwrap();
        let closing = service.begin_close("conversation").unwrap();

        // 模拟关闭线程等待 Runtime：新启动、重复关闭和旧读线程均不能改变关闭占用。
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    assert_eq!(
                        service
                            .prepare_session("conversation", &identity("different"))
                            .unwrap_err()
                            .code,
                        "AI_NATIVE_SESSION_BUSY"
                    );
                    assert_eq!(
                        service.begin_close("conversation").unwrap_err().code,
                        "AI_NATIVE_SESSION_BUSY"
                    );
                    assert!(
                        service
                            .prepare_session("another-conversation", &identity("model"))
                            .is_ok()
                    );
                    assert_eq!(
                        service
                            .save(&old.lease, &record(identity("model"), "late"))
                            .unwrap_err()
                            .code,
                        "AI_NATIVE_SESSION_STALE"
                    );
                    assert!(service.clear_missing_binding(&old.lease).is_err());
                    service.finish(&old.lease).unwrap();
                })
                .join()
                .unwrap();
        });
        assert_eq!(
            repository
                .get("conversation")
                .unwrap()
                .unwrap()
                .native_session_id,
            "old"
        );
        service.delete(&closing).unwrap();
        assert_eq!(
            service
                .prepare_session("conversation", &identity("model"))
                .unwrap_err()
                .code,
            "AI_NATIVE_SESSION_BUSY"
        );
        drop(closing);

        let next = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        assert!(next.native_session_id.is_none());
        service
            .save(&next.lease, &record(identity("model"), "new"))
            .unwrap();
        service.finish(&old.lease).unwrap();
        service.ensure_current(&next.lease).unwrap();
        assert_eq!(
            repository
                .get("conversation")
                .unwrap()
                .unwrap()
                .native_session_id,
            "new"
        );
    }

    #[test]
    fn failed_close_releases_its_guard_for_retry_without_reviving_old_writes() {
        let service = AiNativeSessionService::new(Arc::new(MemoryRepository::default()));
        let old = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        service
            .save(&old.lease, &record(identity("model"), "saved"))
            .unwrap();
        let close = || -> Result<(), AppError> {
            let _closing = service.begin_close("conversation")?;
            Err(AppError::new(
                "TEST_CLEANUP_FAILED",
                "synthetic cleanup failure",
                true,
            ))
        };
        assert_eq!(close().unwrap_err().code, "TEST_CLEANUP_FAILED");
        assert!(service.ensure_current(&old.lease).is_err());
        let retry = service.begin_close("conversation").unwrap();
        service.delete(&retry).unwrap();
        drop(retry);
        assert!(
            service
                .prepare_session("conversation", &identity("model"))
                .unwrap()
                .native_session_id
                .is_none()
        );
    }

    #[test]
    fn finish_releases_detached_reader_without_invalidating_the_next_generation() {
        let repository = Arc::new(MemoryRepository::default());
        let service = AiNativeSessionService::new(repository);
        let process = service
            .prepare_session("conversation", &identity("gpt-5"))
            .unwrap();
        let reader = process.lease.clone();
        service.finish(&process.lease).unwrap();
        drop(process);
        assert!(service.ensure_current(&reader).is_err());
        let next = service
            .prepare_session("conversation", &identity("gpt-5"))
            .unwrap();
        let record = AiNativeSessionRecord {
            conversation_id: "conversation".into(),
            native_session_id: "new-thread".into(),
            identity: identity("gpt-5"),
            updated_at_ms: 1,
        };
        service.save(&next.lease, &record).unwrap();
        assert!(service.save(&reader, &record).is_err());
        assert!(service.clear_missing_binding(&reader).is_err());
        service.finish(&reader).unwrap();
        service.ensure_current(&next.lease).unwrap();
        assert_eq!(
            service
                .prepare_session("conversation", &identity("gpt-5"))
                .unwrap_err()
                .code,
            "AI_NATIVE_SESSION_BUSY"
        );
        service.finish(&next.lease).unwrap();
        assert_eq!(
            service
                .prepare_session("conversation", &identity("gpt-5"))
                .unwrap()
                .native_session_id,
            Some("new-thread".into())
        );
    }

    #[test]
    fn returns_only_identity_compatible_native_sessions() {
        let repository = Arc::new(MemoryRepository::default());
        let service = AiNativeSessionService::new(repository.clone());
        let prepared = service
            .prepare_session("conversation", &identity("gpt-5"))
            .expect("prepare first session");
        service
            .save(
                &prepared.lease,
                &AiNativeSessionRecord {
                    conversation_id: "conversation".into(),
                    native_session_id: "thread-one".into(),
                    identity: identity("gpt-5"),
                    updated_at_ms: 1,
                },
            )
            .expect("save binding");
        drop(prepared);

        assert_eq!(
            service
                .prepare_session("conversation", &identity("gpt-5"))
                .expect("load compatible")
                .native_session_id,
            Some("thread-one".into())
        );
        assert_eq!(
            service
                .prepare_session("conversation", &identity("gpt-6"))
                .expect("invalidate incompatible")
                .native_session_id,
            None,
        );
        assert!(repository.0.lock().expect("record lock").is_none());
    }

    #[test]
    fn reset_rejects_a_late_save_from_an_older_generation() {
        let repository = Arc::new(MemoryRepository::default());
        let service = AiNativeSessionService::new(repository.clone());
        let prepared = service
            .prepare_session("conversation", &identity("gpt-5"))
            .expect("prepare session");

        let closing = service.begin_close("conversation").unwrap();
        service.delete(&closing).expect("reset session");
        let error = service
            .save(
                &prepared.lease,
                &AiNativeSessionRecord {
                    conversation_id: "conversation".into(),
                    native_session_id: "late-thread".into(),
                    identity: identity("gpt-5"),
                    updated_at_ms: 2,
                },
            )
            .expect_err("late save must fail");

        assert_eq!(error.code, "AI_NATIVE_SESSION_STALE");
        assert!(repository.0.lock().expect("record lock").is_none());
    }

    fn record(identity: AiNativeSessionIdentity, id: &str) -> AiNativeSessionRecord {
        AiNativeSessionRecord {
            conversation_id: "conversation".into(),
            native_session_id: id.into(),
            identity,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn every_provider_can_save_and_resume_a_fallback_replacement() {
        for provider in ["codex", "claude-code", "grok"] {
            let service = AiNativeSessionService::new(Arc::new(MemoryRepository::default()));
            let mut identity = identity("model");
            identity.provider = provider.into();
            let first = service.prepare_session("conversation", &identity).unwrap();
            service
                .save(&first.lease, &record(identity.clone(), "missing"))
                .unwrap();
            drop(first);

            let fallback = service.prepare_session("conversation", &identity).unwrap();
            assert_eq!(fallback.native_session_id.as_deref(), Some("missing"));
            service.clear_missing_binding(&fallback.lease).unwrap();
            service
                .save(&fallback.lease, &record(identity.clone(), "replacement"))
                .unwrap();
            drop(fallback);
            assert_eq!(
                service
                    .prepare_session("conversation", &identity)
                    .unwrap()
                    .native_session_id
                    .as_deref(),
                Some("replacement")
            );
        }
    }

    #[test]
    fn rejected_start_does_not_steal_ownership_or_delete_the_existing_binding() {
        let repository = Arc::new(MemoryRepository::default());
        let service = AiNativeSessionService::new(repository.clone());
        let first = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        service
            .save(&first.lease, &record(identity("model"), "first"))
            .unwrap();
        assert_eq!(
            service
                .prepare_session("conversation", &identity("different-model"))
                .unwrap_err()
                .code,
            "AI_NATIVE_SESSION_BUSY"
        );
        service
            .save(&first.lease, &record(identity("model"), "still-current"))
            .unwrap();
        assert_eq!(
            repository
                .get("conversation")
                .unwrap()
                .unwrap()
                .native_session_id,
            "still-current"
        );

        // 克隆租约也持有启动权，只有最后一个使用者退出才能继续发送。
        let clone = first.lease.clone();
        drop(first);
        assert!(
            service
                .prepare_session("conversation", &identity("model"))
                .is_err()
        );
        drop(clone);
        assert!(
            service
                .prepare_session("conversation", &identity("model"))
                .is_ok()
        );
    }

    #[test]
    fn stale_fallback_cannot_remove_a_replacement_after_reset() {
        let repository = Arc::new(MemoryRepository::default());
        let service = AiNativeSessionService::new(repository.clone());
        let old = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        let closing = service.begin_close("conversation").unwrap();
        service.delete(&closing).unwrap();
        drop(closing);
        let new = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        service
            .save(&new.lease, &record(identity("model"), "new"))
            .unwrap();
        assert_eq!(
            service.clear_missing_binding(&old.lease).unwrap_err().code,
            "AI_NATIVE_SESSION_STALE"
        );
        assert_eq!(
            repository
                .get("conversation")
                .unwrap()
                .unwrap()
                .native_session_id,
            "new"
        );
    }

    #[test]
    fn release_invalidates_pending_writes_but_keeps_persisted_context() {
        let service = AiNativeSessionService::new(Arc::new(MemoryRepository::default()));
        let first = service
            .prepare_session("conversation", &identity("model"))
            .unwrap();
        service
            .save(&first.lease, &record(identity("model"), "saved"))
            .unwrap();
        let closing = service.begin_close("conversation").unwrap();
        assert_eq!(
            service
                .save(&first.lease, &record(identity("model"), "late"))
                .unwrap_err()
                .code,
            "AI_NATIVE_SESSION_STALE"
        );
        drop(closing);
        assert_eq!(
            service
                .prepare_session("conversation", &identity("model"))
                .unwrap()
                .native_session_id
                .as_deref(),
            Some("saved")
        );
    }

    #[test]
    fn restart_can_resume_legacy_ssh_binding_with_a_new_tab_id() {
        let repository = Arc::new(MemoryRepository::default());
        repository
            .upsert(&record(identity("model"), "persisted"))
            .unwrap();
        let service = AiNativeSessionService::new(repository);
        let mut reconnected = identity("model");
        reconnected.target_session_id = Some("remote:7:new-tab".into());
        assert_eq!(
            service
                .prepare_session("conversation", &reconnected)
                .unwrap()
                .native_session_id
                .as_deref(),
            Some("persisted")
        );
        reconnected.connection_id = Some(8);
        assert!(
            service
                .prepare_session("conversation", &reconnected)
                .unwrap()
                .native_session_id
                .is_none()
        );
    }

    #[test]
    fn local_target_and_model_options_must_still_match() {
        let mut original = identity("model");
        original.connection_id = None;
        original.target_session_id = Some("local:one".into());
        for change in ["target", "provider", "model", "effort", "speed"] {
            let mut other = original.clone();
            match change {
                "target" => other.target_session_id = Some("local:two".into()),
                "provider" => other.provider = "grok".into(),
                "model" => other.model = Some("other".into()),
                "effort" => other.reasoning_effort = Some("low".into()),
                "speed" => other.fast_mode = Some(true),
                _ => unreachable!(),
            }
            assert!(!original.compatible_with(&other), "{change}");
        }
    }
}
