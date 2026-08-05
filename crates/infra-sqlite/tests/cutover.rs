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
    // 三审3：marker 缺失 + 孤儿 StoryForge DB（带 authority_binding）→ Stale
    // （ambiguous），而非 Absent。这比「当作空白新用户」更安全：避免被当作
    // blank 并在后续被静默覆盖。
    let status = inspect_marker(&request.plan);
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "orphan DB must be Stale (ambiguous), not Absent; got {status:?}"
    );

    // Recovery: a clean re-run should succeed — the orphan DB belongs to THIS
    // cutover (same data_dir + source → same authority identity), so publish
    // lets it aside and re-publishes.
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

// ── Gate 5 审查一.2：marker 必须绑定实际数据库 ──────────────────────────

/// 与 sample_source 内容不同的第二份源（得到不同的 manifest hash）。
fn other_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        &serde_json::json!([{
            "id": "card-2", "name": "Other Hero", "source_character_id": "char-2"
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        &serde_json::json!([{
            "id": "camp-2", "card_id": "card-2", "name": "Other",
            "created_at": "2026-07-14T00:00:00Z", "revision": 0,
            "chronicle_revision": 0, "conversation_id": "conv-2", "lineage_id": "lin-2"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conv-2.json"),
        &serde_json::json!({
            "id": "conv-2", "campaign_id": "camp-2", "character_id": null,
            "created_at": "2026-07-14T00:00:00Z", "updated_at": "2026-07-14T00:00:00Z",
            "nodes": []
        }),
    );
    write_json(&dir.join("instances.json"), &serde_json::json!([]));
    write_json(&dir.join("knowledge.json"), &serde_json::json!([]));
    write_json(&dir.join("tasks.json"), &serde_json::json!([]));
    write_json(&dir.join("round_summaries.json"), &serde_json::json!([]));
    write_json(&dir.join("turns.json"), &serde_json::json!([]));
}

#[test]
fn cutover_persists_authority_binding_in_db_and_marker() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());
    match run_cutover(&request).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed, got {other:?}"),
    }

    // marker 携带 authority_id + cutover_nonce。
    let marker: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.path().join("storyforge.backend.json")).unwrap(),
    )
    .unwrap();
    let marker_aid = marker["authority_id"]
        .as_str()
        .expect("marker authority_id");
    let marker_nonce = marker["cutover_nonce"]
        .as_str()
        .expect("marker cutover_nonce");
    assert!(!marker_aid.is_empty());
    assert!(!marker_nonce.is_empty());

    // DB 记录同一身份（authority_binding 行 + import_runs 列）。
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let (db_aid, db_nonce): (String, String) = db
        .connection()
        .query_row(
            "SELECT authority_id, cutover_nonce FROM authority_binding WHERE id = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(db_aid, marker_aid, "DB authority_id must match marker");
    assert_eq!(db_nonce, marker_nonce, "DB nonce must match marker");
    let runs: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM import_runs \
             WHERE status='completed' AND authority_id = ?1 AND cutover_nonce = ?2",
            rusqlite::params![marker_aid, marker_nonce],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(runs, 1, "completed import_runs must carry the binding");
}

#[test]
fn marker_a_with_database_b_is_rejected_not_already_cutover() {
    let dir_a = TempDir::new().unwrap();
    sample_source(dir_a.path());
    let req_a = make_request(dir_a.path());
    match run_cutover(&req_a).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed, got {other:?}"),
    }

    // 第二份源在另一目录做真实导入 → 不同的 manifest hash + 身份。
    let dir_b = TempDir::new().unwrap();
    other_source(dir_b.path());
    let req_b = make_request(dir_b.path());
    match run_cutover(&req_b).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("expected Completed, got {other:?}"),
    }

    // 用 DB B 替换 dir_a 的最终 DB，保留 marker A（marker A ↔ DB B 不绑定）。
    let db_b_bytes = fs::read(dir_b.path().join("storyforge.sqlite3")).unwrap();
    fs::copy(
        dir_b.path().join("storyforge.sqlite3"),
        dir_a.path().join("storyforge.sqlite3"),
    )
    .unwrap();
    let marker_a_bytes = fs::read(dir_a.path().join("storyforge.backend.json")).unwrap();

    // inspect_marker 必须判 Stale，而不是 SqliteAuthoritative。
    let status = inspect_marker(&req_a.plan);
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "marker A + DB B must be Stale, got {status:?}"
    );

    // 启动路径必须报错（NOT AlreadyCutover），字节不得被改动。
    let err = recover_or_verify(&req_a)
        .expect_err("marker A + DB B must be rejected with an error, not AlreadyCutover");
    assert!(
        err.to_string().contains("stale"),
        "error must mention stale marker, got: {err}"
    );
    assert_eq!(
        fs::read(dir_a.path().join("storyforge.sqlite3")).unwrap(),
        db_b_bytes,
        "DB B bytes must be untouched"
    );
    assert_eq!(
        fs::read(dir_a.path().join("storyforge.backend.json")).unwrap(),
        marker_a_bytes,
        "marker A bytes must be untouched"
    );
}

#[test]
fn marker_version_newer_than_supported_is_stale() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let marker = serde_json::json!({
        "version": 999,
        "backend": "sqlite",
        "schema_version": 1,
        "manifest_hash": "whatever",
        "created_at": "2026-07-13T00:00:00Z"
    });
    fs::write(
        dir.path().join("storyforge.backend.json"),
        serde_json::to_vec_pretty(&marker).unwrap(),
    )
    .unwrap();

    let status = inspect_marker(&make_plan(dir.path()));
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "unknown/too-new marker version must be stale, got {status:?}"
    );
    let err = run_cutover(&make_request(dir.path())).unwrap_err();
    assert!(
        err.to_string().contains("stale"),
        "run_cutover must refuse a too-new marker, got: {err}"
    );
}

#[test]
fn marker_without_binding_against_db_without_completed_import_is_stale() {
    // 老式 marker（无 authority_id）配对“无 completed import”的 DB → Stale。
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let db_path = dir.path().join("storyforge.sqlite3");
    {
        let mut db = Database::open(&db_path).unwrap();
        storyforge_infra_sqlite::migrations::migrate(&mut db).unwrap();
    }
    let marker = serde_json::json!({
        "version": 1,
        "backend": "sqlite",
        "schema_version": 8,
        "manifest_hash": "legacy-hash",
        "created_at": "2026-07-13T00:00:00Z"
    });
    fs::write(
        dir.path().join("storyforge.backend.json"),
        serde_json::to_vec_pretty(&marker).unwrap(),
    )
    .unwrap();

    let status = inspect_marker(&make_plan(dir.path()));
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "marker without binding + DB without completed import must be Stale, got {status:?}"
    );
}

// ── Gate 5 审查一.5：audit 先于 marker；marker 之后无可失败步骤 ──────────

#[test]
fn fault_after_audit_before_marker_leaves_json_authoritative() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    // 新顺序：publish → audit → marker。AfterAudit 必须发生在 marker 之前：
    // 故障后不得有 marker，JSON 保持权威。
    let err = run_cutover_with_fault(&request, CutoverFault::AfterAudit).unwrap_err();
    assert!(
        err.to_string().contains("after audit"),
        "fault message must identify the audit step, got: {err}"
    );

    // DB 已发布但 marker 未写 → JSON 仍权威（marker 不存在）。
    assert!(
        dir.path().join("storyforge.sqlite3").exists(),
        "DB is published before the marker step"
    );
    assert!(
        !dir.path().join("storyforge.backend.json").exists(),
        "audit happens BEFORE marker write: no marker may exist after AfterAudit"
    );
    // 三审3：marker 缺失 + 孤儿 DB → Stale（ambiguous），而非 Absent。
    let status = inspect_marker(&request.plan);
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "orphan DB must be Stale, not Absent; got {status:?}"
    );

    // 恢复：重新跑完整 cutover 成功（发布过的自身 DB 可让位）。
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

#[test]
fn marker_write_failure_keeps_json_authoritative_and_recovers() {
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    let request = make_request(dir.path());

    // 用目录占住 marker 路径：write_marker_atomically 的 rename 必然失败。
    let marker_dir = dir.path().join("storyforge.backend.json");
    fs::create_dir(&marker_dir).unwrap();

    let err = run_cutover(&request).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("marker")
            || err.to_string().contains("rename")
            || err.to_string().contains("directory"),
        "marker write failure must surface, got: {err}"
    );

    // marker 未写（目录原样保留）；JSON 源未动。
    assert!(marker_dir.is_dir(), "marker path is still a directory");
    assert!(dir.path().join("campaigns.json").exists());

    // 清理后恢复成功。
    fs::remove_dir(&marker_dir).unwrap();
    let outcome = recover_or_verify(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    assert!(dir.path().join("storyforge.backend.json").is_file());
}

// ─── 三审3：孤儿 DB（marker 缺失 + 带 authority_binding）的判别 ─────────
//
// 正向（属于本次 cutover）：中断的 publish 残留 → Stale 但可恢复（已在
//   fault_after_publish_before_marker_recovers_on_restart 覆盖）。
// 负向（不属于本次 cutover）：把**另一个** data_dir 的 cutover 产物 DB 拷进
//   一个**不同源**的 data_dir（不同 manifest hash → 不同 authority_id）→ 必须
//   Stale 且 run_cutover fail closed，DB 字节不变（绝不静默覆盖未知库）。

#[test]
fn orphan_db_with_mismatched_identity_is_stale_and_fail_closed() {
    // 源 A：在 data_dir_A 跑完整 cutover，得到带 authority_binding 的 DB（身份 A）。
    let dir_a = TempDir::new().unwrap();
    sample_source(dir_a.path());
    let request_a = make_request(dir_a.path());
    match run_cutover(&request_a).unwrap() {
        CutoverOutcome::Completed(_) => {}
        other => panic!("cutover A must complete: {other:?}"),
    }
    let db_a = dir_a.path().join("storyforge.sqlite3");
    assert!(db_a.exists(), "DB A must exist after cutover");
    let db_a_bytes = fs::read(&db_a).unwrap();

    // 源 B：**不同**的 data_dir + 不同 JSON 内容（campaign 名不同 → 不同 manifest
    // hash → 不同身份），保留外键一致（id 不变，仅 name 改）。
    let dir_b = TempDir::new().unwrap();
    sample_source(dir_b.path());
    write_json(
        &dir_b.path().join("campaigns.json"),
        &serde_json::json!([{
            "id": "camp-1", "card_id": "card-1", "name": "DifferentNameForB",
            "created_at": "2026-07-13T00:00:00Z", "revision": 0,
            "chronicle_revision": 0, "conversation_id": "conv-1", "lineage_id": "lin-1"
        }]),
    );
    // 把 A 的 DB（身份 A）拷进 B 的目录当作「marker 缺失 + 孤儿 DB」。无 marker。
    let db_b = dir_b.path().join("storyforge.sqlite3");
    fs::write(&db_b, &db_a_bytes).unwrap();
    assert!(!dir_b.path().join("storyforge.backend.json").exists());

    // 判别：inspect_marker 必须是 Stale（孤儿 DB 检测），而非 Absent。
    let status = inspect_marker(&request_b_plan(dir_b.path()));
    assert!(
        matches!(status, MarkerStatus::Stale { .. }),
        "orphan DB must be Stale, not Absent; got {status:?}"
    );

    // run_cutover 必须 fail closed（身份不匹配，绝不覆盖未知库）。
    let request_b = make_request(dir_b.path());
    let err = run_cutover(&request_b).unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("stale backend marker") || msg.contains("refusing"),
        "mismatched orphan DB must fail closed, got: {msg}"
    );

    // DB 字节不变（未被覆盖/让位）。
    assert_eq!(
        db_a_bytes,
        fs::read(&db_b).unwrap(),
        "orphan DB bytes must be untouched when identity does not match"
    );
    // 仍未写 marker。
    assert!(!dir_b.path().join("storyforge.backend.json").exists());
}

/// helper：仅构建 plan（不跑 cutover），用于 inspect_marker。
fn request_b_plan(dir: &Path) -> CutoverPlan {
    let db_path = dir.join("storyforge.sqlite3");
    CutoverPlan::new(dir, &db_path)
}

#[test]
fn blank_new_user_without_db_is_absent_not_stale() {
    // 真正的空白新用户：无 marker 且无 DB → Absent（可正常 cutover），
    // 不被误判为孤儿。
    let dir = TempDir::new().unwrap();
    sample_source(dir.path());
    // 无 DB 文件。
    assert!(!dir.path().join("storyforge.sqlite3").exists());
    assert_eq!(
        inspect_marker(&request_b_plan(dir.path())),
        MarkerStatus::Absent,
        "blank user (no DB) must be Absent"
    );
    // cutover 正常完成。
    let request = make_request(dir.path());
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
}

// ── Gate 7: fresh-start（空白新用户 → 空 SQLite 权威）────────────────

#[test]
fn fresh_start_completes_and_writes_marker() {
    // 空目录（无任何 legacy JSON 布局）= 全新用户：直接初始化空 SQLite 权威，
    // 不导入、不报「源缺失」错误。
    let dir = TempDir::new().unwrap();
    let request = make_request(dir.path());

    let outcome = run_cutover(&request).unwrap();
    match outcome {
        CutoverOutcome::Completed(report) => {
            assert_eq!(report.cards, 0);
            assert_eq!(report.campaigns, 0);
            assert_eq!(report.conversations, 0);
            assert!(report.schema_version >= 3);
            assert!(!report.manifest_hash.is_empty());
        }
        CutoverOutcome::AlreadyCutover(_) => panic!("first fresh start should complete"),
    }

    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
    assert!(dir.path().join("storyforge.sqlite3").exists());
    // 没有任何 JSON 文件被创建（不双写）。
    assert!(!dir.path().join("cards.json").exists());
    assert!(!dir.path().join("campaigns.json").exists());
}

#[test]
fn fresh_start_is_idempotent_on_restart() {
    let dir = TempDir::new().unwrap();
    let request = make_request(dir.path());

    let first = run_cutover(&request).unwrap();
    assert!(matches!(first, CutoverOutcome::Completed(_)));

    // 重启：marker 在握 → 只审计，不重跑 fresh 初始化。
    let second = run_cutover(&request).unwrap();
    assert!(
        matches!(second, CutoverOutcome::AlreadyCutover(_)),
        "fresh start must be idempotent on restart"
    );
}

#[test]
fn fresh_start_fault_after_import_leaves_no_authority() {
    // 全新初始化在 import 阶段注入故障：不得留下 marker / 最终 DB。
    let dir = TempDir::new().unwrap();
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterImport).unwrap_err();
    assert!(err.to_string().contains("after fresh import"));

    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
    assert!(!dir.path().join("storyforge.sqlite3.cutover-tmp").exists());
}

#[test]
fn fresh_start_fault_after_publish_before_marker_recovers_on_restart() {
    // 最危险窗口：空库已发布但 marker 未写。重启后孤儿 DB 必须被识别为
    // 「本次 fresh cutover 的自身产物」（空 hash 身份派生）并恢复续跑，
    // 绝不 fail-closed 卡死新用户。
    let dir = TempDir::new().unwrap();
    let request = make_request(dir.path());

    let err = run_cutover_with_fault(&request, CutoverFault::AfterPublishBeforeMarker).unwrap_err();
    assert!(err.to_string().contains("before marker"));
    // 孤儿 DB 存在但无 marker → Stale（ambiguous），与正式 cutover 同语义。
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::Stale { .. }
    ));
    assert!(dir.path().join("storyforge.sqlite3").exists());

    // 重启：run_cutover 识别自身产物 → 恢复 → 完成。
    let outcome = run_cutover(&request).unwrap();
    assert!(matches!(outcome, CutoverOutcome::Completed(_)));
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));

    // 恢复后的库是有效的空库（schema 版本正确、计数为零）。
    let db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();
    let version = current_version(&db).unwrap();
    let count: i64 = db
        .connection()
        .query_row("SELECT COUNT(*) FROM character_cards", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    assert!(version >= 3);
}

#[test]
fn missing_collections_import_as_empty_like_json_store() {
    // Gate 7 候选周期发现 #1：JSON store 对缺失集合文件视为空。默认切换后
    // cutover 必须同口径——只存在 cards.json 的 legacy 树正常迁移，数据不丢。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &serde_json::json!([{
            "id": "card-1", "name": "Hero", "source_character_id": null
        }]),
    );
    let request = make_request(dir.path());

    let outcome = run_cutover(&request).unwrap();
    match outcome {
        CutoverOutcome::Completed(report) => {
            assert_eq!(report.cards, 1);
            assert_eq!(report.campaigns, 0);
            assert_eq!(report.turns, 0);
        }
        CutoverOutcome::AlreadyCutover(_) => panic!("first cutover should complete"),
    }
    assert!(matches!(
        inspect_marker(&request.plan),
        MarkerStatus::SqliteAuthoritative { .. }
    ));
}

#[test]
fn corrupt_legacy_file_fails_closed_never_treated_as_fresh() {
    // 有数据但坏了（文件存在且不可解析）：绝不当作空集合吞掉，也绝不当作
    // 全新用户跳过——必须 fail-closed（§12.2「不遇错静默创建空数据库」）。
    let dir = TempDir::new().unwrap();
    write_json(
        &dir.path().join("cards.json"),
        &serde_json::json!({"not": "an array"}),
    );
    let request = make_request(dir.path());

    let err = run_cutover(&request).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("source")
            || err.to_string().to_lowercase().contains("corrupt")
            || err.to_string().to_lowercase().contains("array"),
        "corrupt legacy file must fail closed, got: {err}"
    );
    assert_eq!(inspect_marker(&request.plan), MarkerStatus::Absent);
    assert!(!dir.path().join("storyforge.sqlite3").exists());
    assert!(!dir.path().join("storyforge.backend.json").exists());
}

#[test]
fn orphan_fresh_cutover_identity_does_not_match_foreign_db() {
    // 孤儿 DB 身份防护也覆盖 fresh 分支：不属于本次空 hash 身份的数据库
    // （如别处复制来的正式 cutover 产物）绝不能被当作 fresh 残留放行。
    // 先做一个**正式 cutover**（有源数据），把其 DB 复制到另一个全新目录，
    // 再对该全新目录跑 cutover → 必须 fail-closed（不是自身产物）。
    let src = TempDir::new().unwrap();
    sample_source(src.path());
    let src_request = make_request(src.path());
    run_cutover(&src_request).unwrap();

    let target = TempDir::new().unwrap();
    fs::copy(
        src.path().join("storyforge.sqlite3"),
        target.path().join("storyforge.sqlite3"),
    )
    .unwrap();
    let target_request = make_request(target.path());

    // 无 marker + 存在 StoryForge DB → Stale；不是 fresh 空库身份 → 拒绝。
    assert!(matches!(
        inspect_marker(&target_request.plan),
        MarkerStatus::Stale { .. }
    ));
    let err = run_cutover(&target_request).unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("stale"),
        "foreign StoryForge DB must fail closed, got: {err}"
    );
}
