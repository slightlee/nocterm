//! SSH 终端 UI 会话映射与 AI 可见执行的控制面状态。
//!
//! Registry 保存连接、workspace 会话、终端和 generation 的权威绑定，并为
//! SSH 可见执行提供 Prompt Ready、输入门控和输出订阅。本地终端的对应能力在
//! `local_terminal` 模块，两者不共享具体结构，但契约语义保持一致。
//!
//! 与本地的有意差异是：门控命中时 `ssh_terminal_write` 返回稳定错误码，
//! 而不是静默吞掉用户输入。

use std::{
    collections::HashMap,
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

use super::osc633::Osc633Event;

/// 当前 SSH 终端的 UI 会话与后端 PTY 映射，同时为后续可见执行预留订阅与门控。
/// 订阅只驻留内存，发送失败的订阅会在下一次输出时自动移除。
#[derive(Default)]
pub struct SshTerminalRegistry {
    /// binding 与 generation 必须在同一临界区更新，避免并发重连把旧终端写回。
    binding_state: Mutex<SshBindingState>,
    /// terminal_id -> 订阅者；由唯一 backend reader 线程投递解码后文本。
    subscribers: Mutex<HashMap<String, Vec<mpsc::Sender<String>>>>,
    /// terminal_id -> integration 状态（OSC 633 观察与输入门控）。
    integrations: Mutex<HashMap<String, SshIntegration>>,
    /// terminal_id -> source 注入命令的回显抑制器（一次性，隐藏后自动关闭）。
    source_suppressions: Mutex<HashMap<String, SourceEchoSuppressor>>,
}

/// source 注入命令的回显抑制：注入命令（`source '<rc>'; rm -f -- '<rc>'`）
/// 的 shell 回显对用户是噪音且暴露实现细节，按逻辑行跨分块隐藏；
/// 隐藏一条后自动关闭，避免误伤用户后续输出。
struct SourceEchoSuppressor {
    /// 注入命令的特征片段（.rc 路径，命令回显行必含）。
    path: String,
    /// 跨分块的行缓冲。
    pending: String,
}

/// workspace 会话绑定；generation 随每次 bind 单调递增，旧授权自然失效。
#[derive(Debug, Clone, PartialEq, Eq)]
struct SshBinding {
    connection_id: i64,
    terminal_id: String,
    generation: u64,
}

#[derive(Default)]
struct SshBindingState {
    bindings: HashMap<String, SshBinding>,
    /// generation 历史在关闭后保留，使旧授权无法在重连后重新生效。
    generations: HashMap<String, u64>,
}

/// 单个 SSH 终端的 integration 状态（镜像本地 `TerminalIntegration`）。
///
/// 门控直接挂在 integration 上：D 事件到达即解除门控，与本地语义一致；
/// owner（executionId）保证只有同一次执行能释放自己的门控。
struct SshIntegration {
    completions: u64,
    last_exit_code: i64,
    /// 占用者 executionId；None 表示空闲。
    gate_owner: Option<String>,
    /// bootstrap 注入的 nonce：只接受携带匹配 nonce 的 integration 事件
    /// 作为 Prompt Ready 证据；None 表示无 bootstrap（普通 shell 或用户自装）。
    expected_nonce: Option<String>,
    /// 正面确认的 Prompt Ready：匹配 nonce 的 A/D 事件置位；用户键盘写入清除。
    prompt_ready: bool,
}

impl SshIntegration {
    fn new() -> Self {
        Self {
            completions: 0,
            last_exit_code: 0,
            gate_owner: None,
            expected_nonce: None,
            prompt_ready: false,
        }
    }
}

impl SshTerminalRegistry {
    /// 绑定 workspace 会话到新打开的终端，generation 在该会话内单调递增。
    ///
    /// 重连（同 workspace session 再次 open）会得到新 terminal ID 与更大的
    /// generation；旧绑定被替换，旧 generation 的授权在 verify 时失效。
    pub fn bind(
        &self,
        workspace_session_id: &str,
        terminal_id: &str,
        connection_id: i64,
    ) -> Result<u64, String> {
        let mut state = self
            .binding_state
            .lock()
            .map_err(|_| "SSH 终端绑定状态不可用".to_string())?;
        let generation = state
            .generations
            .get(workspace_session_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        state
            .generations
            .insert(workspace_session_id.to_string(), generation);
        state.bindings.insert(
            workspace_session_id.to_string(),
            SshBinding {
                connection_id,
                terminal_id: terminal_id.to_string(),
                generation,
            },
        );
        Ok(generation)
    }

    /// 解析 workspace 会话的完整绑定；可见执行 dispatch 前的权威来源。
    pub fn resolve(
        &self,
        workspace_session_id: &str,
    ) -> Result<Option<(String, u64, i64)>, String> {
        let state = self
            .binding_state
            .lock()
            .map_err(|_| "SSH 终端绑定状态不可用".to_string())?;
        Ok(state.bindings.get(workspace_session_id).map(|binding| {
            (
                binding.terminal_id.clone(),
                binding.generation,
                binding.connection_id,
            )
        }))
    }

    /// 可见执行 dispatch 前的绑定一致性校验：workspace -> terminal -> generation
    /// 四元仍匹配才放行；connectionId 不能只验证存在，必须属于该终端。
    /// 任何不匹配都必须返回稳定目标失效错误，绝不能改用同连接的其他终端。
    pub fn verify_binding(
        &self,
        workspace_session_id: &str,
        terminal_id: &str,
        expected_generation: u64,
        expected_connection_id: i64,
    ) -> Result<(), String> {
        let Some((current_terminal, current_generation, current_connection_id)) =
            self.resolve(workspace_session_id)?
        else {
            return Err("目标 SSH 终端不存在或已关闭".to_string());
        };
        if current_connection_id != expected_connection_id {
            return Err("目标 SSH 终端与连接不匹配".to_string());
        }
        if current_terminal != terminal_id {
            return Err("目标 SSH 终端已重新连接，请重新发起 AI 任务".to_string());
        }
        if current_generation != expected_generation {
            return Err("目标 SSH 终端已重新连接，旧授权已失效".to_string());
        }
        Ok(())
    }

    /// 终端关闭时清理全部控制面状态。
    pub fn unbind(&self, workspace_session_id: &str, terminal_id: &str) {
        if let Ok(mut state) = self.binding_state.lock()
            && state
                .bindings
                .get(workspace_session_id)
                .is_some_and(|binding| binding.terminal_id == terminal_id)
        {
            state.bindings.remove(workspace_session_id);
        }
        self.unbind_terminal(terminal_id);
    }

    /// 按 terminal 清理：主动关闭、读中断和重连替换都走这里。
    pub fn unbind_terminal(&self, terminal_id: &str) {
        if let Ok(mut state) = self.binding_state.lock() {
            state
                .bindings
                .retain(|_, binding| binding.terminal_id != terminal_id);
        }
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.remove(terminal_id);
        }
        if let Ok(mut integrations) = self.integrations.lock() {
            integrations.remove(terminal_id);
        }
        if let Ok(mut suppressions) = self.source_suppressions.lock() {
            suppressions.remove(terminal_id);
        }
    }

    /// 订阅终端解码后输出；发送失败的订阅在下一次输出时自动移除。
    pub fn subscribe(&self, terminal_id: &str) -> mpsc::Receiver<String> {
        let (sender, receiver) = mpsc::channel();
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers
                .entry(terminal_id.to_string())
                .or_default()
                .push(sender);
        }
        receiver
    }

    /// 向全部订阅者分发解码后文本；只由唯一的 backend reader 线程调用。
    pub fn publish(&self, terminal_id: &str, data: &str) {
        if let Ok(mut subscribers) = self.subscribers.lock()
            && let Some(listeners) = subscribers.get_mut(terminal_id)
        {
            listeners.retain(|sender| sender.send(data.to_string()).is_ok());
        }
    }

    /// 读取线程上报 633 事件：首个事件即证明集成激活；D 事件推进完成计数。
    ///
    /// D 事件意味着 Shell 已回到提示符：顺带解除输入门控。这同时覆盖"中断后
    /// 迟到恢复"的场景——执行方等待窗口内未观察到恢复而保留门控时，由读取
    /// 线程在恢复真正发生的那一刻放行用户输入与后续 AI 命令。
    ///
    /// nonce 过滤：注册 expected_nonce 后，缺失或不匹配 nonce 的事件都必须丢弃；
    /// 未 bootstrap 的普通 shell 仍可观测事件，但不能成为 Prompt Ready 证据。
    pub fn record_integration_event(&self, terminal_id: &str, event: &Osc633Event) {
        if let Ok(mut integrations) = self.integrations.lock() {
            let integration = integrations
                .entry(terminal_id.to_string())
                .or_insert_with(SshIntegration::new);
            // nonce 过滤在最前：伪造/串线的事件不得激活 integration、推进计数
            // 或触碰门控——全部状态变化都发生在通过过滤之后。
            if let Some(expected) = &integration.expected_nonce {
                match event.event_nonce() {
                    Some(nonce) if nonce == expected => {}
                    _ => return,
                }
            }
            if let Osc633Event::CommandFinished { code, .. } = event {
                integration.completions += 1;
                integration.last_exit_code = *code;
                // D 事件证明 Shell 已回到提示符：无论当时的占用者是谁都放行，
                // 与本地"迟到恢复由读取线程放行"的语义一致。nonce 过滤由
                // Prompt Ready 状态机负责，这里保持计数完整。
                integration.gate_owner = None;
                integration.prompt_ready = true;
            }
            if let Osc633Event::PromptStarted { .. } = event {
                integration.prompt_ready = true;
            }
        }
    }

    /// bootstrap 成功后注册 integration 计划；之后只有匹配 nonce 的事件
    /// 才能作为 Prompt Ready 证据。
    pub fn register_integration(&self, terminal_id: &str, nonce: &str) {
        if let Ok(mut integrations) = self.integrations.lock() {
            let integration = integrations
                .entry(terminal_id.to_string())
                .or_insert_with(SshIntegration::new);
            integration.expected_nonce = Some(nonce.to_string());
        }
    }

    /// Prompt Ready 正面证据是否成立：bootstrap 终端必须观察到匹配 nonce 的
    /// 提示符事件；未 bootstrap 的终端恒为 false（可见执行不得据此写入）。
    pub fn prompt_ready(&self, terminal_id: &str) -> bool {
        self.integrations
            .lock()
            .ok()
            .and_then(|integrations| {
                integrations
                    .get(terminal_id)
                    .map(|state| state.prompt_ready && state.expected_nonce.is_some())
            })
            .unwrap_or(false)
    }

    /// 当前完成计数快照；执行方写入命令前获取，只接受快照之后的新完成信号。
    pub fn completion_snapshot(&self, terminal_id: &str) -> u64 {
        self.integrations
            .lock()
            .ok()
            .and_then(|integrations| integrations.get(terminal_id).map(|state| state.completions))
            .unwrap_or(0)
    }

    /// 等待快照之后的新完成信号并返回其退出码；超时返回 None。
    pub fn wait_for_completion(
        &self,
        terminal_id: &str,
        snapshot: u64,
        timeout: Duration,
    ) -> Option<i64> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(integrations) = self.integrations.lock()
                && let Some(state) = integrations.get(terminal_id)
                && state.completions > snapshot
            {
                return Some(state.last_exit_code);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// 在同一临界区内验证 Prompt Ready 并争用输入门控，避免用户输入与 AI dispatch
    /// 分别读写状态时把两条命令拼接到同一个 Shell 编辑缓冲区。
    pub fn try_reserve_visible_execution(&self, terminal_id: &str, owner: &str) -> bool {
        let Ok(mut integrations) = self.integrations.lock() else {
            return false;
        };
        let integration = integrations
            .entry(terminal_id.to_string())
            .or_insert_with(SshIntegration::new);
        if integration.gate_owner.is_some()
            || !integration.prompt_ready
            || integration.expected_nonce.is_none()
        {
            return false;
        }
        integration.gate_owner = Some(owner.to_string());
        integration.prompt_ready = false;
        true
    }

    /// 用户输入在同一控制面锁内完成 gate 检查与 Prompt Ready 失效。
    /// 返回 false 表示 AI 已占用终端，本次输入不得进入底层 SSH 写队列。
    pub fn begin_user_input(&self, terminal_id: &str) -> bool {
        let Ok(mut integrations) = self.integrations.lock() else {
            return false;
        };
        let integration = integrations
            .entry(terminal_id.to_string())
            .or_insert_with(SshIntegration::new);
        if integration.gate_owner.is_some() {
            return false;
        }
        integration.prompt_ready = false;
        true
    }

    /// 释放门控；owner 不匹配时拒绝释放，避免迟到事件解除新命令的门控。
    pub fn release_input_gate(&self, terminal_id: &str, owner: &str) -> bool {
        let Ok(mut integrations) = self.integrations.lock() else {
            return false;
        };
        let Some(integration) = integrations.get_mut(terminal_id) else {
            return false;
        };
        match &integration.gate_owner {
            Some(current) if current == owner => {
                integration.gate_owner = None;
                true
            }
            _ => false,
        }
    }

    pub fn input_gated(&self, terminal_id: &str) -> bool {
        self.integrations
            .lock()
            .ok()
            .and_then(|integrations| {
                integrations
                    .get(terminal_id)
                    .map(|state| state.gate_owner.is_some())
            })
            .unwrap_or(false)
    }

    /// 开启 source 注入命令的回显抑制：连接建立时敲入的
    /// ` source '<rc>'; rm -f -- '<rc>'` 回显按逻辑行隐藏（一次性）。
    pub fn begin_source_echo_suppression(&self, terminal_id: &str, path: &str) {
        if let Ok(mut suppressions) = self.source_suppressions.lock() {
            suppressions.insert(
                terminal_id.to_string(),
                SourceEchoSuppressor {
                    path: path.to_string(),
                    pending: String::new(),
                },
            );
        }
    }

    /// 返回允许发送到 xterm 的文本，并隐藏一次 bootstrap source 命令回显。
    pub fn visible_output(&self, terminal_id: &str, data: &str) -> String {
        // source 注入回显抑制优先：注入命令行（含 rc 路径与 rm -f）按逻辑行
        // 跨分块隐藏，隐藏后抑制器自动关闭，不误伤后续输出。
        if let Ok(mut suppressions) = self.source_suppressions.lock()
            && let Some(suppressor) = suppressions.get_mut(terminal_id)
        {
            suppressor.pending.push_str(data);
            let mut visible = String::new();
            while let Some(newline) = suppressor.pending.find('\n') {
                let line: String = suppressor.pending.drain(..=newline).collect();
                if line.contains(&suppressor.path) && line.contains("rm -f") {
                    // 同一网络块可能还包含提示符或业务输出，不能随回显行一起丢弃。
                    let trailing = std::mem::take(&mut suppressor.pending);
                    suppressions.remove(terminal_id);
                    drop(suppressions);
                    visible.push_str(&self.visible_output(terminal_id, &trailing));
                    return visible;
                }
                visible.push_str(&line);
            }
            // 逻辑行未完结：整段缓冲，等下一个分片判定。
            return visible;
        }
        data.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bind(registry: &SshTerminalRegistry, workspace: &str, terminal: &str) -> u64 {
        registry
            .bind(workspace, terminal, 7)
            .expect("bind terminal")
    }

    #[test]
    fn bind_assigns_monotonic_generation_per_workspace_session() {
        let registry = SshTerminalRegistry::default();
        assert_eq!(bind(&registry, "ws-1", "ssh-1"), 1);
        // 重连得到新 terminal 与更大的 generation。
        assert_eq!(bind(&registry, "ws-1", "ssh-2"), 2);
        // 另一个会话独立计数。
        assert_eq!(bind(&registry, "ws-2", "ssh-3"), 1);
    }

    #[test]
    fn resolve_returns_latest_binding_after_reconnect() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        assert_eq!(
            registry.resolve("ws-1").unwrap(),
            Some(("ssh-1".into(), 1, 7))
        );
        bind(&registry, "ws-1", "ssh-2");
        assert_eq!(
            registry.resolve("ws-1").unwrap(),
            Some(("ssh-2".into(), 2, 7))
        );
    }

    #[test]
    fn verify_binding_rejects_stale_generation_and_foreign_terminal() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        bind(&registry, "ws-1", "ssh-2");

        // 旧 generation 的授权立即失效。
        assert!(registry.verify_binding("ws-1", "ssh-1", 1, 7).is_err());
        // 当前绑定匹配。
        assert!(registry.verify_binding("ws-1", "ssh-2", 2, 7).is_ok());
        // connection ID 是权威绑定的一部分，不能由前端与其他终端拼接。
        assert!(registry.verify_binding("ws-1", "ssh-2", 2, 8).is_err());
        // 不同 workspace 会话不能互相顶替。
        assert!(registry.verify_binding("ws-2", "ssh-2", 2, 7).is_err());
        // 未注册会话直接失败。
        assert!(
            registry
                .verify_binding("ws-missing", "ssh-2", 2, 7)
                .is_err()
        );
    }

    #[test]
    fn unbind_terminal_removes_binding_but_generation_history_keeps_rejecting_old_ids() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.unbind("ws-1", "ssh-1");
        assert!(registry.verify_binding("ws-1", "ssh-1", 1, 7).is_err());
        assert_eq!(bind(&registry, "ws-1", "ssh-2"), 2);
        assert!(registry.verify_binding("ws-1", "ssh-2", 1, 7).is_err());
        assert!(registry.verify_binding("ws-1", "ssh-2", 2, 7).is_ok());
    }

    #[test]
    fn user_input_and_visible_reservation_are_mutually_exclusive() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );

        assert!(registry.begin_user_input("ssh-1"));
        assert!(!registry.try_reserve_visible_execution("ssh-1", "exec-1"));

        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );
        assert!(registry.try_reserve_visible_execution("ssh-1", "exec-1"));
        assert!(!registry.begin_user_input("ssh-1"));
    }

    #[test]
    fn input_gate_is_exclusive_and_owner_bound_release() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );

        assert!(registry.try_reserve_visible_execution("ssh-1", "exec-1"));
        // 第二条 AI 命令被占用锁拒绝。
        assert!(!registry.try_reserve_visible_execution("ssh-1", "exec-2"));
        assert!(registry.input_gated("ssh-1"));
        // 错误 owner 不能释放；正确 owner 可以。
        assert!(!registry.release_input_gate("ssh-1", "exec-2"));
        assert!(registry.input_gated("ssh-1"));
        assert!(registry.release_input_gate("ssh-1", "exec-1"));
        assert!(!registry.input_gated("ssh-1"));
    }

    #[test]
    fn completion_event_releases_gate_and_advances_snapshot() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );
        let snapshot = registry.completion_snapshot("ssh-1");
        assert_eq!(snapshot, 0);
        assert!(registry.try_reserve_visible_execution("ssh-1", "exec-1"));

        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::CommandFinished {
                code: 3,
                nonce: Some("nonce-1".into()),
            },
        );
        // D 事件自动解除门控，迟到恢复场景由读取线程放行。
        assert!(!registry.input_gated("ssh-1"));
        assert_eq!(registry.completion_snapshot("ssh-1"), 1);
        assert_eq!(
            registry.wait_for_completion("ssh-1", snapshot, Duration::from_millis(50)),
            Some(3)
        );
        assert_eq!(
            registry.wait_for_completion("ssh-1", 1, Duration::from_millis(50)),
            None
        );
    }

    #[test]
    fn nonce_filter_blocks_mismatched_events_and_prompt_ready_tracks_state() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        assert!(!registry.prompt_ready("ssh-1"));

        // 缺失 nonce 与错误 nonce 一样不可信，不能激活或推进状态机。
        registry.record_integration_event("ssh-1", &Osc633Event::PromptStarted { nonce: None });
        assert!(!registry.prompt_ready("ssh-1"));

        // 不匹配 nonce 的事件是伪造/串线：不得激活 integration、推进计数或
        // 置位 Prompt Ready——全部状态变化都在过滤之后。
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("forged".into()),
            },
        );
        assert!(!registry.prompt_ready("ssh-1"));
        assert_eq!(registry.completion_snapshot("ssh-1"), 0);

        // 匹配 nonce 的 A 事件是正面证据。
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );
        assert!(registry.prompt_ready("ssh-1"));

        // 用户键盘写入使就绪失效。
        assert!(registry.begin_user_input("ssh-1"));
        assert!(!registry.prompt_ready("ssh-1"));

        // 匹配 nonce 的 D 事件重新就绪。
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::CommandFinished {
                code: 0,
                nonce: Some("nonce-1".into()),
            },
        );
        assert!(registry.prompt_ready("ssh-1"));
    }

    #[test]
    fn prompt_ready_requires_registered_bootstrap_nonce() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");

        // 未 bootstrap 的终端即使观察到 633（用户自装 integration）也不得作为
        // 可见执行的 Prompt Ready 证据。
        registry.record_integration_event("ssh-1", &Osc633Event::PromptStarted { nonce: None });
        assert!(!registry.prompt_ready("ssh-1"));

        registry.register_integration("ssh-1", "nonce-2");
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-2".into()),
            },
        );
        assert!(registry.prompt_ready("ssh-1"));
    }

    #[test]
    fn publish_fans_out_to_subscribers_and_drops_dead_ones() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let alive = registry.subscribe("ssh-1");
        let dead = registry.subscribe("ssh-1");
        drop(dead);

        registry.publish("ssh-1", "hello");
        assert_eq!(
            alive.try_recv().expect("alive subscriber receives"),
            "hello".to_string()
        );
        // dead 订阅者被移除后不影响后续分发。
        registry.publish("ssh-1", "world");
        assert_eq!(
            alive.try_recv().expect("second publish"),
            "world".to_string()
        );
    }

    #[test]
    fn unbind_removes_subscribers() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let receiver = registry.subscribe("ssh-1");
        registry.unbind("ws-1", "ssh-1");
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn source_echo_suppression_preserves_trailing_output_in_the_same_chunk() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let path = "/tmp/nocterm-ssh-nonce.rc";
        registry.begin_source_echo_suppression("ssh-1", path);

        let visible = registry.visible_output(
            "ssh-1",
            &format!(" source '{path}'; rm -f -- '{path}'\r\nuser@host:~$ "),
        );

        assert_eq!(visible, "user@host:~$ ");
        assert_eq!(
            registry.visible_output("ssh-1", "echo ok\r\n"),
            "echo ok\r\n"
        );
    }
}
