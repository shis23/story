//! Migration readiness tools: dry-run validation, backup, secret-free export.
//!
//! These APIs never enable SQLite as the production backend and must not mutate
//! the live database when exporting or validating source manifests.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::connection::Database;
use crate::error::{Result, SqliteError};
use crate::migrations;

const REDACTED_FIELD_NAMES: &[&str] = &[
    "api_key",
    "apikey",
    "password",
    "secret",
    "token",
    "access_token",
    "refresh_token",
    "authorization",
    "private_key",
    "credential",
    "credentials",
    "bearer",
    "client_secret",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceManifestReport {
    pub manifest_hash: String,
    pub cards: usize,
    pub campaigns: usize,
    pub instances: usize,
    pub knowledge: usize,
    pub tasks: usize,
    pub summaries: usize,
    pub conversations: usize,
    pub turns: usize,
    /// characters.json 角色库条目数（Gate 5）。
    pub characters: usize,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupCheckpoint {
    pub backup_db_path: PathBuf,
    pub manifest_path: PathBuf,
    pub schema_version: i64,
    pub manifest_hash: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSnapshot {
    pub root_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub redacted_fields: Vec<String>,
}

/// Read-only dry-run over a JSON source tree. Must not open/write a live DB.
pub fn validate_source_manifest(data_dir: impl AsRef<Path>) -> Result<SourceManifestReport> {
    let data_dir = data_dir.as_ref();
    if !data_dir.exists() {
        return Err(SqliteError::ImportSourceMissing(data_dir.to_path_buf()));
    }

    let cards = read_json_array(data_dir.join("cards.json"), true)?;
    let campaigns = read_json_array(data_dir.join("campaigns.json"), true)?;
    let instances = read_json_array(data_dir.join("instances.json"), true)?;
    let knowledge = read_json_array(data_dir.join("knowledge.json"), true)?;
    let tasks = read_json_array(data_dir.join("tasks.json"), true)?;
    let summaries = read_json_array(data_dir.join("round_summaries.json"), true)?;
    let turns = read_json_array(data_dir.join("turns.json"), true)?;
    let conversations = read_conversation_dir(data_dir.join("conversations"))?;
    let mvu_translations = read_json_array(data_dir.join("mvu_translations.json"), true)?;
    let world_info = read_world_info_dir(data_dir.join("campaign_world_info"))?;
    let compress_jobs = read_compress_jobs_array(data_dir)?;
    let characters = read_characters_array(data_dir)?;

    // Keep labels/order identical to the importer so hashes are comparable.
    let mut hasher = Sha256::new();
    hash_named_array(&mut hasher, "cards", &cards);
    hash_named_array(&mut hasher, "campaigns", &campaigns);
    hash_named_array(&mut hasher, "instances", &instances);
    hash_named_array(&mut hasher, "knowledge", &knowledge);
    hash_named_array(&mut hasher, "tasks", &tasks);
    hash_named_array(&mut hasher, "round_summaries", &summaries);
    hash_named_array(&mut hasher, "turns", &turns);
    hash_named_array(&mut hasher, "conversations", &conversations);
    // Gate 4/5 可选集合：与 importer 完全同序、同投影、仅非空参与。
    if !mvu_translations.is_empty() {
        hash_named_array(&mut hasher, "mvu_translations", &mvu_translations);
    }
    if !world_info.is_empty() {
        let payloads: Vec<Value> = world_info.iter().map(|(_, v)| v.clone()).collect();
        hash_named_array(&mut hasher, "campaign_world_info", &payloads);
    }
    if !compress_jobs.is_empty() {
        hash_named_array(&mut hasher, "compress_jobs", &compress_jobs);
    }
    if !characters.is_empty() {
        hash_named_array(&mut hasher, "characters", &characters);
    }
    let manifest_hash = hex_encode(hasher.finalize());

    let mut issues = Vec::new();
    issues.extend(validate_summary_graph(&summaries));
    issues.extend(validate_attempt_ownership(&turns));

    Ok(SourceManifestReport {
        manifest_hash,
        cards: cards.len(),
        campaigns: campaigns.len(),
        instances: instances.len(),
        knowledge: knowledge.len(),
        tasks: tasks.len(),
        summaries: summaries.len(),
        conversations: conversations.len(),
        turns: turns.len(),
        characters: characters.len(),
        issues,
    })
}

/// 读 `campaign_world_info/{campaign_id}.json` 目录；文件名即 campaign_id。
/// importer 与 readiness 共用（hash 必须同投影）。
pub(crate) fn read_world_info_dir(dir: PathBuf) -> Result<Vec<(String, Value)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        let campaign_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                SqliteError::CorruptImportInput(format!(
                    "{}: world info file must have a file stem",
                    path.display()
                ))
            })?
            .to_string();
        let text = fs::read_to_string(&path)?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| SqliteError::CorruptImportInput(format!("{}: {e}", path.display())))?;
        out.push((campaign_id, value));
    }
    Ok(out)
}

/// 读 compress_jobs.json（可选）并投影到 `chronicle_compress_jobs` 列形态
/// （与 `upsert_compress_job` / reverse exporter 完全一致：缺省值、null 可选
/// 字段省略）。importer 与 readiness 共用，保证 hash 可比较。
pub(crate) fn read_compress_jobs_array(data_dir: &Path) -> Result<Vec<Value>> {
    let raw = read_json_array(data_dir.join("compress_jobs.json"), true)?;
    raw.into_iter().map(project_compress_job).collect()
}

fn project_compress_job(job: Value) -> Result<Value> {
    let job_id = job.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        SqliteError::CorruptImportInput("compress_job missing string field 'id'".into())
    })?;
    let campaign_id = job
        .get("campaign_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            SqliteError::CorruptImportInput(
                "compress_job missing string field 'campaign_id'".into(),
            )
        })?;
    let conversation_id = optional_str(&job, "conversation_id");
    let lineage_id = optional_str(&job, "lineage_id");
    let kind = optional_str(&job, "kind").unwrap_or_else(|| "auto".to_string());
    let status = optional_str(&job, "status").unwrap_or_else(|| "pending".to_string());
    let attempts = optional_u64(&job, "attempts").unwrap_or(0);
    let max_attempts = optional_u64(&job, "max_attempts").unwrap_or(5);
    let last_error = optional_str(&job, "last_error");
    let uncovered_a = optional_u64(&job, "uncovered_a_at_enqueue").unwrap_or(0);
    let uncovered_b = optional_u64(&job, "uncovered_b_at_enqueue").unwrap_or(0);
    let created_at = optional_str(&job, "created_at").unwrap_or_default();
    let updated_at = optional_str(&job, "updated_at").unwrap_or_default();
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
    Ok(v)
}

fn optional_str(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(|v| match v {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    })
}

fn optional_u64(value: &Value, key: &str) -> Option<u64> {
    value.get(key).and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().map(|i| i.max(0) as u64))
            .or_else(|| v.as_str()?.parse().ok())
    })
}

/// 读 characters.json（可选）并归一化为 `StoredCharacter` 契约形态
/// `{id, info, imported_at}`——丢弃未知顶层键（CharacterStore 反序列化同样
/// 忽略），保证与 importer 的 hash 及 reverse export 重建形态完全一致。
pub(crate) fn read_characters_array(data_dir: &Path) -> Result<Vec<Value>> {
    let raw = read_json_array(data_dir.join("characters.json"), true)?;
    raw.into_iter().map(normalize_character_entry).collect()
}

fn normalize_character_entry(entry: Value) -> Result<Value> {
    let id = entry.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
        SqliteError::CorruptImportInput("character entry missing string field 'id'".into())
    })?;
    let info = entry.get("info").ok_or_else(|| {
        SqliteError::CorruptImportInput("character entry missing field 'info'".into())
    })?;
    if !info.is_object() {
        return Err(SqliteError::CorruptImportInput(
            "character entry 'info' must be an object".into(),
        ));
    }
    let imported_at = entry
        .get("imported_at")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    Ok(serde_json::json!({
        "id": id,
        "info": info.clone(),
        "imported_at": imported_at,
    }))
}

/// Create a SQLite backup checkpoint + manifest without mutating the live DB contents.
pub fn create_backup_checkpoint(
    db: &Database,
    backup_dir: impl AsRef<Path>,
    label: &str,
) -> Result<BackupCheckpoint> {
    let backup_dir = backup_dir.as_ref();
    let live_path = canonicalize_existing(db.path()).unwrap_or_else(|| db.path().to_path_buf());
    if paths_equal(&live_path, backup_dir) {
        return Err(SqliteError::Other(
            "refusing to write backup into the live database path".into(),
        ));
    }
    if backup_dir.is_file() {
        return Err(SqliteError::Other(format!(
            "backup target is not a directory: {}",
            backup_dir.display()
        )));
    }
    fs::create_dir_all(backup_dir)?;

    let label_is_sensitive = looks_like_secret_text(label) || looks_like_absolute_path_text(label);
    let safe_label = if label_is_sensitive {
        "checkpoint".to_string()
    } else {
        let normalized = label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        if normalized.is_empty() {
            "checkpoint".to_string()
        } else {
            normalized
        }
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut backup_db_path =
        backup_dir.join(format!("storyforge-{safe_label}-{stamp}-{nanos:x}.sqlite3"));
    let mut manifest_path = backup_dir.join(format!(
        "storyforge-{safe_label}-{stamp}-{nanos:x}.manifest.json"
    ));

    // Never overwrite an existing backup/manifest (including symlink targets).
    let mut attempt = 0u32;
    while backup_db_path.exists()
        || manifest_path.exists()
        || is_symlink(&backup_db_path)
        || is_symlink(&manifest_path)
    {
        attempt = attempt.saturating_add(1);
        if attempt > 32 {
            return Err(SqliteError::Other(
                "backup path collision: unable to allocate unique backup filename".into(),
            ));
        }
        backup_db_path = backup_dir.join(format!(
            "storyforge-{safe_label}-{stamp}-{nanos:x}-{attempt}.sqlite3"
        ));
        manifest_path = backup_dir.join(format!(
            "storyforge-{safe_label}-{stamp}-{nanos:x}-{attempt}.manifest.json"
        ));
    }

    if let Some(canon_backup) = canonicalize_existing(&backup_db_path)
        && paths_equal(&live_path, &canon_backup)
    {
        return Err(SqliteError::Other(
            "refusing to overwrite the live database via backup path".into(),
        ));
    }

    // Online backup copies pages; live DB contents remain unchanged.
    {
        let mut dst = Connection::open(&backup_db_path)?;
        let backup = rusqlite::backup::Backup::new(db.connection(), &mut dst)?;
        backup
            .run_to_completion(100, Duration::from_millis(0), None)
            .map_err(|e| SqliteError::Other(format!("sqlite backup failed: {e}")))?;
    }

    // Schema version is read from the backup DB itself.
    let backup_db = Database::open(&backup_db_path)?;
    let integrity: String =
        backup_db
            .connection()
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
    if !integrity.eq_ignore_ascii_case("ok") {
        return Err(SqliteError::Other(format!(
            "backup integrity_check failed: {integrity}"
        )));
    }
    let schema_version = migrations::current_version(&backup_db)?;
    let mut hasher = Sha256::new();
    hasher.update(fs::read(&backup_db_path)?);
    let manifest_hash = hex_encode(hasher.finalize());

    let manifest = serde_json::json!({
        "label": if label_is_sensitive { "[REDACTED]" } else { safe_label.as_str() },
        "created_at": chrono::Utc::now().to_rfc3339(),
        "schema_version": schema_version,
        "backup_db": backup_db_path.file_name().and_then(|s| s.to_str()),
        "manifest_hash": manifest_hash,
    });
    // Refuse racey overwrite if another process created the manifest meanwhile.
    if manifest_path.exists() || is_symlink(&manifest_path) {
        return Err(SqliteError::Other(format!(
            "backup manifest already exists: {}",
            manifest_path.display()
        )));
    }
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(SqliteError::from)?,
    )?;

    Ok(BackupCheckpoint {
        backup_db_path,
        manifest_path,
        schema_version,
        manifest_hash,
        label: if label_is_sensitive {
            "[REDACTED]".to_string()
        } else {
            safe_label
        },
    })
}

/// Read-only export suitable for rollback inspection. Must redact secrets.
pub fn export_readonly_snapshot(
    db: &Database,
    export_dir: impl AsRef<Path>,
) -> Result<ExportSnapshot> {
    let export_dir = export_dir.as_ref();
    fs::create_dir_all(export_dir)?;

    // Single-transaction consistent snapshot of live tables.
    let mut conn = Connection::open(db.path())?;
    conn.pragma_update(None, "query_only", true)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;

    let mut redacted_fields = BTreeSet::new();
    let payload_tables = [
        ("character_cards", "card_id"),
        ("campaigns", "campaign_id"),
        ("character_instances", "instance_id"),
        ("character_knowledge", "knowledge_id"),
        ("story_tasks", "task_id"),
        ("conversations", "conversation_id"),
        ("turns", "turn_id"),
        ("turn_attempts", "attempt_id"),
        ("round_summaries", "summary_id"),
    ];
    for (table, id_column) in payload_tables {
        let rows = export_table_payloads_tx(&tx, table, id_column, &mut redacted_fields)?;
        fs::write(
            export_dir.join(format!("{table}.json")),
            serde_json::to_vec_pretty(&rows).map_err(SqliteError::from)?,
        )?;
    }

    let covers = export_cover_edges_tx(&tx, &mut redacted_fields)?;
    fs::write(
        export_dir.join("round_summary_covers.json"),
        serde_json::to_vec_pretty(&covers).map_err(SqliteError::from)?,
    )?;

    // Operational / ledger tables needed for rollback inspection.
    let jobs = export_named_rows_tx(
        &tx,
        "chronicle_publication_jobs",
        &[
            "publication_id",
            "campaign_id",
            "job_id",
            "base_chronicle_revision",
            "target_chronicle_revision",
            "parent_ids_json",
            "child_covered_by_json",
            "payload_hash",
            "status",
            "created_at",
            "completed_at",
        ],
        &mut redacted_fields,
    )?;
    fs::write(
        export_dir.join("chronicle_publication_jobs.json"),
        serde_json::to_vec_pretty(&jobs).map_err(SqliteError::from)?,
    )?;

    let commits = export_named_rows_tx(
        &tx,
        "mutation_commits",
        &[
            "commit_id",
            "campaign_id",
            "turn_id",
            "attempt_id",
            "expected_revision",
            "target_revision",
            "terminal_status",
            "payload_hash",
            "committed_at",
        ],
        &mut redacted_fields,
    )?;
    fs::write(
        export_dir.join("mutation_commits.json"),
        serde_json::to_vec_pretty(&commits).map_err(SqliteError::from)?,
    )?;

    let imports = export_named_rows_tx(
        &tx,
        "import_runs",
        &[
            "run_id",
            "source_root",
            "source_manifest_hash",
            "status",
            "started_at",
            "finished_at",
            "error",
        ],
        &mut redacted_fields,
    )?;
    fs::write(
        export_dir.join("import_runs.json"),
        serde_json::to_vec_pretty(&imports).map_err(SqliteError::from)?,
    )?;

    let schema_migrations = export_named_rows_tx(
        &tx,
        "schema_migrations",
        &["version", "name", "applied_at", "checksum"],
        &mut redacted_fields,
    )?;
    fs::write(
        export_dir.join("schema_migrations.json"),
        serde_json::to_vec_pretty(&schema_migrations).map_err(SqliteError::from)?,
    )?;

    let schema_version: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    tx.commit()?;

    let redacted_fields: Vec<String> = redacted_fields.into_iter().collect();
    let manifest_path = export_dir.join("export_manifest.json");
    // Do not write secret field names or values into on-disk export artifacts.
    let manifest = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "schema_version": schema_version,
        "redacted_field_count": redacted_fields.len(),
        "mode": "readonly-rollback-inspection",
        "tables": [
            "character_cards",
            "campaigns",
            "character_instances",
            "character_knowledge",
            "story_tasks",
            "conversations",
            "turns",
            "turn_attempts",
            "round_summaries",
            "round_summary_covers",
            "chronicle_publication_jobs",
            "mutation_commits",
            "import_runs",
            "schema_migrations"
        ],
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(SqliteError::from)?,
    )?;

    Ok(ExportSnapshot {
        root_dir: export_dir.to_path_buf(),
        manifest_path,
        redacted_fields,
    })
}

fn export_table_payloads_tx(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    redacted_fields: &mut BTreeSet<String>,
) -> Result<Vec<Value>> {
    let sql = format!("SELECT {id_column}, payload_json FROM {table} ORDER BY {id_column}");
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((id, payload))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, payload) = row?;
        let mut value: Value = serde_json::from_str(&payload).map_err(|e| {
            SqliteError::Other(format!("corrupt payload_json in {table} id={id}: {e}"))
        })?;
        redact_value(&mut value, redacted_fields);
        let mut exported_id = Value::String(id);
        redact_value(&mut exported_id, redacted_fields);
        out.push(serde_json::json!({
            "id": exported_id,
            "payload": value,
        }));
    }
    Ok(out)
}

fn export_cover_edges_tx(
    tx: &rusqlite::Transaction<'_>,
    redacted_fields: &mut BTreeSet<String>,
) -> Result<Vec<Value>> {
    let mut stmt = tx.prepare(
        "SELECT parent_id, child_id FROM round_summary_covers ORDER BY parent_id, child_id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(serde_json::json!({
            "parent_id": row.get::<_, String>(0)?,
            "child_id": row.get::<_, String>(1)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let mut value = row?;
        redact_value(&mut value, redacted_fields);
        out.push(value);
    }
    Ok(out)
}

fn export_named_rows_tx(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    columns: &[&str],
    redacted_fields: &mut BTreeSet<String>,
) -> Result<Vec<Value>> {
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1 LIMIT 1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Ok(Vec::new());
    }
    let sql = format!("SELECT {} FROM {table} ORDER BY rowid", columns.join(", "));
    let mut stmt = tx.prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let mut map = serde_json::Map::new();
        for (idx, name) in columns.iter().enumerate() {
            let value: Option<String> = match row.get_ref(idx)? {
                rusqlite::types::ValueRef::Null => None,
                rusqlite::types::ValueRef::Integer(v) => Some(v.to_string()),
                rusqlite::types::ValueRef::Real(v) => Some(v.to_string()),
                rusqlite::types::ValueRef::Text(v) => Some(String::from_utf8_lossy(v).into_owned()),
                rusqlite::types::ValueRef::Blob(_) => Some("<blob>".into()),
            };
            map.insert(
                (*name).to_string(),
                value.map(Value::String).unwrap_or(Value::Null),
            );
        }
        Ok(Value::Object(map))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let mut value = row?;
        redact_value(&mut value, redacted_fields);
        if let Value::Object(map) = &mut value
            && let Some(source_root) = map.get_mut("source_root")
            && !source_root.is_null()
        {
            *source_root = Value::String("[REDACTED_PATH]".into());
            redacted_fields.insert("source_root".into());
        }
        out.push(value);
    }
    Ok(out)
}

fn redact_value(value: &mut Value, redacted_fields: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if is_secret_field(&key) {
                    map.remove(&key);
                    // Never expose the hostile source key through ExportSnapshot metadata.
                    redacted_fields.insert("sensitive_field".into());
                } else if let Some(child) = map.get_mut(&key) {
                    redact_value(child, redacted_fields);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item, redacted_fields);
            }
        }
        Value::String(text) if looks_like_secret_text(text) => {
            *text = "[REDACTED]".into();
            redacted_fields.insert("free_text_secret".into());
        }
        Value::String(text) if looks_like_absolute_path_text(text) => {
            *text = "[REDACTED]".into();
            redacted_fields.insert("absolute_path".into());
        }
        Value::String(_) => {}
        _ => {}
    }
}

fn is_secret_field(key: &str) -> bool {
    let lower = key.to_ascii_lowercase().replace('-', "_");
    let compact: String = lower
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    REDACTED_FIELD_NAMES.iter().any(|name| {
        let compact_name: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        lower == *name
            || lower.ends_with(&format!("_{name}"))
            || lower.ends_with(name)
            || lower.contains(name)
            || compact == compact_name
            || compact.ends_with(&compact_name)
    })
}

fn looks_like_secret_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let compact = lower.replace([' ', '\t'], "");
    compact.contains("api_key=")
        || compact.contains("api_key:")
        || compact.contains("apikey=")
        || compact.contains("apikey:")
        || compact.contains("token=")
        || compact.contains("token:")
        || lower.contains("bearer ")
        || compact.contains("password=")
        || compact.contains("password:")
        || compact.contains("secret=")
        || compact.contains("secret:")
        || compact.contains("credential=")
        || compact.contains("credential:")
        || compact.contains("privatekey=")
        || compact.contains("privatekey:")
        || looks_like_bare_credential(text)
}

fn looks_like_bare_credential(text: &str) -> bool {
    let bytes = text.as_bytes();
    for start in 0..bytes.len().saturating_sub(2) {
        if bytes[start..].starts_with(b"sk-") {
            let credential_len = bytes[start..]
                .iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || **byte == b'-' || **byte == b'_')
                .count();
            if credential_len >= 24 {
                return true;
            }
        }
    }

    text.split_whitespace().any(|token| {
        let token = token
            .trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-' && c != '_');
        let parts = token.split('.').collect::<Vec<_>>();
        token.starts_with("eyJ")
            && parts.len() == 3
            && parts.iter().all(|part| {
                part.len() >= 8
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            })
    })
}

fn looks_like_absolute_path_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(3).any(|window| {
        window[0].is_ascii_alphabetic()
            && window[1] == b':'
            && (window[2] == b'\\' || window[2] == b'/')
    }) || text.contains("\\\\")
        || [
            "/home/", "/Users/", "/data/", "/tmp/", "/var/", "/root/", "/etc/",
        ]
        .iter()
        .any(|prefix| text.contains(prefix))
}

pub(crate) fn validate_summary_graph(summaries: &[Value]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut by_id: HashMap<String, &Value> = HashMap::new();
    for summary in summaries {
        let Some(id) = summary.get("id").and_then(|v| v.as_str()) else {
            issues.push("summary missing id".into());
            continue;
        };
        if by_id.insert(id.to_string(), summary).is_some() {
            issues.push(format!("duplicate summary id {id}"));
        }
    }

    // Build reverse maps for bidirectional consistency.
    let mut parent_to_children: HashMap<String, HashSet<String>> = HashMap::new();
    let mut child_to_parent: HashMap<String, String> = HashMap::new();
    for summary in summaries {
        let Some(id) = summary.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if summary.get("lineage_id").and_then(|v| v.as_str()).is_none() {
            issues.push(format!("summary {id} missing lineage_id"));
        }
        let level = summary.get("level").and_then(|v| v.as_u64());
        if !matches!(level, Some(0..=2)) {
            issues.push(format!("summary {id} has invalid level {level:?}"));
        }
        if let Some(covered_by) = summary.get("covered_by").and_then(|v| v.as_str()) {
            if !by_id.contains_key(covered_by) {
                issues.push(format!(
                    "summary graph missing covered_by parent {covered_by} for {id}"
                ));
            } else {
                child_to_parent.insert(id.to_string(), covered_by.to_string());
                parent_to_children
                    .entry(covered_by.to_string())
                    .or_default()
                    .insert(id.to_string());
            }
        }
        if let Some(Value::Array(covers)) = summary.get("covers") {
            for child in covers {
                match child.as_str() {
                    Some(child_id) if by_id.contains_key(child_id) => {
                        parent_to_children
                            .entry(id.to_string())
                            .or_default()
                            .insert(child_id.to_string());
                    }
                    Some(child_id) => issues.push(format!(
                        "summary graph missing cover child {child_id} for parent {id}"
                    )),
                    None => issues.push(format!("summary {id} has non-string cover entry")),
                }
            }
        }
    }

    // Bidirectional covers <-> covered_by consistency.
    for summary in summaries {
        let Some(id) = summary.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(Value::Array(covers)) = summary.get("covers") {
            for child in covers {
                let Some(child_id) = child.as_str() else {
                    continue;
                };
                match child_to_parent.get(child_id) {
                    Some(parent) if parent == id => {}
                    Some(parent) => issues.push(format!(
                        "summary graph covered_by mismatch: child {child_id} covered_by={parent}, parent {id} covers it"
                    )),
                    None => issues.push(format!(
                        "summary graph covered_by missing for covered child {child_id} of parent {id}"
                    )),
                }
            }
        }
        if let Some(covered_by) = summary.get("covered_by").and_then(|v| v.as_str()) {
            let parent_covers = parent_to_children.get(covered_by);
            let listed = parent_covers.is_some_and(|set| set.contains(id));
            // Also require parent.covers array explicitly list the child when present.
            if let Some(parent) = by_id.get(covered_by) {
                let covers = parent
                    .get("covers")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let in_covers = covers.iter().any(|c| c.as_str() == Some(id));
                if !in_covers {
                    issues.push(format!(
                        "summary graph cover mismatch: child {id} covered_by={covered_by} but parent covers omit it"
                    ));
                }
            } else if !listed {
                issues.push(format!(
                    "summary graph cover mismatch: child {id} covered_by={covered_by}"
                ));
            }
        }

        // Scope consistency against parent when covered.
        if let Some(covered_by) = summary.get("covered_by").and_then(|v| v.as_str())
            && let Some(parent) = by_id.get(covered_by)
        {
            for field in ["campaign_id", "conversation_id", "lineage_id"] {
                let child_v = summary.get(field).and_then(|v| v.as_str());
                let parent_v = parent.get(field).and_then(|v| v.as_str());
                if child_v.is_none() || parent_v.is_none() || child_v != parent_v {
                    issues.push(format!(
                        "summary graph scope mismatch on {field}: child {id}={:?}, parent {covered_by}={:?}",
                        child_v, parent_v
                    ));
                }
            }
        }

        let covers = summary
            .get("covers")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if covers.is_empty() {
            if summary
                .get("level")
                .and_then(|v| v.as_u64())
                .is_some_and(|level| level > 0)
            {
                issues.push(format!("summary {id} level requires non-empty covers"));
            }
            continue;
        }
        let parent_level = summary.get("level").and_then(|v| v.as_u64());
        if !matches!(parent_level, Some(1 | 2)) {
            issues.push(format!(
                "summary {id} invalid parent level {parent_level:?}"
            ));
            continue;
        }
        let mut spans = Vec::new();
        for child_id in covers.iter().filter_map(|v| v.as_str()) {
            let Some(child) = by_id.get(child_id) else {
                continue;
            };
            let child_level = child.get("level").and_then(|v| v.as_u64());
            if child_level != parent_level.map(|level| level - 1) {
                issues.push(format!(
                    "summary {id} level {parent_level:?} cannot cover child {child_id} level {child_level:?}"
                ));
            }
            if let Some(start) = child.get("turn").and_then(|v| v.as_u64()) {
                let end = child
                    .get("turn_end")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(start);
                if end < start {
                    issues.push(format!(
                        "summary {child_id} has invalid turn span {start}-{end}"
                    ));
                }
                spans.push((start, end));
            } else {
                issues.push(format!("summary {child_id} missing turn span"));
            }
        }
        spans.sort_unstable();
        for window in spans.windows(2) {
            if window[1].0 != window[0].1.saturating_add(1) {
                issues.push(format!(
                    "summary {id} covers are not continuous between turns {} and {}",
                    window[0].1, window[1].0
                ));
            }
        }
        if let (Some((expected_start, _)), Some((_, expected_end))) = (spans.first(), spans.last())
        {
            let parent_start = summary.get("turn").and_then(|v| v.as_u64());
            let parent_end = summary
                .get("turn_end")
                .and_then(|v| v.as_u64())
                .or(parent_start);
            if parent_start != Some(*expected_start) || parent_end != Some(*expected_end) {
                issues.push(format!(
                    "summary {id} turn span {parent_start:?}-{parent_end:?} does not match children {expected_start}-{expected_end}"
                ));
            }
        }
    }

    issues
}

pub(crate) fn validate_attempt_ownership(turns: &[Value]) -> Vec<String> {
    let mut issues = Vec::new();
    let mut owners: HashMap<String, String> = HashMap::new();
    for turn in turns {
        let Some(turn_id) = turn.get("turn_id").and_then(|v| v.as_str()) else {
            issues.push("turn missing turn_id".into());
            continue;
        };
        let Some(Value::Array(attempts)) = turn.get("attempts") else {
            continue;
        };
        for attempt in attempts {
            let Some(attempt_id) = attempt.get("attempt_id").and_then(|v| v.as_str()) else {
                issues.push(format!("turn {turn_id} has attempt without attempt_id"));
                continue;
            };
            if let Some(prev) = owners.insert(attempt_id.to_string(), turn_id.to_string()) {
                issues.push(format!(
                    "duplicate attempt ownership: attempt {attempt_id} claimed by turns {prev} and {turn_id}"
                ));
            }
        }
    }
    issues
}

fn read_json_array(path: PathBuf, optional: bool) -> Result<Vec<Value>> {
    if !path.exists() {
        if optional {
            return Ok(Vec::new());
        }
        return Err(SqliteError::ImportSourceMissing(path));
    }
    let raw = fs::read_to_string(&path)?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|e| SqliteError::CorruptImportInput(format!("{}: {e}", path.display())))?;
    match value {
        Value::Array(items) => Ok(items),
        other => Err(SqliteError::CorruptImportInput(format!(
            "{} must be a JSON array, got {}",
            path.display(),
            other
        ))),
    }
}

fn read_conversation_dir(dir: PathBuf) -> Result<Vec<Value>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    // Match importer path-order sorting so hashes stay aligned.
    paths.sort();
    let mut items: Vec<Value> = Vec::new();
    for path in paths {
        let raw = fs::read_to_string(&path)?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|e| SqliteError::CorruptImportInput(format!("{}: {e}", path.display())))?;
        if !value.is_object() {
            return Err(SqliteError::CorruptImportInput(format!(
                "{}: expected conversation object",
                path.display()
            )));
        }
        items.push(value);
    }
    Ok(items)
}

fn hash_named_array(hasher: &mut Sha256, name: &str, items: &[Value]) {
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    // Match importer: sort compact JSON of each item, then hash.
    let mut encoded: Vec<String> = items.iter().map(stable_json).collect();
    encoded.sort();
    for item in encoded {
        hasher.update(item.as_bytes());
        hasher.update(b"\n");
    }
}

fn stable_json(value: &Value) -> String {
    // serde_json Value serializes object keys in BTreeMap order.
    serde_json::to_string(value).unwrap_or_default()
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

fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

fn canonicalize_existing(path: &Path) -> Option<PathBuf> {
    fs::canonicalize(path).ok()
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
