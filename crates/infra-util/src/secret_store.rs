//! Secret reference helpers and system credential storage.
//!
//! Stores keep only `storyforge-secret:v1:*` references on disk. The concrete
//! secret value lives in the OS credential store through the `keyring` crate.

pub const SECRET_REF_PREFIX: &str = "storyforge-secret:v1:";
pub const DEFAULT_SECRET_SERVICE: &str = "StoryForge";

pub trait SecretStore: Send + Sync {
    fn put_secret(&self, secret_ref: &str, secret: &str) -> Result<(), String>;
    fn get_secret(&self, secret_ref: &str) -> Result<String, String>;
    fn delete_secret(&self, secret_ref: &str) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct SystemSecretStore {
    service: String,
}

impl Default for SystemSecretStore {
    fn default() -> Self {
        Self {
            service: DEFAULT_SECRET_SERVICE.into(),
        }
    }
}

impl SystemSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, secret_ref: &str) -> Result<keyring::Entry, String> {
        if !is_secret_ref(secret_ref) {
            return Err(format!("无效 SecretRef: {secret_ref}"));
        }
        keyring::Entry::new(&self.service, secret_ref)
            .map_err(|e| format!("打开系统凭据项失败: {e}"))
    }
}

impl SecretStore for SystemSecretStore {
    fn put_secret(&self, secret_ref: &str, secret: &str) -> Result<(), String> {
        self.entry(secret_ref)?
            .set_password(secret)
            .map_err(|e| format!("写入系统凭据失败: {e}"))
    }

    fn get_secret(&self, secret_ref: &str) -> Result<String, String> {
        self.entry(secret_ref)?
            .get_password()
            .map_err(|e| format!("读取系统凭据失败: {e}"))
    }

    fn delete_secret(&self, secret_ref: &str) -> Result<(), String> {
        match self.entry(secret_ref)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("删除系统凭据失败: {e}")),
        }
    }
}

pub fn make_secret_ref(kind: &str, id: &str) -> String {
    format!("{SECRET_REF_PREFIX}{kind}:{id}")
}

pub fn is_secret_ref(value: &str) -> bool {
    value.starts_with(SECRET_REF_PREFIX)
}

pub fn resolve_secret_value(
    value_or_ref: &str,
    secret_store: &dyn SecretStore,
) -> Result<String, String> {
    if is_secret_ref(value_or_ref) {
        secret_store.get_secret(value_or_ref)
    } else {
        Ok(value_or_ref.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_ref_round_trip_shape() {
        let secret_ref = make_secret_ref("llm-connection", "abc");
        assert_eq!(secret_ref, "storyforge-secret:v1:llm-connection:abc");
        assert!(is_secret_ref(&secret_ref));
        assert!(!is_secret_ref("sk-plain"));
    }
}
