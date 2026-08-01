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
use rusqlite::OptionalExtension;
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
    /// 导出的角色库条目数（Gate 5）。
    pub characters: usize,
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
    // Campaigns are normalized so the legacy top-level `story_clock` field is
    // repaired from the authoritative variables entry before export (Gate 4).
    let campaigns = export_table_array_normalized(
        &tx,
        "campaigns",
        "campaign_id",
        &mut unsupported,
        |value| {
            normalize_campaign_story_clock(value);
        },
    )?;
    let instances =
        export_table_array(&tx, "character_instances", "instance_id", &mut unsupported)?;
    let knowledge =
        export_table_array(&tx, "character_knowledge", "knowledge_id", &mut unsupported)?;
    let tasks = export_table_array(&tx, "story_tasks", "task_id", &mut unsupported)?;
    let summaries = export_table_array(&tx, "round_summaries", "summary_id", &mut unsupported)?;
    let turns = export_table_array(&tx, "turns", "turn_id", &mut unsupported)?;
    let mvu = export_table_array(
        &tx,
        "mvu_translations",
        "source_character_id",
        &mut unsupported,
    )?;

    // 活跃 pre-accept 状态（pending outbox）无法无损表达：明确阻止导出，而不是丢弃。
    if table_exists(&tx, "preaccept_outbox")? {
        let pending: i64 = tx.query_row(
            "SELECT COUNT(*) FROM preaccept_outbox WHERE status = 'pending'",
            [],
            |row| row.get(0),
        )?;
        if pending > 0 {
            return Err(SqliteError::Other(format!(
                "refusing reverse export: {pending} pending pre-accept outbox row(s) represent an active pre-accept state that the JSON backend cannot express losslessly"
            )));
        }
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM preaccept_outbox", [], |row| {
            row.get(0)
        })?;
        if count > 0 {
            unsupported.push(format!(
                "preaccept_outbox:{count} rows (SQLite-native pre-accept recovery ledger; no JSON equivalent file)"
            ));
        } else {
            unsupported.push(
                "preaccept_outbox:empty (SQLite-native table; no JSON equivalent file)".into(),
            );
        }
    }

    // 只读恢复台账：JSON 后端无等价文件，显式分类而非静默丢弃。
    for (table, label) in [
        ("mutation_commits", "turn accept ledger"),
        ("chronicle_publication_jobs", "chronicle publication ledger"),
    ] {
        if table_exists(&tx, table)? {
            let count: i64 = tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })?;
            unsupported.push(format!(
                "{table}:{count} rows (SQLite-native {label}; no JSON equivalent file)"
            ));
        }
    }

    // Conversations are stored as individual files matching the JSON layout.
    let conversations = export_conversations(&tx, &stage_dir, &mut unsupported)?;

    // 本局世界书：JSON 布局 campaign_world_info/{campaign_id}.json（可无损表达）。
    let world_info = export_world_info(&tx, &stage_dir)?;

    // Chronicle 压缩任务：JSON 布局 compress_jobs.json（可无损表达）。
    let compress_jobs = export_compress_jobs(&tx, &mut unsupported)?;

    // 角色库：JSON 布局 characters.json（StoredCharacter 契约形态，可无损表达）。
    let characters = export_characters(&tx, &mut unsupported)?;

    // Write array files into the staging tree only.
    write_array_file(&stage_dir, "cards.json", &cards)?;
    write_array_file(&stage_dir, "campaigns.json", &campaigns)?;
    write_array_file(&stage_dir, "instances.json", &instances)?;
    write_array_file(&stage_dir, "knowledge.json", &knowledge)?;
    write_array_file(&stage_dir, "tasks.json", &tasks)?;
    write_array_file(&stage_dir, "round_summaries.json", &summaries)?;
    write_array_file(&stage_dir, "turns.json", &turns)?;
    write_array_file(&stage_dir, "mvu_translations.json", &mvu)?;
    write_array_file(&stage_dir, "compress_jobs.json", &compress_jobs)?;
    write_array_file(&stage_dir, "characters.json", &characters)?;

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
        &mvu,
        &world_info,
        &compress_jobs,
        &characters,
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
        characters: characters.len(),
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
        "extras": {
            "mvu_translations": mvu.len(),
            "campaign_world_info": world_info.len(),
            "compress_jobs": compress_jobs.len(),
            "characters": characters.len(),
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

fn table_exists(tx: &rusqlite::Transaction<'_>, table: &str) -> Result<bool> {
    let exists: Option<i64> = tx
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
            [table],
            |row| row.get(0),
        )
        .optional()?;
    Ok(exists.is_some())
}

fn export_table_array(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
) -> Result<Vec<Value>> {
    export_table_array_with(tx, table, id_column, unsupported, |_| {})
}

/// Like [`export_table_array`] but lets the caller normalize each payload
/// (e.g. story-clock authority repair) before it is written.
fn export_table_array_normalized(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
    normalize: impl Fn(&mut Value),
) -> Result<Vec<Value>> {
    export_table_array_with(tx, table, id_column, unsupported, normalize)
}

fn export_table_array_with(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    id_column: &str,
    unsupported: &mut Vec<String>,
    normalize: impl Fn(&mut Value),
) -> Result<Vec<Value>> {
    if !table_exists(tx, table)? {
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

        normalize(&mut value);

        // Redact secret-shaped values before writing to the export artifact.
        redact_secret_values(&mut value, unsupported);

        out.push(value);
    }
    Ok(out)
}

/// Gate 4 story-clock authority: repair the legacy top-level `story_clock`
/// field from the authoritative `variables` entry inside the exported payload,
/// so a reverse export never carries the stale half of the old dual
/// representation.
fn normalize_campaign_story_clock(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let Some(Value::Array(variables)) = map.get("variables") else {
        return;
    };
    let authoritative = variables.iter().find_map(|v| {
        let obj = v.as_object()?;
        if obj.get("key")?.as_str()? == "story_clock" {
            obj.get("value").and_then(Value::as_str).map(str::to_string)
        } else {
            None
        }
    });
    let Some(authoritative) = authoritative else {
        return;
    };
    match map.get_mut("story_clock") {
        Some(Value::String(field)) if field == &authoritative => {}
        Some(slot) => *slot = Value::String(authoritative),
        None => {
            map.insert("story_clock".to_string(), Value::String(authoritative));
        }
    }
}

/// Export the per-campaign world info books into `campaign_world_info/` files
/// (JSON store layout `campaign_world_info/{campaign_id}.json`).
fn export_world_info(tx: &rusqlite::Transaction<'_>, stage_dir: &Path) -> Result<Vec<Value>> {
    if !table_exists(tx, "campaign_world_info")? {
        return Ok(Vec::new());
    }
    let info_dir = stage_dir.join("campaign_world_info");
    fs::create_dir_all(&info_dir)?;

    let mut stmt = tx.prepare(
        "SELECT campaign_id, payload_json FROM campaign_world_info ORDER BY campaign_id",
    )?;
    let rows = stmt.query_map([], |row| {
        let campaign_id: String = row.get(0)?;
        let payload: String = row.get(1)?;
        Ok((campaign_id, payload))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (campaign_id, payload) = row?;
        let value: Value = serde_json::from_str(&payload).map_err(|e| {
            SqliteError::Other(format!(
                "corrupt payload_json in campaign_world_info id={campaign_id}: {e}"
            ))
        })?;
        let file = info_dir.join(format!("{campaign_id}.json"));
        fs::write(
            &file,
            serde_json::to_vec_pretty(&value).map_err(SqliteError::from)?,
        )?;
        out.push(value);
    }
    Ok(out)
}

/// Export Chronicle compress jobs as `compress_jobs.json` in the JSON
/// `CompressJob` shape (id/campaign_id/conversation_id/lineage_id/kind/status/
/// attempts/max_attempts/last_error/uncovered_*/created_at/updated_at).
fn export_compress_jobs(
    tx: &rusqlite::Transaction<'_>,
    unsupported: &mut Vec<String>,
) -> Result<Vec<Value>> {
    if !table_exists(tx, "chronicle_compress_jobs")? {
        return Ok(Vec::new());
    }
    let mut stmt = tx.prepare(
        r#"
        SELECT job_id, campaign_id, conversation_id, lineage_id, kind, status, attempts,
               max_attempts, last_error, uncovered_a_at_enqueue, uncovered_b_at_enqueue,
               created_at, updated_at
        FROM chronicle_compress_jobs ORDER BY job_id
        "#,
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
        let mut value = serde_json::json!({
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
        let obj = value.as_object_mut().expect("json! object is an object");
        if let Some(v) = conversation_id {
            obj.insert("conversation_id".to_string(), Value::String(v));
        }
        if let Some(v) = lineage_id {
            obj.insert("lineage_id".to_string(), Value::String(v));
        }
        if let Some(v) = last_error {
            obj.insert("last_error".to_string(), Value::String(v));
        }
        redact_secret_values(&mut value, unsupported);
        out.push(value);
    }
    Ok(out)
}

/// 导出角色库为 characters.json（`StoredCharacter` 契约形态
/// `{id, info, imported_at}`，与 importer 归一化及 CharacterStore 反序列化一致）。
fn export_characters(
    tx: &rusqlite::Transaction<'_>,
    unsupported: &mut Vec<String>,
) -> Result<Vec<Value>> {
    if !table_exists(tx, "characters")? {
        return Ok(Vec::new());
    }
    let mut stmt = tx.prepare(
        r#"
        SELECT character_id, info_json, imported_at
        FROM characters ORDER BY character_id
        "#,
    )?;
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
        let mut value = serde_json::json!({
            "id": character_id,
            "info": info,
            "imported_at": imported_at,
        });
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
    mvu: &[Value],
    world_info: &[Value],
    compress_jobs: &[Value],
    characters: &[Value],
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
        ("mvu_translations", mvu),
        ("campaign_world_info", world_info),
        ("compress_jobs", compress_jobs),
        ("characters", characters),
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
