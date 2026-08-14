use std::sync::Arc;

use nocterm_domain::platform::PlatformProbe;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthSnapshot {
    pub service: &'static str,
    pub version: String,
    pub platform: &'static str,
    pub architecture: String,
    pub terminal_backend: &'static str,
    pub credential_store: &'static str,
    pub ssh_transport: &'static str,
}

#[derive(Clone)]
pub struct HealthService {
    platform_probe: Arc<dyn PlatformProbe>,
    version: String,
}

impl HealthService {
    pub fn new(platform_probe: Arc<dyn PlatformProbe>, version: impl Into<String>) -> Self {
        Self {
            platform_probe,
            version: version.into(),
        }
    }

    pub fn check(&self) -> HealthSnapshot {
        let capabilities = self.platform_probe.capabilities();
        HealthSnapshot {
            service: "nocterm-desktop",
            version: self.version.clone(),
            platform: capabilities.platform.as_str(),
            architecture: capabilities.architecture,
            terminal_backend: capabilities.terminal_backend,
            credential_store: capabilities.credential_store,
            ssh_transport: capabilities.ssh_transport,
        }
    }
}

#[cfg(test)]
mod tests {
    use nocterm_domain::platform::{PlatformCapabilities, PlatformKind, PlatformProbe};

    use super::*;

    struct FakePlatformProbe;

    impl PlatformProbe for FakePlatformProbe {
        fn capabilities(&self) -> PlatformCapabilities {
            PlatformCapabilities {
                platform: PlatformKind::Windows,
                architecture: "x86_64".to_string(),
                terminal_backend: "conpty",
                credential_store: "credential-manager",
                ssh_transport: "openssh-probe",
            }
        }
    }

    #[test]
    fn returns_a_platform_independent_health_snapshot() {
        let service = HealthService::new(Arc::new(FakePlatformProbe), "0.1.0");
        let health = service.check();

        assert_eq!(health.service, "nocterm-desktop");
        assert_eq!(health.platform, "windows");
        assert_eq!(health.terminal_backend, "conpty");
        assert_eq!(health.version, "0.1.0");
    }
}
