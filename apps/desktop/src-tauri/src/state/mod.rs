mod local_terminal;
pub mod session_password;

pub use local_terminal::LocalTerminalRegistry;
pub(crate) use local_terminal::local_completion_status;

use crate::commands::{
    ai_process::AiProcessManager, ai_runtime::PersistentProviderRuntimeRegistry,
};

use nocterm_application::{
    ai_audit::AiAuditService,
    connection::ConnectionService,
    health::HealthService,
    settings::SettingsService,
    terminal::{LocalTerminalService, TerminalService},
};
use nocterm_infrastructure::{
    ssh::{SshTerminalManager, sftp::SftpManager},
    terminal::LocalTerminalManager,
};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use self::session_password::SessionPasswords;

pub struct AppState {
    health_service: HealthService,
    connection_service: ConnectionService,
    settings_service: SettingsService,
    ai_audit_service: AiAuditService,
    terminal_service: TerminalService,
    local_terminal_service: LocalTerminalService,
    /// 进程内 SFTP 会话管理器：与终端共用 russh 后端，承载远程文件浏览与传输。
    sftp_manager: Arc<SftpManager>,
    /// 终端交互输入的登录口令缓存，仅内存驻留，供同连接的其他标签与 SFTP 复用。
    /// 用 `Arc` 是为了让终端的输出线程能持有一份句柄，在会话收尾时归还租约。
    session_passwords: Arc<SessionPasswords>,
    ai_processes: Arc<AiProcessManager>,
    ai_provider_runtimes: Arc<PersistentProviderRuntimeRegistry>,
    ai_gateway: Arc<AiGatewayState>,
    local_terminals: Arc<LocalTerminalRegistry>,
}

/// MCP Bridge 的进程绑定表。token 只覆盖一次性任务或一个持续 Provider 会话，
/// 始终固定到同一终端目标；这里只保存内存绑定，不保存命令、输出或凭据。
#[derive(Default)]
pub struct AiGatewayState {
    bindings: Mutex<HashMap<String, AiGatewayEntry>>,
    approvals: Mutex<HashMap<String, AiApprovalWaiter>>,
    tool_calls: Mutex<HashMap<String, VecDeque<Instant>>>,
    observed_tool_calls: Mutex<HashSet<String>>,
    endpoint: Mutex<Option<String>>,
}

const AI_TOOL_RATE_WINDOW: Duration = Duration::from_secs(60);
const AI_TOOL_RATE_LIMIT: usize = 30;

struct AiApprovalWaiter {
    task_token: String,
    session_id: String,
    sender: mpsc::Sender<AiApprovalDecision>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiApprovalDecision {
    Approved,
    Rejected,
    Revoked,
}

/// 通用终端命令的授权策略。策略绑定到 AI turn，未知值由 DTO 层拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiCommandPolicy {
    /// 仅分析，不允许 Provider 调用通用终端执行入口。
    DenyAll,
    /// 仅自动执行严格受限的简单只读命令，其余命令仍需确认。
    AutoSafe,
    /// 每次访问终端都显示确认卡片，包括结构化只读检查。
    ConfirmEach,
    /// 当前会话内的终端操作无需确认；仍受目标绑定、审计、限流、超时和取消约束。
    FullAccess,
}

/// Provider 进程可以跨 turn 存活，但工具授权只在一个活动 turn 内有效。
struct AiGatewayEntry {
    binding: AiGatewayBinding,
    active: bool,
    cancellation: Arc<AtomicBool>,
}

/// 单次 Gateway 请求取得的不可变授权快照；停止任务会原子地翻转取消标记。
#[derive(Clone)]
pub struct AiGatewayAccess {
    pub binding: AiGatewayBinding,
    pub cancellation: Arc<AtomicBool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AiTarget {
    Ssh {
        connection_id: i64,
    },
    Local {
        session_id: String,
        terminal_id: String,
    },
}

/// Token 绑定同时携带脱敏审计身份；Provider 请求不能覆盖这些后端元数据。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiGatewayBinding {
    pub target: AiTarget,
    pub provider: String,
    pub session_id: String,
    pub command_policy: AiCommandPolicy,
}

impl AiGatewayState {
    pub fn bind(&self, token: String, binding: AiGatewayBinding) -> Result<(), String> {
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())?;
        if bindings.contains_key(&token) {
            return Err("AI Bridge token 冲突，请重试".to_string());
        }
        bindings.insert(
            token,
            AiGatewayEntry {
                binding,
                active: false,
                cancellation: Arc::new(AtomicBool::new(true)),
            },
        );
        Ok(())
    }

    /// 非活动 token 仍可完成 MCP initialize/tools/list，但 tools/call 会由活动状态拒绝。
    pub fn access_for(&self, token: &str) -> Result<Option<AiGatewayAccess>, String> {
        self.bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())
            .map(|bindings| {
                bindings.get(token).map(|entry| AiGatewayAccess {
                    binding: entry.binding.clone(),
                    cancellation: Arc::clone(&entry.cancellation),
                })
            })
    }

    /// 激活一个新 turn 前先撤销旧快照；持续 Provider 只能复用传输，不能复用授权。
    pub fn activate_session(
        &self,
        token: &str,
        session_id: String,
        command_policy: AiCommandPolicy,
    ) -> Result<(), String> {
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())?;
        let entry = bindings
            .get_mut(token)
            .ok_or_else(|| "AI Bridge 任务已结束，请重新发起请求".to_string())?;
        entry.active = false;
        entry.cancellation.store(true, Ordering::Release);
        self.clear_token_runtime_state(token)?;
        entry.binding.session_id = session_id;
        entry.binding.command_policy = command_policy;
        entry.cancellation = Arc::new(AtomicBool::new(false));
        entry.active = true;
        Ok(())
    }

    /// 停止或完成一个 turn 时保留 Provider 连接，但立即关闭该 turn 的全部工具能力。
    pub fn deactivate_session(&self, session_id: &str) -> Result<bool, String> {
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())?;
        let tokens = bindings
            .iter_mut()
            .filter_map(|(token, entry)| {
                (entry.active && entry.binding.session_id == session_id).then(|| {
                    entry.active = false;
                    entry.cancellation.store(true, Ordering::Release);
                    token.clone()
                })
            })
            .collect::<Vec<_>>();
        for token in &tokens {
            self.clear_token_runtime_state(token)?;
        }
        Ok(!tokens.is_empty())
    }

    pub fn revoke(&self, token: &str) {
        if let Ok(mut bindings) = self.bindings.lock() {
            if let Some(entry) = bindings.remove(token) {
                entry.cancellation.store(true, Ordering::Release);
            }
            let _ = self.clear_token_runtime_state(token);
        }
    }

    /// 调用方持有 bindings 锁，锁顺序固定为 bindings -> calls -> observations -> approvals。
    fn clear_token_runtime_state(&self, token: &str) -> Result<(), String> {
        self.tool_calls
            .lock()
            .map_err(|_| "AI Bridge 限流状态不可用".to_string())?
            .remove(token);
        self.observed_tool_calls
            .lock()
            .map_err(|_| "AI Bridge 调用状态不可用".to_string())?
            .remove(token);
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| "AI 命令审批状态不可用".to_string())?;
        let approval_ids = approvals
            .iter()
            .filter(|(_, waiter)| waiter.task_token == token)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for approval_id in approval_ids {
            if let Some(waiter) = approvals.remove(&approval_id) {
                let _ = waiter.sender.send(AiApprovalDecision::Revoked);
            }
        }
        Ok(())
    }

    pub fn request_approval(
        &self,
        task_token: String,
        approval_id: String,
    ) -> Result<mpsc::Receiver<AiApprovalDecision>, String> {
        // 与 revoke 保持 bindings -> approvals 的加锁顺序，避免 token 撤销后插入孤立审批。
        let bindings = self
            .bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())?;
        let Some(entry) = bindings.get(&task_token).filter(|entry| entry.active) else {
            return Err("AI Bridge 任务已结束，请重新发起请求".to_string());
        };
        let session_id = entry.binding.session_id.clone();
        let (sender, receiver) = mpsc::channel();
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| "AI 命令审批状态不可用".to_string())?;
        // 当前 UI 每个任务只展示一条确认请求；并行请求必须显式失败，不能覆盖或隐藏命令。
        if approvals
            .values()
            .any(|waiter| waiter.task_token == task_token)
        {
            return Err("当前 AI 任务已有终端命令等待确认".to_string());
        }
        approvals.insert(
            approval_id,
            AiApprovalWaiter {
                task_token,
                session_id,
                sender,
            },
        );
        drop(bindings);
        Ok(receiver)
    }

    pub fn resolve_approval(
        &self,
        approval_id: &str,
        session_id: &str,
        approved: bool,
    ) -> Result<bool, String> {
        let mut approvals = self
            .approvals
            .lock()
            .map_err(|_| "AI 命令审批状态不可用".to_string())?;
        if approvals
            .get(approval_id)
            .is_none_or(|waiter| waiter.session_id != session_id)
        {
            return Ok(false);
        }
        let resolved = approvals.remove(approval_id).is_some_and(|waiter| {
            waiter
                .sender
                .send(if approved {
                    AiApprovalDecision::Approved
                } else {
                    AiApprovalDecision::Rejected
                })
                .is_ok()
        });
        Ok(resolved)
    }

    /// 每个任务 token 独立限流，防止异常 Provider 在短时间内占满 SSH channel 和线程。
    /// 先确认 token 仍有效，再登记调用；达到上限时 fail-closed，不执行远端操作。
    pub fn check_tool_rate(&self, token: &str) -> Result<(), String> {
        let bindings = self
            .bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())?;
        if bindings.get(token).is_none_or(|entry| !entry.active) {
            return Err("AI Bridge 任务已结束，请重新发起请求".to_string());
        }
        let now = Instant::now();
        let mut calls = self
            .tool_calls
            .lock()
            .map_err(|_| "AI Bridge 限流状态不可用".to_string())?;
        let recent = calls.entry(token.to_string()).or_default();
        while recent
            .front()
            .is_some_and(|instant| now.duration_since(*instant) >= AI_TOOL_RATE_WINDOW)
        {
            recent.pop_front();
        }
        if recent.len() >= AI_TOOL_RATE_LIMIT {
            return Err("AI 工具调用过于频繁，请稍后重试".to_string());
        }
        recent.push_back(now);
        drop(bindings);
        Ok(())
    }

    /// 只记录 MCP/ACP 清单中的有效工具；未知调用不能解锁 Provider 输出。
    pub fn mark_tool_call(&self, token: &str) -> Result<(), String> {
        let bindings = self
            .bindings
            .lock()
            .map_err(|_| "AI Bridge 目标状态不可用".to_string())?;
        if bindings.get(token).is_none_or(|entry| !entry.active) {
            return Err("AI Bridge 任务已结束，请重新发起请求".to_string());
        }
        self.observed_tool_calls
            .lock()
            .map_err(|_| "AI Bridge 调用状态不可用".to_string())?
            .insert(token.to_string());
        Ok(())
    }

    /// Headless Provider 只在真实有效请求进入 Gateway 后才能发布终端相关回答。
    pub fn has_tool_calls(&self, token: &str) -> Result<bool, String> {
        self.observed_tool_calls
            .lock()
            .map_err(|_| "AI Bridge 调用状态不可用".to_string())
            .map(|calls| calls.contains(token))
    }

    pub fn set_endpoint(&self, endpoint: String) {
        if let Ok(mut value) = self.endpoint.lock() {
            *value = Some(endpoint);
        }
    }

    pub fn endpoint(&self) -> Option<String> {
        self.endpoint.lock().ok()?.clone()
    }
}

impl AppState {
    pub fn new(
        health_service: HealthService,
        connection_service: ConnectionService,
        settings_service: SettingsService,
        ai_audit_service: AiAuditService,
    ) -> Self {
        Self {
            health_service,
            connection_service,
            settings_service,
            ai_audit_service,
            terminal_service: TerminalService::new(Arc::new(SshTerminalManager::default())),
            local_terminal_service: LocalTerminalService::new(Arc::new(
                LocalTerminalManager::default(),
            )),
            sftp_manager: Arc::new(SftpManager::default()),
            session_passwords: Arc::new(SessionPasswords::default()),
            ai_processes: Arc::new(AiProcessManager::default()),
            ai_provider_runtimes: Arc::new(PersistentProviderRuntimeRegistry::default()),
            ai_gateway: Arc::new(AiGatewayState::default()),
            local_terminals: Arc::new(LocalTerminalRegistry::default()),
        }
    }

    pub fn health_service(&self) -> &HealthService {
        &self.health_service
    }

    pub fn connection_service(&self) -> &ConnectionService {
        &self.connection_service
    }

    pub fn settings_service(&self) -> &SettingsService {
        &self.settings_service
    }

    pub fn ai_audit_service(&self) -> &AiAuditService {
        &self.ai_audit_service
    }

    pub fn terminal_service(&self) -> &TerminalService {
        &self.terminal_service
    }

    pub fn local_terminal_service(&self) -> &LocalTerminalService {
        &self.local_terminal_service
    }

    /// 返回共享的 SFTP 会话管理器，供远程文件命令复用同一会话池。
    pub fn sftp_manager(&self) -> &Arc<SftpManager> {
        &self.sftp_manager
    }

    /// 返回会话级口令缓存：终端交互输入的口令由此供 SFTP 与后续终端标签复用。
    /// 返回 `Arc` 引用而非裸引用，调用方可克隆出句柄交给会话线程归还租约。
    pub fn session_passwords(&self) -> &Arc<SessionPasswords> {
        &self.session_passwords
    }

    pub fn ai_processes(&self) -> &Arc<AiProcessManager> {
        &self.ai_processes
    }

    pub fn ai_provider_runtimes(&self) -> &Arc<PersistentProviderRuntimeRegistry> {
        &self.ai_provider_runtimes
    }

    pub fn ai_gateway(&self) -> &Arc<AiGatewayState> {
        &self.ai_gateway
    }

    pub fn local_terminals(&self) -> &Arc<LocalTerminalRegistry> {
        &self.local_terminals
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::atomic::Ordering, time::Duration};

    use super::{AiApprovalDecision, AiCommandPolicy, AiGatewayBinding, AiGatewayState, AiTarget};

    fn ssh_binding() -> AiGatewayBinding {
        AiGatewayBinding {
            target: AiTarget::Ssh { connection_id: 1 },
            provider: "codex".into(),
            session_id: "ai-one".into(),
            command_policy: AiCommandPolicy::AutoSafe,
        }
    }

    fn bind_active(gateway: &AiGatewayState, token: &str, session_id: &str) {
        gateway.bind(token.into(), ssh_binding()).unwrap();
        gateway
            .activate_session(token, session_id.into(), AiCommandPolicy::AutoSafe)
            .unwrap();
    }

    #[test]
    fn gateway_resolves_or_rejects_pending_command_approvals() {
        let gateway = AiGatewayState::default();
        bind_active(&gateway, "task-one", "ai-one");
        let approved = gateway
            .request_approval("task-one".into(), "approval-one".into())
            .unwrap();
        assert!(
            !gateway
                .resolve_approval("approval-one", "ai-other", true)
                .unwrap()
        );
        assert!(
            gateway
                .resolve_approval("approval-one", "ai-one", true)
                .unwrap()
        );
        assert_eq!(
            approved.recv_timeout(Duration::from_millis(50)).unwrap(),
            AiApprovalDecision::Approved
        );

        let revoked = gateway
            .request_approval("task-one".into(), "approval-two".into())
            .unwrap();
        gateway.revoke("task-one");
        assert_eq!(
            revoked.recv_timeout(Duration::from_millis(50)).unwrap(),
            AiApprovalDecision::Revoked
        );
        assert!(gateway.access_for("task-one").unwrap().is_none());
    }

    #[test]
    fn gateway_rejects_approval_after_task_token_is_revoked() {
        let gateway = AiGatewayState::default();
        bind_active(&gateway, "task-one", "ai-one");
        gateway.revoke("task-one");

        let error = gateway
            .request_approval("task-one".into(), "approval-one".into())
            .expect_err("revoked task must not create an approval");

        assert!(error.contains("任务已结束"));
        assert!(
            !gateway
                .resolve_approval("approval-one", "ai-one", true)
                .unwrap()
        );
    }

    #[test]
    fn gateway_rejects_a_second_pending_approval_for_the_same_task() {
        let gateway = AiGatewayState::default();
        bind_active(&gateway, "task-one", "ai-one");
        let first = gateway
            .request_approval("task-one".into(), "approval-one".into())
            .unwrap();

        let error = gateway
            .request_approval("task-one".into(), "approval-two".into())
            .expect_err("a hidden parallel approval must be rejected");

        assert!(error.contains("已有终端命令等待确认"));
        gateway.revoke("task-one");
        assert_eq!(
            first.recv_timeout(Duration::from_millis(50)).unwrap(),
            AiApprovalDecision::Revoked
        );
    }

    #[test]
    fn gateway_rate_limits_each_task_and_clears_state_on_revoke() {
        let gateway = AiGatewayState::default();
        bind_active(&gateway, "task-one", "ai-one");
        assert!(!gateway.has_tool_calls("task-one").unwrap());
        gateway.mark_tool_call("task-one").unwrap();
        assert!(gateway.has_tool_calls("task-one").unwrap());
        for _ in 0..30 {
            gateway.check_tool_rate("task-one").unwrap();
        }
        assert!(gateway.check_tool_rate("task-one").is_err());

        gateway.revoke("task-one");
        assert!(!gateway.has_tool_calls("task-one").unwrap());
        assert!(gateway.check_tool_rate("task-one").is_err());
        bind_active(&gateway, "task-one", "ai-one");
        assert!(!gateway.has_tool_calls("task-one").unwrap());
        assert!(gateway.check_tool_rate("task-one").is_ok());
        assert!(!gateway.has_tool_calls("task-one").unwrap());
        gateway.mark_tool_call("task-one").unwrap();
        assert!(gateway.has_tool_calls("task-one").unwrap());
    }

    #[test]
    fn gateway_updates_only_the_session_metadata_for_a_reused_token() {
        let gateway = AiGatewayState::default();
        bind_active(&gateway, "task-one", "ai-one");
        let first_access = gateway
            .access_for("task-one")
            .unwrap()
            .expect("first access");
        gateway.mark_tool_call("task-one").unwrap();
        assert!(gateway.has_tool_calls("task-one").unwrap());

        gateway
            .deactivate_session("ai-one")
            .expect("deactivate first turn");
        assert!(first_access.cancellation.load(Ordering::Acquire));
        assert!(!gateway.has_tool_calls("task-one").unwrap());
        assert!(gateway.check_tool_rate("task-one").is_err());
        gateway
            .activate_session("task-one", "ai-two".into(), AiCommandPolicy::ConfirmEach)
            .expect("update session");
        assert!(!gateway.has_tool_calls("task-one").unwrap());

        let access = gateway
            .access_for("task-one")
            .expect("read binding")
            .expect("binding exists");
        assert!(!access.cancellation.load(Ordering::Acquire));
        assert_eq!(access.binding.session_id, "ai-two");
        assert_eq!(access.binding.provider, "codex");
        assert_eq!(access.binding.command_policy, AiCommandPolicy::ConfirmEach);
        assert_eq!(access.binding.target, AiTarget::Ssh { connection_id: 1 });
    }

    #[test]
    fn deactivating_a_turn_rejects_its_pending_approval() {
        let gateway = AiGatewayState::default();
        bind_active(&gateway, "task-one", "ai-one");
        let pending = gateway
            .request_approval("task-one".into(), "approval-one".into())
            .unwrap();

        assert!(gateway.deactivate_session("ai-one").unwrap());
        assert_eq!(
            pending.recv_timeout(Duration::from_millis(50)).unwrap(),
            AiApprovalDecision::Revoked
        );
        assert!(
            !gateway
                .resolve_approval("approval-one", "ai-one", true)
                .unwrap()
        );
    }
}
