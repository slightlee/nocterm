use keyring::Entry;
use nocterm_domain::credential::CredentialStore;

/// 系统凭据适配器只暴露密钥操作，不让上层感知 Keychain/Credential Manager 差异。
#[derive(Debug)]
pub struct CredentialError(String);

impl std::fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CredentialError {}

#[derive(Default)]
pub struct SystemCredentialStore;

impl SystemCredentialStore {
    pub fn store(
        &self,
        connection_id: &str,
        credential_kind: &str,
        secret: &str,
    ) -> Result<(), CredentialError> {
        entry(connection_id, credential_kind)?
            .set_password(secret)
            .map_err(error("写入系统凭据失败"))
    }

    pub fn read(
        &self,
        connection_id: &str,
        credential_kind: &str,
    ) -> Result<String, CredentialError> {
        entry(connection_id, credential_kind)?
            .get_password()
            .map_err(error("读取系统凭据失败"))
    }

    pub fn delete(
        &self,
        connection_id: &str,
        credential_kind: &str,
    ) -> Result<(), CredentialError> {
        entry(connection_id, credential_kind)?
            .delete_credential()
            .map_err(error("删除系统凭据失败"))
    }
}

impl CredentialStore for SystemCredentialStore {
    fn store(
        &self,
        connection_id: &str,
        credential_kind: &str,
        secret: &str,
    ) -> Result<(), String> {
        Self::store(self, connection_id, credential_kind, secret).map_err(|error| error.to_string())
    }

    fn read(&self, connection_id: &str, credential_kind: &str) -> Result<String, String> {
        Self::read(self, connection_id, credential_kind).map_err(|error| error.to_string())
    }

    fn delete(&self, connection_id: &str, credential_kind: &str) -> Result<(), String> {
        Self::delete(self, connection_id, credential_kind).map_err(|error| error.to_string())
    }
}

fn entry(connection_id: &str, credential_kind: &str) -> Result<Entry, CredentialError> {
    Entry::new(
        "com.nocterm.desktop.ssh",
        &format!("{connection_id}:{credential_kind}"),
    )
    .map_err(error("创建系统凭据引用失败"))
}

fn error<Source: std::fmt::Display>(
    context: &'static str,
) -> impl FnOnce(Source) -> CredentialError {
    move |source| CredentialError(format!("{context}: {source}"))
}
