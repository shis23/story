//! Typed storage backend selector for the opt-in SQLite path.
//!
//! JSON remains the production default. SQLite is only selected when an explicit
//! override is present. The selector is resolved once at process startup and is
//! fixed for the process lifetime — runtime switching is rejected.
//!
//! This module never opens a database or performs I/O. It only parses and
//! validates the selection, keeping the default path free of SQLite coupling.

use std::env;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The configured storage backend.
///
/// `Json` is the serde default and the value used when no selector is present.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    /// The default production backend (JSON files).
    #[default]
    Json,
    /// The opt-in transactional backend (SQLite).
    ///
    /// Requires an explicit configuration override; never chosen implicitly.
    Sqlite,
}

impl fmt::Display for StorageBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StorageBackend::Json => f.write_str("json"),
            StorageBackend::Sqlite => f.write_str("sqlite"),
        }
    }
}

impl StorageBackend {
    /// Stable wire name used in config, env vars, and the backend marker.
    pub fn as_str(self) -> &'static str {
        match self {
            StorageBackend::Json => "json",
            StorageBackend::Sqlite => "sqlite",
        }
    }

    /// Parse a backend name, rejecting unknown values.
    pub fn parse(value: &str) -> Result<Self, BackendSelectionError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(StorageBackend::Json),
            "sqlite" => Ok(StorageBackend::Sqlite),
            other => Err(BackendSelectionError::UnknownValue(other.to_string())),
        }
    }
}

/// Errors produced while resolving the storage backend selection.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendSelectionError {
    /// The configured value did not match a known backend.
    #[error("unknown storage backend value: {0:?} (expected \"json\" or \"sqlite\")")]
    UnknownValue(String),

    /// The backend was changed after it had been fixed for the process.
    #[error("storage backend cannot be changed after startup: was {previous}, now {current}")]
    RuntimeSwitch {
        previous: &'static str,
        current: &'static str,
    },
}

/// Resolution inputs for the storage backend.
///
/// Precedence (low → high):
/// 1. `config_value` (from a settings file / explicit struct field)
/// 2. `env_value` (from `env_name`)
///
/// When both are absent the default (`Json`) is used.
#[derive(Debug, Clone, Default)]
pub struct BackendSelection {
    /// Value parsed from an explicit config field, if any.
    pub config_value: Option<String>,
    /// Value parsed from the environment variable named `env_name`, if any.
    pub env_value: Option<String>,
    /// The environment variable name consulted for `env_value`.
    pub env_name: &'static str,
}

impl BackendSelection {
    /// Build a selection from the process environment.
    ///
    /// `config_value` is the optional value from a settings file or explicit
    /// field. The environment variable `env_name` (default
    /// `STORYFORGE_STORAGE_BACKEND`) overrides it when set and non-empty.
    pub fn from_env_with(config_value: Option<String>, env_name: &'static str) -> Self {
        let env_value = env::var(env_name).ok().filter(|v| !v.trim().is_empty());
        BackendSelection {
            config_value,
            env_value,
            env_name,
        }
    }

    /// Convenience using the canonical environment variable name.
    pub fn from_env(config_value: Option<String>) -> Self {
        Self::from_env_with(config_value, DEFAULT_BACKEND_ENV_VAR)
    }

    /// Resolve the final backend, defaulting to JSON when nothing is set.
    pub fn resolve(&self) -> Result<StorageBackend, BackendSelectionError> {
        if let Some(raw) = &self.env_value {
            return StorageBackend::parse(raw);
        }
        if let Some(raw) = &self.config_value {
            return StorageBackend::parse(raw);
        }
        Ok(StorageBackend::Json)
    }

    /// Whether the selection is explicit (i.e. not just the default).
    pub fn is_explicit(&self) -> bool {
        self.env_value.is_some() || self.config_value.is_some()
    }
}

/// Canonical environment variable name for the opt-in override.
pub const DEFAULT_BACKEND_ENV_VAR: &str = "STORYFORGE_STORAGE_BACKEND";

/// Fixed, process-lifetime backend guard.
///
/// Once the backend is resolved it must not change. This guard records the
/// pinned value and rejects any attempt to switch at runtime.
#[derive(Debug, Clone)]
pub struct PinnedBackend {
    backend: StorageBackend,
    source: BackendSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendSource {
    Default,
    Config,
    Env,
}

impl PinnedBackend {
    pub fn new(backend: StorageBackend, source: BackendSource) -> Self {
        PinnedBackend { backend, source }
    }

    pub fn backend(&self) -> StorageBackend {
        self.backend
    }

    pub fn source(&self) -> BackendSource {
        self.source
    }

    pub fn is_sqlite(&self) -> bool {
        matches!(self.backend, StorageBackend::Sqlite)
    }

    /// Resolve and pin the backend for the process from a selection.
    pub fn resolve(selection: &BackendSelection) -> Result<Self, BackendSelectionError> {
        if let Some(raw) = &selection.env_value {
            let backend = StorageBackend::parse(raw)?;
            return Ok(PinnedBackend::new(backend, BackendSource::Env));
        }
        if let Some(raw) = &selection.config_value {
            let backend = StorageBackend::parse(raw)?;
            return Ok(PinnedBackend::new(backend, BackendSource::Config));
        }
        Ok(PinnedBackend::new(
            StorageBackend::Json,
            BackendSource::Default,
        ))
    }

    /// Reject any attempt to change the backend after it has been pinned.
    pub fn assert_unchanged(&self, proposed: StorageBackend) -> Result<(), BackendSelectionError> {
        if self.backend != proposed {
            return Err(BackendSelectionError::RuntimeSwitch {
                previous: self.backend.as_str(),
                current: proposed.as_str(),
            });
        }
        Ok(())
    }
}

/// Redacted diagnostics suitable for logging — never includes paths or secrets.
#[derive(Debug, Clone, Serialize)]
pub struct BackendDiagnostics {
    pub backend: &'static str,
    pub source: &'static str,
    pub schema_version: Option<i64>,
}

impl BackendDiagnostics {
    pub fn from_pinned(pinned: &PinnedBackend, schema_version: Option<i64>) -> Self {
        BackendDiagnostics {
            backend: pinned.backend.as_str(),
            source: match pinned.source() {
                BackendSource::Default => "default",
                BackendSource::Config => "config",
                BackendSource::Env => "env",
            },
            schema_version,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backend_is_json() {
        assert_eq!(StorageBackend::default(), StorageBackend::Json);
    }

    #[test]
    fn serde_uses_lowercase_and_defaults_to_json() {
        let json: StorageBackend = serde_json::from_str("\"json\"").unwrap();
        assert_eq!(json, StorageBackend::Json);
        let sqlite: StorageBackend = serde_json::from_str("\"sqlite\"").unwrap();
        assert_eq!(sqlite, StorageBackend::Sqlite);
        let defaulted: StorageBackend = serde_json::from_str("null").unwrap_or_default();
        assert_eq!(defaulted, StorageBackend::Json);
        assert_eq!(
            serde_json::to_string(&StorageBackend::Sqlite).unwrap(),
            "\"sqlite\""
        );
    }

    #[test]
    fn parse_accepts_known_values_case_insensitively() {
        assert_eq!(StorageBackend::parse("json").unwrap(), StorageBackend::Json);
        assert_eq!(StorageBackend::parse("JSON").unwrap(), StorageBackend::Json);
        assert_eq!(
            StorageBackend::parse("  SQLite ").unwrap(),
            StorageBackend::Sqlite
        );
    }

    #[test]
    fn parse_rejects_unknown_values() {
        let err = StorageBackend::parse("postgres").unwrap_err();
        assert!(matches!(err, BackendSelectionError::UnknownValue(_)));
        let err = StorageBackend::parse("").unwrap_err();
        assert!(matches!(err, BackendSelectionError::UnknownValue(_)));
    }

    #[test]
    fn resolve_defaults_to_json_when_nothing_set() {
        let sel = BackendSelection {
            config_value: None,
            env_value: None,
            env_name: DEFAULT_BACKEND_ENV_VAR,
        };
        let pinned = PinnedBackend::resolve(&sel).unwrap();
        assert_eq!(pinned.backend(), StorageBackend::Json);
        assert_eq!(pinned.source(), BackendSource::Default);
        assert!(!pinned.is_sqlite());
    }

    #[test]
    fn resolve_explicit_sqlite_from_config() {
        let sel = BackendSelection {
            config_value: Some("sqlite".into()),
            env_value: None,
            env_name: DEFAULT_BACKEND_ENV_VAR,
        };
        let pinned = PinnedBackend::resolve(&sel).unwrap();
        assert_eq!(pinned.backend(), StorageBackend::Sqlite);
        assert_eq!(pinned.source(), BackendSource::Config);
        assert!(pinned.is_sqlite());
    }

    #[test]
    fn env_override_wins_over_config() {
        let sel = BackendSelection {
            config_value: Some("sqlite".into()),
            env_value: Some("json".into()),
            env_name: DEFAULT_BACKEND_ENV_VAR,
        };
        let pinned = PinnedBackend::resolve(&sel).unwrap();
        assert_eq!(pinned.backend(), StorageBackend::Json);
        assert_eq!(pinned.source(), BackendSource::Env);
    }

    #[test]
    fn runtime_switch_is_rejected() {
        let pinned = PinnedBackend::new(StorageBackend::Json, BackendSource::Default);
        let err = pinned.assert_unchanged(StorageBackend::Sqlite).unwrap_err();
        assert!(matches!(err, BackendSelectionError::RuntimeSwitch { .. }));
        // Same value is fine.
        assert!(pinned.assert_unchanged(StorageBackend::Json).is_ok());
    }

    #[test]
    fn diagnostics_omit_paths_and_secrets() {
        let pinned = PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env);
        let diag = BackendDiagnostics::from_pinned(&pinned, Some(4));
        let json = serde_json::to_string(&diag).unwrap();
        assert!(json.contains("\"backend\":\"sqlite\""));
        assert!(json.contains("\"source\":\"env\""));
        assert!(json.contains("\"schema_version\":4"));
        // No Windows drive letters (C:\), no user paths, no temp dirs.
        assert!(!json.contains("C:\\"));
        assert!(!json.contains("/tmp/"));
        assert!(!json.contains("Users"));
    }
}
