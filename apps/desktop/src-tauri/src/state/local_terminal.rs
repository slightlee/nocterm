//! 本地终端 UI 会话映射，以及 AI 与可见 PTY 共享输出时的边界协议。

use std::{
    collections::HashMap,
    sync::{Mutex, mpsc},
    time::{Duration, Instant},
};

use super::osc633::Osc633Event;

/// 完整随机标记用于完成校验；这个短保留前缀仅用于识别被 Shell 重绘拆散的 UI 回显。
const LOCAL_COMPLETION_REDACTION_PREFIX: &str = "__NOCTERM_";

/// 当前本地终端的 UI 会话与后端 PTY 映射，同时为 AI Bridge 分发同一 PTY 的输出。
/// 订阅只驻留内存，发送失败的订阅会在下一次输出时自动移除。
#[derive(Default)]
pub struct LocalTerminalRegistry {
    bindings: Mutex<HashMap<String, String>>,
    subscribers: Mutex<HashMap<String, Vec<mpsc::Sender<String>>>>,
    output_filters: Mutex<HashMap<String, TerminalOutputFilter>>,
    integrations: Mutex<HashMap<String, TerminalIntegration>>,
}

/// 单个终端的 shell integration 状态（#82）。
///
/// - `active` 在读取线程观察到首个 633 序列时翻转，AI 执行流据此选择集成或探针路径；
/// - `completions` 是 D 事件的单调计数，执行方在写入命令前取快照，
///   仅接受快照之后的新完成信号——启动期首提示符的 D;0 因此天然被忽略；
/// - `input_gated` 在 AI 命令执行期间拦截用户键盘写入，防止用户手敲命令的
///   完成信号冒充 AI 命令的退出码。
struct TerminalIntegration {
    active: bool,
    completions: u64,
    last_exit_code: i64,
    input_gated: bool,
}

impl TerminalIntegration {
    fn new() -> Self {
        Self {
            active: false,
            completions: 0,
            last_exit_code: 0,
            input_gated: false,
        }
    }
}

/// AI 在可见 PTY 中追加完成标记时，终端 UI 只隐藏内部命令及标记结果。
/// 原始输出仍发送给进程内订阅者，用于可靠判断命令何时完成。
struct TerminalOutputFilter {
    marker: String,
    pending: String,
    completed: bool,
    redacting_logical_line: bool,
}

impl LocalTerminalRegistry {
    pub fn bind(&self, session_id: String, terminal_id: String) {
        if let Ok(mut bindings) = self.bindings.lock() {
            bindings.insert(session_id, terminal_id);
        }
    }

    pub fn terminal_for(&self, session_id: &str) -> Option<String> {
        self.bindings.lock().ok()?.get(session_id).cloned()
    }

    pub fn unbind(&self, session_id: &str, terminal_id: &str) {
        if let Ok(mut bindings) = self.bindings.lock()
            && bindings
                .get(session_id)
                .is_some_and(|value| value == terminal_id)
        {
            bindings.remove(session_id);
        }
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.remove(terminal_id);
        }
        if let Ok(mut filters) = self.output_filters.lock() {
            filters.remove(terminal_id);
        }
        if let Ok(mut integrations) = self.integrations.lock() {
            integrations.remove(terminal_id);
        }
    }

    pub fn unbind_terminal(&self, terminal_id: &str) {
        if let Ok(mut bindings) = self.bindings.lock() {
            bindings.retain(|_, value| value != terminal_id);
        }
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.remove(terminal_id);
        }
        if let Ok(mut filters) = self.output_filters.lock() {
            filters.remove(terminal_id);
        }
        if let Ok(mut integrations) = self.integrations.lock() {
            integrations.remove(terminal_id);
        }
    }

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
    pub fn record_integration_event(&self, terminal_id: &str, event: &Osc633Event) {
        if let Ok(mut integrations) = self.integrations.lock() {
            let integration = integrations
                .entry(terminal_id.to_string())
                .or_insert_with(TerminalIntegration::new);
            integration.active = true;
            if let Osc633Event::CommandFinished(code) = event {
                integration.completions += 1;
                integration.last_exit_code = *code;
                integration.input_gated = false;
            }
        }
    }

    /// 集成是否已激活（读取线程观察到过至少一个 633 序列）。
    pub fn integration_active(&self, terminal_id: &str) -> bool {
        self.integrations
            .lock()
            .ok()
            .and_then(|integrations| {
                integrations
                    .get(terminal_id)
                    .map(|integration| integration.active)
            })
            .unwrap_or(false)
    }

    /// 当前完成计数快照；执行方写入命令前获取，只接受快照之后的新完成信号。
    pub fn completion_snapshot(&self, terminal_id: &str) -> u64 {
        self.integrations
            .lock()
            .ok()
            .and_then(|integrations| {
                integrations
                    .get(terminal_id)
                    .map(|integration| integration.completions)
            })
            .unwrap_or(0)
    }

    /// 等待快照之后的新完成信号并返回其退出码；超时返回 None。
    ///
    /// 轮询而非条件变量：被等待的命令本身以秒计，几十毫秒的粒度足够，
    /// 且与取消检查共用同一节拍，避免额外唤醒链和锁借用复杂度。
    pub fn wait_for_completion(
        &self,
        terminal_id: &str,
        snapshot: u64,
        timeout: Duration,
    ) -> Option<i64> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(integrations) = self.integrations.lock()
                && let Some(integration) = integrations.get(terminal_id)
                && integration.completions > snapshot
            {
                return Some(integration.last_exit_code);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// AI 命令执行期间置位：用户键盘写入被拦截（内部执行写入不走此门）。
    /// 该标志同时兼作 AI 侧占用锁——并发执行请求经 `try_acquire_input_gate`
    /// 原子争用，同一终端同时最多一条 AI 命令。
    pub fn set_input_gate(&self, terminal_id: &str, gated: bool) {
        if let Ok(mut integrations) = self.integrations.lock() {
            integrations
                .entry(terminal_id.to_string())
                .or_insert_with(TerminalIntegration::new)
                .input_gated = gated;
        }
    }

    /// 原子争用输入门控：空闲则占用并返回 true；已被 AI 命令占用返回 false。
    pub fn try_acquire_input_gate(&self, terminal_id: &str) -> bool {
        if let Ok(mut integrations) = self.integrations.lock() {
            let integration = integrations
                .entry(terminal_id.to_string())
                .or_insert_with(TerminalIntegration::new);
            if integration.input_gated {
                return false;
            }
            integration.input_gated = true;
            return true;
        }
        false
    }

    pub fn input_gated(&self, terminal_id: &str) -> bool {
        self.integrations
            .lock()
            .ok()
            .and_then(|integrations| {
                integrations
                    .get(terminal_id)
                    .map(|integration| integration.input_gated)
            })
            .unwrap_or(false)
    }

    pub fn begin_output_marker_filter(
        &self,
        terminal_id: &str,
        marker: &str,
    ) -> Result<(), String> {
        let mut filters = self
            .output_filters
            .lock()
            .map_err(|_| "本地终端输出过滤状态不可用".to_string())?;
        // 一个可见 PTY 只有一条输入流；并发写入会让命令、交互输入和完成标记互相串扰。
        if filters
            .get(terminal_id)
            .is_some_and(|filter| !filter.completed)
        {
            return Err("当前本地终端已有 AI 命令正在执行，请等待完成后重试".to_string());
        }
        // 取消调用方可能已返回，但读取线程仍会处理迟到的完成 marker。
        // completed 证明 Shell 已重新取得控制权，此时才能用新 marker 原子替换旧状态。
        filters.insert(
            terminal_id.to_string(),
            TerminalOutputFilter {
                marker: marker.to_string(),
                pending: String::new(),
                completed: false,
                redacting_logical_line: false,
            },
        );
        Ok(())
    }

    pub fn clear_output_marker_filter(&self, terminal_id: &str, marker: &str) {
        if let Ok(mut filters) = self.output_filters.lock()
            && filters
                .get(terminal_id)
                .is_some_and(|filter| filter.marker == marker)
        {
            filters.remove(terminal_id);
        }
    }

    /// 返回允许发送到 xterm 的文本。过滤按完整行工作，因此即使标记被 PTY 分块也不会泄漏。
    pub fn visible_output(&self, terminal_id: &str, data: &str) -> String {
        let Ok(mut filters) = self.output_filters.lock() else {
            return data.to_string();
        };
        let Some(filter) = filters.get_mut(terminal_id) else {
            return data.to_string();
        };
        if filter.completed {
            return redact_delayed_marker_output(filter, data);
        }
        filter.pending.push_str(data);
        let mut visible = String::new();
        let mut completed = false;
        while let Some(newline) = filter.pending.find('\n') {
            let line: String = filter.pending.drain(..=newline).collect();
            if !line.contains(LOCAL_COMPLETION_REDACTION_PREFIX) {
                visible.push_str(&line);
                continue;
            }
            // 命令回显和标记结果都隐藏；只有“标记 + 数字状态”才代表完成。
            if local_completion_status(&line, &filter.marker).is_some() {
                completed = true;
                break;
            }
        }
        // 某些交互式 Shell 只用回车分隔回显与结果，不能依赖 `lines()`。
        if !completed && local_completion_status(&filter.pending, &filter.marker).is_some() {
            completed = true;
        }
        if completed {
            if let Some(first_marker) = filter.pending.find(&filter.marker) {
                let hidden_line_start = filter.pending[..first_marker]
                    .rfind(['\r', '\n'])
                    .map_or(0, |index| index + 1);
                visible.push_str(&filter.pending[..hidden_line_start]);
                let completion_end = local_completion_status(&filter.pending, &filter.marker)
                    .map(|(index, status_length, _)| index + filter.marker.len() + status_length)
                    .unwrap_or(first_marker + filter.marker.len());
                let suffix = &filter.pending[completion_end..];
                // 退出码属于内部协议，和完成标记一样不能泄漏到 xterm。
                visible.push_str(suffix.trim_start_matches(['\r', '\n']));
            } else {
                visible.push_str(&filter.pending);
            }
            filter.pending.clear();
            // 执行线程按相同 marker 释放占用，避免另一条命令在结果发布前插入同一 PTY。
            filter.completed = true;
        }
        visible
    }
}

/// Shell 可能在退出状态已经返回后再次重绘上一条输入。此时过滤器仍处于占用期，
/// 继续删除包含保留前缀的逻辑行。zsh/ZLE 会用多次回车重绘长命令，随机标记
/// 可能被光标控制序列拆散，因此从首次识别前缀起一直抑制到该逻辑行换行。
fn redact_delayed_marker_output(filter: &mut TerminalOutputFilter, data: &str) -> String {
    filter.pending.push_str(data);
    let mut visible = String::new();
    while let Some(boundary) = filter.pending.find(['\r', '\n']) {
        let bytes = filter.pending.as_bytes();
        let terminator_length = usize::from(
            bytes
                .get(boundary + 1)
                .is_some_and(|next| *next != bytes[boundary] && matches!(*next, b'\r' | b'\n')),
        ) + 1;
        let record: String = filter
            .pending
            .drain(..boundary + terminator_length)
            .collect();
        if record.contains(LOCAL_COMPLETION_REDACTION_PREFIX) {
            filter.redacting_logical_line = true;
        }
        if !filter.redacting_logical_line {
            visible.push_str(&record);
        }
        if record.ends_with('\n') {
            filter.redacting_logical_line = false;
        }
    }
    if filter.pending.contains(LOCAL_COMPLETION_REDACTION_PREFIX) {
        filter.redacting_logical_line = true;
        filter.pending.clear();
    } else if filter.redacting_logical_line {
        filter.pending.clear();
    } else if !contains_marker_prefix(&filter.pending, LOCAL_COMPLETION_REDACTION_PREFIX) {
        visible.push_str(&filter.pending);
        filter.pending.clear();
    }
    visible
}

/// 至少两个字符的保留前缀会暂存在过滤器中，等待下一 PTY 分片后再决定显示。
/// 只在完成后的短暂稳定窗口使用，不会延迟正常的交互式终端输入。
fn contains_marker_prefix(value: &str, marker: &str) -> bool {
    value.char_indices().any(|(index, _)| {
        let candidate = &value[index..];
        candidate.len() >= 2 && candidate.len() < marker.len() && marker.starts_with(candidate)
    })
}

/// 返回完成标记位置、状态长度和状态值；命令回显中的变量表达式不会通过校验。
pub(crate) fn local_completion_status(output: &str, marker: &str) -> Option<(usize, usize, i64)> {
    output.match_indices(marker).find_map(|(index, _)| {
        let suffix = &output[index + marker.len()..];
        let sign_length = usize::from(suffix.starts_with('-'));
        let digit_count = suffix[sign_length..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .map(char::len_utf8)
            .sum::<usize>();
        if digit_count == 0 {
            return None;
        }
        let status_length = sign_length + digit_count;
        let boundary = suffix[status_length..].chars().next();
        if boundary.is_some_and(|character| !matches!(character, '\r' | '\n')) {
            return None;
        }
        suffix[..status_length]
            .parse::<i64>()
            .ok()
            .map(|status| (index, status_length, status))
    })
}

/// 删除 Shell 回显中的内部命令行，只把真实命令输出返回给 Provider。
/// UI 与 Provider 共用相同保留前缀，避免 Windows 单行包装暴露实现细节。
pub(crate) fn sanitize_local_command_output(output: &str) -> String {
    output
        .split_inclusive(['\r', '\n'])
        .filter(|record| !record.contains(LOCAL_COMPLETION_REDACTION_PREFIX))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::Osc633Event;
    use super::{LocalTerminalRegistry, sanitize_local_command_output};

    #[test]
    fn integration_state_tracks_activation_completion_and_gate() {
        let registry = LocalTerminalRegistry::default();
        assert!(!registry.integration_active("local-1"));

        // 首个事件即激活；启动期首提示符的 D;0 会计数但不影响后续快照语义。
        registry.record_integration_event("local-1", &Osc633Event::PromptStarted);
        assert!(registry.integration_active("local-1"));

        let snapshot = registry.completion_snapshot("local-1");
        registry.record_integration_event("local-1", &Osc633Event::CommandFinished(7));
        assert_eq!(
            registry.wait_for_completion("local-1", snapshot, Duration::from_secs(1)),
            Some(7)
        );
        // 快照已被超过：即使再次等待也立即返回已知结果，而不是等待新信号。
        assert_eq!(
            registry.wait_for_completion("local-1", snapshot, Duration::from_millis(30)),
            Some(7)
        );
    }

    #[test]
    fn completion_event_releases_the_input_gate() {
        let registry = LocalTerminalRegistry::default();
        assert!(registry.try_acquire_input_gate("local-1"));
        assert!(registry.input_gated("local-1"));
        // 执行方等待恢复超时保留门控后，迟到的 D 由读取线程放行。
        registry.record_integration_event("local-1", &Osc633Event::CommandFinished(0));
        assert!(!registry.input_gated("local-1"));
    }

    #[test]
    fn input_gate_occupancy_rejects_concurrent_ai_commands() {
        let registry = LocalTerminalRegistry::default();
        assert!(registry.try_acquire_input_gate("local-1"));
        assert!(!registry.try_acquire_input_gate("local-1"));
        registry.set_input_gate("local-1", false);
        assert!(registry.try_acquire_input_gate("local-1"));
    }

    #[test]
    fn unbind_clears_integration_state() {
        let registry = LocalTerminalRegistry::default();
        registry.record_integration_event("local-1", &Osc633Event::PromptStarted);
        registry.unbind_terminal("local-1");
        assert!(!registry.integration_active("local-1"));
        assert_eq!(registry.completion_snapshot("local-1"), 0);
    }

    #[test]
    fn routes_output_to_subscribers() {
        let registry = LocalTerminalRegistry::default();
        registry.bind("local:one".into(), "local-1".into());
        let receiver = registry.subscribe("local-1");
        registry.publish("local-1", "visible output");
        assert_eq!(
            receiver.recv_timeout(Duration::from_millis(50)).unwrap(),
            "visible output"
        );
    }

    #[test]
    fn strips_internal_command_echo_without_dropping_command_output() {
        let output = concat!(
            "PS C:\\> $__nocterm_source = 'pwd'; Write-Output \"__NOCTERM_LOCAL_AI_DONE_x__0\"\r\n",
            "C:\\Users\\runneradmin\r\n",
            "__NOCTERM_LOCAL_AI_DONE_x__0\r\n"
        );

        assert_eq!(
            sanitize_local_command_output(output).trim(),
            r"C:\Users\runneradmin"
        );
    }

    #[test]
    fn hides_chunked_ai_completion_marker_from_ui() {
        let registry = LocalTerminalRegistry::default();
        let marker = "__NOCTERM_LOCAL_AI_DONE_1__";
        registry
            .begin_output_marker_filter("local-1", marker)
            .unwrap();

        let first = registry.visible_output(
            "local-1",
            "➜ ~ pwd\r\n/Users/ming\r\n➜ ~ echo __NOCTERM_LOCAL_AI_",
        );
        let second = registry.visible_output(
            "local-1",
            "DONE_1__%ERRORLEVEL%\r\n__NOCTERM_LOCAL_AI_DONE_1__7\r\n➜ ~ ",
        );

        assert_eq!(first, "➜ ~ pwd\r\n/Users/ming\r\n");
        assert_eq!(second, "➜ ~ ");
        assert!(!format!("{first}{second}").contains(marker));
    }

    #[test]
    fn hides_carriage_return_delimited_marker() {
        let registry = LocalTerminalRegistry::default();
        let marker = "__NOCTERM_LOCAL_AI_DONE_2__";
        registry
            .begin_output_marker_filter("local-2", marker)
            .unwrap();

        let visible = registry.visible_output(
            "local-2",
            &format!("C:\\>echo {marker}%ERRORLEVEL%\r{marker}0\rC:\\>"),
        );

        assert_eq!(visible, "C:\\>");
        assert!(!visible.contains(marker));
    }

    #[test]
    fn hides_delayed_marker_redraw_after_completion() {
        let registry = LocalTerminalRegistry::default();
        let marker = "__NOCTERM_LOCAL_AI_DONE_3__";
        registry
            .begin_output_marker_filter("local-3", marker)
            .unwrap();
        let completed =
            registry.visible_output("local-3", &format!("pwd\r\n/tmp\r\n{marker}0\r\n➜ ~ "));
        assert_eq!(completed, "pwd\r\n/tmp\r\n➜ ~ ");

        let delayed = registry.visible_output(
            "local-3",
            &format!("printf '\\n{marker}%s\\n' \"$?\"\r\n{marker}0\r\n➜ ~ "),
        );

        assert_eq!(delayed, "➜ ~ ");
        assert!(!delayed.contains("printf"));
        assert!(!delayed.contains(marker));
    }

    #[test]
    fn hides_a_delayed_marker_redraw_split_across_pty_chunks() {
        let registry = LocalTerminalRegistry::default();
        let marker = "__NOCTERM_LOCAL_AI_DONE_4__";
        registry
            .begin_output_marker_filter("local-4", marker)
            .unwrap();
        registry.visible_output("local-4", &format!("{marker}0\r\n➜ ~ "));

        let first = registry.visible_output("local-4", "➜ ~ printf '\\n__NOCTERM_LOCAL_AI_");
        let second = registry.visible_output(
            "local-4",
            "DONE_4__%s\\n' \"$?\"\r\n__NOCTERM_LOCAL_AI_DONE_4__0\r\n➜ ~ ",
        );

        assert!(first.is_empty());
        assert_eq!(second, "➜ ~ ");
        assert!(!format!("{first}{second}").contains("printf"));
        assert!(!format!("{first}{second}").contains(marker));
    }

    #[test]
    fn hides_zsh_redraw_fragments_after_the_reserved_prefix() {
        let registry = LocalTerminalRegistry::default();
        let marker = "__NOCTERM_LOCAL_AI_DONE_5__";
        registry
            .begin_output_marker_filter("local-5", marker)
            .unwrap();
        registry.visible_output("local-5", &format!("{marker}0\r\n➜ ~ "));

        let visible = registry.visible_output(
            "local-5",
            "printf '\\n__NOCTERM_LOCAL_AI_DONE\r<ERM_LOCAL_AI_DONE_\r<AI_DONE_5__%s\\n' \"$?\"\r\r\n➜ ~ ",
        );

        assert_eq!(visible, "➜ ~ ");
        assert!(!visible.contains("printf"));
        assert!(!visible.contains("<ERM_LOCAL_AI_DONE_"));
    }

    #[test]
    fn rejects_overlapping_ai_commands_until_the_previous_marker_arrives() {
        let registry = LocalTerminalRegistry::default();
        let first_marker = "__NOCTERM_LOCAL_AI_DONE_first__";
        registry
            .begin_output_marker_filter("local-6", first_marker)
            .unwrap();

        assert!(
            registry
                .begin_output_marker_filter("local-6", "marker-two")
                .unwrap_err()
                .contains("已有 AI 命令")
        );
        registry.visible_output("local-6", &format!("{first_marker}130\r\nPS> "));
        assert!(
            registry
                .begin_output_marker_filter("local-6", "marker-two")
                .is_ok()
        );
    }

    #[test]
    fn unbinding_removes_target_and_output_channel() {
        let registry = LocalTerminalRegistry::default();
        registry.bind("local:one".into(), "local-1".into());
        let receiver = registry.subscribe("local-1");
        registry.unbind("local:one", "local-1");
        assert!(registry.terminal_for("local:one").is_none());
        assert!(receiver.recv_timeout(Duration::from_millis(50)).is_err());
    }
}
