//! SQLite → portable JSON reverse export.
//!
//! This produces a JSON data-directory layout matching what the JSON importer
//! reads, so an operator can roll back from SQLite to JSON by importing the
//! exported directory. The export is **explicit, versioned, validated, and
//! secret-safe**: it never deletes the SQLite DB or original JSON backup.
//!
//! The export reads from the SQLite database and stages files under a temporary
//! directory, then atomically renames the staging tree into the final export
//! directory. Absolute paths, path-escape sequences, and secret-shaped values
//! are rejected or redacted.

use std::fs;
use std::path::{Component, Path, PathBuf};

use rusqlite::Connection;
use rusqlite::TransactionBehavior;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::connection::Database;
use crate::error::{Result, SqliteError};

/// Filename for the reverse-export manifest.
pub const EXPORT_MANIFEST_FILENAME: &str = "reverse_export_manifest.json";

/// Report describing the reverse export result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReverseExportReport {
    pub export_manifest_hash: String,
    pub schema_version: i64,
    pub cards: usize,
    pub campaigns: usize,
    pub instances: usize,
    pub knowledge: usize,
    pub tasks: usize,
    pub summaries: usize,
    pub conversations: usize,
    pub turns: usize,
    pub unsupported_fields: Vec<String>,
}

/// Result of a reverse export operation.
#[derive(Debug, Clone)]
pub struct ReverseExportResult {
    pub report: ReverseExportReport,
    pub export_dir: PathBuf,
    pub manifest_path: PathBuf,
}

/// Export a SQLite database to a portable JSON directory layout.
///
/// The target directory must not be the live database directory. Original
/// JSON or SQLite data is never modified or deleted. The export is staged
/// under `<export_dir>.tmp` and published atomically.
pub fn export_sqlite_to_json(
    db: &Database,
    export_dir: impl AsRef<Path>,
) -> Result<ReverseExportResult> {
    let export_dir = export_dir.as_ref().to_path_buf();

    // Refuse to export into the live DB directory.
    let live_parent = db
        .path()
        .parent()
        .map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
    if export_dir.exists() {
        let export_canon = fs::canonicalize(&export_dir)
            .map_err(|e| SqliteError::Other(format!("cannot resolve export dir: {e}")))?;
        if let Some(live) = &live_parent
            && live == &export_canon
        {
            return Err(SqliteError::Other(
                "refusing to export into the live database directory".into(),
            ));
        }
    } else if let Some(parent) = export_dir.parent() {
        let parent_canon = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        if let Some(live) = &live_parent
            && live == &parent_canon
            && export_dir
                .file_name()
                .is_some_and(|name| name == live.file_name().unwrap_or_default())
        {
            return Err(SqliteError::Other(
                "refusing to export into the live database directory".into(),
            ));
        }
    }

    // Stage under a sibling temp directory, then atomically publish.
    let stage_dir = {
        let parent = export_dir.parent().unwrap_or_else(|| Path::new("."));
        let name = export_dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("export");
        parent.join(format!(
            ".{name}.staging-{}",
            chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
        ))
    };
    if stage_dir.exists() {
        fs::remove_dir_all(&stage_dir)?;
    }
    fs::create_dir_all(stage_dir.join("conversations"))?;

    // Single read-only transaction for a consistent snapshot.
    let mut conn = Connection::open(db.path())?;
    conn.pragma_update(None, "query_only", true)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;

    let schema_version: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut unsupported = Vec::new();

    // Export each table's payload_json as a JSON array.
    let cards = export_table_array(&tx, "character_cards", "card_id", &mut unsupported)?;
    let campaigns = export_table_array(&tx, "campaigns", "campaign_id", &mut unsupported)?;
    let instances =
        export_table_array(&tx, "character_instances", "instance_id", &mut unsupported)?;
    let knowledge =
        export_table_array(&tx, "character_knowledge", "knowledge_id", &mut unsupported)?;
    let tasks = export_table_array(&tx, "story_tasks", "task_id", &mut unsupported)?;
    let summaries = export_table_array(&tx, "round_summaries", "summary_id", &mut unsupported)?;
    let turns = export_table_array(&tx, "turns", "turn_id", &mut unsupported)?;

    // Conversations are stored as individual files matching the JSON layout.
    let conversations = export_conversations(&tx, &stage_dir, &mut unsupported)?;

    // Write array files into the staging tree only.
    write_array_file(&stage_dir, "cards.json", &cards)?;
    write_array_file(&stage_dir, "campaigns.json", &campaigns)?;
    write_array_file(&stage_dir, "instances.json", &instances)?;
    write_array_file(&stage_dir, "knowledge.json", &knowledge)?;
    write_array_file(&stage_dir, "tasks.json", &tasks)?;
    write_array_file(&stage_dir, "round_summaries.json", &summaries)?;
    write_array_file(&stage_dir, "turns.json", &turns)?;

    tx.commit()?;

    // Build the manifest hash over all exported content (sorted, stable).
    let manifest_hash = collect_export_hash(
        &cards,
        &campaigns,
        &instances,
        &knowledge,
        &tasks,
        &summaries,
        &turns,
        &conversations,
    );

    let report = ReverseExportReport {
        export_manifest_hash: manifest_hash,
        schema_version,
        cards: cards.len(),
        campaigns: campaigns.len(),
        instances: instances.len(),
        knowledge: knowledge.len(),
        tasks: tasks.len(),
        summaries: summaries.len(),
        conversations: conversations.len(),
        turns: turns.len(),
        unsupported_fields: unsupported,
    };

    let staged_manifest_path = stage_dir.join(EXPORT_MANIFEST_FILENAME);
    let manifest = serde_json::json!({
        "created_at": chrono::Utc::now().to_rfc3339(),
        "direction": "sqlite-to-json-rollback",
        "schema_version": schema_version,
        "source_backend": "sqlite",
        "target_backend": "json",
        "export_manifest_hash": report.export_manifest_hash,
        "counts": {
            "cards": report.cards,
            "campaigns": report.campaigns,
            "instances": report.instances,
            "knowledge": report.knowledge,
            "tasks": report.tasks,
            "summaries": report.summaries,
            "conversations": report.conversations,
            "turns": report.turns,
        },
        "unsupported_fields": report.unsupported_fields,
        "note": "Import this directory via the JSON importer to roll back to JSON backend.",
    });
    fs::write(
        &staged_manifest_path,
        serde_json::to_vec_pretty(&manifest).map_err(SqliteError::from)?,
    )?;

    // Atomic publish: move any previous export aside, then rename the staging
    // tree into place. Stale conversation files cannot remain because the whole
    // tree is replaced.
    if export_dir.exists() {
        let aside = {
            let parent = export_dir.parent().unwrap_or_else(|| Path::new("."));
            let name = export_dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("export");
            parent.join(format!(
                ".{name}.pre-export-{}",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%.3fZ")
            ))
        };
        fs::rename(&export_dir, &aside)?;
    }
    if let Some(parent) = export_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&stage_dir, &export_dir)?;

    let manifest_path = export_dir.join(EXPORT_MANIFEST_FILENAME);
    Ok(ReverseExportResult {
        report,
        export_dir,
        manifest_path,
    })
}

fn export_table_array(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    if exists.is_none() {
        return Ok(Vec::new());
    }

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

        // Merge the id into the payload so the exported record is self-contained
        // and matches the JSON store layout (which uses the id as a field).
        if let Value::Object(map) = &mut value
            && !map.contains_key("id")
        {
            map.insert("id".to_string(), Value::String(id));
        }

        // Redact secret-shaped values before writing to the export artifact.
        redact_secret_values(&mut value, unsupported);

        out.push(value);
    }
    Ok(out)
}

fn export_conversations(
    tx: &rusqlite::Transaction<'_>,
    stage_dir: &Path,
    unsupported: &mut Vec<String>,
) -> Result<Vec<Value>> {
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='conversations'",
            [],
            |row| row.get(0),
        )
        .ok()
        .flatten();
    if exists.is_none() {
        return Ok(Vec::new());
    }

    let mut stmt = tx.prepare(
        "SELECT conversation_id, payload_json FROM conversations ORDER BY conversation_id",
    )?;
    let rows = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((id, payload))
    })?;

    let mut out = Vec::new();
    let conv_dir = stage_dir.join("conversations");
    for row in rows {
        let (id, payload) = row?;
        let safe_id = sanitize_conversation_filename(&id)?;
        let mut value: Value = serde_json::from_str(&payload).map_err(|e| {
            SqliteError::Other(format!(
                "corrupt payload_json in conversations id={id}: {e}"
            ))
        })?;
        if let Value::Object(map) = &mut value
            && !map.contains_key("id")
        {
            map.insert("id".to_string(), Value::String(id.clone()));
        }
        redact_secret_values(&mut value, unsupported);

        // Write the individual conversation file under the staging tree only.
        let conv_path = conv_dir.join(format!("{safe_id}.json"));
        // Defense-in-depth: reject any path that would escape the conversations dir.
        let canon_parent = fs::canonicalize(&conv_dir).unwrap_or_else(|_| conv_dir.clone());
        if let Some(parent) = conv_path.parent() {
            let parent_canon = if parent.exists() {
                fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf())
            } else {
                parent.to_path_buf()
            };
            if parent_canon != canon_parent && parent != conv_dir {
                return Err(SqliteError::Other(format!(
                    "conversation path escaped staging directory: {id}"
                )));
            }
        }
        fs::write(
            &conv_path,
            serde_json::to_vec_pretty(&value).map_err(SqliteError::from)?,
        )?;
        out.push(value);
    }
    Ok(out)
}

/// Reject path-escape and absolute path shapes in conversation ids before they
/// are used as filenames.
fn sanitize_conversation_filename(id: &str) -> Result<String> {
    if id.is_empty() {
        return Err(SqliteError::Other(
            "conversation id is empty; refusing export".into(),
        ));
    }
    if id.contains('\0') {
        return Err(SqliteError::Other(
            "conversation id contains NUL; refusing export".into(),
        ));
    }
    let path = Path::new(id);
    if path.is_absolute() {
        return Err(SqliteError::Other(format!(
            "conversation id looks absolute; refusing export: {id}"
        )));
    }
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => {
                let s = part.to_string_lossy();
                if s == ".." || s == "." {
                    return Err(SqliteError::Other(format!(
                        "conversation id has path components; refusing export: {id}"
                    )));
                }
                components.push(s.into_owned());
            }
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => {
                return Err(SqliteError::Other(format!(
                    "conversation id has path components; refusing export: {id}"
                )));
            }
        }
    }
    if components.len() != 1 {
        return Err(SqliteError::Other(format!(
            "conversation id must be a single path segment; refusing export: {id}"
        )));
    }
    let name = &components[0];
    // Keep only portable filename characters.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        // Still safe as a single segment after component checks; replace unsafe chars.
        let sanitized: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
            return Err(SqliteError::Other(format!(
                "conversation id sanitizes to empty/unsafe name: {id}"
            )));
        }
        return Ok(sanitized);
    }
    Ok(name.clone())
}

/// Recursively redact values whose keys look like secrets, and free-text that
/// looks like credentials. Records the field category (never the hostile key).
fn redact_secret_values(value: &mut Value, unsupported: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in keys {
                if is_secret_field(&key) {
                    map.remove(&key);
                    if !unsupported.iter().any(|f| f == "sensitive_field") {
                        unsupported.push("sensitive_field".into());
                    }
                } else if let Some(child) = map.get_mut(&key) {
                    redact_secret_values(child, unsupported);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_secret_values(item, unsupported);
            }
        }
        Value::String(text) if looks_like_secret_text(text) => {
            *text = "[REDACTED]".into();
            if !unsupported.iter().any(|f| f == "free_text_secret") {
                unsupported.push("free_text_secret".into());
            }
        }
        Value::String(text) if looks_like_absolute_path(text) => {
            *text = "[REDACTED]".into();
            if !unsupported.iter().any(|f| f == "absolute_path") {
                unsupported.push("absolute_path".into());
            }
        }
        _ => {}
    }
}

fn is_secret_field(key: &str) -> bool {
    let lower = key.to_ascii_lowercase().replace('-', "_");
    let names = [
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
    names
        .iter()
        .any(|name| lower == *name || lower.contains(name))
}

fn looks_like_secret_text(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let compact = lower.replace([' ', '\t'], "");
    compact.contains("api_key=")
        || compact.contains("api_key:")
        || compact.contains("token=")
        || compact.contains("token:")
        || lower.contains("bearer ")
        || compact.contains("password=")
        || compact.contains("password:")
        || compact.contains("secret=")
        || compact.contains("secret:")
        || compact.contains("credential=")
        || compact.contains("credential:")
        || {
            let bytes = text.as_bytes();
            bytes.windows(3).any(|w| w == b"sk-") && text.len() >= 24
        }
}

fn looks_like_absolute_path(text: &str) -> bool {
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

fn write_array_file(dir: &Path, filename: &str, items: &[Value]) -> Result<()> {
    let path = dir.join(filename);
    fs::write(
        &path,
        serde_json::to_vec_pretty(items).map_err(SqliteError::from)?,
    )?;
    Ok(())
}

fn compute_export_hash(tables: &[(&str, &[Value])]) -> String {
    let mut hasher = Sha256::new();
    for (name, items) in tables {
        hash_named_array(&mut hasher, name, items);
    }
    hex_encode(hasher.finalize())
}

#[allow(clippy::too_many_arguments)]
fn collect_export_hash(
    cards: &[Value],
    campaigns: &[Value],
    instances: &[Value],
    knowledge: &[Value],
    tasks: &[Value],
    summaries: &[Value],
    turns: &[Value],
    conversations: &[Value],
) -> String {
    compute_export_hash(&[
        ("cards", cards),
        ("campaigns", campaigns),
        ("instances", instances),
        ("knowledge", knowledge),
        ("tasks", tasks),
        ("round_summaries", summaries),
        ("turns", turns),
        ("conversations", conversations),
    ])
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

/// Read a reverse export manifest from disk for inspection.
pub fn read_export_manifest(path: &Path) -> Result<Map<String, Value>> {
    let raw = fs::read_to_string(path)?;
    let value: Value = serde_json::from_str(&raw)?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(SqliteError::Other(
            "export manifest is not a JSON object".into(),
        )),
    }
}
