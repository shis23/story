//! Application-level storage backend wiring.
//!
//! This module resolves the storage backend at process startup and, when
//! SQLite is explicitly selected, runs the fail-closed cutover. JSON remains
//! the production default — no dual-write, no automatic data deletion.
//!
//! The existing JSON stores (`CampaignStore`, `TurnStore`, `ConversationStore`)
//! are **not** replaced when SQLite is selected. Instead, the cutover produces
//! a SQLite database that is authoritative for future SQLite-native operations,
//! while the JSON stores continue to be used until a full store migration is
//! completed in a separate workstream. This wiring is deliberately additive:
//! it makes the backend selection observable and verified without changing the
//! runtime data path.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use storyforge_infra_sqlite::backend::{
    BackendDiagnostics, BackendSelection, PinnedBackend, StorageBackend,
};
use storyforge_infra_sqlite::cutover::{
    CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker, recover_or_verify,
};
use storyforge_infra_sqlite::migrations::current_version;

/// The pinned backend for this process, resolved once at startup.
static PINNED: OnceLock<PinnedBackend> = OnceLock::new();

/// The canonical SQLite database filename in the app data directory.
pub const SQLITE_DB_FILENAME: &str = "storyforge.sqlite3";

/// Result of resolving the backend at startup.
#[derive(Debug, Clone)]
pub struct BackendResolution {
    pub pinned: PinnedBackend,
    pub db_path: Option<PathBuf>,
    pub diagnostics: BackendDiagnostics,
    pub cutover_performed: bool,
}

impl BackendResolution {
    pub fn is_sqlite(&self) -> bool {
        self.pinned.is_sqlite()
    }
}

/// Resolve and pin the storage backend for this process.
///
/// When SQLite is explicitly selected, this also runs the cutover (or verifies
/// it has already completed). When JSON is selected (the default), no database
/// is opened and no cutover runs.
///
/// This function is safe to call multiple times — the first call pins the
/// backend, and subsequent calls return the cached resolution.
pub fn resolve_backend(data_dir: &Path) -> Result<BackendResolution, BackendWiringError> {
    // If already pinned, return cached resolution.
    if let Some(pinned) = PINNED.get() {
        let schema_version = pinned
            .is_sqlite()
            .then(|| current_version_sqlite(&data_dir.join(SQLITE_DB_FILENAME)));
        return Ok(BackendResolution {
            pinned: pinned.clone(),
            db_path: pinned
                .is_sqlite()
                .then(|| data_dir.join(SQLITE_DB_FILENAME)),
            diagnostics: BackendDiagnostics::from_pinned(pinned, schema_version.flatten()),
            cutover_performed: false,
        });
    }

    let resolution = resolve_backend_inner(data_dir)?;
    let _ = PINNED.set(resolution.pinned.clone());
    Ok(resolution)
}

/// Inner resolution logic without the OnceLock — testable in isolation.
fn resolve_backend_inner(data_dir: &Path) -> Result<BackendResolution, BackendWiringError> {
    let selection = BackendSelection::from_env(None);
    let pinned = PinnedBackend::resolve(&selection)
        .map_err(|e| BackendWiringError::Selection(format!("{e}")))?;

    match pinned.backend() {
        StorageBackend::Json => {
            let diag = BackendDiagnostics::from_pinned(&pinned, None);
            Ok(BackendResolution {
                pinned,
                db_path: None,
                diagnostics: diag,
                cutover_performed: false,
            })
        }
        StorageBackend::Sqlite => {
            let db_path = data_dir.join(SQLITE_DB_FILENAME);
            let plan = CutoverPlan::new(data_dir, &db_path);

            // Run the cutover (or verify if already done).
            let request = CutoverRequest {
                plan: plan.clone(),
                label: "app-startup".to_string(),
            };
            let outcome = recover_or_verify(&request)
                .map_err(|e| BackendWiringError::Cutover(format!("{e}")))?;

            let cutover_performed = matches!(outcome, CutoverOutcome::Completed(_));

            let schema_version = {
                let db = storyforge_infra_sqlite::Database::open(&db_path)
                    .map_err(|e| BackendWiringError::Cutover(format!("reopen: {e}")))?;
                current_version(&db).unwrap_or(0)
            };

            let diag = BackendDiagnostics::from_pinned(&pinned, Some(schema_version));

            Ok(BackendResolution {
                pinned,
                db_path: Some(db_path),
                diagnostics: diag,
                cutover_performed,
            })
        }
    }
}

/// Check the current marker status without performing a cutover.
pub fn check_marker_status(data_dir: &Path) -> MarkerStatus {
    let plan = CutoverPlan::new(data_dir, data_dir.join(SQLITE_DB_FILENAME));
    inspect_marker(&plan)
}

/// The SQLite database path for the given data directory, regardless of
/// whether SQLite is currently selected.
pub fn sqlite_db_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SQLITE_DB_FILENAME)
}

fn current_version_sqlite(db_path: &Path) -> Option<i64> {
    if !db_path.exists() {
        return None;
    }
    let db = storyforge_infra_sqlite::Database::open(db_path).ok()?;
    current_version(&db).ok()
}

/// Errors produced during backend wiring.
#[derive(Debug, thiserror::Error)]
pub enum BackendWiringError {
    #[error("backend selection error: {0}")]
    Selection(String),
    #[error("cutover error: {0}")]
    Cutover(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_json(path: &Path, value: &serde_json::Value) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    }

    fn sample_source(dir: &Path) {
        write_json(
            &dir.join("cards.json"),
            &serde_json::json!([{
                "id": "card-1", "name": "Hero", "source_character_id": null
            }]),
        );
        write_json(
            &dir.join("campaigns.json"),
            &serde_json::json!([{
                "id": "camp-1", "card_id": "card-1", "name": "Main",
                "created_at": "2026-07-13T00:00:00Z", "revision": 0,
                "chronicle_revision": 0, "conversation_id": "conv-1", "lineage_id": "lin-1"
            }]),
        );
        write_json(
            &dir.join("conversations").join("conv-1.json"),
            &serde_json::json!({
                "id": "conv-1", "campaign_id": "camp-1", "character_id": null,
                "created_at": "2026-07-13T00:00:00Z", "updated_at": "2026-07-13T00:00:00Z",
                "nodes": []
            }),
        );
        write_json(&dir.join("instances.json"), &serde_json::json!([]));
        write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
        write_json(&dir.join("tasks.json"), &serde_json::json!([]));
        write_json(
            &dir.join("round_summaries.json"),
            &serde_json::json!([{
                "id": "sum-a1", "campaign_id": "camp-1", "conversation_id": "conv-1",
                "turn": 1, "content": "leaf", "created_at": "2026-07-13T00:00:00Z",
                "level": 0, "lineage_id": "lin-1", "code": "A0001"
            }]),
        );
        write_json(&dir.join("turns.json"), &serde_json::json!([]));
    }

    #[test]
    fn default_resolution_is_json_without_database() {
        let dir = TempDir::new().unwrap();
        // Ensure env var is not set.
        // SAFETY: test-only; no concurrent threads depend on this env var.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(!resolution.is_sqlite());
        assert!(resolution.db_path.is_none());
        assert!(!resolution.cutover_performed);
        // No SQLite file created.
        assert!(!dir.path().join(SQLITE_DB_FILENAME).exists());
    }

    #[test]
    fn sqlite_resolution_runs_cutover() {
        let dir = TempDir::new().unwrap();
        sample_source(dir.path());
        // SAFETY: test-only.
        unsafe {
            std::env::set_var("STORYFORGE_STORAGE_BACKEND", "sqlite");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert!(resolution.is_sqlite());
        assert!(resolution.cutover_performed);
        assert!(dir.path().join(SQLITE_DB_FILENAME).exists());

        // Cleanup env for other tests.
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
    }

    #[test]
    fn marker_status_absent_when_no_marker() {
        let dir = TempDir::new().unwrap();
        let status = check_marker_status(dir.path());
        assert_eq!(status, MarkerStatus::Absent);
    }

    #[test]
    fn no_dual_write_json_stores_not_affected_by_resolution() {
        // When JSON is selected, resolve_backend must not open any database.
        let dir = TempDir::new().unwrap();
        // SAFETY: test-only.
        unsafe {
            std::env::remove_var("STORYFORGE_STORAGE_BACKEND");
        }
        let resolution = resolve_backend_inner(dir.path()).unwrap();
        assert_eq!(resolution.pinned.backend(), StorageBackend::Json);
        // No marker, no database.
        assert!(!dir.path().join("storyforge.backend.json").exists());
        assert!(!dir.path().join(SQLITE_DB_FILENAME).exists());
    }
}
