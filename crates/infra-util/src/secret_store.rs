//! Secret reference helpers and system credential storage.
//!
//! Stores keep only `storyforge-secret:v1:*` references on disk. The concrete
//! secret value lives in the OS credential store through the `keyring` crate.

pub const SECRET_REF_PREFIX: &str = "storyforge-secret:v1:";
pub const DEFAULT_SECRET_SERVICE: &str = "StoryForge";

use std::sync::OnceLock;

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
        ensure_native_store()?;
        keyring::Entry::new(&self.service, secret_ref)
            .map_err(|e| format!("打开系统凭据项失败: {e}"))
    }
}

fn ensure_native_store() -> Result<(), String> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(init_native_store).clone()
}

#[cfg(target_os = "windows")]
fn init_native_store() -> Result<(), String> {
    let store = windows_native_keyring_store::Store::new()
        .map_err(|e| format!("初始化系统凭据库失败: {e}"))?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(target_os = "android")]
fn init_native_store() -> Result<(), String> {
    // android_native_keyring_store::Store::new() calls ndk_context::android_context()
    // internally, which panics (not returns Err) when ndk-context has not been
    // initialized yet. Wrap in catch_unwind so the panic becomes an Err; this
    // lets the upstream tracing::warn! guard in ConnectionStore
    // (connection_store.rs migrate_plaintext_api_keys) keep the api_key as
    // plaintext instead of crashing the process. Once ndk-context is properly
    // initialized (see tauri-app setup hook), Store::new() succeeds and this
    // catch_unwind is a no-op.
    let store = std::panic::catch_unwind(android_native_keyring_store::Store::new)
        .map_err(|_| {
            "Android Keystore 初始化 panic（ndk-context 未初始化），凭据降级为明文".to_string()
        })?
        .map_err(|e| format!("初始化系统凭据库失败: {e}"))?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn init_native_store() -> Result<(), String> {
    let store = apple_native_keyring_store::keychain::Store::new()
        .map_err(|e| format!("初始化系统凭据库失败: {e}"))?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
fn init_native_store() -> Result<(), String> {
    let store = zbus_secret_service_keyring_store::Store::new()
        .map_err(|e| format!("初始化系统凭据库失败: {e}"))?;
    keyring_core::set_default_store(store);
    Ok(())
}

#[cfg(not(any(
    target_os = "windows",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
fn init_native_store() -> Result<(), String> {
    Err("当前平台没有配置系统凭据库后端".into())
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

    #[test]
    #[ignore = "writes a throwaway secret to the OS credential store"]
    fn system_keyring_write_read_delete_roundtrip() {
        let store = SystemSecretStore::new(format!("StoryForgeTest-{}", unique_id()));
        let secret_ref = make_secret_ref("keyring-smoke", &unique_id());
        let secret = format!("test-secret-{}", unique_id());

        store.delete_secret(&secret_ref).unwrap();
        store.put_secret(&secret_ref, &secret).unwrap();
        assert_eq!(store.get_secret(&secret_ref).unwrap(), secret);
        store.delete_secret(&secret_ref).unwrap();
        assert!(store.get_secret(&secret_ref).is_err());
    }

    /// Plaintext values must pass through resolve_secret_value unchanged.
    /// This is the Android degraded-mode correctness invariant: when the
    /// Keystore is unavailable (init_native_store returned Err via
    /// catch_unwind), ConnectionStore keeps api_key as plaintext, and every
    /// read path must resolve it back to the same plaintext without error.
    #[test]
    fn resolve_secret_value_passes_plaintext_through() {
        let store = SystemSecretStore::new("test-plaintext-passthrough");
        // Plaintext (not a secret ref) → returned as-is, no store access.
        let plain = "sk-plaintext-fallback-key";
        let resolved = resolve_secret_value(plain, &store).unwrap();
        assert_eq!(resolved, plain);
    }

    /// A non-SecretRef value is never confused with a SecretRef.
    /// Discriminates the plaintext fallback from the keystore path: a real
    /// key that happens to start with "sk-" must NOT be treated as a ref.
    #[test]
    fn plaintext_starting_with_sk_is_not_treated_as_secret_ref() {
        let val = "sk-something-1234567890";
        assert!(!is_secret_ref(val));
        assert!(val.starts_with("sk-"));
    }

    fn unique_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        format!(
            "{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )
    }
}
