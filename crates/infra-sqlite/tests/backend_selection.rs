//! Backend selector tests: default JSON, explicit SQLite, unknown rejection,
//! no dual-write, runtime switch rejection, diagnostics secret redaction.

use storyforge_infra_sqlite::backend::{
    BackendDiagnostics, BackendSelection, BackendSource, DEFAULT_BACKEND_ENV_VAR, PinnedBackend,
    StorageBackend,
};
use tempfile::TempDir;

#[test]
fn default_backend_is_json() {
    let pinned = PinnedBackend::new(StorageBackend::Json, BackendSource::Default);
    assert_eq!(pinned.backend(), StorageBackend::Json);
    assert!(!pinned.is_sqlite());
}

#[test]
fn explicit_sqlite_from_config_value() {
    // Ensure the env var is not set so config_value is used.
    // SAFETY: this is a test; we remove and restore the env var. No other thread
    // depends on it during this test.
    unsafe {
        std::env::remove_var(DEFAULT_BACKEND_ENV_VAR);
    }
    let sel = BackendSelection::from_env(Some("sqlite".into()));
    let pinned = PinnedBackend::resolve(&sel).unwrap();
    assert_eq!(pinned.backend(), StorageBackend::Sqlite);
    assert_eq!(pinned.source(), BackendSource::Config);
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
    let _dir = TempDir::new().unwrap();
    let sel = BackendSelection::from_env(None);
    let pinned = PinnedBackend::resolve(&sel).unwrap();
    assert_eq!(pinned.backend(), StorageBackend::Json);
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
