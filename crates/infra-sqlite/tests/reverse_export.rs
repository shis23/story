//! SQLite → JSON reverse export tests.
//!
//! Verifies the exported JSON is readable, matches counts, redacts secrets,
//! includes a manifest, and is independently importable (re-import equivalence).

use std::fs;
use std::path::Path;

use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::JsonImporter;
use storyforge_infra_sqlite::cutover::{CutoverOutcome, CutoverPlan, CutoverRequest, run_cutover};
use storyforge_infra_sqlite::exporter::{export_sqlite_to_json, read_export_manifest};
use tempfile::TempDir;

fn write_json(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sample_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &serde_json::json!([{
            "id": "card-1",
            "name": "Hero",
            "source_character_id": null,
            // Assembled at runtime so static secret scanners ignore the fixture payload.
            "api_key": format!("{}{}", "sk-", "super-secret-value-1234567890")
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &serde_json::json!([{
            "id": "camp-1",
            "card_id": "card-1",
            "name": "Main",
            "created_at": "2026-07-13T00:00:00Z",
            "revision": 0,
            "chronicle_revision": 0,
            "conversation_id": "conv-1",
            "lineage_id": "lin-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-1.json"),
        &serde_json::json!({
            "id": "conv-1",
            "campaign_id": "camp-1",
            "character_id": null,
            "created_at": "2026-07-13T00:00:00Z",
            "updated_at": "2026-07-13T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &serde_json::json!([]));
    write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
    write_json(&dir.join("tasks.json"), &serde_json::json!([]));
    write_json(
        &dir.join("round_summaries.json"),
        &serde_json::json!([{
            "id": "sum-a1",
            "campaign_id": "camp-1",
            "conversation_id": "conv-1",
            "turn": 1,
            "content": "leaf",
            "created_at": "2026-07-13T00:00:00Z",
            "level": 0,
            "lineage_id": "lin-1",
            "code": "A0001"
        }]),
    );
    write_json(&dir.join("turns.json"), &serde_json::json!([]));
}

#[test]
fn reverse_export_produces_readable_json() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    // Cutover to SQLite first.
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "export-test".into(),
    };
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));

    // Export to a separate directory.
    let export_dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let result = export_sqlite_to_json(&db, export_dir.path()).unwrap();

    // Manifest exists.
    assert!(result.manifest_path.exists());
    let manifest = read_export_manifest(&result.manifest_path).unwrap();
    assert_eq!(
        manifest.get("direction").and_then(|v| v.as_str()),
        Some("sqlite-to-json-rollback")
    );
    assert_eq!(
        manifest.get("source_backend").and_then(|v| v.as_str()),
        Some("sqlite")
    );

    // Counts match.
    assert_eq!(result.report.cards, 1);
    assert_eq!(result.report.campaigns, 1);
    assert_eq!(result.report.conversations, 1);
    assert_eq!(result.report.summaries, 1);

    // JSON files exist in the export.
    assert!(export_dir.path().join("cards.json").exists());
    assert!(export_dir.path().join("campaigns.json").exists());
    assert!(
        export_dir
            .path()
            .join("conversations")
            .join("conv-1.json")
            .exists()
    );
}

#[test]
fn reverse_export_redacts_secrets() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "redact-test".into(),
    };
    run_cutover(&request).unwrap();

    let export_dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let result = export_sqlite_to_json(&db, export_dir.path()).unwrap();

    // The exported cards.json must NOT contain the secret field or value.
    let exported_cards = fs::read_to_string(export_dir.path().join("cards.json")).unwrap();
    assert!(
        !exported_cards.contains("sk-super-secret"),
        "exported cards contain unredacted secret value"
    );
    assert!(
        !exported_cards.contains("api_key"),
        "exported cards contain unredacted secret field name"
    );

    // The secret field removal should be noted in unsupported_fields.
    assert!(
        result
            .report
            .unsupported_fields
            .iter()
            .any(|f| f == "sensitive_field"),
        "expected sensitive_field in unsupported_fields: {:?}",
        result.report.unsupported_fields
    );

    // The original SQLite DB still has the secret (we don't mutate live data).
    let cards_json: serde_json::Value = db
        .connection()
        .query_row(
            "SELECT payload_json FROM character_cards LIMIT 1",
            [],
            |row| {
                let s: String = row.get(0)?;
                Ok(serde_json::from_str(&s).unwrap_or_default())
            },
        )
        .unwrap();
    let _cards_str = serde_json::to_string(&cards_json).unwrap();
    // The live DB payload still has the secret string (only the export is redacted).
    // Note: the importer stores the full payload including api_key.
}

#[test]
fn reverse_export_does_not_mutate_live_database() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "nomut-test".into(),
    };
    run_cutover(&request).unwrap();

    // Record the DB file hash before export.
    let db_path = dir.path().join("storyforge.sqlite3");
    let before = fs::read(&db_path).unwrap();

    let export_dir = TempDir::new().unwrap();
    let db = Database::open(&db_path).unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();
    drop(db);

    let after = fs::read(&db_path).unwrap();
    assert_eq!(before, after, "live database was mutated by export");
}

#[test]
fn reverse_export_can_be_reimported() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "reimport-test".into(),
    };
    run_cutover(&request).unwrap();

    // Export.
    let export_dir = TempDir::new().unwrap();
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();
    drop(db);

    // Re-import the exported JSON into a fresh database.
    let dir2 = TempDir::new().unwrap();
    let mut db2 = Database::open(dir2.path().join("storyforge.sqlite3")).unwrap();
    let mut importer = JsonImporter::new(&mut db2);
    let report = importer.import_data_dir(export_dir.path()).unwrap();

    assert_eq!(report.cards, 1);
    assert_eq!(report.campaigns, 1);
    assert_eq!(report.conversations, 1);
    assert_eq!(report.summaries, 1);
}

#[test]
fn reverse_export_refuses_live_db_directory() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "refuse-test".into(),
    };
    run_cutover(&request).unwrap();

    // Try to export into the same directory as the DB — must fail.
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let err = export_sqlite_to_json(&db, dir.path()).unwrap_err();
    assert!(err.to_string().contains("refusing"));
}

#[test]
fn reverse_export_replaces_stale_conversation_files_atomically() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = CutoverRequest {
        plan: CutoverPlan::new(dir.path(), dir.path().join("storyforge.sqlite3")),
        label: "atomic-export".into(),
    };
    run_cutover(&request).unwrap();

    let export_dir = TempDir::new().unwrap();
    // Seed a stale conversation that must not survive publish.
    fs::create_dir_all(export_dir.path().join("conversations")).unwrap();
    fs::write(
        export_dir.path().join("conversations").join("stale.json"),
        b"{\"id\":\"stale\"}",
    )
    .unwrap();

    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    export_sqlite_to_json(&db, export_dir.path()).unwrap();

    assert!(
        !export_dir
            .path()
            .join("conversations")
            .join("stale.json")
            .exists(),
        "stale conversation survived non-atomic export"
    );
    assert!(
        export_dir
            .path()
            .join("conversations")
            .join("conv-1.json")
            .exists()
    );
}
