//! 文件请求与连接配置更新共享同一连接锁，目标版本在当前进程内单调递增。
use crate::error::AppError;
use nocterm_domain::connection::ConnectionProfile;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

#[derive(Debug, PartialEq, Eq)]
struct TargetIdentity {
    // 展示名称、分组和启动目录不改变远端文件身份，不纳入目标版本。
    host: String,
    port: u16,
    username: String,
    authentication: &'static str,
    private_key_path: Option<String>,
}

#[derive(Default)]
pub struct FileTargetState {
    identity: Option<TargetIdentity>,
    revision: u64,
}
static NEXT_REVISION: AtomicU64 = AtomicU64::new(1);
// 不使用 updated_at 秒级时间：同一秒内改动或切回旧主机也必须得到新版本。
// 状态仅在当前进程中使用，编辑内容不跨进程恢复，因此无需持久化该序列。

impl FileTargetState {
    /// 版本不包含主机、用户或凭据，前端只能把打开时取得的版本原样带回。
    pub fn observe(&mut self, profile: &ConnectionProfile) -> String {
        let identity = TargetIdentity {
            host: profile.host.clone(),
            port: profile.port,
            username: profile.username.clone(),
            authentication: profile.authentication.as_str(),
            private_key_path: profile.private_key_path.clone(),
        };
        if self.identity.as_ref() != Some(&identity) {
            self.identity = Some(identity);
            self.invalidate();
        }
        self.revision.to_string()
    }

    pub fn invalidate(&mut self) {
        // 已保存密码变更未体现在 identity 中，由凭据变更入口显式失效。
        self.revision = NEXT_REVISION.fetch_add(1, Ordering::Relaxed);
    }

    pub fn require_revision(
        &mut self,
        profile: &ConnectionProfile,
        expected: &str,
    ) -> Result<String, AppError> {
        // 先观察权威资料再比较旧版本；客户端传 force 不能更换保存目标。
        let current = self.observe(profile);
        if current != expected {
            return Err(AppError::new(
                "SFTP_REMOTE_TARGET_CHANGED",
                "连接已变更，请保留编辑内容并重新打开文件",
                false,
            ));
        }
        Ok(current)
    }
}

#[derive(Default)]
pub struct FileTargetRegistry(Mutex<HashMap<i64, Arc<Mutex<FileTargetState>>>>);
// 保留已删除连接的版本槽位，避免同一进程内删除/重建后旧版本重新变得有效。

impl FileTargetRegistry {
    /// 调用方持有返回的锁直到读写结束；配置更新用 try_lock 拒绝与保存交错执行。
    pub fn connection(&self, id: i64) -> Result<Arc<Mutex<FileTargetState>>, AppError> {
        let mut entries = self
            .0
            .lock()
            .map_err(|_| AppError::new("SFTP_REMOTE_TARGET_CHANGED", "连接状态不可用", true))?;
        Ok(entries.entry(id).or_default().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nocterm_domain::connection::AuthenticationMethod;
    fn profile() -> ConnectionProfile {
        ConnectionProfile {
            id: 1,
            name: "test".into(),
            host: "old.example.test".into(),
            port: 22,
            username: "tester".into(),
            authentication: AuthenticationMethod::SshAgent,
            created_at: 0,
            updated_at: 0,
            group_id: None,
            group_name: None,
            remark: None,
            sync_mode: String::new(),
            execution_target: String::new(),
            remote_initial_path: None,
            icon: None,
            sort_order: None,
            private_key_path: None,
            credential_kind: None,
            credential_status: "missing".into(),
        }
    }
    #[test]
    fn target_changes_reject_old_revisions_even_when_updated_within_the_same_second() {
        let mut state = FileTargetState::default();
        let mut profile = profile();
        let original = state.observe(&profile);
        profile.name = "renamed".into();
        assert_eq!(state.observe(&profile), original);
        profile.host = "other.example.test".into();
        assert_eq!(
            state
                .require_revision(&profile, &original)
                .unwrap_err()
                .code,
            "SFTP_REMOTE_TARGET_CHANGED"
        );
        profile.host = "old.example.test".into();
        assert_ne!(state.observe(&profile), original);
        let before_password_change = state.observe(&profile);
        state.invalidate();
        assert!(
            state
                .require_revision(&profile, &before_password_change)
                .is_err()
        );
    }
    #[test]
    fn registry_serializes_operations_only_for_the_same_connection() {
        let registry = FileTargetRegistry::default();
        let first = registry.connection(1).unwrap();
        let same = registry.connection(1).unwrap();
        let other = registry.connection(2).unwrap();
        let _saving = first.lock().unwrap();
        assert!(same.try_lock().is_err());
        assert!(other.try_lock().is_ok());
    }
}
