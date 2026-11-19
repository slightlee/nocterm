//! 系统关窗和退出先向已就绪的 UI 请求确认，不能依赖可能落后的 dirty 快照。
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone)]
pub enum CloseIntent {
    Window(String),
    Exit,
}
#[derive(Default)]
struct CloseState {
    // pending 保存唯一的系统动作；前端只回传 ID，不能另选关闭目标。
    enabled: bool,
    sequence: u64,
    pending: Option<(u64, CloseIntent)>,
    approved: bool,
}
#[derive(Default)]
pub struct FileEditorCloseState(Mutex<CloseState>);

impl FileEditorCloseState {
    pub fn enable(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.enabled = true;
        }
    }
    pub fn restore(&self) {
        // 系统动作失败时撤回批准，后续点击仍需重新确认。
        if let Ok(mut state) = self.0.lock() {
            state.approved = false;
        }
    }

    /// 重复请求不叠加确认框；只有匹配当前请求的回复才可以执行关窗。
    fn request(&self, intent: CloseIntent) -> (bool, Option<u64>) {
        // 锁异常时保留窗口，不能无条件丢弃用户内容。
        let Ok(mut state) = self.0.lock() else {
            return (true, None);
        };
        if !state.enabled || state.approved {
            return (false, None);
        }
        if state.pending.is_some() {
            return (true, None);
        }
        state.sequence += 1;
        let id = state.sequence;
        state.pending = Some((id, intent));
        (true, Some(id))
    }

    pub fn resolve(&self, request_id: u64, allow: bool) -> Option<CloseIntent> {
        let mut state = self.0.lock().ok()?;
        if state.pending.as_ref()?.0 != request_id {
            // 旧对话的回复不能清理新的待处理请求。
            return None;
        }
        let (_, intent) = state.pending.take()?;
        if !allow {
            return None;
        }
        state.approved = true;
        Some(intent)
    }
}

pub fn request_close(app: &AppHandle, intent: CloseIntent) -> bool {
    // 系统事件循环不等待 UI：同步阻止关闭，再通过异步应答执行动作。
    let state = app.state::<FileEditorCloseState>();
    let (prevent, request_id) = state.request(intent);
    if let Some(request_id) = request_id
        && let Err(error) = app.emit("nocterm://file-editor-close-requested", request_id)
    {
        state.resolve(request_id, false);
        // 发送失败撤销待处理请求，窗口保留，用户可再次尝试。
        eprintln!("发送文件关闭确认失败：{error}");
    }
    prevent
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_and_stale_responses_cannot_bypass_the_guard() {
        let state = FileEditorCloseState::default();
        assert!(!state.request(CloseIntent::Exit).0);
        state.enable();
        let (prevent, id) = state.request(CloseIntent::Exit);
        assert!(prevent);
        assert!(state.request(CloseIntent::Exit).1.is_none());
        assert!(state.resolve(id.unwrap() + 1, true).is_none());
        assert!(state.resolve(id.unwrap(), false).is_none());
        let next = state.request(CloseIntent::Window("main".into())).1.unwrap();
        assert!(matches!(
            state.resolve(next, true),
            Some(CloseIntent::Window(_))
        ));
        assert!(!state.request(CloseIntent::Exit).0);
    }
}
