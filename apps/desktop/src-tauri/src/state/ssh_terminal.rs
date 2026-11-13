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

/// bootstrap 注入回显抑制的时间上界：正常链路中首个可信 nonce-bound OSC 远早
/// 于此到达；超时说明远端钩子未安装或链路异常，继续吞输出只会让终端假死。
pub(crate) const INTEGRATION_ECHO_SUPPRESSION_WINDOW: Duration = Duration::from_secs(5);
/// 抑制窗口内的缓冲上限；达到后转入降级直通，不再无界累积远端输出。
const SUPPRESSION_BUFFER_LIMIT: usize = 128 * 1024;

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
    /// terminal_id -> integration 注入命令的回显抑制器（一次性，隐藏后自动关闭）。
    integration_suppressions: Mutex<HashMap<String, IntegrationEchoSuppressor>>,
}

/// integration 注入命令的回显对用户是噪音且暴露实现细节，按逻辑行跨块隐藏。
/// POSIX 以路径 + 清理命令双重匹配，PowerShell 使用内存注入的固定调用 token。
///
/// 抑制窗口是有界的：截止（[`INTEGRATION_ECHO_SUPPRESSION_WINDOW`]）或缓冲
/// 超限后转入降级直通——只剔除可确证的内部行，绝不无限吞输出。否则远端
/// 钩子一旦安装失败，终端会静默假死到重连为止。
struct IntegrationEchoSuppressor {
    token: String,
    required_fragment: String,
    /// 跨分块缓冲。只有完整逻辑行或可信 nonce-bound OSC 才能结束抑制，避免
    /// 第二个随机路径先于行尾到达时把后续分块误当成普通输出。
    pending: String,
    /// 窗口截止时刻，由调用方按注入时机给定；测试可传入过去时刻。
    deadline: Instant,
    /// 降级直通：不再缓冲未分类输出，仅按严格判定剔除完整内部行。
    degraded: bool,
}

impl IntegrationEchoSuppressor {
    /// POSIX 注入命令同时包含 source 路径和 rm 路径。随机路径出现两次且清理
    /// 片段位于两者之间时确认命中，但必须等待逻辑行结束后才能移除抑制器。
    fn posix_echo_range(&self) -> Option<std::ops::Range<usize>> {
        let first_token = self.pending.find(&self.token)?;
        let second_token = self.pending[first_token + self.token.len()..]
            .find(&self.token)
            .map(|offset| first_token + self.token.len() + offset)?;
        self.pending[first_token + self.token.len()..second_token].find(&self.required_fragment)?;

        let source_at = self.pending[..first_token].rfind("source ")?;
        // source_command 有意带一个前导空格以配合 HISTCONTROL=ignorespace；只移除
        // 这个注入空格，保留提示符自身末尾的空格与样式控制序列。
        let start = source_at
            .checked_sub(1)
            .filter(|index| self.pending.as_bytes()[*index] == b' ')
            .unwrap_or(source_at);
        let mut end = second_token + self.token.len();
        // token 不含 shell 引号；末尾引号属于内部命令，也必须一起隐藏。
        while self
            .pending
            .as_bytes()
            .get(end)
            .is_some_and(|byte| matches!(*byte, b'\'' | b'"' | b' ' | b'\t'))
        {
            end += 1;
        }
        // 注入命令尾部还有提示符清理 printf（见 bootstrap::source_command）；
        // 同一行剩余部分同属内部命令，延伸到行尾一起隐藏，只保留回显换行。
        while self
            .pending
            .as_bytes()
            .get(end)
            .is_some_and(|byte| !matches!(*byte, b'\r' | b'\n'))
        {
            end += 1;
        }
        // 未终结行仍可能继续回显内部 printf 或被 ANSI 重绘。提前返回范围会
        // 删除抑制器，使下一网络块直接泄漏到 xterm。
        self.pending[end..].find(['\r', '\n'])?;
        Some(start..end)
    }

    /// Shell 行编辑器可能用 ANSI 重绘注入行。匹配前移除显示控制序列，避免
    /// 精确 token 被样式片段打断后把 source/Base64 内部命令透传给 xterm。
    fn line_is_internal(&self, line: &str) -> bool {
        let normalized = terminal_text_for_match(line);
        normalized.contains(&self.token) && normalized.contains(&self.required_fragment)
    }

    /// 降级直通使用的内部行判定：只匹配可确证的注入特征。窗口已经失败，此时
    /// 宁可漏删（外观噪音）也不能误吞用户命令（可用性问题）。
    fn line_is_conclusively_internal(&self, line: &str) -> bool {
        let normalized = terminal_text_for_match(line);
        normalized.contains(&self.token) && normalized.contains(&self.required_fragment)
    }

    fn complete_internal_line_range(&self) -> Option<std::ops::Range<usize>> {
        let mut line_start = 0;
        while let Some(relative_end) = self.pending[line_start..].find('\n') {
            let line_end = line_start + relative_end + 1;
            if self.line_is_internal(&self.pending[line_start..line_end]) {
                return Some(line_start..line_end);
            }
            line_start = line_end;
        }
        None
    }

    fn redacted_pending(&self) -> String {
        self.pending
            .split_inclusive('\n')
            .filter(|line| !self.line_is_internal(line))
            .collect()
    }

    /// 降级前的最后清理：交还缓冲中可安全显示的完整行（未终结行尾保留在
    /// 缓冲，继续走降级直通）。
    fn take_redacted_pending_conclusive(&mut self) -> String {
        let mut visible = String::new();
        while let Some(newline) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=newline).collect();
            if !self.line_is_conclusively_internal(&line) {
                visible.push_str(&line);
            }
        }
        visible
    }

    /// 降级直通：按完整逻辑行剔除可确证内部行，其余原样放行。单个未终结行
    /// 超过缓冲上限时，只有不含本会话精确锚点的内容才放行；已确证的内部
    /// 注入即使没有换行也必须丢弃，不能因上限保护反向造成协议泄漏。
    fn degraded_pass(&mut self, data: &str) -> String {
        self.pending.push_str(data);
        let mut visible = String::new();
        while let Some(newline) = self.pending.find('\n') {
            let line: String = self.pending.drain(..=newline).collect();
            if !self.line_is_conclusively_internal(&line) {
                visible.push_str(&line);
            }
        }
        if self.pending.len() > SUPPRESSION_BUFFER_LIMIT {
            let pending = std::mem::take(&mut self.pending);
            if !self.line_is_conclusively_internal(&pending) {
                visible.push_str(&pending);
            }
        }
        visible
    }
}

/// 仅用于内部注入回显特征匹配：去除 CSI/OSC 等显示控制，并处理退格重绘。
/// 返回值不发送到 UI，不能替代终端模拟器的完整解析。
fn terminal_text_for_match(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
    }

    let mut state = State::Text;
    let mut output = String::with_capacity(input.len());
    for character in input.chars() {
        state = match state {
            State::Text => match character {
                '\u{1b}' => State::Escape,
                '\u{8}' => {
                    output.pop();
                    State::Text
                }
                value if value == '\n' || value == '\r' || value == '\t' || value >= ' ' => {
                    output.push(value);
                    State::Text
                }
                _ => State::Text,
            },
            State::Escape => match character {
                '[' => State::Csi,
                ']' => State::Osc,
                _ => State::Text,
            },
            State::Csi => {
                if ('@'..='~').contains(&character) {
                    State::Text
                } else {
                    State::Csi
                }
            }
            State::Osc => match character {
                '\u{7}' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
            State::OscEscape => {
                if character == '\\' {
                    State::Text
                } else {
                    State::Osc
                }
            }
        };
    }
    output
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
    /// bootstrap 不可用时供执行路由使用的稳定降级码；只保存产品契约，
    /// 不把 probe/upload 等基础设施细节写入 execution ledger。
    bootstrap_fallback_reason: Option<&'static str>,
    /// 正面确认的 Prompt Ready：匹配 nonce 的 A/D 事件置位；用户键盘写入清除。
    prompt_ready: bool,
}

/// 单次 OSC 观察结果。nonce_bound 只对 bootstrap 注册的匹配 nonce 成立。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct SshIntegrationObservation {
    pub nonce_bound: bool,
}

impl SshIntegration {
    fn new() -> Self {
        Self {
            completions: 0,
            last_exit_code: 0,
            gate_owner: None,
            expected_nonce: None,
            bootstrap_fallback_reason: None,
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
        if let Ok(mut suppressions) = self.integration_suppressions.lock() {
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
    pub(crate) fn record_integration_event(
        &self,
        terminal_id: &str,
        event: &Osc633Event,
    ) -> SshIntegrationObservation {
        let Ok(mut integrations) = self.integrations.lock() else {
            return SshIntegrationObservation::default();
        };
        let integration = integrations
            .entry(terminal_id.to_string())
            .or_insert_with(SshIntegration::new);
        // nonce 过滤在最前：伪造/串线的事件不得激活 integration、推进计数
        // 或触碰门控——全部状态变化都发生在通过过滤之后。
        let nonce_bound = if let Some(expected) = &integration.expected_nonce {
            match event.event_nonce() {
                Some(nonce) if nonce == expected => true,
                _ => {
                    // 丢弃留痕：正常链路中远端只在钩子生效后发事件，这里出现
                    // 丢弃说明存在串线或残留的旧 integration，需要证据定位。
                    eprintln!(
                        "[nocterm-ssh-integration] 丢弃 nonce 不匹配事件：terminal={terminal_id} \
                         received={:?} expected={expected}",
                        event.event_nonce()
                    );
                    return SshIntegrationObservation::default();
                }
            }
        } else {
            false
        };
        if let Osc633Event::CommandFinished { code, .. } = event {
            integration.completions += 1;
            if integration.completions == 1 {
                // 首个可信 D 是"远端钩子已生效"的正面证据，与 bootstrap 注入
                // 日志配对后即可判断 integration 链路是否完整。
                eprintln!(
                    "[nocterm-ssh-integration] 首个可信完成事件已确认：terminal={terminal_id}"
                );
            }
            integration.last_exit_code = *code;
            // D 事件证明 Shell 已回到提示符：无论当时的占用者是谁都放行。
            integration.gate_owner = None;
            integration.prompt_ready = true;
            return SshIntegrationObservation { nonce_bound };
        }
        if let Osc633Event::PromptStarted { .. } = event {
            integration.prompt_ready = true;
        }
        SshIntegrationObservation { nonce_bound }
    }

    /// bootstrap 成功后注册 integration 计划；之后只有匹配 nonce 的事件
    /// 才能作为 Prompt Ready 证据。
    pub fn register_integration(&self, terminal_id: &str, nonce: &str) {
        if let Ok(mut integrations) = self.integrations.lock() {
            let integration = integrations
                .entry(terminal_id.to_string())
                .or_insert_with(SshIntegration::new);
            integration.expected_nonce = Some(nonce.to_string());
            integration.bootstrap_fallback_reason = None;
        }
    }

    /// 注册 bootstrap 失败事实，并把内部阶段原因归一化为可持久化的稳定契约。
    pub fn register_integration_unavailable(&self, terminal_id: &str, reason: &'static str) {
        if let Ok(mut integrations) = self.integrations.lock() {
            let integration = integrations
                .entry(terminal_id.to_string())
                .or_insert_with(SshIntegration::new);
            integration.expected_nonce = None;
            integration.prompt_ready = false;
            integration.bootstrap_fallback_reason = Some(match reason {
                "unsupported_shell" => "SSH_SHELL_UNSUPPORTED",
                _ => "SSH_INTEGRATION_UNAVAILABLE",
            });
        }
    }

    /// 可见执行未就绪时返回稳定降级原因。调用方只在 `prompt_ready == false`
    /// 时使用；bootstrap 成功后的两种失败必须区分：
    /// - `completions == 0`：从未观察到一次可信完成事件，远端钩子大概率没有
    ///   生效（注入行未被执行或脚本失败），用户重试只会继续降级；
    /// - `completions > 0`：曾经就绪、后被用户键入失效，等待下一次可信 A/D
    ///   恢复（用户在终端按回车即自愈）。
    pub fn visible_unavailable_reason(&self, terminal_id: &str) -> &'static str {
        let Ok(integrations) = self.integrations.lock() else {
            return "SSH_CONTROL_PLANE_UNAVAILABLE";
        };
        let Some(integration) = integrations.get(terminal_id) else {
            return "SSH_CONTROL_PLANE_UNAVAILABLE";
        };
        integration.bootstrap_fallback_reason.unwrap_or_else(|| {
            if integration.expected_nonce.is_none() {
                return "SSH_CONTROL_PLANE_UNAVAILABLE";
            }
            if integration.completions == 0 {
                eprintln!(
                    "[nocterm-ssh-integration] 未见任何可信完成事件，判定 integration 未生效：\
                     terminal={terminal_id}"
                );
                "SSH_INTEGRATION_NOT_CONFIRMED"
            } else {
                eprintln!(
                    "[nocterm-ssh-integration] 就绪被用户输入失效，等待下一次可信提示符事件：\
                     terminal={terminal_id} completions={}",
                    integration.completions
                );
                "SSH_PROMPT_NOT_READY"
            }
        })
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
        let Ok(integrations) = self.integrations.lock() else {
            return false;
        };
        let Some(integration) = integrations.get(terminal_id) else {
            return false;
        };
        integration.gate_owner.is_some()
    }

    /// 开启 integration 注入命令的回显抑制。token 始终是每会话随机锚点；
    /// required_fragment 再约束 POSIX source/rm 或 PowerShell 内存解码形状。
    /// deadline 之前未分类输出 fail closed（防注入回显泄漏），之后降级直通。
    pub fn begin_integration_echo_suppression(
        &self,
        terminal_id: &str,
        token: &str,
        required_fragment: &str,
        deadline: Instant,
    ) {
        if let Ok(mut suppressions) = self.integration_suppressions.lock() {
            suppressions.insert(
                terminal_id.to_string(),
                IntegrationEchoSuppressor {
                    token: token.to_string(),
                    required_fragment: required_fragment.to_string(),
                    pending: String::new(),
                    deadline,
                    degraded: false,
                },
            );
        }
    }

    /// 首个可信 nonce-bound OSC 证明 bootstrap 已执行完成。若 PTY 没有回显注入
    /// 命令，则把此前缓冲的正常提示符交还 UI；若仍含内部 token，则丢弃该逻辑行。
    pub fn finish_integration_echo_on_osc(&self, terminal_id: &str) -> String {
        let Ok(mut suppressions) = self.integration_suppressions.lock() else {
            return String::new();
        };
        let Some(suppressor) = suppressions.remove(terminal_id) else {
            return String::new();
        };
        if suppressor.degraded {
            // 降级模式可能仍持有迟到注入的未终结尾部。可信 OSC 不能成为
            // 原样交还内部协议的旁路；只有不含精确会话锚点的尾部才可见。
            return if suppressor.line_is_conclusively_internal(&suppressor.pending) {
                String::new()
            } else {
                suppressor.pending
            };
        }
        // 在可信 OSC 到达前，完整行也不提前放行。此处统一剥离经过 ANSI 重绘
        // 或 token 截断的内部注入行；PTY 无 echo 时，普通提示符会原样交还。
        suppressor.redacted_pending()
    }

    /// 返回允许发送到 xterm 的文本，并隐藏一次性的 bootstrap 注入回显。
    pub fn visible_output(&self, terminal_id: &str, data: &str) -> String {
        if let Some(visible) = self.drain_integration_echo(terminal_id, data) {
            return visible;
        }
        data.to_string()
    }

    /// integration 注入回显过滤。命中即一次性移除抑制器并递归处理剩余尾部；
    /// 窗口内无法分类的输出保持隔离（fail closed），窗口截止或超限后降级直通。
    fn drain_integration_echo(&self, terminal_id: &str, data: &str) -> Option<String> {
        let mut suppressions = self.integration_suppressions.lock().ok()?;
        let suppressor = suppressions.get_mut(terminal_id)?;

        if suppressor.degraded {
            return Some(suppressor.degraded_pass(data));
        }

        // 窗口截止或缓冲超限：bootstrap 钩子大概率未安装或链路异常。转入
        // 降级直通，先按严格判定清理缓冲，避免终端在钩子缺失时静默假死。
        if Instant::now() >= suppressor.deadline
            || suppressor.pending.len().saturating_add(data.len()) > SUPPRESSION_BUFFER_LIMIT
        {
            suppressor.degraded = true;
            let flushed = suppressor.take_redacted_pending_conclusive();
            let visible = suppressor.degraded_pass(data);
            return Some(format!("{flushed}{visible}"));
        }

        suppressor.pending.push_str(data);
        if let Some(range) = suppressor.posix_echo_range() {
            let mut visible = suppressor.pending[..range.start].to_string();
            let mut trailing = &suppressor.pending[range.end..];
            // 没有提示符前缀时整行都是内部注入，连同行结束符一起删除；
            // 提示符同行时保留换行，让后续提示符仍从下一行开始。
            if visible.is_empty() {
                trailing = trailing
                    .strip_prefix("\r\n")
                    .or_else(|| trailing.strip_prefix(['\r', '\n']))
                    .unwrap_or(trailing);
            }
            let trailing = trailing.to_string();
            suppressions.remove(terminal_id);
            drop(suppressions);
            visible.push_str(&self.visible_output(terminal_id, &trailing));
            return Some(visible);
        }
        if let Some(range) = suppressor.complete_internal_line_range() {
            let mut visible = suppressor.pending[..range.start].to_string();
            let trailing = suppressor.pending[range.end..].to_string();
            suppressions.remove(terminal_id);
            drop(suppressions);
            visible.push_str(&self.visible_output(terminal_id, &trailing));
            return Some(visible);
        }
        // 无法分类的输出在可信 nonce-bound OSC 或窗口截止前保持隔离，这样
        // ANSI 重绘或 token 截断不会把 source/Base64 注入行提前泄漏到终端。
        Some(String::new())
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

    /// 常规用例使用远未来截止时刻，专注于抑制窗口内的语义；降级直通语义由
    /// 显式传入过去时刻的专属用例覆盖。
    fn suppression_deadline() -> Instant {
        Instant::now() + Duration::from_secs(60)
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
    fn bootstrap_failure_maps_to_stable_visible_fallback_reasons() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");

        assert_eq!(
            registry.visible_unavailable_reason("ssh-1"),
            "SSH_CONTROL_PLANE_UNAVAILABLE"
        );
        registry.register_integration_unavailable("ssh-1", "unsupported_shell");
        assert_eq!(
            registry.visible_unavailable_reason("ssh-1"),
            "SSH_SHELL_UNSUPPORTED"
        );
        registry.register_integration_unavailable("ssh-1", "upload_remote_failed");
        assert_eq!(
            registry.visible_unavailable_reason("ssh-1"),
            "SSH_INTEGRATION_UNAVAILABLE"
        );

        // 同一 terminal 后续成功注册时必须清除旧失败事实，等待可信 Prompt 事件。
        registry.register_integration("ssh-1", "nonce-1");
        // nonce 已注册但从未观察到可信完成事件：不能表述为"提示符未就绪"，
        // 必须指向 integration 未生效，否则用户无从修复。
        assert_eq!(
            registry.visible_unavailable_reason("ssh-1"),
            "SSH_INTEGRATION_NOT_CONFIRMED"
        );
        // 首个可信 D 建立就绪后，用户键入使其失效：此时才是"提示符未就绪"，
        // 且用户在终端按回车（新 D/A）即可自愈。
        registry.record_integration_event(
            "ssh-1",
            &Osc633Event::CommandFinished {
                code: 0,
                nonce: Some("nonce-1".into()),
            },
        );
        registry.begin_user_input("ssh-1");
        assert_eq!(
            registry.visible_unavailable_reason("ssh-1"),
            "SSH_PROMPT_NOT_READY"
        );
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
        registry.begin_integration_echo_suppression("ssh-1", path, "rm -f", suppression_deadline());

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

    #[test]
    fn source_echo_suppression_hides_prompt_cleanup_printf_suffix() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let path = "/tmp/nocterm-ssh-nonce.rc";
        registry.begin_integration_echo_suppression("ssh-1", path, "rm -f", suppression_deadline());

        // 注入命令尾部的 printf 清理序列同属内部命令：整段命令文本隐藏，
        // 但回显换行必须透传，保持本地与远端的光标行一致。
        let visible = registry.visible_output(
            "ssh-1",
            &format!(
                "root@host:~#  source '{path}'; rm -f -- '{path}'; printf '\\033[1A\\033[2K\\r'\r\nroot@host:~# "
            ),
        );

        assert_eq!(visible, "root@host:~# \r\nroot@host:~# ");
    }

    #[test]
    fn source_echo_suppression_covers_initial_directory_in_activation_batch() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let path = "/tmp/nocterm-ssh-nonce.rc";
        registry.begin_integration_echo_suppression("ssh-1", path, "rm -f", suppression_deadline());

        let visible = registry.visible_output(
            "ssh-1",
            &format!(
                "root@host:~#  source '{path}'; rm -f -- '{path}'; printf '\\033[1A\\033[2K\\r'; cd '/srv/app'\r\nroot@host:/srv/app# "
            ),
        );

        assert_eq!(visible, "root@host:~# \r\nroot@host:/srv/app# ");
        assert!(!visible.contains("cd '/srv/app'"));
    }

    #[test]
    fn source_echo_suppression_hides_prompt_inline_command_without_newline() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let path = "/tmp/nocterm-ssh-8a688ebbf852d2acb09e8f830fd5967c.rc";
        registry.begin_integration_echo_suppression("ssh-1", path, "rm -f", suppression_deadline());

        let first = registry.visible_output(
            "ssh-1",
            &format!("root@host:~#  source '{path}'; rm -f -- '/tmp/nocterm-ssh-"),
        );
        let second = registry.visible_output("ssh-1", "8a688ebbf852d2acb09e8f830fd5967c.rc'");

        assert!(first.is_empty());
        assert!(second.is_empty());
        assert_eq!(
            registry.visible_output("ssh-1", "\rnext prompt# "),
            "root@host:~# \rnext prompt# "
        );
    }

    #[test]
    fn source_echo_suppression_handles_carriage_return_terminated_echo() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        let path = "/tmp/nocterm-ssh-nonce.rc";
        registry.begin_integration_echo_suppression("ssh-1", path, "rm -f", suppression_deadline());

        let visible = registry.visible_output(
            "ssh-1",
            &format!("root@host:~#  source '{path}'; rm -f -- '{path}'\rroot@host:~# "),
        );

        assert_eq!(visible, "root@host:~# \rroot@host:~# ");
    }

    #[test]
    fn trusted_prompt_finishes_bootstrap_suppression_when_pty_echo_is_disabled() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "/tmp/nocterm-ssh-nonce-1.rc",
            "rm -f",
            suppression_deadline(),
        );

        assert!(registry.visible_output("ssh-1", "root@host:~# ").is_empty());
        let observation = registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );

        assert!(observation.nonce_bound);
        assert_eq!(
            registry.finish_integration_echo_on_osc("ssh-1"),
            "root@host:~# "
        );
        assert_eq!(
            registry.visible_output("ssh-1", "echo ready\r\n"),
            "echo ready\r\n"
        );
    }

    #[test]
    fn bootstrap_suppression_degrades_to_redacted_pass_through_after_overflow() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "/tmp/nocterm-ssh-nonce-1.rc",
            "rm -f",
            suppression_deadline(),
        );

        // 超限触发降级：单个无换行的超长行按“必然是用户数据”原样放行，
        // 不再像旧实现那样从超限时刻起永久吞输出。
        let oversized = "x".repeat(SUPPRESSION_BUFFER_LIMIT + 1);
        assert_eq!(registry.visible_output("ssh-1", &oversized), oversized);
        assert_eq!(
            registry.visible_output("ssh-1", "user output\r\n"),
            "user output\r\n"
        );
        let forged = registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("wrong".into()),
            },
        );
        assert!(!forged.nonce_bound);
        // 降级直通仍剔除可确证的注入行（携带本会话随机路径）。
        let injected = registry.visible_output(
            "ssh-1",
            " source '/tmp/nocterm-ssh-nonce-1.rc'; rm -f -- '/tmp/nocterm-ssh-nonce-1.rc'\r\n",
        );
        assert!(injected.is_empty());

        let trusted = registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );
        assert!(trusted.nonce_bound);
        // 降级期间完整行已实时交还，缓冲只剩未终结行尾（此处为空）。
        assert!(registry.finish_integration_echo_on_osc("ssh-1").is_empty());
        assert_eq!(registry.visible_output("ssh-1", "visible"), "visible");
    }

    #[test]
    fn posix_bootstrap_suppression_degrades_after_deadline_and_keeps_output_flowing() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        // 截止时刻在过去：模拟钩子安装失败后远端继续输出。
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "/tmp/nocterm-ssh-nonce-1.rc",
            "rm -f",
            Instant::now() - Duration::from_millis(1),
        );

        // 降级直通按完整逻辑行放行；未终结行在换行到达时交还。
        assert!(registry.visible_output("ssh-1", "root@host:~# ").is_empty());
        assert_eq!(
            registry.visible_output("ssh-1", "\r\n"),
            "root@host:~# \r\n"
        );
        // POSIX 注入行凭随机路径仍可确证并被剔除。
        let injected = registry.visible_output(
            "ssh-1",
            " source '/tmp/nocterm-ssh-nonce-1.rc'; rm -f -- '/tmp/nocterm-ssh-nonce-1.rc'\r\n",
        );
        assert!(injected.is_empty());
        assert_eq!(
            registry.visible_output("ssh-1", "echo alive\r\n"),
            "echo alive\r\n"
        );
    }

    #[test]
    fn trusted_osc_never_flushes_degraded_unterminated_bootstrap_echo() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.register_integration("ssh-1", "nonce-1");
        let path = "/tmp/nocterm-ssh-nonce-1.rc";
        registry.begin_integration_echo_suppression(
            "ssh-1",
            path,
            "rm -f",
            Instant::now() - Duration::from_millis(1),
        );

        // 现场复现形态：第二个随机路径和换行都尚未到达，随后可信 OSC 先完成。
        let partial = format!("root@host:~#  source '{path}'; rm -f -- '/tmp/nocterm-s");
        assert!(registry.visible_output("ssh-1", &partial).is_empty());
        let observation = registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );
        assert!(observation.nonce_bound);
        assert!(registry.finish_integration_echo_on_osc("ssh-1").is_empty());
        assert_eq!(
            registry.visible_output("ssh-1", "root@host:~# "),
            "root@host:~# "
        );
    }

    #[test]
    fn powershell_degraded_pass_through_never_swallows_user_commands() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "abc",
            "FromBase64String",
            Instant::now() - Duration::from_millis(1),
        );

        // PowerShell 的固定 token 是通用 API 名，单独出现不得吞掉用户命令。
        assert_eq!(
            registry.visible_output("ssh-1", "grep FromBase64String log.txt\r\n"),
            "grep FromBase64String log.txt\r\n"
        );
        // 叠加注入语法特征（nonce 绑定或 scriptblock 解码）才判定为内部行。
        let injected = registry.visible_output(
            "ssh-1",
            " $env:NOCTERM_OSC_NONCE='abc'; . ([scriptblock]::Create([Convert]::FromBase64String('QQ==')))\r\n",
        );
        assert!(injected.is_empty());
        assert_eq!(
            registry.visible_output("ssh-1", "Write-Output ok\r\n"),
            "Write-Output ok\r\n"
        );
    }

    #[test]
    fn powershell_integration_echo_suppression_preserves_same_chunk_prompt() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "abc",
            "FromBase64String",
            suppression_deadline(),
        );

        let visible = registry.visible_output(
            "ssh-1",
            " $env:NOCTERM_OSC_NONCE='abc'; . ([Convert]::FromBase64String('QQ=='))\r\nPS C:\\> ",
        );

        assert_eq!(visible, "PS C:\\> ");
        assert_eq!(
            registry.visible_output("ssh-1", "Write-Output ok\r\n"),
            "Write-Output ok\r\n"
        );
    }

    #[test]
    fn integration_echo_suppression_handles_ansi_redraw_and_truncated_tokens() {
        let registry = SshTerminalRegistry::default();
        bind(&registry, "ws-1", "ssh-1");
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "abc",
            "FromBase64String",
            suppression_deadline(),
        );

        let ansi_redraw = registry.visible_output(
            "ssh-1",
            " $env:NOCTERM_OSC_NONCE='abc'; [Convert]::FromBase64\x1b[31mString('QQ==')\x1b[0m\r\nPS C:\\> ",
        );
        assert_eq!(ansi_redraw, "PS C:\\> ");
        assert!(!ansi_redraw.contains("FromBase64"));

        // 非本会话随机路径不能被宽泛特征误判；可信 OSC 收尾时应原样交还。
        registry.register_integration("ssh-1", "nonce-1");
        registry.begin_integration_echo_suppression(
            "ssh-1",
            "/tmp/nocterm-ssh-expected.rc",
            "rm -f",
            suppression_deadline(),
        );
        let unrelated = " source '/tmp/nocterm-ssh-other.rc'; rm -f -- '/tmp/nocterm-ssh-other.rc'\r\nuser@host:~$ ";
        assert!(registry.visible_output("ssh-1", unrelated).is_empty());
        let observation = registry.record_integration_event(
            "ssh-1",
            &Osc633Event::PromptStarted {
                nonce: Some("nonce-1".into()),
            },
        );
        assert!(observation.nonce_bound);
        assert_eq!(registry.finish_integration_echo_on_osc("ssh-1"), unrelated);
    }
}
