//! Provider 子进程的生命周期和任务级资源清理。
//! 该模块不解析 Provider 协议，只管理 Child、Bridge token 与临时配置文件的所有权。

use std::{
    collections::HashMap,
    path::PathBuf,
    process::{Child, Command, ExitStatus},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::commands::ai_provider::cleanup_paths;

/// 在 Windows 上为 CLI 子进程设置 CREATE_NO_WINDOW：GUI 子系统的宿主进程
/// 拉起控制台程序（codex/claude/grok CLI）时，Windows 默认会为子进程新建
/// 终端窗口；常驻 AI Runtime 在应用启动时即拉起 Provider，表现为打开应用
/// 即弹出终端。非 Windows 平台无此概念，空实现。
pub(crate) fn hide_console_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

#[derive(Default)]
pub struct AiProcessManager {
    state: Mutex<AiProcessState>,
}

#[derive(Default)]
struct AiProcessState {
    children: HashMap<String, AiProcess>,
    startups: HashMap<String, (String, Arc<AtomicBool>)>,
}

/// 在版本/恢复探测前登记，直到正式进程交接完成；失败路径由 Drop 撤销登记。
pub struct AiProcessStartup {
    manager: Arc<AiProcessManager>,
    session_id: String,
    cancellation: Arc<AtomicBool>,
}

impl AiProcessStartup {
    pub fn cancellation(&self) -> &AtomicBool {
        &self.cancellation
    }

    pub fn check(&self) -> Result<(), String> {
        if self.cancellation.load(Ordering::Acquire) {
            Err("AI 会话启动已取消".into())
        } else {
            Ok(())
        }
    }
}

impl Drop for AiProcessStartup {
    fn drop(&mut self) {
        if let Ok(mut state) = self.manager.state.lock()
            && state
                .startups
                .get(&self.session_id)
                .is_some_and(|(_, cancellation)| Arc::ptr_eq(cancellation, &self.cancellation))
        {
            state.startups.remove(&self.session_id);
        }
    }
}

/// Provider 子进程及其不可变的 SSH 目标身份；目标不能从后续 Prompt 推断。
pub struct AiProcess {
    pub child: Child,
    pub conversation_id: String,
    pub connection_id: Option<i64>,
    pub bridge_token: Option<String>,
    pub cleanup_paths: Vec<PathBuf>,
}

/// 持续 Provider 共享的进程句柄；任何退出路径最终都只能回收一次 Child。
pub struct ManagedChild {
    state: Mutex<ManagedChildState>,
}

/// stdout 与 stderr 独立读取时，用于确保最终诊断先于统一退出事件送达前端。
#[derive(Default)]
pub struct StreamCompletion {
    completed: Mutex<bool>,
    condition: Condvar,
}

impl StreamCompletion {
    pub fn complete(&self) {
        if let Ok(mut completed) = self.completed.lock() {
            *completed = true;
            self.condition.notify_all();
        }
    }

    pub fn wait(&self, timeout: Duration) {
        let Ok(completed) = self.completed.lock() else {
            return;
        };
        if !*completed {
            let _ = self.condition.wait_timeout(completed, timeout);
        }
    }
}

enum ManagedChildState {
    Running(Child),
    Exited(Option<i32>),
}

impl ManagedChild {
    pub fn new(child: Child) -> Self {
        Self {
            state: Mutex::new(ManagedChildState::Running(child)),
        }
    }

    /// stdout 关闭后等待进程自然退出；超出宽限期说明协议已损坏，必须强制回收。
    pub fn wait_after_output_closed(&self, grace: Duration) -> Option<i32> {
        let deadline = Instant::now() + grace;
        loop {
            if let Some(code) = self.try_reap() {
                return code;
            }
            if Instant::now() >= deadline {
                return self.terminate();
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// 外层 Some 表示句柄已回收，内层 Option 表示平台是否提供数字退出码。
    fn try_reap(&self) -> Option<Option<i32>> {
        let mut state = self.state.lock().ok()?;
        match &mut *state {
            ManagedChildState::Running(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    let code = exit_code(status);
                    *state = ManagedChildState::Exited(code);
                    Some(code)
                }
                Ok(None) | Err(_) => None,
            },
            ManagedChildState::Exited(code) => Some(*code),
        }
    }

    pub fn terminate(&self) -> Option<i32> {
        let mut state = self.state.lock().ok()?;
        match &mut *state {
            ManagedChildState::Running(child) => {
                let _ = child.kill();
                let code = child.wait().ok().and_then(exit_code);
                *state = ManagedChildState::Exited(code);
                code
            }
            ManagedChildState::Exited(code) => *code,
        }
    }
}

fn exit_code(status: ExitStatus) -> Option<i32> {
    status.code()
}

impl Drop for AiProcess {
    fn drop(&mut self) {
        // std::process::Child 的 Drop 不会终止进程；注册表销毁或异常路径也必须回收 Provider。
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        cleanup_paths(&self.cleanup_paths);
    }
}

pub enum AiProcessPoll {
    Running,
    Finished(AiProcess),
    Missing,
    Failed(String),
}

impl AiProcessManager {
    /// 预占 session ID 并记录所属对话，使 stop 和 reset 都能定位尚未产生 Child 的任务。
    pub fn begin_start(
        self: &Arc<Self>,
        session_id: &str,
        conversation_id: &str,
    ) -> Result<AiProcessStartup, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用")?;
        if state.startups.contains_key(session_id) || state.children.contains_key(session_id) {
            return Err("AI 会话标识已被占用".into());
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        state.startups.insert(
            session_id.into(),
            (conversation_id.into(), Arc::clone(&cancellation)),
        );
        Ok(AiProcessStartup {
            manager: Arc::clone(self),
            session_id: session_id.into(),
            cancellation,
        })
    }

    /// 和进程登记共用状态锁；取消完成后，迟到的 spawn 结果无法重新进入进程表。
    pub fn cancel_startup(&self, session_id: &str) -> Result<bool, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用")?;
        if let Some((_, cancellation)) = state.startups.get(session_id) {
            cancellation.store(true, Ordering::Release);
            return Ok(true);
        }
        Ok(false)
    }

    /// reset/release 同时取消探测和回收正式进程，不能仅撤销持久绑定。
    pub fn close_conversation(&self, conversation_id: &str) -> Result<Vec<AiProcess>, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用")?;
        for (id, cancellation) in state.startups.values() {
            if id == conversation_id {
                cancellation.store(true, Ordering::Release);
            }
        }
        let ids = state
            .children
            .iter()
            .filter(|(_, process)| process.conversation_id == conversation_id)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        Ok(ids
            .into_iter()
            .filter_map(|id| state.children.remove(&id))
            .collect())
    }

    /// 消费 Child 的所有权；被取消或重复登记时也由 AiProcess::drop 在锁外完成回收。
    pub fn insert(
        &self,
        startup: &AiProcessStartup,
        child: Child,
        connection_id: Option<i64>,
        bridge_token: Option<String>,
        cleanup_paths: Vec<PathBuf>,
    ) -> Result<(), String> {
        let mut process = AiProcess {
            child,
            conversation_id: String::new(),
            connection_id,
            bridge_token,
            cleanup_paths,
        };
        let mut state = self
            .state
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用".to_string())?;
        startup.check()?;
        let Some((conversation_id, cancellation)) = state.startups.get(&startup.session_id) else {
            return Err("AI 会话启动已取消".into());
        };
        if !Arc::ptr_eq(cancellation, &startup.cancellation) {
            return Err("AI 会话启动已取消".into());
        }
        process.conversation_id = conversation_id.clone();
        if state.children.contains_key(&startup.session_id) {
            return Err("AI 会话标识已被占用".to_string());
        }
        state.children.insert(startup.session_id.clone(), process);
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<Option<AiProcess>, String> {
        self.state
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用".to_string())
            .map(|mut state| state.children.remove(id))
    }

    /// 只在 Provider 已退出后转移所有权；运行期间保留进程，停止命令才能可靠找到它。
    pub fn poll(&self, id: &str) -> AiProcessPoll {
        let Ok(mut state) = self.state.lock() else {
            return AiProcessPoll::Failed("AI Provider 进程状态不可用".to_string());
        };
        let Some(process) = state.children.get_mut(id) else {
            return AiProcessPoll::Missing;
        };
        match process.child.try_wait() {
            Ok(None) => AiProcessPoll::Running,
            Ok(Some(_)) | Err(_) => state
                .children
                .remove(id)
                .map(AiProcessPoll::Finished)
                .unwrap_or(AiProcessPoll::Missing),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{process::Command, process::Stdio, sync::Arc, time::Duration};

    use super::{AiProcessManager, ManagedChild};

    fn short_lived_child() -> std::process::Child {
        Command::new(std::env::current_exe().expect("test executable"))
            .arg("--list")
            // 测试子进程只用于观察退出状态，不能把测试清单写回父测试的捕获管道。
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn test child")
    }

    #[test]
    fn duplicate_session_id_does_not_replace_the_tracked_process() {
        let manager = Arc::new(AiProcessManager::default());
        let startup = manager.begin_start("ai-one", "conversation").unwrap();
        manager
            .insert(&startup, short_lived_child(), None, None, Vec::new())
            .expect("insert first process");

        let error = manager
            .insert(&startup, short_lived_child(), None, None, Vec::new())
            .expect_err("reject duplicate process");

        assert!(error.contains("已被占用"));
        assert!(manager.remove("ai-one").expect("remove process").is_some());
    }

    #[test]
    fn cancelled_probe_cannot_register_a_late_process() {
        let manager = Arc::new(AiProcessManager::default());
        let startup = manager.begin_start("probe", "conversation").unwrap();
        assert!(manager.cancel_startup("probe").unwrap());
        assert!(startup.check().is_err());
        assert!(
            manager
                .insert(&startup, short_lived_child(), None, None, Vec::new())
                .is_err()
        );
        assert!(manager.remove("probe").unwrap().is_none());
        drop(startup);
        assert!(manager.begin_start("probe", "conversation").is_ok());
    }

    #[test]
    fn conversation_close_cancels_probe_and_removes_registered_child() {
        let manager = Arc::new(AiProcessManager::default());
        let running = manager.begin_start("running", "one").unwrap();
        let probing = manager.begin_start("probing", "one").unwrap();
        let other = manager.begin_start("other", "two").unwrap();
        manager
            .insert(&running, short_lived_child(), None, None, Vec::new())
            .unwrap();
        let removed = manager.close_conversation("one").unwrap();
        assert_eq!(removed.len(), 1);
        assert!(running.check().is_err());
        assert!(probing.check().is_err());
        assert!(other.check().is_ok());
        assert!(manager.remove("running").unwrap().is_none());
    }

    #[test]
    fn managed_child_reaps_once_and_keeps_the_observed_exit_state() {
        let child = ManagedChild::new(short_lived_child());

        // 并行门禁下测试二进制的子进程启动可能明显变慢；这里验证状态保持而非超时策略。
        let first = child.wait_after_output_closed(Duration::from_secs(30));
        let second = child.terminate();

        assert_eq!(first, Some(0));
        assert_eq!(second, first);
    }
}
