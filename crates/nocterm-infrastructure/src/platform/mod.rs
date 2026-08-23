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

/// SSH 传输实现：进程内 `russh`，与系统是否安装 OpenSSH 无关（见 ADR-002）。
///
/// 这里曾经 spawn `ssh -V` 去探测系统 OpenSSH 并上报 `openssh-available`。
/// 迁移到进程内实现后该探测既无用又有害：没装 OpenSSH 的 Windows 会被报成
/// `openssh-unavailable`，让用户以为连不上 SSH；而从 GUI 进程 spawn 控制台程序
/// 还会闪出黑窗，并把启动路径卡在一次同步的进程创建与等待上。
const fn ssh_transport() -> &'static str {
    "in-process-russh"
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
        // SSH 传输与平台、与系统有没有装 OpenSSH 都无关，两个平台上都必须是进程内实现。
        assert_eq!(capabilities.ssh_transport, "in-process-russh");
    }
}
