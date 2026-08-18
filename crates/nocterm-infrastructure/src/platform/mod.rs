#[cfg(any(target_os = "macos", target_os = "windows"))]
use std::process::Command;

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
            ssh_transport: ssh_transport(),
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

fn ssh_transport() -> &'static str {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let binary = if cfg!(target_os = "windows") {
            "ssh.exe"
        } else {
            "/usr/bin/ssh"
        };
        return match Command::new(binary).arg("-V").output() {
            Ok(output) if output.status.success() || !output.stderr.is_empty() => {
                "openssh-available"
            }
            _ => "openssh-unavailable",
        };
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
        assert!(matches!(
            capabilities.ssh_transport,
            "openssh-available" | "openssh-unavailable" | "unsupported"
        ));
        #[cfg(target_os = "macos")]
        assert_eq!(capabilities.ssh_transport, "openssh-available");
    }
}
