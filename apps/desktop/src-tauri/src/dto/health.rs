use nocterm_application::health::HealthSnapshot;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    service: &'static str,
    version: String,
    platform: &'static str,
    architecture: String,
    terminal_backend: &'static str,
    credential_store: &'static str,
    ssh_transport: &'static str,
}

impl From<HealthSnapshot> for HealthResponse {
    fn from(value: HealthSnapshot) -> Self {
        Self {
            service: value.service,
            version: value.version,
            platform: value.platform,
            architecture: value.architecture,
            terminal_backend: value.terminal_backend,
            credential_store: value.credential_store,
            ssh_transport: value.ssh_transport,
        }
    }
}
