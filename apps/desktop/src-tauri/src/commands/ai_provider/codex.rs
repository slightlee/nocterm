//! Codex 使用持久化 app-server，会话生命周期由专用协议适配器管理。

use super::{ProviderAdapter, ProviderLaunch, ProviderLaunchPlan};

pub(super) static ADAPTER: &dyn ProviderAdapter = &CodexAdapter;

struct CodexAdapter;

impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn command(&self) -> &'static str {
        "codex"
    }

    fn prepare_launch(&self, _launch: ProviderLaunch<'_>) -> Result<ProviderLaunchPlan, String> {
        Ok(ProviderLaunchPlan::Persistent)
    }
}
