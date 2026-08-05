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
use crate::connection::{Database, STORYFORGE_APPLICATION_ID};
use crate::error::{Result, SqliteError};
use crate::importer::JsonImporter;
use crate::lease::AuthorityLeaseGuard;
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
///
/// Optional fields use `#[serde(default)]` so older markers still parse.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendMarker {
    pub version: u32,
    pub backend: String,
    pub schema_version: i64,
    pub manifest_hash: String,
    pub created_at: String,
    /// Authority identity bound to the published database (cutover-time random id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_id: Option<String>,
    /// One-time cutover nonce, co-stored in the DB `authority_binding` row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cutover_nonce: Option<String>,
}

impl BackendMarker {
    pub fn sqlite(
        schema_version: i64,
        manifest_hash: &str,
        authority_id: &str,
        cutover_nonce: &str,
    ) -> Self {
        BackendMarker {
            version: MARKER_VERSION,
            backend: StorageBackend::Sqlite.as_str().to_string(),
            schema_version,
            manifest_hash: manifest_hash.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            authority_id: Some(authority_id.to_string()),
            cutover_nonce: Some(cutover_nonce.to_string()),
        }
    }

    pub fn json_authoritative() -> Self {
        BackendMarker {
            version: MARKER_VERSION,
            backend: StorageBackend::Json.as_str().to_string(),
            schema_version: 0,
            manifest_hash: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
            authority_id: None,
            cutover_nonce: None,
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
        /// Present when the marker carries an authority binding (Gate 5+).
        authority_id: Option<String>,
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
    /// Fail after the final published-DB audit but **before** the marker is written.
    AfterAudit,
    /// Fail after the marker is written (no further fallible steps remain;
    /// retained for fault-matrix compatibility — report construction only).
    AfterMarker,
}

// (L-2: 此前这里定义的 `CutoverState` 枚举及其文档注释从未被使用——状态判定
// 实际由下方 `inspect_marker` 返回的 `MarkerStatus` 承担。已删除该死枚举、其
// 文档注释与 lib.rs 重导出。)

/// Inspect the marker file and database state to determine authority.
pub fn inspect_marker(plan: &CutoverPlan) -> MarkerStatus {
    let marker_path = plan.marker_path();
    if !marker_path.exists() {
        // 三审3：marker 缺失不能一律当作「空白新用户」。若 db_path 存在且是一个
        // 带 authority_binding 的 StoryForge DB（中断的 cutover 残留），必须判
        // Stale（ambiguous）让操作者显式处理，而非被当作 blank 并可能在后续
        // cutover 中覆盖。真正的空白新用户 = 无 marker 且无 StoryForge DB。
        if orphan_storyforge_db_exists(&plan.db_path) {
            return MarkerStatus::Stale {
                reason: "marker absent but a StoryForge database with authority binding exists \
                         (interrupted cutover); resolve manually or re-run cutover"
                    .into(),
            };
        }
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
    // Unknown / too-new marker version is always stale — refuse regardless of env.
    if marker.version > MARKER_VERSION {
        return MarkerStatus::Stale {
            reason: format!(
                "marker version {} is newer than supported {}",
                marker.version, MARKER_VERSION
            ),
        };
    }
    if marker.version == 0 {
        return MarkerStatus::Stale {
            reason: "marker version 0 is invalid".into(),
        };
    }
    match marker.backend() {
        Ok(StorageBackend::Sqlite) => {
            if !plan.db_path.exists() {
                return MarkerStatus::Stale {
                    reason: "marker claims sqlite but database file is missing".into(),
                };
            }
            // Verify schema + authority binding (marker ↔ DB).
            match verify_database_with_marker(&plan.db_path, &marker) {
                Ok(()) => MarkerStatus::SqliteAuthoritative {
                    schema_version: marker.schema_version,
                    manifest_hash: marker.manifest_hash.clone(),
                    authority_id: marker.authority_id.clone(),
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

/// 全新用户检测：`data_dir` 里是否存在任何 legacy JSON 布局文件。
///
/// 覆盖 readiness 的七个核心文件（缺失即 fail-closed）、`conversations/`
/// 目录、可选集合文件（mvu_translations / compress_jobs / characters）与旧
/// 版 active 指针。任意一个存在 → 必须走正常 cutover（fail-closed 源校验）；
/// 全部不存在 → 真正的空白新用户，允许直接初始化空 SQLite 权威。
fn legacy_json_layout_present(data_dir: &Path) -> bool {
    const CORE_FILES: [&str; 7] = [
        "cards.json",
        "campaigns.json",
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
    ];
    const OPTIONAL_FILES: [&str; 4] = [
        "mvu_translations.json",
        "compress_jobs.json",
        "characters.json",
        "active_campaign.json",
    ];
    CORE_FILES.iter().any(|name| data_dir.join(name).exists())
        || data_dir.join("conversations").exists()
        || data_dir.join("campaign_world_info").exists()
        || OPTIONAL_FILES
            .iter()
            .any(|name| data_dir.join(name).exists())
}

/// 计算「已迁移空库」的确定性内容 hash（Gate 7 fresh start / 孤儿身份判定）。
///
/// 与 `recompute_db_content_hash` 同投影：空库的 hash 只取决于 schema 迁移
/// 序列，同一版本代码下恒定。因此 fresh cutover 与中断恢复能派生同一身份。
fn empty_db_content_hash() -> Result<String> {
    let mut db = Database::open_in_memory()?;
    crate::migrations::migrate(&mut db)?;
    recompute_db_content_hash(&db)
}

/// Gate 7：空白新用户初始化——不导入任何 JSON，直接建立空 SQLite 权威。
///
/// 与正式 cutover 共用同一套机制：temp DB → 迁移 → 空 manifest hash →
/// 身份绑定 + completed import_runs → 备份检查点 → 校验 → 原子发布 → 审计 →
/// marker 最后写。任一步失败都保持「无 marker、JSON（若存在）仍权威」；
/// 中断残留可恢复续跑（`orphan_belongs_to_this_cutover` 用同一空 hash 身份
/// 识别自身产物）。不存在「静默空库顶替用户数据」的路径——本分支只在确认
/// 目录里没有任何 legacy 布局文件时进入。
fn run_fresh_start_cutover(
    request: &CutoverRequest,
    fault: CutoverFault,
) -> Result<CutoverOutcome> {
    let plan = &request.plan;

    // Step F1: 丢弃可能的中断残留 temp DB。
    discard_temp_db(plan);

    // Step F2: 全新 temp DB（空 schema）+ 身份绑定 + completed import_runs。
    let mut fresh_db = Database::open(plan.temp_db_path())?;
    crate::migrations::migrate(&mut fresh_db)?;
    let schema_version = crate::migrations::current_version(&fresh_db)?;

    // 空 manifest hash：由已迁移的空库确定性重建（与 verify 同投影）。
    let empty_hash = recompute_db_content_hash(&fresh_db)?;
    let (authority_id, cutover_nonce) = new_authority_identity(&plan.data_dir, &empty_hash);

    // 与 importer 同形态的 completed import_runs 行（空源、空 hash）——
    // `verify_imported_database` 与 `validate_marker_db_binding` 都依赖它。
    let run_id = new_fresh_run_id();
    let now = chrono::Utc::now().to_rfc3339();
    fresh_db.connection().execute(
        "INSERT INTO import_runs \
         (run_id, source_root, source_manifest_hash, status, started_at, finished_at, error) \
         VALUES (?1, ?2, ?3, 'completed', ?4, ?5, NULL)",
        rusqlite::params![
            run_id,
            plan.data_dir.display().to_string(),
            empty_hash,
            now,
            now,
        ],
    )?;
    write_authority_binding(&fresh_db, &authority_id, &cutover_nonce)?;

    if fault == CutoverFault::AfterImport {
        drop(fresh_db);
        discard_temp_db(plan);
        return Err(SqliteError::Other(
            "injected fault: after fresh import".into(),
        ));
    }

    // Step F3: 备份检查点（空库的审计轨迹，与正式 cutover 同一目录/格式）。
    let backup = readiness::create_backup_checkpoint(&fresh_db, &plan.backup_dir, &request.label)?;
    drop(fresh_db);

    if fault == CutoverFault::AfterBackup {
        discard_temp_db(plan);
        return Err(SqliteError::Other(
            "injected fault: after backup checkpoint".into(),
        ));
    }

    // Step F4: 校验空库（schema 版本 + 全零计数 + import_runs/hash 一致）。
    let verify_db = Database::open(plan.temp_db_path())?;
    let empty_manifest = SourceManifestReport {
        manifest_hash: empty_hash.clone(),
        cards: 0,
        campaigns: 0,
        instances: 0,
        knowledge: 0,
        tasks: 0,
        summaries: 0,
        conversations: 0,
        turns: 0,
        characters: 0,
        issues: Vec::new(),
    };
    verify_imported_database(&verify_db, &empty_manifest, schema_version)?;
    drop(verify_db);

    if fault == CutoverFault::AfterVerify {
        discard_temp_db(plan);
        return Err(SqliteError::Other(
            "injected fault: after verification".into(),
        ));
    }

    // Step F5: 原子发布 + 审计 + marker（提交点）。
    atomic_publish_db(plan, &empty_hash, &authority_id)?;

    if fault == CutoverFault::AfterPublishBeforeMarker {
        return Err(SqliteError::Other(
            "injected fault: after publish, before marker".into(),
        ));
    }

    audit_published_database(&plan.db_path)?;

    if fault == CutoverFault::AfterAudit {
        return Err(SqliteError::Other(
            "injected fault: after audit, before marker".into(),
        ));
    }

    let marker = BackendMarker::sqlite(schema_version, &empty_hash, &authority_id, &cutover_nonce);
    write_marker_atomically(&plan.marker_path(), &marker)?;

    if fault == CutoverFault::AfterMarker {
        return Err(SqliteError::Other(
            "injected fault: after marker write".into(),
        ));
    }

    let report = CutoverReport::from_manifest_and_import(
        &empty_manifest,
        schema_version,
        false,
        &backup.label,
    );
    Ok(CutoverOutcome::Completed(report))
}

/// Fresh-start import_runs run id（与 importer 同风格，不引入 uuid 依赖）。
fn new_fresh_run_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("fresh-{nanos:x}")
}

///
/// 重新计算源 JSON 的 manifest hash（与正式 cutover 同一 readiness 路径），派生
/// authority_id（与 `new_authority_identity` 同一确定性公式），再用只读探测查
/// 孤儿 DB 的 authority_binding 是否匹配。
///
/// - 匹配 → 这是本次 cutover 的中断残留，可恢复（`run_cutover` 继续，发布时
///   `owned_by_storyforge_readonly` 让旧 DB 让位）。
/// - 不匹配 / 打不开 → 非本次 cutover 的孤儿 DB（不同 data_dir 或不同源派生），
///   必须 fail closed（绝不静默覆盖）。
/// 三审3：判断 marker 缺失时残留的孤儿 DB 是否**属于本次 cutover**。
///
/// 重新计算源 JSON 的 manifest hash（与正式 cutover 同一 readiness 路径），派生
/// authority_id（与 `new_authority_identity` 同一确定性公式），再用只读探测查
/// 孤儿 DB 的 authority_binding 是否匹配。
///
/// - 匹配 → 这是本次 cutover 的中断残留，可恢复（`run_cutover` 继续，发布时
///   `owned_by_storyforge_readonly` 让旧 DB 让位）。
/// - 不匹配 / 打不开 → 非本次 cutover 的孤儿 DB（不同 data_dir 或不同源派生），
///   必须 fail closed（绝不静默覆盖）。
///
/// Gate 7（默认切换）：全新用户目录（无任何 legacy 布局文件）改用「已迁移空库
/// 的确定性 hash」派生身份，使中断的 fresh cutover 同样可恢复续跑。
fn orphan_belongs_to_this_cutover(plan: &CutoverPlan) -> bool {
    if !legacy_json_layout_present(&plan.data_dir) {
        // 源 manifest 无法对空目录计算（核心文件必需），改用空库 hash。
        let Ok(empty_hash) = empty_db_content_hash() else {
            return false;
        };
        let (authority_id, _nonce) = new_authority_identity(&plan.data_dir, &empty_hash);
        return orphan_db_matches_authority(&plan.db_path, &authority_id);
    }
    // 源 manifest 必须可计算（否则连 cutover 都进不去，交由后续步骤报错）。
    let Ok(manifest) = readiness::validate_source_manifest(&plan.data_dir) else {
        return false;
    };
    let (authority_id, _nonce) = new_authority_identity(&plan.data_dir, &manifest.manifest_hash);
    orphan_db_matches_authority(&plan.db_path, &authority_id)
}

/// 只读探测：孤儿 DB 的 authority_binding.authority_id 是否等于给定身份。
fn orphan_db_matches_authority(path: &Path, expected_authority_id: &str) -> bool {
    if !path.exists() {
        return false;
    }
    use rusqlite::OpenFlags;
    let Some(uri) = path.to_str() else {
        return false;
    };
    let uri = format!("file:{uri}?mode=ro&immutable=1");
    let conn = match rusqlite::Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let bound: Option<String> = conn
        .query_row(
            "SELECT authority_id FROM authority_binding WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .ok();
    bound.as_deref() == Some(expected_authority_id)
}

/// Verify a database is openable, migrated, and bound to the marker identity.
fn verify_database_with_marker(db_path: &Path, marker: &BackendMarker) -> Result<()> {
    let db = Database::open(db_path)?;
    let version = crate::migrations::current_version(&db)?;
    if version != marker.schema_version {
        return Err(SqliteError::Other(format!(
            "database schema version {version} != marker version {}",
            marker.schema_version
        )));
    }
    // Confirm the DB can answer a basic read query.
    let _: i64 =
        db.connection()
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get(0)
            })?;

    // Authority identity: when the marker carries a binding, the DB must match.
    // Markers without authority_id (legacy) still require a completed import_runs
    // row whose source hash matches the marker's manifest_hash.
    validate_marker_db_binding(&db, marker)?;
    Ok(())
}

/// Ensure the marker is bound to the actual database (authority_id + manifest).
fn validate_marker_db_binding(db: &Database, marker: &BackendMarker) -> Result<()> {
    // Completed import_runs row whose source hash matches the marker.
    let stored: Option<(Option<String>, Option<String>, String)> = db
        .connection()
        .query_row(
            "SELECT authority_id, cutover_nonce, source_manifest_hash              FROM import_runs WHERE status = 'completed'              ORDER BY finished_at DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok();
    let Some((db_authority, db_nonce, db_hash)) = stored else {
        return Err(SqliteError::Other(
            "marker/DB binding rejected: no completed import_runs row".into(),
        ));
    };
    if db_hash != marker.manifest_hash {
        return Err(SqliteError::Other(format!(
            "marker/DB binding rejected: manifest_hash mismatch marker={} db={db_hash}",
            marker.manifest_hash
        )));
    }

    if let Some(ref marker_aid) = marker.authority_id {
        // Prefer authority_binding table; fall back to import_runs columns.
        let binding = read_authority_binding(db)?;
        let (bound_aid, bound_nonce) = match binding {
            Some((a, n)) => (a, n),
            None => (
                db_authority.unwrap_or_default(),
                db_nonce.unwrap_or_default(),
            ),
        };
        if bound_aid.is_empty() || bound_aid != *marker_aid {
            return Err(SqliteError::Other(format!(
                "marker/DB binding rejected: authority_id mismatch marker={marker_aid} db={bound_aid}"
            )));
        }
        if let Some(ref marker_nonce) = marker.cutover_nonce
            && bound_nonce != *marker_nonce
        {
            return Err(SqliteError::Other(
                "marker/DB binding rejected: cutover_nonce mismatch".into(),
            ));
        }
    }
    Ok(())
}

fn read_authority_binding(db: &Database) -> Result<Option<(String, String)>> {
    let has_table: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='authority_binding'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_table == 0 {
        return Ok(None);
    }
    let row = db
        .connection()
        .query_row(
            "SELECT authority_id, cutover_nonce FROM authority_binding WHERE id = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok();
    Ok(row)
}

/// Generate the authority identity for a cutover attempt.
///
/// `authority_id` 由 (规范化 data_dir, source manifest hash) 确定性派生：同一
/// 数据目录 + 同一源的重试（recover / 中断续跑）得到同一身份，发布探测可以安全
/// 识别「上次中断发布的自身产物」；不同目录或不同源的库身份不同，绝不误放行。
/// `cutover_nonce` 每次尝试随机生成，作为一次性绑定凭证写入 DB + marker。
fn new_authority_identity(data_dir: &Path, source_manifest_hash: &str) -> (String, String) {
    use sha2::{Digest, Sha256};
    let canonical = fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update(source_manifest_hash.as_bytes());
    let digest = hex_encode(hasher.finalize());
    let authority_id = format!("aid-{}", &digest[..16]);

    // 一次性 nonce：时间 + 进程 + 计数器混合，保证同身份的两attempt可区分。
    let nonce = {
        let mut hasher = Sha256::new();
        hasher.update(authority_id.as_bytes());
        hasher.update(b"|");
        hasher.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos().to_le_bytes())
                .unwrap_or([0; 16]),
        );
        hasher.update(std::process::id().to_le_bytes());
        hex_encode(hasher.finalize())
    };
    let cutover_nonce = nonce.chars().take(32).collect();
    (authority_id, cutover_nonce)
}

/// Persist the authority binding into the published (or temp) database.
fn write_authority_binding(db: &Database, authority_id: &str, cutover_nonce: &str) -> Result<()> {
    db.connection().execute(
        "INSERT INTO authority_binding (id, authority_id, cutover_nonce, created_at) \
         VALUES (1, ?1, ?2, ?3) \
         ON CONFLICT(id) DO UPDATE SET \
            authority_id = excluded.authority_id, \
            cutover_nonce = excluded.cutover_nonce, \
            created_at = excluded.created_at",
        rusqlite::params![authority_id, cutover_nonce, chrono::Utc::now().to_rfc3339()],
    )?;
    // Also stamp the latest completed import_runs row for defense in depth.
    // H-4：cutover「提交点」路径。authority_binding 已写入但 import_runs UPDATE
    // 失败时不能静默吞没——validate_marker_db_binding（cutover.rs:465）的回退分支
    // 会读 import_runs.authority_id，UPDATE 失败会导致身份判定错误。cutover 刚
    // import 完必有 completed 行，传播错误（影响 0 行本身就是不一致信号）。
    db.connection().execute(
        "UPDATE import_runs SET authority_id = ?1, cutover_nonce = ?2 \
         WHERE status = 'completed' AND run_id = ( \
            SELECT run_id FROM import_runs WHERE status = 'completed' \
            ORDER BY finished_at DESC LIMIT 1 \
         )",
        rusqlite::params![authority_id, cutover_nonce],
    )?;
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
            ..
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
            // 三审3：孤儿 DB（中断的 cutover 残留）若属于本次 cutover（同身份），
            // 允许恢复继续；否则 fail closed（绝不静默覆盖无关/未知 DB）。
            if orphan_belongs_to_this_cutover(plan) {
                // 本次 cutover 的中断残留 → 继续（发布时让位旧 DB）。
            } else {
                return Err(SqliteError::Other(format!(
                    "stale backend marker; refusing cutover until resolved: {reason}"
                )));
            }
        }
    }

    // ── Step 1: Acquire a migration lock ─────────────────────────────
    // We hold an open file handle to a lock file for the duration of the
    // cutover. On Windows this prevents concurrent rename/replace of the
    // database; on Unix flock provides the same.
    let _lock_guard = acquire_cutover_lock(plan)?;

    // 权威写者租约：EXCLUSIVE 覆盖整个 cutover（跨进程互斥；同进程可重入，
    // 与进程启动时持有的 SHARED 租约兼容）。普通写者进程（JSON/SQLite）持有
    // SHARED 时，并发 cutover 必须 fail closed（审查一.3）。
    let _lease_guard = AuthorityLeaseGuard::acquire_exclusive_in(&plan.data_dir)?;

    // Re-check authority after lock to close the TOCTOU window between the
    // pre-lock inspect and exclusive lock acquisition.
    match inspect_marker(plan) {
        MarkerStatus::SqliteAuthoritative {
            schema_version,
            ref manifest_hash,
            ..
        } => {
            let report = audit_sqlite_authoritative(plan, schema_version, manifest_hash)?;
            return Ok(CutoverOutcome::AlreadyCutover(report));
        }
        MarkerStatus::JsonAuthoritative | MarkerStatus::Absent => {}
        MarkerStatus::Stale { reason } => {
            // 三审3：加锁后再查——孤儿 DB 属于本次 cutover 才允许继续。
            if orphan_belongs_to_this_cutover(plan) {
                // 继续。
            } else {
                return Err(SqliteError::Other(format!(
                    "stale backend marker after lock; refusing cutover until resolved: {reason}"
                )));
            }
        }
    }

    if fault == CutoverFault::AfterLock {
        return Err(SqliteError::Other(
            "injected fault: after lock acquisition".into(),
        ));
    }

    // ── Step 1.5: 空白新用户 → 直接初始化空 SQLite 权威（Gate 7 默认切换）──
    // 无 marker 且目录里没有任何 legacy JSON 布局文件 = 全新用户：不存在可
    // 导入的旧数据，直接建立空 SQLite 权威（空库 + 身份绑定 + marker），
    // 跳过 JSON 导入。部分布局（缺核心文件）仍走下方 fail-closed 校验——
    // 绝不把「有数据但坏了」误判成「新用户」而静默建空库。
    if !legacy_json_layout_present(&plan.data_dir) {
        return run_fresh_start_cutover(request, fault);
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

    // ── Step 2.5: 生成权威身份 ────────────────────────────────────────
    // 身份与 (data_dir, source hash) 确定性绑定 + 随机 nonce：写入 temp DB
    // （authority_binding + import_runs 列）与 marker，把 marker 绑定到实际
    // 发布的数据库（审查 P1：marker 不得授权无关 DB，发布探测也靠它识别自身
    // 产物）。
    let (authority_id, cutover_nonce) =
        new_authority_identity(&plan.data_dir, &manifest.manifest_hash);

    // ── Step 3: Import JSON → temp DB ────────────────────────────────
    // If a temp DB from a previous failed attempt exists, discard it first.
    discard_temp_db(plan);

    let mut import_db = Database::open(plan.temp_db_path())?;
    crate::migrations::migrate(&mut import_db)?;
    let mut importer = JsonImporter::new(&mut import_db);
    let import_report = importer.import_data_dir(&plan.data_dir)?;
    // 把身份绑定写入 temp DB：发布后即使 marker 未写，最终 DB 也自证归属。
    write_authority_binding(&import_db, &authority_id, &cutover_nonce)?;
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
    atomic_publish_db(plan, &manifest.manifest_hash, &authority_id)?;

    if fault == CutoverFault::AfterPublishBeforeMarker {
        // The DB is published but the marker is not. This is the ambiguous
        // window. On the next run, inspect_marker will see Absent (no marker)
        // and re-run the cutover, which will re-import and re-publish.
        // JSON remains authoritative.
        return Err(SqliteError::Other(
            "injected fault: after publish, before marker".into(),
        ));
    }

    // ── Step 7: 审计已发布 DB（必须发生在 marker 之前）──────────────
    // 审查一.5：marker 是提交点；marker 写入之后不允许再有任何可失败步骤。
    audit_published_database(&plan.db_path)?;

    if fault == CutoverFault::AfterAudit {
        // 审计通过后、marker 写入前失败：DB 已发布但 JSON 仍权威。
        return Err(SqliteError::Other(
            "injected fault: after audit, before marker".into(),
        ));
    }

    // ── Step 8: Write the marker LAST ────────────────────────────────
    let marker = BackendMarker::sqlite(
        schema_version,
        &manifest.manifest_hash,
        &authority_id,
        &cutover_nonce,
    );
    write_marker_atomically(&plan.marker_path(), &marker)?;

    if fault == CutoverFault::AfterMarker {
        return Err(SqliteError::Other(
            "injected fault: after marker write".into(),
        ));
    }

    // 至此无任何可失败步骤：只做纯内存报告构造。
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
pub(crate) fn recompute_db_content_hash(db: &Database) -> Result<String> {
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

    /// 读 (campaign_id, payload_json) 对，投影与源侧 manifest 完全一致：
    /// 每行参与 hash 前绑定 campaign_id，保证 hash 区分不同 campaign。
    fn load_world_info_pairs(db: &Database) -> Result<Vec<(String, Value)>> {
        let mut stmt = db
            .connection()
            .prepare("SELECT campaign_id, payload_json FROM campaign_world_info")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (campaign_id, raw) = row?;
            let value: Value = serde_json::from_str(&raw).map_err(|e| {
                SqliteError::Other(format!(
                    "corrupt payload_json in campaign_world_info id={campaign_id}: {e}"
                ))
            })?;
            out.push((campaign_id, value));
        }
        Ok(out)
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
    // 世界书必须绑定 campaign_id：两局内容相同但归属不同 campaign 的条目
    // hash 必须不同（与 readiness/importer 的投影完全一致）。
    let world_info = load_world_info_pairs(db)?;
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
        crate::readiness::hash_world_info_pairs(&mut hasher, &world_info);
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
    // L-1：用 COUNT(<id_column>) 而非 COUNT(*)，让 id_column 真正参与校验——
    // 若列名拼错或为 NULL 会被 SQLite 报错（COUNT(不存在的列) → SQL 错误），
    // 不再是 `let _ = id_column` 给人的「按 id 校验」错觉。这些表的 id_column
    // 是 NOT NULL 主键，所以 COUNT(id) == COUNT(*)，数值不变。
    let sql = format!("SELECT COUNT({id_column}) FROM {table}");
    let actual: i64 = db
        .connection()
        .query_row(&sql, [], |row| row.get(0))
        .map_err(|e| SqliteError::Other(format!("count {table}.{id_column}: {e}")))?;
    if actual as usize != expected {
        return Err(SqliteError::Other(format!(
            "{table} count {actual} != source {expected}"
        )));
    }
    Ok(())
}

/// Atomically publish the temp database to the final path.
///
/// On Windows, `fs::rename` fails if the destination exists, so we remove a
/// stale final DB first (only safe because the marker hasn't been written).
fn atomic_publish_db(
    plan: &CutoverPlan,
    expected_manifest_hash: &str,
    expected_authority_id: &str,
) -> Result<()> {
    let temp = plan.temp_db_path();
    let final_path = &plan.db_path;

    // (M-1：删除调试残留 `[BI2]` eprintln——它在生产路径把绝对 DB 路径打到 stderr，
    // 泄漏用户名/安装目录。服务器端 tracing 已覆盖诊断需求。)
    if !temp.exists() {
        return Err(SqliteError::Other(
            "temp database missing during publish".into(),
        ));
    }

    // If a final DB already exists (re-publish after partial failure),
    // move it aside. JSON is still authoritative at this point.
    if final_path.exists() {
        // 只允许让位「上次中断发布的自身产物」（带 storyforge schema 迁移 +
        // application_id + 匹配源 hash 的 completed import + 匹配身份的绑定）；
        // 无关/损坏的既有数据库必须 fail closed——绝不静默让新 authority 顶替
        // 用户数据（无 marker 不意味着目标文件可以被覆盖）。
        //
        // ⚠️ 顺序很关键：必须**先**用**只读、不修改 PRAGMA** 的方式判定所有权，
        // 任何失败直接返回；确认是自身产物后再删除 -wal/-shm sidecar 并改名让位。
        // 否则若目标是被其它程序**正在使用**的 WAL 数据库，先删 sidecar 会丢
        // 未 checkpoint 的事务，而用会改 journal_mode 的 open 去探测外部库会写
        // 入它的 header（破坏数据）。审查跟进 P1。
        if !owned_by_storyforge_readonly(final_path, expected_manifest_hash, expected_authority_id)?
        {
            return Err(SqliteError::Other(format!(
                "refusing to overwrite existing non-StoryForge database at {}; \
                 move or delete it to proceed",
                final_path.display()
            )));
        }
        // 所有权确认（自身上次中断发布的产物）：现在可以安全清理 sidecar。
        // 审查一.5：清理失败必须传播（NotFound 除外），绝不静默吞掉。
        for sidecar in ["-wal", "-shm"] {
            let path = format!("{}{sidecar}", final_path.display());
            remove_sidecar_ignoring_missing(&path)?;
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
            remove_sidecar_ignoring_missing(&path)?;
        }
    }

    // 三审7：原子 rename temp → final（删除 .probe 来回 rename 探测——无意义的 IO
    // 噪音，且在并发下有竞态；目标已确认不存在或已让位，直接 rename）。
    fs::rename(temp, final_path)?;

    // 审查一.5：rename 后 fsync 已发布 DB 文件（持久化发布结果）。
    fsync_file(final_path)?;

    Ok(())
}

/// Remove a sidecar file; `NotFound` is treated as success (nothing to clean),
/// any other error must propagate — a silently swallowed cleanup failure could
/// leave a half-removed sidecar or hide a permission problem.
fn remove_sidecar_ignoring_missing(path: &str) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(SqliteError::Other(format!(
            "failed to remove database sidecar {path}: {e}"
        ))),
    }
}

/// fsync a file。注意：Windows 上 `FlushFileBuffers` 要求句柄有**写**访问
/// （只读句柄会返回 ERROR_ACCESS_DENIED），所以这里用 write 方式打开。
fn fsync_file(path: &Path) -> Result<()> {
    let file = fs::OpenOptions::new().write(true).open(path)?;
    file.sync_all()?;
    Ok(())
}

/// fsync the parent directory so the rename itself is durable. Windows cannot
/// open directories for fsync, so this is unix-only.
#[cfg(unix)]
fn fsync_parent_dir(path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let dir = fs::File::open(parent)?;
    dir.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn fsync_parent_dir(_path: &Path) -> Result<()> {
    Ok(())
}

/// 三审3：只读孤儿 DB 探测——判断 `path` 是否是一个**任何** StoryForge DB
/// （application_id 魔数 + schema_migrations≥1 + authority_binding 行存在）。
///
/// 与 `owned_by_storyforge_readonly` 的区别：本函数**不**要求身份/hash 匹配本次
/// cutover——它只回答「这是不是一个 StoryForge 权威库残留」。用于 marker 缺失时
/// 区分「空白新用户」（无 DB）与「中断的 cutover 残留」（有 StoryForge DB）。
///
/// 同样只读、不改 PRAGMA、不创建文件；任何打开/读取错误 → false（保守当作非孤儿，
/// 让下游 cutover 的 publish 阶段 `owned_by_storyforge_readonly` 再做严格判定）。
fn orphan_storyforge_db_exists(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    use rusqlite::OpenFlags;
    let Some(uri) = path.to_str() else {
        return false;
    };
    let uri = format!("file:{uri}?mode=ro&immutable=1");
    let conn = match rusqlite::Connection::open_with_flags(
        &uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return false,
    };
    // 1. application_id 魔数。
    let app_id: i64 = conn
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .unwrap_or(-1);
    if app_id != i64::from(STORYFORGE_APPLICATION_ID) {
        return false;
    }
    // 2. schema_migrations 存在且 MAX(version)>=1。
    let has_table: i64 = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_migrations' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if has_table != 1 {
        return false;
    }
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap_or(0);
    if version < 1 {
        return false;
    }
    // 3. authority_binding 行存在（StoryForge cutover 写入的身份绑定表）。
    let bound: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM authority_binding WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    bound > 0
}

/// Read-only ownership probe: open the file with `SQLITE_OPEN_READONLY` (no
/// `READ_WRITE`, no `CREATE`, **no PRAGMA writes**) and require ALL of:
///
/// 1. `PRAGMA application_id == STORYFORGE_APPLICATION_ID`
/// 2. a `schema_migrations` table whose `MAX(version) >= 1`
/// 3. a completed `import_runs` row whose `source_manifest_hash` equals the
///    manifest being cut over
/// 4. an `authority_binding` row whose `authority_id` equals this cutover's
///    derived identity
///
/// This never mutates the target database: it cannot flip WAL mode, cannot
/// create the file, and cannot truncate `-wal`/`-shm`. Any error (unreadable,
/// not a database, locked) is treated as "not ours" so the caller fails
/// closed rather than risking a foreign DB.
fn owned_by_storyforge_readonly(
    path: &Path,
    expected_manifest_hash: &str,
    expected_authority_id: &str,
) -> Result<bool> {
    use rusqlite::OpenFlags;
    // SQLITE_OPEN_READ_WRITE must NOT be set: opening a WAL-mode DB read/write
    // can checkpoint/rotate the -wal sidecar; opening foreign DBs read/write
    // lets rusqlite's PRAGMA setup touch their header. Read-only is a pure
    // probe. URI mode lets us pass `?mode=ro` and `immutable=1` so we do not
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

    // 1. application_id 魔数：非 StoryForge 库（含带同名 schema_migrations 的
    //    外部库）在此被拒绝。
    let app_id: i64 = conn
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .unwrap_or(-1);
    if app_id != i64::from(STORYFORGE_APPLICATION_ID) {
        return Ok(false);
    }

    // 2. schema_migrations 存在且 MAX(version)>=1。
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
    if version < 1 {
        return Ok(false);
    }

    // 3. 最近一次 completed import 的源 hash 必须等于本次 cutover 的源。
    let stored_hash: Option<String> = conn
        .query_row(
            "SELECT source_manifest_hash FROM import_runs \
             WHERE status = 'completed' ORDER BY finished_at DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();
    if stored_hash.as_deref() != Some(expected_manifest_hash) {
        return Ok(false);
    }

    // 4. 权威身份绑定：DB 记录的身份必须等于本次 cutover 派生的身份
    //    （不同 data_dir / 不同源 → 不同身份 → 拒绝让位）。
    let bound: Option<String> = conn
        .query_row(
            "SELECT authority_id FROM authority_binding WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .ok();
    if bound.as_deref() != Some(expected_authority_id) {
        return Ok(false);
    }

    Ok(true)
}

/// Write the marker atomically (write to temp file, fsync, then rename and
/// fsync again; fsync the parent directory on unix).
fn write_marker_atomically(marker_path: &Path, marker: &BackendMarker) -> Result<()> {
    let content = serde_json::to_vec_pretty(marker)?;
    let tmp_path = marker_path.with_extension("json.tmp");
    fs::write(&tmp_path, &content)?;
    // 审查一.5：rename 前 fsync tmp 文件。
    fsync_file(&tmp_path)?;
    fs::rename(&tmp_path, marker_path)?;
    // 审查一.5：rename 后 fsync marker 文件 + 父目录（unix）。
    fsync_file(marker_path)?;
    fsync_parent_dir(marker_path)?;
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
