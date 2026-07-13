//! Migration readiness tools: dry-run validation, backup, secret-free export.

use std::fs;
use std::path::PathBuf;

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::conversation::Conversation;
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::migrations::{
    builtin_migrations, current_version, migrate, migrate_with,
};
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use storyforge_infra_sqlite::publication::SqliteChronicleRepository;
use storyforge_infra_sqlite::readiness::{
    BackupCheckpoint, ExportSnapshot, SourceManifestReport, create_backup_checkpoint,
    export_readonly_snapshot, validate_source_manifest,
};
use tempfile::TempDir;

fn write_json(path: &std::path::Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}

fn sample_source(dir: &std::path::Path) {
    write_json(
        &dir.join("cards.json"),
        &serde_json::json!([{
            "id": "card-1",
            "name": "Hero",
            "source_character_id": null
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
fn dry_run_source_manifest_validation_reports_counts_without_writes() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let report: SourceManifestReport = validate_source_manifest(dir.path()).unwrap();
    assert_eq!(report.cards, 1);
    assert_eq!(report.campaigns, 1);
    assert_eq!(report.conversations, 1);
    assert_eq!(report.summaries, 1);
    assert!(!report.manifest_hash.is_empty());
    assert!(report.issues.is_empty());

    // Dry-run must not create a live SQLite database as a side effect.
    assert!(!dir.path().join("storyforge.sqlite3").exists());
}

#[test]
fn dry_run_flags_partial_and_invalid_source_graphs() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    write_json(
        &dir.path().join("round_summaries.json"),
        &serde_json::json!([
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
                "content": "stage",
                "created_at": "t",
                "level": 1,
                "covers": ["missing-child"]
            }
        ]),
    );
    let report = validate_source_manifest(dir.path()).unwrap();
    assert!(!report.issues.is_empty());
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.contains("missing") || i.contains("cover") || i.contains("graph")),
        "{:?}",
        report.issues
    );
}

#[test]
fn backup_checkpoint_writes_manifest_and_does_not_mutate_live_db() {
    let dir = TempDir::new().unwrap();
    let live_path = dir.path().join("live.sqlite3");
    let mut db = Database::open(&live_path).unwrap();
    migrate(&mut db).unwrap();

    let mut campaign = Campaign::new(Id::from_str("card"), "Live");
    campaign.id = Id::from_str("camp-live");
    let mut conversation = Conversation::new(None, Some(campaign.id.clone()));
    conversation.id = Id::from_str("conv-live");
    campaign.conversation_id = Some(conversation.id.clone());
    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();
    let before_version = current_version(&db).unwrap();
    let before_campaigns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap();

    let backup_dir = dir.path().join("backup");
    let checkpoint: BackupCheckpoint =
        create_backup_checkpoint(&db, &backup_dir, "pre-cutover").unwrap();
    assert!(checkpoint.backup_db_path.exists());
    assert!(checkpoint.manifest_path.exists());
    assert_eq!(checkpoint.schema_version, before_version);
    assert!(!checkpoint.manifest_hash.is_empty());

    let after_version = current_version(&db).unwrap();
    let after_campaigns: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(after_version, before_version);
    assert_eq!(after_campaigns, before_campaigns);

    // Backup itself is openable and contains the campaign row.
    let backup_db = Database::open(&checkpoint.backup_db_path).unwrap();
    let backup_campaigns: i64 = backup_db
        .connection()
        .query_row("SELECT COUNT(*) FROM campaigns", [], |r| r.get(0))
        .unwrap();
    assert_eq!(backup_campaigns, 1);
}

#[test]
fn readonly_export_contains_no_secrets_and_does_not_mutate_live_db() {
    let dir = TempDir::new().unwrap();
    let live_path = dir.path().join("live.sqlite3");
    let mut db = Database::open(&live_path).unwrap();
    migrate(&mut db).unwrap();

    let mut campaign = Campaign::new(Id::from_str("card"), "Export");
    campaign.id = Id::from_str("camp-export");
    let mut conversation = Conversation::new(None, Some(campaign.id.clone()));
    conversation.id = Id::from_str("conv-export");
    campaign.conversation_id = Some(conversation.id.clone());
    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    // Plant a secret-like string that must never appear in export artifacts.
    db.connection()
        .execute(
            "UPDATE campaigns SET payload_json = json_set(payload_json, '$.api_key', 'super-secret-key')",
            [],
        )
        .unwrap();
    let live_payload: String = db
        .connection()
        .query_row(
            "SELECT payload_json FROM campaigns WHERE campaign_id = 'camp-export'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(live_payload.contains("super-secret-key"));

    let export_dir = dir.path().join("export");
    let snapshot: ExportSnapshot = export_readonly_snapshot(&db, &export_dir).unwrap();
    assert!(snapshot.root_dir.exists());
    assert!(snapshot.manifest_path.exists());

    // Live secret remains, proving export did not rewrite the live row away.
    let live_after: String = db
        .connection()
        .query_row(
            "SELECT payload_json FROM campaigns WHERE campaign_id = 'camp-export'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(live_after, live_payload);

    let mut found_secret = false;
    for entry in walkdir_files(&export_dir) {
        let text = fs::read_to_string(&entry).unwrap_or_default();
        if text.contains("super-secret-key") || text.contains("api_key") {
            found_secret = true;
            break;
        }
    }
    assert!(
        !found_secret,
        "export must redact secrets for rollback inspection"
    );
    assert!(snapshot.redacted_fields.contains(&"api_key".to_string()));
}

#[test]
fn v1_and_v2_databases_upgrade_to_v3_publication_schema() {
    let mut db = Database::open_in_memory().unwrap();
    let migrations = builtin_migrations();
    assert!(migrations.iter().any(|m| m.version == 3));

    // V1 only
    let v1 = migrations.iter().find(|m| m.version == 1).unwrap().clone();
    assert_eq!(migrate_with(&mut db, &[v1]).unwrap(), vec![1]);
    db.connection()
        .execute(
            "INSERT INTO character_cards (card_id, name, payload_json) VALUES ('card-v1', 'V1', '{}')",
            [],
        )
        .unwrap();

    // Upgrade through V2 and V3
    assert_eq!(migrate(&mut db).unwrap(), vec![2, 3]);
    assert_eq!(current_version(&db).unwrap(), 3);
    let cards: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM character_cards WHERE card_id = 'card-v1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cards, 1);
    for table in ["mutation_commits", "chronicle_publication_jobs"] {
        let exists: i64 = db
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "missing {table}");
    }
}

#[test]
fn publication_seed_survives_v3_and_job_table_is_empty_until_publish() {
    let dir = TempDir::new().unwrap();
    let mut db = Database::open(dir.path().join("db.sqlite3")).unwrap();
    migrate(&mut db).unwrap();
    assert_eq!(current_version(&db).unwrap(), 3);

    let mut campaign = Campaign::new(Id::from_str("card"), "Jobs");
    campaign.id = Id::from_str("camp-jobs");
    let mut conversation = Conversation::new(None, Some(campaign.id.clone()));
    conversation.id = Id::from_str("conv-jobs");
    campaign.conversation_id = Some(conversation.id.clone());
    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    let summary = RoundSummary::new(
        campaign.id.clone(),
        conversation.id.clone(),
        1,
        "leaf".into(),
    );
    SqliteChronicleRepository::seed_summary(&mut db, &summary).unwrap();
    assert_eq!(
        SqliteChronicleRepository::count_publication_jobs(&db).unwrap(),
        0
    );
}

fn walkdir_files(root: &std::path::Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    fn walk(path: &std::path::Path, out: &mut Vec<PathBuf>) {
        if path.is_file() {
            out.push(path.to_path_buf());
            return;
        }
        if let Ok(entries) = fs::read_dir(path) {
            for entry in entries.flatten() {
                walk(&entry.path(), out);
            }
        }
    }
    walk(root, &mut out);
    out
}
