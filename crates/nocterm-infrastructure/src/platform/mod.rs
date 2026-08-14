use nocterm_domain::platform::{PlatformCapabilities, PlatformKind, PlatformProbe};

#[derive(Debug, Default)]
pub struct SystemPlatformProbe;

impl PlatformProbe for SystemPlatformProbe {
    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            platform: platform_kind(),
            architecture: std::env::consts::ARCH.to_string(),
            terminal_backend: terminal_backend(),
            credential_store: credential_store(),
            ssh_transport: "openssh-capability-probe",
        }
    }
}

const fn platform_kind() -> PlatformKind {
    #[cfg(target_os = "macos")]
    {
        return PlatformKind::MacOs;
    }
    #[cfg(target_os = "windows")]
    {
        return PlatformKind::Windows;
    }
    #[allow(unreachable_code)]
    PlatformKind::Unsupported
}

const fn terminal_backend() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        return "macos-pty";
    }
    #[cfg(target_os = "windows")]
    {
        return "windows-conpty";
    }
    #[allow(unreachable_code)]
    "unsupported"
}

const fn credential_store() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        return "macos-keychain";
    }
    #[cfg(target_os = "windows")]
    {
        return "windows-credential-manager";
    }
    #[allow(unreachable_code)]
    "unsupported"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_compiled_platform() {
        let capabilities = SystemPlatformProbe.capabilities();
        assert_eq!(capabilities.architecture, std::env::consts::ARCH);
        assert_ne!(capabilities.terminal_backend, "");
        assert_ne!(capabilities.credential_store, "");
    }
}
