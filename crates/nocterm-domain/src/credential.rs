/// 系统凭据存储的领域边界，调用方不感知 Keychain 或 Credential Manager。
pub trait CredentialStore: Send + Sync {
    fn store(&self, connection_id: &str, credential_kind: &str, secret: &str)
    -> Result<(), String>;

    fn read(&self, connection_id: &str, credential_kind: &str) -> Result<String, String>;

    fn delete(&self, connection_id: &str, credential_kind: &str) -> Result<(), String>;
}
