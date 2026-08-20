use keyring::Entry;
use nocterm_domain::credential::CredentialStore;
use std::collections::HashMap;
use std::sync::Mutex;

/// 系统凭据适配器只暴露密钥操作，不让上层感知 Keychain/Credential Manager 差异。
#[derive(Debug)]
pub struct CredentialError(String);

impl std::fmt::Display for CredentialError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CredentialError {}

/// 系统凭据服务在进程内缓存已成功读取的值，避免 SFTP 每次目录请求都重新触发平台授权。
/// 明文只存在当前进程内存，写入或删除时立即失效对应缓存。
///
/// 平台读取也在同一把锁内完成。SSH 终端和 SFTP 可能同时首次读取同一凭据，
/// 若先释放锁再访问钥匙串，两条请求会同时触发 macOS 授权对话框。
#[derive(Default)]
pub struct SystemCredentialStore {
    cache: Mutex<HashMap<String, String>>,
}

impl SystemCredentialStore {
    pub fn store(
        &self,
        connection_id: &str,
        credential_kind: &str,
        secret: &str,
    ) -> Result<(), CredentialError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| CredentialError("系统凭据缓存锁已损坏".to_string()))?;
        entry(connection_id, credential_kind)?
            .set_password(secret)
            .map_err(error("写入系统凭据失败"))?;
        cache.insert(
            cache_key(connection_id, credential_kind),
            secret.to_string(),
        );
        Ok(())
    }

    pub fn read(
        &self,
        connection_id: &str,
        credential_kind: &str,
    ) -> Result<String, CredentialError> {
        let key = cache_key(connection_id, credential_kind);
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| CredentialError("系统凭据缓存锁已损坏".to_string()))?;
        if let Some(secret) = cache.get(&key).cloned() {
            return Ok(secret);
        }
        let secret = entry(connection_id, credential_kind)?
            .get_password()
            .map_err(error("读取系统凭据失败"))?;
        cache.insert(key, secret.clone());
        Ok(secret)
    }

    pub fn delete(
        &self,
        connection_id: &str,
        credential_kind: &str,
    ) -> Result<(), CredentialError> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| CredentialError("系统凭据缓存锁已损坏".to_string()))?;
        entry(connection_id, credential_kind)?
            .delete_credential()
            .map_err(error("删除系统凭据失败"))?;
        cache.remove(&cache_key(connection_id, credential_kind));
        Ok(())
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

fn cache_key(connection_id: &str, credential_kind: &str) -> String {
    format!("{connection_id}:{credential_kind}")
}

fn entry(connection_id: &str, credential_kind: &str) -> Result<Entry, CredentialError> {
    Entry::new(
        "com.nocterm.desktop.ssh",
        &cache_key(connection_id, credential_kind),
    )
    .map_err(error("创建系统凭据引用失败"))
}

fn error<Source: std::fmt::Display>(
    context: &'static str,
) -> impl FnOnce(Source) -> CredentialError {
    move |source| CredentialError(format!("{context}: {source}"))
}
