//! Importer diagnostics for duplicate Attempt ownership and invalid source graphs.

use std::fs;

use serde_json::json;
use storyforge_infra_sqlite::{Database, ImportStatus, JsonImporter, SqliteError};
use tempfile::TempDir;

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn base_source(dir: &std::path::Path) {
    write_json(
        &dir.join("cards.json"),
        &json!([{ "id": "card-1", "name": "Hero" }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &json!([{
            "id": "camp-1",
            "card_id": "card-1",
            "name": "Main",
            "created_at": "t",
            "conversation_id": "conv-1",
            "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &json!({
            "id": "conv-1",
            "campaign_id": "camp-1",
            "created_at": "t",
            "updated_at": "t",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &json!([]));
    write_json(&dir.join("knowledge.json"), &json!([]));
    write_json(&dir.join("tasks.json"), &json!([]));
    write_json(&dir.join("round_summaries.json"), &json!([]));
}

#[test]
fn importer_rejects_duplicate_attempt_ownership_with_clear_diagnostics() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(
        &dir.path().join("turns.json"),
        &json!([
            {
                "turn_id": "turn-1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "input_node_id": "n1",
                "base_campaign_revision": 0,
                "status": "committed",
                "created_at": "t",
                "updated_at": "t",
                "attempts": [{
                    "attempt_id": "attempt-shared",
                    "variant_id": "v1",
                    "draft_hash": "h1",
                    "status": "committed",
                    "created_at": "t"
                }]
            },
            {
                "turn_id": "turn-2",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "input_node_id": "n2",
                "base_campaign_revision": 0,
                "status": "committed",
                "created_at": "t",
                "updated_at": "t",
                "attempts": [{
                    "attempt_id": "attempt-shared",
                    "variant_id": "v2",
                    "draft_hash": "h2",
                    "status": "committed",
                    "created_at": "t"
                }]
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("attempt-shared")
                    && (msg.contains("duplicate")
                        || msg.contains("owned")
                        || msg.contains("rehang")
                        || msg.contains("turn")),
                "{msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }

    // All-or-nothing: no business rows remain.
    let campaigns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap_or(0);
    let turns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM turns", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(campaigns, 0);
    assert_eq!(turns, 0);
}

#[test]
fn importer_rejects_partial_summary_graph_without_half_import() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(&dir.path().join("turns.json"), &json!([]));
    write_json(
        &dir.path().join("round_summaries.json"),
        &json!([
            {
                "id": "sum-a1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "content": "leaf",
                "created_at": "t",
                "level": 0,
                "covered_by": "missing-parent"
            },
            {
                "id": "sum-b1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "turn_end": 1,
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "covers": ["sum-a1", "missing-child"]
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("missing") || msg.contains("cover") || msg.contains("graph"),
                "{msg}"
            );
        }
        other => panic!("expected CorruptImportInput diagnostics, got {other}"),
    }

    let summaries: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM round_summaries", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(summaries, 0);
}

#[test]
fn importer_still_accepts_valid_source_all_or_nothing() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(&dir.path().join("turns.json"), &json!([]));
    // Consistent graph: A covered by B.
    write_json(
        &dir.path().join("round_summaries.json"),
        &json!([
            {
                "id": "sum-a1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "content": "leaf",
                "created_at": "t",
                "level": 0,
                "lineage_id": "lin-1",
                "code": "A0001",
                "covered_by": "sum-b1"
            },
            {
                "id": "sum-b1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "turn_end": 1,
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "lineage_id": "lin-1",
                "code": "B0001",
                "covers": ["sum-a1"]
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let report = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap();
    assert_eq!(report.status, ImportStatus::Completed);
    assert_eq!(report.summaries, 2);
}

#[test]
fn importer_rejects_covers_covered_by_mismatch_and_scope_drift() {
    let dir = TempDir::new().unwrap();
    base_source(dir.path());
    write_json(&dir.path().join("turns.json"), &json!([]));
    write_json(
        &dir.path().join("round_summaries.json"),
        &json!([
            {
                "id": "sum-a1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 1,
                "content": "leaf",
                "created_at": "t",
                "level": 0,
                "lineage_id": "lin-1",
                "code": "A0001",
                "covered_by": "sum-b1"
            },
            {
                "id": "sum-b1",
                "campaign_id": "camp-1",
                "conversation_id": "conv-other",
                "turn": 1,
                "turn_end": 1,
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "lineage_id": "lin-other",
                "code": "B0001",
                "covers": ["sum-a1", "sum-a2"]
            },
            {
                "id": "sum-a2",
                "campaign_id": "camp-1",
                "conversation_id": "conv-1",
                "turn": 2,
                "content": "leaf2",
                "created_at": "t",
                "level": 0,
                "lineage_id": "lin-1",
                "code": "A0002"
            }
        ]),
    );

    let mut db = Database::open_in_memory().unwrap();
    let err = JsonImporter::new(&mut db)
        .import_data_dir(dir.path())
        .unwrap_err();
    match err {
        SqliteError::CorruptImportInput(msg) => {
            assert!(
                msg.contains("cover")
                    || msg.contains("covered_by")
                    || msg.contains("scope")
                    || msg.contains("conversation")
                    || msg.contains("lineage"),
                "{msg}"
            );
        }
        other => panic!("expected CorruptImportInput, got {other}"),
    }
    let summaries: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM round_summaries", [], |r| r.get(0))
        .unwrap_or(0);
    assert_eq!(summaries, 0);
}
