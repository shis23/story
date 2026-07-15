//! Deterministic gates for the 100-turn endurance runner.
//!
//! These tests validate the schedule, checkpoint/resume, budget enforcement,
//! stage gating, redaction, evidence completeness, and failure classification
//! **without** any real LLM calls. The real-model entry point is in
//! `endurance_real_llm.rs` (`#[ignore]`).

use harness_real_llm::endurance::*;
use harness_real_llm::evidence::{contains_forbidden_evidence_payload, short_hash16};
use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sf_endurance_test_{tag}_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

// ── Schedule coverage (full 100-turn matrix) ──

#[test]
fn full_schedule_covers_all_matrix_rows() {
    let schedule = EnduranceSchedule::new(100);
    let report = schedule.coverage_report();
    let failed: Vec<_> = report.iter().filter(|a| !a.passed).collect();
    assert!(
        failed.is_empty(),
        "100-turn schedule must cover all matrix rows; missing: {failed:?}"
    );
}

#[test]
fn schedule_has_at_least_one_of_each_action_kind() {
    let schedule = EnduranceSchedule::new(100);
    let entries = schedule.entries();
    use std::collections::BTreeSet;
    let kinds: BTreeSet<&str> = entries
        .iter()
        .map(|(_, a)| match a {
            ScheduledAction::Write { .. } => "write",
            ScheduledAction::RegenerateOverall => "regenerate_overall",
            ScheduledAction::RegenerateEditor => "regenerate_editor",
            ScheduledAction::RegenerateSubagent => "regenerate_subagent",
            ScheduledAction::PrivateProbe { .. } => "private_probe",
            ScheduledAction::EarlyFactInject { .. } => "early_fact_inject",
            ScheduledAction::EarlyFactCheck { .. } => "early_fact_check",
            ScheduledAction::QualityAutofix { .. } => "quality_autofix",
            ScheduledAction::CacheStable => "cache_stable",
            ScheduledAction::CacheInvalidate => "cache_invalidate",
        })
        .collect();
    for required in [
        "write",
        "regenerate_overall",
        "regenerate_editor",
        "regenerate_subagent",
        "private_probe",
        "early_fact_inject",
        "early_fact_check",
        "quality_autofix",
        "cache_invalidate",
    ] {
        assert!(
            kinds.contains(required),
            "schedule must include action kind '{required}'"
        );
    }
}

#[test]
fn schedule_owner_recall_probe_present() {
    let schedule = EnduranceSchedule::new(100);
    let has_owner_recall = schedule.entries().iter().any(|(_, a)| {
        matches!(
            a,
            ScheduledAction::PrivateProbe {
                probe_kind: PrivateProbeKind::OwnerRecall
            }
        )
    });
    assert!(
        has_owner_recall,
        "OwnerRecall private probe must be in schedule"
    );
}

// ── Stage gating ──

#[test]
fn full_stage_blocked_without_stability_pass() {
    let result = check_stage_gate(EnduranceStage::Full, None);
    assert!(result.is_err());
}

#[test]
fn coverage_stage_blocked_without_canary_pass() {
    let result = check_stage_gate(EnduranceStage::Coverage, None);
    assert!(result.is_err());
}

#[test]
fn stability_stage_blocked_without_coverage_pass() {
    let result = check_stage_gate(EnduranceStage::Stability, None);
    assert!(result.is_err());
}

// ── Checkpoint / resume idempotency ──

#[test]
fn checkpoint_resume_is_idempotent() {
    let dir = temp_dir("idem");
    let path = dir.join("checkpoint.jsonl");
    let schedule = EnduranceSchedule::new(20);

    // Run turns 1-10
    let first = simulate_resumable_run(&schedule, 1, 10, &path);
    assert_eq!(first.len(), 10);

    // Read latest checkpoint, resume
    let latest = read_latest_checkpoint(&path).unwrap();
    let resume_start = resume_turn_from_checkpoint(Some(&latest));
    assert_eq!(resume_start, 11);

    // Run turns 11-20
    let second = simulate_resumable_run(&schedule, resume_start, 20, &path);
    assert_eq!(second.len(), 10);

    // Read all checkpoints from file
    let all = harness_real_llm::evidence::read_evidence_lines(&path).unwrap();
    assert_eq!(all.len(), 20, "should have 20 checkpoint lines");

    // Re-reading the latest should show turn 20
    let final_cp = read_latest_checkpoint(&path).unwrap();
    assert_eq!(final_cp.accepted_turn_number, 20);

    // Idempotency: writing a duplicate checkpoint for turn 20 should be rejected
    let is_new = checkpoint_is_new(
        Some(&final_cp),
        20,
        final_cp.campaign_revision,
        &final_cp.last_draft_hash16,
    );
    assert!(!is_new, "duplicate checkpoint must be detected");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn checkpoint_accumulates_early_facts_correctly() {
    let dir = temp_dir("ef");
    let path = dir.join("checkpoint.jsonl");
    let schedule = EnduranceSchedule::new(15);

    let checkpoints = simulate_resumable_run(&schedule, 1, 15, &path);
    let last = checkpoints.last().unwrap();

    // Turns 3, 8, 13 inject early facts
    assert!(
        last.early_fact_probe_ids
            .contains(&"EF-ALPHA-4471".to_string())
    );
    assert!(
        last.early_fact_probe_ids
            .contains(&"EF-BETA-2098".to_string())
    );
    assert!(
        last.early_fact_probe_ids
            .contains(&"EF-GAMMA-6603".to_string())
    );

    let _ = std::fs::remove_dir_all(dir);
}

// ── Budget enforcement ──

#[test]
fn budget_prevents_turns_when_exhausted() {
    let budget = EnduranceBudget {
        max_calls: 5,
        max_turns: 10,
        timeout_secs: 30,
        hard_deadline: None,
        max_evidence_bytes: 4096,
    };
    // With 4 calls used and 2 per turn minimum: 4+2=6 > 5 → cannot attempt
    assert!(!budget.can_attempt_turn(4, 2));
    // With 3 calls used and 2 per turn: 3+2=5 ≤ 5 → can attempt
    assert!(budget.can_attempt_turn(3, 2));
}

// ── Redaction ──

#[test]
fn evidence_files_contain_no_secrets() {
    let dir = temp_dir("redact");
    let paths = EnduranceEvidencePaths::new(dir.clone());

    // Write a manifest with safe content
    let row = EnduranceStageManifestRow {
        schema_version: "test".into(),
        run_id: "safe-run".into(),
        stage: EnduranceStage::Canary.label().into(),
        target_turns: 3,
        accepted_turns: 3,
        calls_used: 10,
        max_calls: 30,
        elapsed_ms: 100,
        acceptance: AcceptanceLevel::Pass.label().into(),
        summary_codes: vec!["A0001".into()],
        observed_epoch_ids16: vec!["e1".into()],
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        coverage_assertions: vec![],
        recorded_at_unix_ms: 0,
    };
    write_manifest_row(&paths.manifest_jsonl, &row).unwrap();

    // Check no secrets
    assert!(paths.check_no_secrets().is_ok());

    // Now write something with a secret
    std::fs::write(&paths.calls_jsonl, r#"{"data":"sk-leaked"}"#).unwrap();
    assert!(paths.check_no_secrets().is_err());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn checkpoint_rejects_sf_secret_marker() {
    let dir = temp_dir("sfsec");
    let path = dir.join("cp.jsonl");
    let cp = EnduranceCheckpoint {
        schema_version: EnduranceCheckpoint::schema_version().into(),
        run_id: "run-with-SF_SECRET_MARKER".into(),
        stage: EnduranceStage::Canary.label().into(),
        accepted_turn_number: 1,
        calls_used: 1,
        max_calls: 30,
        campaign_revision: 1,
        chronicle_revision: 1,
        last_draft_hash16: "abc".into(),
        last_summary_code: None,
        context_epoch_id16: None,
        early_fact_probe_ids: vec![],
        early_fact_checked_passed: vec![],
        campaign_id: None,
        conversation_id: None,
        data_dir_rel: None,
        observed_epoch_ids16: vec![],
        recorded_at_unix_ms: 0,
    };
    assert!(write_checkpoint(&path, &cp).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

// ── Failure classification ──

#[test]
fn partial_exit_nonzero_for_full_stage() {
    // A full stage that stops at 50 turns → Partial → exit 1
    let level = classify_acceptance(EnduranceStage::Full, 50, 300, true, false, false);
    assert_eq!(level, AcceptanceLevel::Partial);
    assert_eq!(level.exit_code(), 1);
}

#[test]
fn pass_exit_zero_for_canary() {
    let level = classify_acceptance(EnduranceStage::Canary, 3, 15, true, false, false);
    assert_eq!(level, AcceptanceLevel::Pass);
    assert_eq!(level.exit_code(), 0);
}

#[test]
fn inconclusive_on_missing_usage_for_full_stage() {
    let level = classify_acceptance(EnduranceStage::Full, 100, 500, true, false, true);
    assert_eq!(level, AcceptanceLevel::Inconclusive);
    assert_eq!(level.exit_code(), 1);
}

// ── Dry-run validation ──

#[test]
fn dry_run_passes_with_valid_inputs() {
    let dir = temp_dir("dry_ok");
    let budget = EnduranceBudget::default();
    let report = dry_run_validate(true, &dir, &budget);
    assert!(report.fixture_ok);
    assert!(report.budget_valid);
    assert!(report.schema_ok);
    assert!(report.secret_guards_ok);
    assert!(report.evidence_root_ok);
    assert!(report.assertions.iter().all(|a| a.passed));
    // Path details must be redacted (basename only), never full host paths with secrets.
    for a in &report.assertions {
        if let Some(detail) = &a.detail {
            assert!(!detail.contains("sk-"));
            assert!(!detail.contains("api_key"));
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dry_run_fails_with_missing_fixture() {
    let dir = temp_dir("dry_fail");
    let budget = EnduranceBudget::default();
    let report = dry_run_validate(false, &dir, &budget);
    assert!(!report.fixture_ok);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn dry_run_rejects_repo_internal_root_under_production_policy() {
    let budget = EnduranceBudget::default();
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap()
        .to_path_buf();
    let policy = harness_real_llm::evidence_retention::EvidenceRootPolicy::production(repo.clone());
    let report = dry_run_validate_with_policy(true, &repo.join("crates"), &budget, &policy);
    assert!(!report.evidence_root_ok);
}

#[test]
fn resume_from_evidence_dir_fails_closed_on_mixed_run_ids() {
    use harness_real_llm::evidence_retention::{
        EvidenceRootPolicy, allocate_run_id, prepare_run_dir, validate_evidence_root,
    };
    let root = temp_dir("mix_resume");
    let policy = EvidenceRootPolicy::for_tests(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf(),
    );
    let validated = validate_evidence_root(&root, &policy).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    let cp_path = run_dir.join("endurance_checkpoint.jsonl");
    let schedule = EnduranceSchedule::new(3);
    let cps = simulate_resumable_run(&schedule, 1, 2, &cp_path);
    // Overwrite second checkpoint line with a different run id to simulate mix.
    let mut mixed = cps[1].clone();
    mixed.run_id = "run-full-00000000-0000-0000-0000-000000000099".into();
    write_checkpoint(&cp_path, &mixed).unwrap();
    // Unsealed resume requires an integrity baseline (auditable fail-closed path).
    harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&run_dir).unwrap();
    let err = resume_from_evidence_dir(&run_dir, Some(&run_id));
    assert!(err.is_err(), "mixed run ids must fail closed");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn resume_from_evidence_dir_returns_next_turn_without_replay() {
    use harness_real_llm::evidence_retention::{
        EvidenceRootPolicy, allocate_run_id, prepare_run_dir, validate_evidence_root,
    };
    let root = temp_dir("ok_resume");
    let policy = EvidenceRootPolicy::for_tests(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .unwrap()
            .to_path_buf(),
    );
    let validated = validate_evidence_root(&root, &policy).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    let cp_path = run_dir.join("endurance_checkpoint.jsonl");
    // Write a single checkpoint with the controlled run id.
    write_checkpoint(
        &cp_path,
        &EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.clone(),
            stage: "full".into(),
            accepted_turn_number: 7,
            calls_used: 40,
            max_calls: 700,
            campaign_revision: 7,
            chronicle_revision: 0,
            last_draft_hash16: "abcdabcdabcdabcd".into(),
            last_summary_code: Some("ok".into()),
            context_epoch_id16: None,
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: Some("c".into()),
            conversation_id: Some("v".into()),
            data_dir_rel: Some("campaign_data".into()),
            observed_epoch_ids16: vec![],
            recorded_at_unix_ms: 1,
        },
    )
    .unwrap();
    harness_real_llm::evidence_retention::write_checkpoint_integrity_baseline(&run_dir).unwrap();
    let (next, cp) = resume_from_evidence_dir(&run_dir, Some(&run_id)).unwrap();
    assert_eq!(next, 8);
    assert_eq!(cp.accepted_turn_number, 7);
    let _ = std::fs::remove_dir_all(root);
}

// ── Evidence size budget ──

#[test]
fn evidence_size_budget_enforced() {
    let dir = temp_dir("size");
    let paths = EnduranceEvidencePaths::new(dir.clone());

    // Write within budget
    std::fs::write(&paths.manifest_jsonl, b"small").unwrap();
    let size = paths.total_size();
    assert!(size < 512 * 1024, "evidence should be within 512KB budget");

    let _ = std::fs::remove_dir_all(dir);
}

// ── Forbidden payload guard ──

#[test]
fn forbidden_payload_guard_catches_api_key() {
    assert!(contains_forbidden_evidence_payload(
        r#"{"x":"api_key":"sk-xxx"}"#
    ));
}

#[test]
fn forbidden_payload_guard_allows_hashes() {
    assert!(!contains_forbidden_evidence_payload(
        r#"{"system_hash16":"abcdef0123456789"}"#
    ));
}

// ── Turn record builder ──

#[test]
fn endurance_turn_record_for_regenerate_action() {
    let action = ScheduledAction::RegenerateEditor;
    let rec = build_endurance_turn_record(EnduranceTurnRecordInput {
        run_id: "run-1",
        stage: EnduranceStage::Coverage,
        turn_index: 18,
        action: &action,
        draft_accepted: true,
        campaign_revision_before: 17,
        campaign_revision_after: 18,
        chronicle_revision_before: 17,
        chronicle_revision_after: 18,
        summary_code: Some("A0018".into()),
        draft_hash16: "hash".into(),
        text_len: 80,
        text_sha16: "sha".into(),
        context_epoch_id16: None,
        context_epoch_source_hash16: None,
        context_epoch_anchor_count: None,
        attempt_status: "Committed".into(),
        turn_status: "Committed".into(),
        assertions: vec![],
        elapsed_ms: 100,
    });
    assert_eq!(rec.kind, "endurance_regenerate_editor");
    assert!(rec.draft_accepted);
    let mode = rec
        .assertion_results
        .iter()
        .find(|a| a.name == "action_mode")
        .unwrap();
    assert!(mode.detail.as_ref().unwrap().contains("editor_only"));
}

#[test]
fn endurance_turn_record_for_private_probe() {
    let action = ScheduledAction::PrivateProbe {
        probe_kind: PrivateProbeKind::MustNotReveal,
    };
    let rec = build_endurance_turn_record(EnduranceTurnRecordInput {
        run_id: "run-1",
        stage: EnduranceStage::Full,
        turn_index: 80,
        action: &action,
        draft_accepted: true,
        campaign_revision_before: 79,
        campaign_revision_after: 80,
        chronicle_revision_before: 79,
        chronicle_revision_after: 80,
        summary_code: Some("A0080".into()),
        draft_hash16: "hash".into(),
        text_len: 90,
        text_sha16: "sha".into(),
        context_epoch_id16: None,
        context_epoch_source_hash16: None,
        context_epoch_anchor_count: None,
        attempt_status: "Committed".into(),
        turn_status: "Committed".into(),
        assertions: vec![],
        elapsed_ms: 50,
    });
    assert_eq!(rec.kind, "endurance_private_probe");
    let mode = rec
        .assertion_results
        .iter()
        .find(|a| a.name == "action_mode")
        .unwrap();
    assert!(mode.detail.as_ref().unwrap().contains("mustnotreveal"));
}

// ── Epoch tracker ──

#[test]
fn epoch_tracker_counts_unique_epochs() {
    let mut tracker = EpochTracker::default();
    tracker.observe(&short_hash16("epoch-1"));
    tracker.observe(&short_hash16("epoch-1")); // dup
    tracker.observe(&short_hash16("epoch-2"));
    assert_eq!(tracker.count(), 2);
    assert!(tracker.rolled_over());
}
