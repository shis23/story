//! Migration readiness tools: dry-run validation, backup, secret-free export.
//!
//! These APIs never enable SQLite as the production backend and must not mutate
//! the live database when exporting or validating source manifests.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use serde_json::{Map, Value};
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

    let mut hasher = Sha256::new();
    hash_named_array(&mut hasher, "cards", &cards);
    hash_named_array(&mut hasher, "campaigns", &campaigns);
    hash_named_array(&mut hasher, "instances", &instances);
    hash_named_array(&mut hasher, "knowledge", &knowledge);
    hash_named_array(&mut hasher, "tasks", &tasks);
    hash_named_array(&mut hasher, "summaries", &summaries);
    hash_named_array(&mut hasher, "turns", &turns);
    hash_named_array(&mut hasher, "conversations", &conversations);
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
        issues,
    })
}

/// Create a SQLite backup checkpoint + manifest without mutating the live DB contents.
pub fn create_backup_checkpoint(
    db: &Database,
    backup_dir: impl AsRef<Path>,
    label: &str,
) -> Result<BackupCheckpoint> {
    let backup_dir = backup_dir.as_ref();
    fs::create_dir_all(backup_dir)?;
    let safe_label = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let backup_db_path = backup_dir.join(format!("storyforge-{safe_label}-{stamp}.sqlite3"));
    let manifest_path = backup_dir.join(format!("storyforge-{safe_label}-{stamp}.manifest.json"));

    // Online backup copies pages; live DB contents remain unchanged.
    {
        let mut dst = Connection::open(&backup_db_path)?;
        let backup = rusqlite::backup::Backup::new(db.connection(), &mut dst)?;
        backup
            .run_to_completion(100, Duration::from_millis(0), None)
            .map_err(|e| SqliteError::Other(format!("sqlite backup failed: {e}")))?;
    }

    let schema_version = migrations::current_version(db).unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(fs::read(&backup_db_path)?);
    let manifest_hash = hex_encode(hasher.finalize());

    let manifest = serde_json::json!({
        "label": label,
        "created_at": chrono::Utc::now().to_rfc3339(),
        "schema_version": schema_version,
        "source_path": db.path().display().to_string(),
        "backup_db": backup_db_path.file_name().and_then(|s| s.to_str()),
        "manifest_hash": manifest_hash,
    });
    fs::write(
        &manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(SqliteError::from)?,
    )?;

    Ok(BackupCheckpoint {
        backup_db_path,
        manifest_path,
        schema_version,
        manifest_hash,
        label: label.to_string(),
    })
}

/// Read-only export suitable for rollback inspection. Must redact secrets.
pub fn export_readonly_snapshot(
    db: &Database,
    export_dir: impl AsRef<Path>,
) -> Result<ExportSnapshot> {
    let export_dir = export_dir.as_ref();
    fs::create_dir_all(export_dir)?;
    let mut redacted_fields = BTreeSet::new();

    let tables = [
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

    for (table, id_column) in tables {
        let path = export_dir.join(format!("{table}.json"));
        let rows = export_table_payloads(db, table, id_column, &mut redacted_fields)?;
        fs::write(
            path,
            serde_json::to_vec_pretty(&rows).map_err(SqliteError::from)?,
        )?;
    }

    // Covers edge list (no payload secrets expected).
    let covers = export_cover_edges(db)?;
    fs::write(
        export_dir.join("round_summary_covers.json"),
        serde_json::to_vec_pretty(&covers).map_err(SqliteError::from)?,
    )?;

    let redacted_fields: Vec<String> = redacted_fields.into_iter().collect();
    let manifest_path = export_dir.join("export_manifest.json");
    // Do not write secret field names or values into on-disk export artifacts.
    let manifest = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "source_path": db.path().display().to_string(),
        "schema_version": migrations::current_version(db).unwrap_or(0),
        "redacted_field_count": redacted_fields.len(),
        "mode": "readonly-rollback-inspection",
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

fn export_table_payloads(
    db: &Database,
    table: &str,
    id_column: &str,
    redacted_fields: &mut BTreeSet<String>,
) -> Result<Vec<Value>> {
    // table/id_column are internal literals only.
    let sql = format!("SELECT {id_column}, payload_json FROM {table} ORDER BY {id_column}");
    let mut stmt = db.connection().prepare(&sql)?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((id, payload))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, payload) = row?;
        let mut value: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        redact_value(&mut value, redacted_fields);
        out.push(serde_json::json!({
            "id": id,
            "payload": value,
        }));
    }
    Ok(out)
}

fn export_cover_edges(db: &Database) -> Result<Vec<Value>> {
    let mut stmt = db.connection().prepare(
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
        out.push(row?);
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
                    redacted_fields.insert(key);
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
        _ => {}
    }
}

fn is_secret_field(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    REDACTED_FIELD_NAMES
        .iter()
        .any(|name| lower == *name || lower.ends_with(&format!("_{name}")))
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

    for summary in summaries {
        let Some(id) = summary.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if let Some(covered_by) = summary.get("covered_by").and_then(|v| v.as_str())
            && !by_id.contains_key(covered_by)
        {
            issues.push(format!(
                "summary graph missing covered_by parent {covered_by} for {id}"
            ));
        }
        if let Some(Value::Array(covers)) = summary.get("covers") {
            for child in covers {
                match child.as_str() {
                    Some(child_id) if by_id.contains_key(child_id) => {}
                    Some(child_id) => issues.push(format!(
                        "summary graph missing cover child {child_id} for parent {id}"
                    )),
                    None => issues.push(format!("summary {id} has non-string cover entry")),
                }
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
    let value: Value = serde_json::from_str(&raw)?;
    match value {
        Value::Array(items) => Ok(items),
        Value::Null if optional => Ok(Vec::new()),
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
    let mut items: Vec<Value> = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path)?;
        items.push(serde_json::from_str(&raw)?);
    }
    items.sort_by(|a, b| {
        let left = a.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        let right = b.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        left.cmp(right)
    });
    Ok(items)
}

fn hash_named_array(hasher: &mut Sha256, name: &str, items: &[Value]) {
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    for item in items {
        hasher.update(stable_json(item).as_bytes());
        hasher.update(b"\n");
    }
}

fn stable_json(value: &Value) -> String {
    // Keep dry-run hashing compatible with importer-style stable serialization.
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for key in keys {
                out.insert(key.clone(), map.get(key).cloned().unwrap_or(Value::Null));
            }
            Value::Object(out).to_string()
        }
        other => other.to_string(),
    }
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
