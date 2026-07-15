//! Fail-closed JSON → SQLite cutover tests.
//!
//! Each test proves that JSON remains authoritative until the marker is
//! published, and that interrupted cutovers recover without dual truth.

use std::fs;
use std::path::Path;

use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{
    CutoverFault, CutoverOutcome, CutoverPlan, CutoverRequest, MarkerStatus, inspect_marker,
    recover_or_verify, run_cutover, run_cutover_with_fault,
};
use storyforge_infra_sqlite::migrations::current_version;
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

fn make_plan(dir: &Path) -> CutoverPlan {
    CutoverPlan::new(dir, dir.join("storyforge.sqlite3"))
}

fn make_request(dir: &Path) -> CutoverRequest {
    CutoverRequest {
        plan: make_plan(dir),
        label: "test-cutover".into(),
    }
}

// ── Clean cutover ──────────────────────────────────────────────────

#[test]
fn clean_cutover_completes_and_writes_marker() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let outcome = run_cutover(&request).unwrap();
    match outcome {
        CutoverOutcome::Completed(report) => {
            assert_eq!(report.cards, 1);
            assert_eq!(report.campaigns, 1);
            assert_eq!(report.conversations, 1);
            assert_eq!(report.summaries, 1);
            assert!(report.schema_version >= 3);
            assert!(!report.manifest_hash.is_empty());
        }
        CutoverOutcome::AlreadyCutover(_) => panic!("first run should complete, not skip"),
    }

    // The marker must exist and claim SQLite.
    let status = inspect_marker(&request.plan);
    match status {
        MarkerStatus::SqliteAuthoritative { .. } => {}
        other => panic!("expected SqliteAuthoritative, got {other:?}"),
    }

    // The database file must exist.
    assert!(dir.path().join("storyforge.sqlite3").exists());

    // Original JSON files must be untouched.
    assert!(dir.path().join("cards.json").exists());
    assert!(dir.path().join("campaigns.json").exists());
}

#[test]
fn idempotent_restart_returns_already_cutover() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    // First run.
    let first = run_cutover(&request).unwrap();
    assert!(matches!(first, CutoverOutcome::Completed(_)));

    // Second run should detect existing marker and verify.
    let second = run_cutover(&request).unwrap();
    match second {
        CutoverOutcome::AlreadyCutover(report) => {
            assert_eq!(report.cards, 1);
        }
        CutoverOutcome::Completed(_) => panic!("second run should skip, not re-complete"),
    }
}

#[test]
fn already_cutover_survives_deleted_json_source() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());
    run_cutover(&request).unwrap();

    // Destroy the original JSON tree. SQLite is authoritative; restart must not
    // re-validate JSON or fail to start.
    fs::remove_file(dir.path().join("cards.json")).unwrap();
    fs::remove_file(dir.path().join("campaigns.json")).unwrap();
    fs::remove_dir_all(dir.path().join("conversations")).unwrap();

    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::AlreadyCutover(_)));
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

// ── Fault injection: JSON stays authoritative ──────────────────────

#[test]
fn fault_after_lock_leaves_json_authoritative() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterLock).unwrap_err();
    assert!(err.to_string().contains("after lock"));

    // No marker, no final DB.
    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
    // JSON untouched.
    assert!(dir.path().join("cards.json").exists());
}

#[test]
fn fault_after_validate_leaves_json_authoritative() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterValidate).unwrap_err();
    assert!(err.to_string().contains("after validation"));

    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
}

#[test]
fn fault_after_backup_leaves_json_authoritative() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterBackup).unwrap_err();
    assert!(err.to_string().contains("after backup"));

    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
    // JSON untouched.
    assert!(dir.path().join("cards.json").exists());
    // Temp DB discarded.
    assert!(!dir.path().join("storyforge.sqlite3.cutover-tmp").exists());
}

#[test]
fn fault_after_import_leaves_json_authoritative() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterImport).unwrap_err();
    assert!(err.to_string().contains("after import"));

    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
    // Temp DB discarded.
    assert!(!dir.path().join("storyforge.sqlite3.cutover-tmp").exists());
}

#[test]
fn fault_after_verify_leaves_json_authoritative() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterVerify).unwrap_err();
    assert!(err.to_string().contains("after verification"));

    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
}

#[test]
fn fault_after_publish_before_marker_recovers_on_restart() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterPublishBeforeMarker).unwrap_err();
    assert!(err.to_string().contains("after publish"));

    // The DB was published but marker not written.
    // inspect_marker sees Absent (no marker), so JSON is still authoritative.
    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);

    // Recovery: a clean re-run should succeed.
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));

    // Now the marker exists.
    let status = inspect_marker(&request.plan);
    assert!(matches!(status, MarkerStatus::SqliteAuthoritative { .. }));
}

#[test]
fn fault_after_marker_completes_silently() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    // The AfterMarker fault fires after the marker is written, so the
    // cutover is effectively complete but the function returns an error.
    let err = run_cutover_with_fault(&request, CutoverFault::AfterMarker).unwrap_err();
    assert!(err.to_string().contains("after marker"));

    // The marker exists and SQLite is authoritative.
    let status = inspect_marker(&request.plan);
    assert!(matches!(status, MarkerStatus::SqliteAuthoritative { .. }));

    // A recovery run should see it's already done.
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::AlreadyCutover(_)));
}

// ── Stale marker / corrupt data ────────────────────────────────────

#[test]
fn stale_marker_with_missing_db_is_rejected() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    // Write a stale marker claiming SQLite but no DB.
    let marker = serde_json::json!({
        "version": 1,
        "backend": "sqlite",
        "schema_version": 4,
        "manifest_hash": "fake",
        "created_at": "2026-07-13T00:00:00Z"
    });
    fs::write(
        dir.path().join("storyforge.backend.json"),
        serde_json::to_vec_pretty(&marker).unwrap(),
    )
    .unwrap();

    let err = run_cutover(&request).unwrap_err();
    assert!(err.to_string().contains("stale"));

    let status = inspect_marker(&request.plan);
    assert!(matches!(status, MarkerStatus::Stale { .. }));
}

#[test]
fn corrupt_json_source_is_rejected() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    // Corrupt the campaigns file.
    fs::write(dir.path().join("campaigns.json"), b"{ broken json").unwrap();

    let request = make_request(dir.path());
    let err = run_cutover(&request).unwrap_err();
    assert!(
        err.to_string().contains("import rejected")
            || err.to_string().contains("corrupt")
            || err.to_string().contains("JSON")
    );
}

#[test]
fn recovery_after_interruption_produces_valid_database() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    // Interrupt at AfterImport.
    let _ = run_cutover_with_fault(&request, CutoverFault::AfterImport);

    // Recovery run.
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));

    // Verify the database is valid and has the right schema.
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let version = current_version(&db).unwrap();
    assert!(version >= 3);
}

// ── No dual-write ──────────────────────────────────────────────────

#[test]
fn cutover_does_not_modify_json_files() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    // Record original JSON content.
    let original_cards = fs::read(dir.path().join("cards.json")).unwrap();
    let original_campaigns = fs::read(dir.path().join("campaigns.json")).unwrap();

    let request = make_request(dir.path());
    run_cutover(&request).unwrap();

    // JSON content must be byte-identical.
    assert_eq!(
        fs::read(dir.path().join("cards.json")).unwrap(),
        original_cards
    );
    assert_eq!(
        fs::read(dir.path().join("campaigns.json")).unwrap(),
        original_campaigns
    );
}

// ── No automatic deletion of user data ─────────────────────────────

#[test]
fn cutover_never_deletes_json_or_original_db() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    run_cutover(&request).unwrap();

    // JSON still there.
    assert!(dir.path().join("cards.json").exists());
    assert!(dir.path().join("campaigns.json").exists());
    assert!(
        dir.path()
            .join("conversations")
            .join("conv-1.json")
            .exists()
    );
    // SQLite DB there.
    assert!(dir.path().join("storyforge.sqlite3").exists());
}

// ── Secret redaction in report ─────────────────────────────────────

#[test]
fn cutover_report_redacts_secret_shaped_labels() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());

    // Pass a secret-shaped label (assembled at runtime for static scanners).
    let secret_label = format!("{}{}", "sk-", "super-secret-api-key-1234567890");
    let request = CutoverRequest {
        plan: make_plan(dir.path()),
        label: secret_label.clone(),
    };
    let outcome = run_cutover(&request).unwrap();
    let report = match outcome {
        CutoverOutcome::Completed(r) => r,
        _ => panic!("expected Completed"),
    };

    // The report's backup_label must be the sanitised version, not the secret.
    assert!(
        !report.backup_label.contains(&secret_label)
            && !report.backup_label.contains("super-secret-api-key"),
        "report leaked secret in backup_label: {}",
        report.backup_label
    );
    // It should be either "checkpoint" or "[REDACTED]" depending on the redaction path.
    assert!(
        report.backup_label == "checkpoint" || report.backup_label == "[REDACTED]",
        "unexpected backup_label: {}",
        report.backup_label
    );

    // Serialise the full report and verify no secret leaks.
    let json = serde_json::to_string(&report).unwrap();
    assert!(
        !json.contains("sk-super-secret"),
        "report JSON leaked secret: {json}"
    );
    // No paths in the report.
    assert!(!json.contains("C:\\"));
    assert!(!json.contains("/tmp/"));
}
