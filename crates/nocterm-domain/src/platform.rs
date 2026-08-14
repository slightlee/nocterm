#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformKind {
    MacOs,
    Windows,
    Unsupported,
}

impl PlatformKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformCapabilities {
    pub platform: PlatformKind,
    pub architecture: String,
    pub terminal_backend: &'static str,
    pub credential_store: &'static str,
    pub ssh_transport: &'static str,
}

pub trait PlatformProbe: Send + Sync {
    fn capabilities(&self) -> PlatformCapabilities;
}

#[cfg(test)]
mod tests {
    use super::PlatformKind;

    #[test]
    fn exposes_stable_platform_codes() {
        assert_eq!(PlatformKind::MacOs.as_str(), "macos");
        assert_eq!(PlatformKind::Windows.as_str(), "windows");
        assert_eq!(PlatformKind::Unsupported.as_str(), "unsupported");
    }
}
