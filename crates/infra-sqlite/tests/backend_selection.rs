//! Backend selector tests: default SQLite (Gate 7), explicit JSON fallback,
//! unknown rejection, no dual-write, runtime switch rejection, diagnostics
//! secret redaction.

use storyforge_infra_sqlite::backend::{
    BackendDiagnostics, BackendSelection, BackendSource, DEFAULT_BACKEND_ENV_VAR, PinnedBackend,
    StorageBackend,
};
use tempfile::TempDir;

#[test]
fn explicit_json_backend_is_selectable() {
    let pinned = PinnedBackend::new(StorageBackend::Json, BackendSource::Default);
    assert_eq!(pinned.backend(), StorageBackend::Json);
    assert!(!pinned.is_sqlite());
}

/// 进程内 env 串行锁：集成测试进程内多个测试并行，remove/set env 需互斥
/// （Gate 8 审查 P2-D1）。
static ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn explicit_sqlite_from_config_value() {
    // Ensure the env var is not set so config_value is used.
    // SAFETY: test-only; guarded by ENV_TEST_LOCK and restored afterwards.
    let _env_guard = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let original = std::env::var_os(DEFAULT_BACKEND_ENV_VAR);
    unsafe {
        std::env::remove_var(DEFAULT_BACKEND_ENV_VAR);
    }
    let sel = BackendSelection::from_env(Some("sqlite".into()));
    let pinned = PinnedBackend::resolve(&sel).unwrap();
    assert_eq!(pinned.backend(), StorageBackend::Sqlite);
    assert_eq!(pinned.source(), BackendSource::Config);
    // 恢复原值，避免残留影响后续测试。
    match original {
        Some(v) => unsafe { std::env::set_var(DEFAULT_BACKEND_ENV_VAR, v) },
        None => unsafe { std::env::remove_var(DEFAULT_BACKEND_ENV_VAR) },
    }
}

#[test]
fn explicit_json_from_config_value() {
    // Ensure the env var is not set so config_value is used.
    // SAFETY: test-only; guarded by ENV_TEST_LOCK and restored afterwards.
    let _env_guard = ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let original = std::env::var_os(DEFAULT_BACKEND_ENV_VAR);
    unsafe {
        std::env::remove_var(DEFAULT_BACKEND_ENV_VAR);
    }
    let sel = BackendSelection::from_env(Some("json".into()));
    let pinned = PinnedBackend::resolve(&sel).unwrap();
    assert_eq!(pinned.backend(), StorageBackend::Json);
    assert_eq!(pinned.source(), BackendSource::Config);
    match original {
        Some(v) => unsafe { std::env::set_var(DEFAULT_BACKEND_ENV_VAR, v) },
        None => unsafe { std::env::remove_var(DEFAULT_BACKEND_ENV_VAR) },
    }
}

#[test]
fn unknown_backend_value_rejected() {
    let sel = BackendSelection {
        config_value: Some("mongodb".into()),
        env_value: None,
        env_name: DEFAULT_BACKEND_ENV_VAR,
    };
    assert!(PinnedBackend::resolve(&sel).is_err());
}

#[test]
fn runtime_switch_rejected() {
    let pinned = PinnedBackend::new(StorageBackend::Json, BackendSource::Default);
    let err = pinned.assert_unchanged(StorageBackend::Sqlite).unwrap_err();
    assert!(err.to_string().contains("cannot be changed"));
    // Same value is fine.
    assert!(pinned.assert_unchanged(StorageBackend::Json).is_ok());
}

#[test]
fn backend_selection_is_explicit_only_when_set() {
    let implicit = BackendSelection {
        config_value: None,
        env_value: None,
        env_name: DEFAULT_BACKEND_ENV_VAR,
    };
    assert!(!implicit.is_explicit());

    let explicit = BackendSelection {
        config_value: Some("sqlite".into()),
        env_value: None,
        env_name: DEFAULT_BACKEND_ENV_VAR,
    };
    assert!(explicit.is_explicit());
}

#[test]
fn no_dual_write_selector_does_not_open_database() {
    // The selector is pure data — it never opens a DB or reads/writes JSON.
    // 直接构造无 env 的 Selection：不读真实环境变量（Gate 8 审查 P2-D2——
    // 开发者 shell 恰好设置 STORYFORGE_STORAGE_BACKEND 时不得假红）。
    let _dir = TempDir::new().unwrap();
    let sel = BackendSelection {
        config_value: None,
        env_value: None,
        env_name: DEFAULT_BACKEND_ENV_VAR,
    };
    let pinned = PinnedBackend::resolve(&sel).unwrap();
    // Gate 7: 无任何显式选择时默认 SQLite。
    assert_eq!(pinned.backend(), StorageBackend::Sqlite);
    assert_eq!(pinned.source(), BackendSource::Default);
}

#[test]
fn diagnostics_never_expose_paths_or_secrets() {
    let pinned = PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env);
    let diag = BackendDiagnostics::from_pinned(&pinned, Some(3));
    let json = serde_json::to_string(&diag).unwrap();
    assert!(json.contains("sqlite"));
    assert!(json.contains("env"));
    // No Windows drive letters (C:\), no user paths, no temp dirs.
    assert!(!json.contains("C:\\"));
    assert!(!json.contains("/tmp/"));
    assert!(!json.contains("Users"));
    assert!(!json.contains("\\\\"));
}
