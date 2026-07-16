//! Privacy-safe SQLite evidence inventory.
//!
//! The snapshot is captured under one deferred transaction. Raw payloads are
//! included only in the in-process canonical SHA-256 stream; callers receive
//! counts and the digest, never story text or credentials.

use rusqlite::TransactionBehavior;
use rusqlite::types::ValueRef;
use sha2::{Digest, Sha256};

use crate::{Database, Result, migrate};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqliteAuditSnapshot {
    pub sqlite_schema_version: u32,
    pub canonical_content_sha256: String,
    pub accepted_content_sha256: String,
    pub turns: u64,
    pub attempts: u64,
    pub committed_turns: u64,
    pub outbox_rows: u64,
    pub round_summaries: u64,
    pub publication_jobs: u64,
    pub ledger_entries: u64,
}

pub fn capture_audit_snapshot(db: &mut Database) -> Result<SqliteAuditSnapshot> {
    migrate(db)?;
    let tx = db
        .connection_mut()
        .transaction_with_behavior(TransactionBehavior::Deferred)?;

    let sqlite_schema_version: i64 = tx.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    let snapshot = SqliteAuditSnapshot {
        sqlite_schema_version: sqlite_schema_version.max(0) as u32,
        canonical_content_sha256: canonical_content_hash(&tx)?,
        accepted_content_sha256: accepted_content_hash(&tx)?,
        turns: count(&tx, "turns")?,
        attempts: count(&tx, "turn_attempts")?,
        committed_turns: tx.query_row(
            "SELECT COUNT(*) FROM turns WHERE status IN ('committed', 'degraded')",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64,
        outbox_rows: count(&tx, "preaccept_outbox")?,
        round_summaries: count(&tx, "round_summaries")?,
        publication_jobs: count(&tx, "chronicle_publication_jobs")?,
        ledger_entries: count(&tx, "mutation_commits")?,
    };
    tx.commit()?;
    Ok(snapshot)
}

fn count(tx: &rusqlite::Transaction<'_>, table: &str) -> Result<u64> {
    let sql = format!("SELECT COUNT(*) FROM {}", quote_identifier(table));
    let value: i64 = tx.query_row(&sql, [], |row| row.get(0))?;
    Ok(value.max(0) as u64)
}

fn canonical_content_hash(tx: &rusqlite::Transaction<'_>) -> Result<String> {
    canonical_hash(tx, false)
}

fn accepted_content_hash(tx: &rusqlite::Transaction<'_>) -> Result<String> {
    canonical_hash(tx, true)
}

fn canonical_hash(tx: &rusqlite::Transaction<'_>, accepted_projection: bool) -> Result<String> {
    let tables = {
        let mut stmt = tx.prepare(
            "SELECT name FROM sqlite_master \
             WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };

    let mut hasher = Sha256::new();
    for table in tables {
        feed_bytes(&mut hasher, b"table");
        feed_bytes(&mut hasher, table.as_bytes());

        let mut columns = {
            let pragma = format!("PRAGMA table_info({})", quote_identifier(&table));
            let mut stmt = tx.prepare(&pragma)?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if accepted_projection && table == "conversations" {
            columns.retain(|column| column != "updated_at");
        }
        if columns.is_empty() {
            continue;
        }
        for column in &columns {
            feed_bytes(&mut hasher, column.as_bytes());
        }

        let column_list = columns
            .iter()
            .map(|column| quote_identifier(column))
            .collect::<Vec<_>>()
            .join(", ");
        let where_clause = if accepted_projection {
            match table.as_str() {
                "turns" => " WHERE status IN ('committed', 'degraded')",
                "turn_attempts" => {
                    " WHERE turn_id IN (SELECT turn_id FROM turns WHERE status IN ('committed', 'degraded'))"
                }
                "preaccept_outbox" => {
                    " WHERE turn_id IN (SELECT turn_id FROM turns WHERE status IN ('committed', 'degraded'))"
                }
                _ => "",
            }
        } else {
            ""
        };
        let query = format!(
            "SELECT {column_list} FROM {}{where_clause} ORDER BY {column_list}",
            quote_identifier(&table)
        );
        let mut stmt = tx.prepare(&query)?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            feed_bytes(&mut hasher, b"row");
            for (index, column) in columns.iter().enumerate() {
                match row.get_ref(index)? {
                    ValueRef::Null => feed_bytes(&mut hasher, b"null"),
                    ValueRef::Integer(value) => {
                        feed_bytes(&mut hasher, b"integer");
                        feed_bytes(&mut hasher, &value.to_be_bytes());
                    }
                    ValueRef::Real(value) => {
                        feed_bytes(&mut hasher, b"real");
                        feed_bytes(&mut hasher, &value.to_bits().to_be_bytes());
                    }
                    ValueRef::Text(value) => {
                        feed_bytes(&mut hasher, b"text");
                        if accepted_projection
                            && table == "conversations"
                            && column == "payload_json"
                        {
                            let mut payload: serde_json::Value = serde_json::from_slice(value)?;
                            if let Some(object) = payload.as_object_mut() {
                                object.remove("updated_at");
                            }
                            feed_bytes(&mut hasher, &serde_json::to_vec(&payload)?);
                        } else {
                            feed_bytes(&mut hasher, value);
                        }
                    }
                    ValueRef::Blob(value) => {
                        feed_bytes(&mut hasher, b"blob");
                        feed_bytes(&mut hasher, value);
                    }
                }
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn feed_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
