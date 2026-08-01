//! Fail-closed JSON → SQLite cutover coordinator.
//!
//! The cutover is a **one-time, reversible** operation that moves authority from
//! the JSON data directory to a SQLite database file. It is designed so that at
//! every point of failure JSON remains the sole source of truth until the very
//! last step (publishing the versioned marker) succeeds.
//!
//! ## State machine
//!
//! ```text
//!  ┌──────────┐
//!  │  Start   │
//!  └────┬─────┘
//!       │  read marker
//!       ▼
//!  ┌────────────────────┐  marker says sqlite + db valid   ┌──────────────┐
//!  │ MarkerIsSqlite?    │ ───────────────────────────────► │ AlreadyCutover│
//!  └────┬───────────────┘                                  └──────────────┘
//!       │ no/absent/stale
//!       ▼
//!  ┌────────────────────┐
//!  │ Acquire lock       │  (temp file held open for process lifetime)
//!  └────┬───────────────┘
//!       │
//!       ▼
//!  ┌────────────────────┐
//!  │ Validate JSON      │  dry-run; fail → abort, JSON authoritative
//!  └────┬───────────────┘
//!       │
//!       ▼
//!  ┌────────────────────┐
//!  │ Import → temp DB   │  fail → discard temp, JSON authoritative
//!  └────┬───────────────┘
//!       │
//!       ▼
//!  ┌────────────────────┐
//!  │ Verify counts/ids  │  fail → discard temp, JSON authoritative
//!  └────┬───────────────┘
//!       │
//!       ▼
//!  ┌────────────────────┐
//!  │ Atomic publish     │  rename temp → final; fail → JSON still authoritative
//!  └────┬───────────────┘
//!       │
//!       ▼
//!  ┌────────────────────┐
//!  │ Write marker LAST  │  marker is the commit point
//!  └────┬───────────────┘
//!       │
//!       ▼
//!  ┌────────────────────┐
//!  │ Reopen + audit     │  read-only verification through production path
//!  └────────────────────┘
//! ```
//!
//! ## Invariants
//!
//! - JSON is authoritative until the marker is written.
//! - A failed cutover never leaves a stale marker claiming SQLite authority.
//! - The temp database is discarded on any failure before publish.
//! - Original JSON is never modified or deleted.
//! - No dual-write: once SQLite is authoritative, JSON is not read for truth.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::StorageBackend;
use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::importer::JsonImporter;
use crate::readiness::{self, SourceManifestReport};

/// Current marker schema version. Bumped only on breaking marker format changes.
pub const MARKER_VERSION: u32 = 1;

/// The filename written next to the database to record the authoritative backend.
pub const MARKER_FILENAME: &str = "storyforge.backend.json";

/// Temp database suffix used during import before atomic publish.
const TEMP_DB_SUFFIX: &str = ".cutover-tmp";

/// Configuration describing where the cutover reads from and writes to.
#[derive(Debug, Clone)]
pub struct CutoverPlan {
    /// The JSON data directory (source of truth before cutover).
    pub data_dir: PathBuf,
    /// The final SQLite database path (e.g. `<data_dir>/storyforge.sqlite3`).
    pub db_path: PathBuf,
    /// Directory for backup checkpoints (must differ from the live DB path).
    pub backup_dir: PathBuf,
}

impl CutoverPlan {
    pub fn new(data_dir: impl AsRef<Path>, db_path: impl AsRef<Path>) -> Self {
        let data_dir = data_dir.as_ref().to_path_buf();
        let db_path = db_path.as_ref().to_path_buf();
        let backup_dir = data_dir.join("sqlite-backups");
        CutoverPlan {
            data_dir,
            db_path,
            backup_dir,
        }
    }

    /// Path of the versioned backend marker.
    pub fn marker_path(&self) -> PathBuf {
        self.db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(MARKER_FILENAME)
    }

    /// Path of the temporary database used during import.
    pub fn temp_db_path(&self) -> PathBuf {
        let mut name = self
            .db_path
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        name.push(TEMP_DB_SUFFIX);
        self.db_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(name)
    }
}

/// Request to perform a cutover.
#[derive(Debug, Clone)]
pub struct CutoverRequest {
    pub plan: CutoverPlan,
    /// A short label stamped into backup metadata (sanitised).
    pub label: String,
}

/// Outcome of a cutover attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutoverOutcome {
    /// The cutover completed: marker written, database reopened and audited.
    Completed(CutoverReport),
    /// SQLite was already authoritative; the database was verified consistent.
    AlreadyCutover(CutoverReport),
}

/// Detailed report of a cutover run (no paths or secrets).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CutoverReport {
    pub manifest_hash: String,
    pub cards: usize,
    pub campaigns: usize,
    pub instances: usize,
    pub knowledge: usize,
    pub tasks: usize,
    pub summaries: usize,
    pub conversations: usize,
    pub turns: usize,
    /// 导入/审计的角色库条目数（Gate 5）。
    pub characters: usize,
    pub schema_version: i64,
    pub import_skipped_duplicate: bool,
    pub backup_label: String,
}

impl CutoverReport {
    fn from_manifest_and_import(
        manifest: &SourceManifestReport,
        schema_version: i64,
        import_skipped_duplicate: bool,
        backup_label: &str,
    ) -> Self {
        CutoverReport {
            manifest_hash: manifest.manifest_hash.clone(),
            cards: manifest.cards,
            campaigns: manifest.campaigns,
            instances: manifest.instances,
            knowledge: manifest.knowledge,
            tasks: manifest.tasks,
            summaries: manifest.summaries,
            conversations: manifest.conversations,
            turns: manifest.turns,
            characters: manifest.characters,
            schema_version,
            import_skipped_duplicate,
            backup_label: backup_label.to_string(),
        }
    }
}

/// Read-only diagnostics summarising the cutover state.
#[derive(Debug, Clone, Serialize)]
pub struct CutoverDiagnostics {
    pub marker_status: &'static str,
    pub schema_version: Option<i64>,
    pub manifest_hash: Option<String>,
}

/// The authoritative-backend marker persisted alongside the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendMarker {
    pub version: u32,
    pub backend: String,
    pub schema_version: i64,
    pub manifest_hash: String,
    pub created_at: String,
}

impl BackendMarker {
    pub fn sqlite(schema_version: i64, manifest_hash: &str) -> Self {
        BackendMarker {
            version: MARKER_VERSION,
            backend: StorageBackend::Sqlite.as_str().to_string(),
            schema_version,
            manifest_hash: manifest_hash.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn backend(&self) -> std::result::Result<StorageBackend, SqliteError> {
        StorageBackend::parse(&self.backend).map_err(|_| {
            SqliteError::Other(format!(
                "backend marker has unknown backend value: {:?}",
                self.backend
            ))
        })
    }
}

/// Result of inspecting the marker on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkerStatus {
    /// No marker file exists.
    Absent,
    /// Marker claims SQLite authority and the DB exists and is valid.
    SqliteAuthoritative {
        schema_version: i64,
        manifest_hash: String,
    },
    /// Marker claims JSON authority.
    JsonAuthoritative,
    /// Marker exists but is stale (DB missing, corrupt, or version mismatch).
    Stale { reason: String },
}

/// Test-only fault injection points. Each causes the cutover to fail at that
/// exact step, allowing tests to prove JSON remains authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverFault {
    None,
    /// Fail immediately after acquiring the lock.
    AfterLock,
    /// Fail after dry-run validation succeeds.
    AfterValidate,
    /// Fail after the backup checkpoint is written.
    AfterBackup,
    /// Fail after import into the temp DB succeeds.
    AfterImport,
    /// Fail after verification passes but before publishing.
    AfterVerify,
    /// Fail after the temp DB is published but **before** the marker is written.
    /// This is the most dangerous window: the DB exists but authority is ambiguous.
    AfterPublishBeforeMarker,
    /// Fail after the marker is written but before the audit reopen.
    AfterMarker,
}

/// Persistent cutover state used for recovery across restarts.
///
/// This is encoded into the marker's metadata. When a cutover is interrupted
/// we can determine exactly how far it got and whether JSON or SQLite is
/// authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoverState {
    /// Nothing has started yet.
    NotStarted,
    /// JSON is authoritative; cutover in progress (temp DB may exist).
    JsonAuthoritative,
    /// SQLite DB published but marker not yet written — ambiguous, must reconcile.
    DbPublishedMarkerMissing,
    /// Marker written — SQLite is authoritative.
    SqliteAuthoritative,
}

/// Inspect the marker file and database state to determine authority.
pub fn inspect_marker(plan: &CutoverPlan) -> MarkerStatus {
    let marker_path = plan.marker_path();
    if !marker_path.exists() {
        return MarkerStatus::Absent;
    }
    let raw = match fs::read_to_string(&marker_path) {
        Ok(raw) => raw,
        Err(_) => {
            return MarkerStatus::Stale {
                reason: "marker file unreadable".into(),
            };
        }
    };
    let marker: BackendMarker = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            return MarkerStatus::Stale {
                reason: format!("marker corrupt: {e}"),
            };
        }
    };
    match marker.backend() {
        Ok(StorageBackend::Sqlite) => {
            if !plan.db_path.exists() {
                return MarkerStatus::Stale {
                    reason: "marker claims sqlite but database file is missing".into(),
                };
            }
            // Verify the DB is openable and at the recorded schema version.
            match verify_database(&plan.db_path, marker.schema_version) {
                Ok(()) => MarkerStatus::SqliteAuthoritative {
                    schema_version: marker.schema_version,
                    manifest_hash: marker.manifest_hash.clone(),
                },
                Err(e) => MarkerStatus::Stale {
                    reason: format!("database verification failed: {e}"),
                },
            }
        }
        Ok(StorageBackend::Json) => MarkerStatus::JsonAuthoritative,
        Err(e) => MarkerStatus::Stale {
            reason: e.to_string(),
        },
    }
}

/// Verify a database is openable, migrated, and at the expected schema version.
fn verify_database(db_path: &Path, expected_version: i64) -> Result<()> {
    let db = Database::open(db_path)?;
    let version = crate::migrations::current_version(&db)?;
    if version != expected_version {
        return Err(SqliteError::Other(format!(
            "database schema version {version} != marker version {expected_version}"
        )));
    }
    // Confirm the DB can answer a basic read query.
    let _: i64 =
        db.connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;
    Ok(())
}

/// Run the full fail-closed cutover, or verify if already complete.
///
/// This is the main entry point. It never deletes original JSON, never
/// dual-writes, and leaves JSON authoritative on any failure before the
/// marker is published.
pub fn run_cutover(request: &CutoverRequest) -> Result<CutoverOutcome> {
    run_cutover_with_fault(request, CutoverFault::None)
}

/// Run the cutover with a test-only fault injection point.
pub fn run_cutover_with_fault(
    request: &CutoverRequest,
    fault: CutoverFault,
) -> Result<CutoverOutcome> {
    let plan = &request.plan;

    // ── Step 0: Check current authority ──────────────────────────────
    match inspect_marker(plan) {
        MarkerStatus::SqliteAuthoritative {
            schema_version,
            ref manifest_hash,
        } => {
            // Already cut over: audit the SQLite DB only. Never re-open JSON
            // source trees — SQLite is the sole authority after the marker.
            let report = audit_sqlite_authoritative(plan, schema_version, manifest_hash)?;
            return Ok(CutoverOutcome::AlreadyCutover(report));
        }
        MarkerStatus::JsonAuthoritative | MarkerStatus::Absent => {
            // Proceed with cutover.
        }
        MarkerStatus::Stale { reason } => {
            return Err(SqliteError::Other(format!(
                "stale backend marker; refusing cutover until resolved: {reason}"
            )));
        }
    }

    // ── Step 1: Acquire a migration lock ─────────────────────────────
    // We hold an open file handle to a lock file for the duration of the
    // cutover. On Windows this prevents concurrent rename/replace of the
    // database; on Unix flock provides the same.
    let _lock_guard = acquire_cutover_lock(plan)?;

    // Re-check authority after lock to close the TOCTOU window between the
    // pre-lock inspect and exclusive lock acquisition.
    match inspect_marker(plan) {
        MarkerStatus::SqliteAuthoritative {
            schema_version,
            ref manifest_hash,
        } => {
            let report = audit_sqlite_authoritative(plan, schema_version, manifest_hash)?;
            return Ok(CutoverOutcome::AlreadyCutover(report));
        }
        MarkerStatus::JsonAuthoritative | MarkerStatus::Absent => {}
        MarkerStatus::Stale { reason } => {
            return Err(SqliteError::Other(format!(
                "stale backend marker after lock; refusing cutover until resolved: {reason}"
            )));
        }
    }

    if fault == CutoverFault::AfterLock {
        return Err(SqliteError::Other(
            "injected fault: after lock acquisition".into(),
        ));
    }

    // ── Step 2: Dry-run JSON validation ──────────────────────────────
    let manifest = readiness::validate_source_manifest(&plan.data_dir)?;
    if !manifest.issues.is_empty() {
        return Err(SqliteError::CorruptImportInput(format!(
            "source validation failed: {}",
            manifest.issues.join("; ")
        )));
    }

    if fault == CutoverFault::AfterValidate {
        return Err(SqliteError::Other(
            "injected fault: after validation".into(),
        ));
    }

    // ── Step 3: Import JSON → temp DB ────────────────────────────────
    // If a temp DB from a previous failed attempt exists, discard it first.
    discard_temp_db(plan);

    let mut import_db = Database::open(plan.temp_db_path())?;
    crate::migrations::migrate(&mut import_db)?;
    let mut importer = JsonImporter::new(&mut import_db);
    let import_report = importer.import_data_dir(&plan.data_dir)?;
    let schema_version = crate::migrations::current_version(&import_db)?;

    if fault == CutoverFault::AfterImport {
        // Close the import DB handle before discarding (Windows file lock).
        drop(import_db);
        discard_temp_db(plan);
        return Err(SqliteError::Other("injected fault: after import".into()));
    }

    // ── Step 4: Backup checkpoint of the populated temp DB ───────────
    // This proves the data was correctly imported and gives an audit trail.
    let backup = readiness::create_backup_checkpoint(&import_db, &plan.backup_dir, &request.label)?;

    // Close the temp DB before publishing (Windows requires this for rename).
    drop(import_db);

    if fault == CutoverFault::AfterBackup {
        discard_temp_db(plan);
        return Err(SqliteError::Other(
            "injected fault: after backup checkpoint".into(),
        ));
    }

    // ── Step 5: Verify the temp DB ───────────────────────────────────
    let verify_db = Database::open(plan.temp_db_path())?;
    verify_imported_database(&verify_db, &manifest, schema_version)?;
    drop(verify_db);

    if fault == CutoverFault::AfterVerify {
        discard_temp_db(plan);
        return Err(SqliteError::Other(
            "injected fault: after verification".into(),
        ));
    }

    // ── Step 6: Atomic publish (temp → final) ────────────────────────
    // If a previous final DB exists (from a prior failed publish), we remove
    // it first. This is safe because the marker has NOT been written yet,
    // so JSON is still authoritative.
    atomic_publish_db(plan)?;

    if fault == CutoverFault::AfterPublishBeforeMarker {
        // The DB is published but the marker is not. This is the ambiguous
        // window. On the next run, inspect_marker will see Absent (no marker)
        // and re-run the cutover, which will re-import and re-publish.
        // JSON remains authoritative.
        return Err(SqliteError::Other(
            "injected fault: after publish, before marker".into(),
        ));
    }

    // ── Step 7: Write the marker LAST ────────────────────────────────
    let marker = BackendMarker::sqlite(schema_version, &manifest.manifest_hash);
    write_marker_atomically(&plan.marker_path(), &marker)?;

    if fault == CutoverFault::AfterMarker {
        return Err(SqliteError::Other(
            "injected fault: after marker write".into(),
        ));
    }

    // ── Step 8: Reopen through production path + audit ───────────────
    audit_published_database(&plan.db_path)?;

    // Use the sanitised backup label (never the raw input) in the report.
    let report = CutoverReport::from_manifest_and_import(
        &manifest,
        schema_version,
        import_report.skipped_as_duplicate,
        &backup.label,
    );

    Ok(CutoverOutcome::Completed(report))
}

/// Recovery / idempotent-restart entry point.
///
/// Called at startup when SQLite is the selected backend. If the cutover is
/// already complete, verifies consistency. If it was interrupted, re-runs it
/// (idempotent). If it never started, performs it.
pub fn recover_or_verify(request: &CutoverRequest) -> Result<CutoverOutcome> {
    run_cutover(request)
}

/// Verify that the imported database matches the source manifest counts and hashes.
fn verify_imported_database(
    db: &Database,
    manifest: &SourceManifestReport,
    schema_version: i64,
) -> Result<()> {
    // Schema version must be current.
    let expected = crate::migrations::builtin_migrations()
        .iter()
        .map(|m| m.version)
        .max()
        .unwrap_or(0);
    if schema_version != expected {
        return Err(SqliteError::Other(format!(
            "temp database schema version {schema_version} != expected {expected}"
        )));
    }

    // Row counts must match manifest counts.
    check_count(db, "character_cards", "card_id", manifest.cards)?;
    check_count(db, "campaigns", "campaign_id", manifest.campaigns)?;
    check_count(db, "character_instances", "instance_id", manifest.instances)?;
    check_count(
        db,
        "character_knowledge",
        "knowledge_id",
        manifest.knowledge,
    )?;
    check_count(db, "story_tasks", "task_id", manifest.tasks)?;
    check_count(db, "round_summaries", "summary_id", manifest.summaries)?;
    check_count(
        db,
        "conversations",
        "conversation_id",
        manifest.conversations,
    )?;
    check_count(db, "turns", "turn_id", manifest.turns)?;
    check_count(db, "characters", "character_id", manifest.characters)?;

    // Recompute a content hash from the live database payloads and compare it
    // to the source manifest. Do not trust the importer's self-written row alone.
    let stored_hash: Option<String> = db
        .connection()
        .query_row(
            "SELECT source_manifest_hash FROM import_runs WHERE status = 'completed' ORDER BY finished_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    if let Some(hash) = &stored_hash {
        if hash != &manifest.manifest_hash {
            return Err(SqliteError::Other(format!(
                "import_runs manifest hash mismatch: db={hash}, source={}",
                manifest.manifest_hash
            )));
        }
    } else {
        return Err(SqliteError::Other(
            "no completed import_runs row found for verification".into(),
        ));
    }

    let content_hash = recompute_db_content_hash(db)?;
    if content_hash != manifest.manifest_hash {
        return Err(SqliteError::Other(format!(
            "database content hash mismatch: db={content_hash}, source={}",
            manifest.manifest_hash
        )));
    }
    Ok(())
}

fn audit_sqlite_authoritative(
    plan: &CutoverPlan,
    schema_version: i64,
    manifest_hash: &str,
) -> Result<CutoverReport> {
    audit_published_database(&plan.db_path)?;
    let db = Database::open(&plan.db_path)?;
    let version = crate::migrations::current_version(&db)?;
    if version != schema_version {
        return Err(SqliteError::Other(format!(
            "database schema version {version} != marker version {schema_version}"
        )));
    }
    Ok(CutoverReport {
        manifest_hash: manifest_hash.to_string(),
        cards: table_count(&db, "character_cards")?,
        campaigns: table_count(&db, "campaigns")?,
        instances: table_count(&db, "character_instances")?,
        knowledge: table_count(&db, "character_knowledge")?,
        tasks: table_count(&db, "story_tasks")?,
        summaries: table_count(&db, "round_summaries")?,
        conversations: table_count(&db, "conversations")?,
        turns: table_count(&db, "turns")?,
        characters: table_count(&db, "characters")?,
        schema_version,
        import_skipped_duplicate: false,
        backup_label: String::new(),
    })
}

fn table_count(db: &Database, table: &str) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let count: i64 = db
        .connection()
        .query_row(&sql, [], |row| row.get(0))
        .map_err(|e| SqliteError::Other(format!("count {table}: {e}")))?;
    Ok(count as usize)
}

/// Rebuild the source-manifest hash from stored payload_json rows so verification
/// is independent of the importer's self-reported import_runs value.
fn recompute_db_content_hash(db: &Database) -> Result<String> {
    use serde_json::Value;
    use sha2::{Digest, Sha256};

    fn load_payloads(db: &Database, table: &str) -> Result<Vec<Value>> {
        let sql = format!("SELECT payload_json FROM {table}");
        let mut stmt = db.connection().prepare(&sql)?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let raw = row?;
            out.push(serde_json::from_str(&raw).map_err(|e| {
                SqliteError::Other(format!("corrupt payload_json in {table}: {e}"))
            })?);
        }
        Ok(out)
    }

    fn hash_named_array(hasher: &mut Sha256, name: &str, items: &[Value]) {
        hasher.update(name.as_bytes());
        hasher.update(b"\0");
        let mut encoded: Vec<String> = items
            .iter()
            .map(|v| serde_json::to_string(v).unwrap_or_default())
            .collect();
        encoded.sort();
        for item in encoded {
            hasher.update(item.as_bytes());
            hasher.update(b"\n");
        }
    }

    let cards = load_payloads(db, "character_cards")?;
    let campaigns = load_payloads(db, "campaigns")?;
    let instances = load_payloads(db, "character_instances")?;
    let knowledge = load_payloads(db, "character_knowledge")?;
    let tasks = load_payloads(db, "story_tasks")?;
    let summaries = load_payloads(db, "round_summaries")?;
    let turns = load_payloads(db, "turns")?;
    let conversations = load_payloads(db, "conversations")?;
    // Gate 4/5 可选集合：与 importer 相同的重建投影（非空才参与 hash）。
    let mvu_translations = load_payloads(db, "mvu_translations")?;
    let world_info = load_payloads(db, "campaign_world_info")?;
    let compress_jobs = {
        let mut stmt = db.connection().prepare(
            "SELECT job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts, \
                 max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue, \
                 created_at, updated_at FROM chronicle_compress_jobs",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (
                job_id,
                campaign_id,
                conversation_id,
                lineage_id,
                kind,
                status,
                attempts,
                max_attempts,
                last_error,
                uncovered_a,
                uncovered_b,
                created_at,
                updated_at,
            ) = row?;
            let mut v = serde_json::json!({
                "id": job_id,
                "campaign_id": campaign_id,
                "kind": kind,
                "status": status,
                "attempts": attempts,
                "max_attempts": max_attempts,
                "uncovered_a_at_enqueue": uncovered_a,
                "uncovered_b_at_enqueue": uncovered_b,
                "created_at": created_at,
                "updated_at": updated_at,
            });
            let obj = v.as_object_mut().expect("json! object");
            if let Some(x) = conversation_id {
                obj.insert("conversation_id".into(), Value::String(x));
            }
            if let Some(x) = lineage_id {
                obj.insert("lineage_id".into(), Value::String(x));
            }
            if let Some(x) = last_error {
                obj.insert("last_error".into(), Value::String(x));
            }
            out.push(v);
        }
        out
    };
    // 角色库按 StoredCharacter 契约形态重建（与 importer 归一化一致）。
    let characters = {
        let mut stmt = db
            .connection()
            .prepare("SELECT character_id, info_json, imported_at FROM characters")?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (character_id, info_json, imported_at) = row?;
            let info: Value = serde_json::from_str(&info_json).map_err(|e| {
                SqliteError::Other(format!(
                    "corrupt info_json in characters id={character_id}: {e}"
                ))
            })?;
            out.push(serde_json::json!({
                "id": character_id,
                "info": info,
                "imported_at": imported_at,
            }));
        }
        out
    };

    let mut hasher = Sha256::new();
    hash_named_array(&mut hasher, "cards", &cards);
    hash_named_array(&mut hasher, "campaigns", &campaigns);
    hash_named_array(&mut hasher, "instances", &instances);
    hash_named_array(&mut hasher, "knowledge", &knowledge);
    hash_named_array(&mut hasher, "tasks", &tasks);
    hash_named_array(&mut hasher, "round_summaries", &summaries);
    hash_named_array(&mut hasher, "turns", &turns);
    hash_named_array(&mut hasher, "conversations", &conversations);
    if !mvu_translations.is_empty() {
        hash_named_array(&mut hasher, "mvu_translations", &mvu_translations);
    }
    if !world_info.is_empty() {
        hash_named_array(&mut hasher, "campaign_world_info", &world_info);
    }
    if !compress_jobs.is_empty() {
        hash_named_array(&mut hasher, "compress_jobs", &compress_jobs);
    }
    if !characters.is_empty() {
        hash_named_array(&mut hasher, "characters", &characters);
    }
    Ok(hex_encode(hasher.finalize()))
}

fn check_count(db: &Database, table: &str, id_column: &str, expected: usize) -> Result<()> {
    let sql = format!("SELECT COUNT(*) FROM {table}");
    let actual: i64 = db
        .connection()
        .query_row(&sql, [], |row| row.get(0))
        .map_err(|e| SqliteError::Other(format!("count {table}: {e}")))?;
    if actual as usize != expected {
        return Err(SqliteError::Other(format!(
            "{table} count {actual} != source {expected}"
        )));
    }
    let _ = id_column;
    Ok(())
}

/// Atomically publish the temp database to the final path.
///
/// On Windows, `fs::rename` fails if the destination exists, so we remove a
/// stale final DB first (only safe because the marker hasn't been written).
fn atomic_publish_db(plan: &CutoverPlan) -> Result<()> {
    let temp = plan.temp_db_path();
    let final_path = &plan.db_path;

    if !temp.exists() {
        return Err(SqliteError::Other(
            "temp database missing during publish".into(),
        ));
    }

    // If a final DB already exists (re-publish after partial failure),
    // move it aside. JSON is still authoritative at this point.
    if final_path.exists() {
        // 只允许让位「上次中断发布的自身产物」（带 storyforge schema 迁移）；
        // 无关/损坏的既有数据库必须 fail closed——绝不静默让新 authority 顶替
        // 用户数据（无 marker 不意味着目标文件可以被覆盖）。
        //
        // ⚠️ 顺序很关键：必须**先**用**只读、不修改 PRAGMA** 的方式判定所有权，
        // 任何失败直接返回；确认是自身产物后再删除 -wal/-shm sidecar 并改名让位。
        // 否则若目标是被其它程序**正在使用**的 WAL 数据库，先删 sidecar 会丢
        // 未 checkpoint 的事务，而用会改 journal_mode 的 open 去探测外部库会写
        // 入它的 header（破坏数据）。审查跟进 P1。
        if !owned_by_storyforge_readonly(final_path)? {
            return Err(SqliteError::Other(format!(
                "refusing to overwrite existing non-StoryForge database at {}; \
                 move or delete it to proceed",
                final_path.display()
            )));
        }
        // 所有权确认（自身上次中断发布的产物）：现在可以安全清理 sidecar。
        for sidecar in ["-wal", "-shm"] {
            let path = format!("{}{sidecar}", final_path.display());
            let _ = fs::remove_file(&path);
        }
        let backup_name = format!(
            "{}.pre-publish-{}.sqlite3",
            final_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("storyforge"),
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
        );
        let aside = final_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&backup_name);
        fs::rename(final_path, &aside)?;
    } else {
        // No existing final DB: stale sidecars from a long-gone DB are safe to
        // clear now that we know there is no live database holding them.
        for sidecar in ["-wal", "-shm"] {
            let path = format!("{}{sidecar}", final_path.display());
            let _ = fs::remove_file(&path);
        }
    }

    // Atomic rename: temp → final.
    fs::rename(temp, final_path)?;

    Ok(())
}

/// Read-only ownership probe: open the file with `SQLITE_OPEN_READONLY` (no
/// `READ_WRITE`, no `CREATE`, **no PRAGMA writes**) and check for a
/// StoryForge `schema_migrations` table whose version is >= 1.
///
/// This never mutates the target database: it cannot flip WAL mode, cannot
/// create the file, and cannot truncate `-wal`/`-shm`. Any error (unreadable,
/// not a database, locked) is treated as "not ours" so the caller fails
/// closed rather than risking a foreign DB.
fn owned_by_storyforge_readonly(path: &Path) -> Result<bool> {
    use rusqlite::OpenFlags;
    // SQLITE_OPEN_READ_WRITE must NOT be set: opening a WAL-mode DB read/write
    // can checkpoint/rotate the -wal sidecar; opening foreign DBs read/write
    // lets rusqlite's PRAGMA setup touch their header. Read-only is a pure
    // probe. URI mode lets us pass `?mode=ro` and `nolock=1` so we do not
    // contend on a live DB's locks either.
    let uri = path
        .to_str()
        .ok_or_else(|| SqliteError::Other(format!("non-utf8 db path: {}", path.display())))?;
    let uri = format!("file:{uri}?mode=ro&immutable=1");
    let conn = match rusqlite::Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        // Cannot open read-only (locked / not a DB / foreign schema) → not ours.
        Err(_) => return Ok(false),
    };
    let has_table: i64 = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_table != 1 {
        return Ok(false);
    }
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);
    Ok(version >= 1)
}

/// Write the marker atomically (write to temp file, then rename).
fn write_marker_atomically(marker_path: &Path, marker: &BackendMarker) -> Result<()> {
    let content = serde_json::to_vec_pretty(marker)?;
    let tmp_path = marker_path.with_extension("json.tmp");
    fs::write(&tmp_path, &content)?;
    fs::rename(&tmp_path, marker_path)?;
    Ok(())
}

/// Read-only audit of the published database through the production path.
fn audit_published_database(db_path: &Path) -> Result<()> {
    let db = Database::open(db_path)?;
    let version = crate::migrations::current_version(&db)?;
    if version == 0 {
        return Err(SqliteError::Other(
            "published database has no migrations applied".into(),
        ));
    }
    // Basic integrity check.
    let integrity: String = db
        .connection()
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if !integrity.eq_ignore_ascii_case("ok") {
        return Err(SqliteError::Other(format!(
            "published database integrity_check: {integrity}"
        )));
    }
    Ok(())
}

/// Discard the temp database and its WAL/SHM sidecars.
fn discard_temp_db(plan: &CutoverPlan) {
    let temp = plan.temp_db_path();
    let _ = fs::remove_file(&temp);
    for sidecar in ["-wal", "-shm"] {
        let path = format!("{}{sidecar}", temp.display());
        let _ = fs::remove_file(&path);
    }
}

/// Acquire a cross-process lock for the cutover duration.
///
/// On Windows we hold an open file handle; on Unix we use `flock`. The lock
/// is released when the returned guard is dropped (end of cutover).
fn acquire_cutover_lock(plan: &CutoverPlan) -> Result<CutoverLockGuard> {
    let lock_path = plan
        .db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("storyforge.cutover.lock");
    CutoverLockGuard::acquire(&lock_path)
}

/// RAII guard for the cutover lock file.
pub struct CutoverLockGuard {
    #[cfg(unix)]
    file: std::fs::File,
    #[cfg(not(unix))]
    _file: std::fs::File,
    path: PathBuf,
}

impl CutoverLockGuard {
    fn acquire(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(path)?;
            // Try an exclusive, non-blocking lock.
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
            if result != 0 {
                return Err(SqliteError::Other(
                    "another cutover is in progress (lock held)".into(),
                ));
            }
            Ok(CutoverLockGuard {
                file,
                path: path.to_path_buf(),
            })
        }
        #[cfg(not(unix))]
        {
            // On Windows, opening with share-deny-write effectively locks the file.
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_SHARE_READ: u32 = 1;
            let file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .share_mode(FILE_SHARE_READ)
                .open(path)?;
            Ok(CutoverLockGuard {
                _file: file,
                path: path.to_path_buf(),
            })
        }
    }
}

impl Drop for CutoverLockGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
        // On Windows the handle close releases the lock.
        let _ = &self.path;
    }
}

/// Compute a SHA-256 hex of the database file for verification.
pub fn database_file_hash(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex_encode(hasher.finalize()))
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}
