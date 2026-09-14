//! Provider 子进程的生命周期和任务级资源清理。
//! 该模块不解析 Provider 协议，只管理 Child、Bridge token 与临时配置文件的所有权。

use std::{collections::HashMap, path::PathBuf, process::Child, sync::Mutex};

use crate::commands::ai_provider::cleanup_paths;

#[derive(Default)]
pub struct AiProcessManager {
    children: Mutex<HashMap<String, AiProcess>>,
}

/// Provider 子进程及其不可变的 SSH 目标身份；目标不能从后续 Prompt 推断。
pub struct AiProcess {
    pub child: Child,
    pub connection_id: Option<i64>,
    pub bridge_token: Option<String>,
    pub cleanup_paths: Vec<PathBuf>,
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
    pub fn insert(
        &self,
        id: String,
        child: Child,
        connection_id: Option<i64>,
        bridge_token: Option<String>,
        cleanup_paths: Vec<PathBuf>,
    ) -> Result<(), String> {
        let process = AiProcess {
            child,
            connection_id,
            bridge_token,
            cleanup_paths,
        };
        let mut children = self
            .children
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用".to_string())?;
        if children.contains_key(&id) {
            return Err("AI 会话标识已被占用".to_string());
        }
        children.insert(id, process);
        Ok(())
    }

    pub fn remove(&self, id: &str) -> Result<Option<AiProcess>, String> {
        self.children
            .lock()
            .map_err(|_| "AI Provider 进程状态不可用".to_string())
            .map(|mut children| children.remove(id))
    }

    /// 只在 Provider 已退出后转移所有权；运行期间保留进程，停止命令才能可靠找到它。
    pub fn poll(&self, id: &str) -> AiProcessPoll {
        let Ok(mut children) = self.children.lock() else {
            return AiProcessPoll::Failed("AI Provider 进程状态不可用".to_string());
        };
        let Some(process) = children.get_mut(id) else {
            return AiProcessPoll::Missing;
        };
        match process.child.try_wait() {
            Ok(None) => AiProcessPoll::Running,
            Ok(Some(_)) | Err(_) => children
                .remove(id)
                .map(AiProcessPoll::Finished)
                .unwrap_or(AiProcessPoll::Missing),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::AiProcessManager;

    fn short_lived_child() -> std::process::Child {
        Command::new(std::env::current_exe().expect("test executable"))
            .arg("--list")
            .spawn()
            .expect("spawn test child")
    }

    #[test]
    fn duplicate_session_id_does_not_replace_the_tracked_process() {
        let manager = AiProcessManager::default();
        manager
            .insert("ai-one".into(), short_lived_child(), None, None, Vec::new())
            .expect("insert first process");

        let error = manager
            .insert("ai-one".into(), short_lived_child(), None, None, Vec::new())
            .expect_err("reject duplicate process");

        assert!(error.contains("已被占用"));
        assert!(manager.remove("ai-one").expect("remove process").is_some());
    }
}
