//! Deterministic gates for M5 evidence retention / archive / cleanup.
//!
//! Zero real-model calls. Validates durable evidence root policy, unique run IDs,
//! atomic manifests, offline hash verify, archive/restore, retention namespace
//! protection, and fail-closed resume. Does not reconstruct the lost 45/100 run.

use harness_real_llm::endurance::{EnduranceCheckpoint, EnduranceEvidencePaths, write_checkpoint};
use harness_real_llm::evidence::{
    EVIDENCE_SCHEMA_VERSION, EvidenceCallRecord, EvidenceTurnRecord, EvidenceWriter,
    contains_forbidden_evidence_payload,
};
use harness_real_llm::evidence_retention::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

fn unique_dir(tag: &str) -> PathBuf {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let dir =
        std::env::temp_dir().join(format!("sf_evret_{tag}_{millis}_{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn test_policy() -> EvidenceRootPolicy {
    EvidenceRootPolicy {
        repo_root: repo_root(),
        allow_ephemeral: true,
        require_explicit: false,
    }
}

fn write_sanitized_fixture(run_dir: &Path, run_id: &str) {
    fs::create_dir_all(run_dir).unwrap();
    let paths = EnduranceEvidencePaths::new(run_dir.to_path_buf());
    let calls = EvidenceWriter::create(&paths.calls_jsonl, run_id).unwrap();
    calls
        .write_call(EvidenceCallRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: run_id.into(),
            suite: "endurance".into(),
            turn_index: 1,
            role: "editor".into(),
            tag: "t1".into(),
            streaming: true,
            request_fp16: "aaaaaaaaaaaaaaaa".into(),
            system_hash16: "bbbbbbbbbbbbbbbb".into(),
            history_hash16: "cccccccccccccccc".into(),
            tail_hash16: "dddddddddddddddd".into(),
            history_len: 2,
            tail_parts: 1,
            msg_count: 3,
            prompt_tokens: 10,
            cached_tokens: 1,
            cache_creation_tokens: 0,
            completion_tokens: 4,
            elapsed_ms: 5,
            outcome: "ok".into(),
            assertion_results: vec![],
            model_label: "mock-model".into(),
            recorded_at_unix_ms: 1,
        })
        .unwrap();

    let turns = EvidenceWriter::create(&paths.turns_jsonl, run_id).unwrap();
    turns
        .write_turn(EvidenceTurnRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: run_id.into(),
            suite: "endurance".into(),
            turn_index: 1,
            kind: "write".into(),
            write_path: "pipeline".into(),
            chronicle_path: "none".into(),
            accept_path: "faithful".into(),
            production_postprocess_complete: false,
            draft_accepted: true,
            force_accept: false,
            quality_error_count: 0,
            quality_warning_count: 0,
            autofix_attempts: 0,
            campaign_revision_before: 0,
            campaign_revision_after: 1,
            chronicle_revision_before: 0,
            chronicle_revision_after: 0,
            summary_code: Some("ok".into()),
            attempt_status: "accepted".into(),
            turn_status: "accepted".into(),
            draft_hash16: "eeeeeeeeeeeeeeee".into(),
            text_len: 12,
            text_sha16: "ffffffffffffffff".into(),
            early_fact_reachable: None,
            context_epoch_id16: Some("1111111111111111".into()),
            context_epoch_source_hash16: None,
            context_epoch_anchor_count: Some(1),
            assertion_results: vec![],
            elapsed_ms: 9,
            recorded_at_unix_ms: 2,
        })
        .unwrap();

    write_checkpoint(
        &paths.checkpoint_jsonl,
        &EnduranceCheckpoint {
            schema_version: EnduranceCheckpoint::schema_version().into(),
            run_id: run_id.into(),
            stage: "canary".into(),
            accepted_turn_number: 1,
            calls_used: 1,
            max_calls: 30,
            campaign_revision: 1,
            chronicle_revision: 0,
            last_draft_hash16: "eeeeeeeeeeeeeeee".into(),
            last_summary_code: Some("ok".into()),
            context_epoch_id16: Some("1111111111111111".into()),
            early_fact_probe_ids: vec![],
            early_fact_checked_passed: vec![],
            campaign_id: Some("camp-1".into()),
            conversation_id: Some("conv-1".into()),
            data_dir_rel: Some("campaign_snapshot".into()),
            observed_epoch_ids16: vec!["1111111111111111".into()],
            recorded_at_unix_ms: 3,
        },
    )
    .unwrap();

    // Non-sensitive campaign snapshot marker (relative only).
    let snap = run_dir.join("campaign_snapshot");
    fs::create_dir_all(&snap).unwrap();
    fs::write(
        snap.join("marker.json"),
        r#"{"schema_version":"campaign-snapshot-marker-v1","kind":"empty"}"#,
    )
    .unwrap();
}

// ── Evidence root policy ──

#[test]
fn rejects_repo_internal_evidence_root() {
    let repo = repo_root();
    let inside = repo.join("crates").join("harness-real-llm");
    let err = validate_evidence_root(&inside, &test_policy()).unwrap_err();
    assert!(
        matches!(err, EvidenceRetentionError::IllegalEvidenceRoot { .. }),
        "repo-internal root must fail closed: {err}"
    );
    assert!(!err.to_string().contains("sk-"));
}

#[test]
fn rejects_live_campaign_data_dir() {
    let live = repo_root().join("data");
    let err = validate_evidence_root(&live, &test_policy()).unwrap_err();
    assert!(matches!(
        err,
        EvidenceRetentionError::IllegalEvidenceRoot { .. }
    ));
}

#[test]
fn rejects_path_escape_components() {
    let base = unique_dir("escape");
    let sneaky = base.join("..").join("outside");
    let policy = test_policy();
    // Even with allow_ephemeral, escape must fail if the resolved path leaves the
    // intended parent or contains `..` in the requested form when not fully
    // normalized under a permitted root.
    let _ = (&sneaky, &policy);
    let err = reject_path_traversal(sneaky.to_string_lossy().as_ref()).unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::PathTraversal { .. }));
    let _ = fs::remove_dir_all(base);
}

#[test]
fn rejects_ephemeral_temp_root_when_not_allowed() {
    let dir = unique_dir("eph");
    let policy = EvidenceRootPolicy {
        repo_root: repo_root(),
        allow_ephemeral: false,
        require_explicit: true,
    };
    let err = validate_evidence_root(&dir, &policy).unwrap_err();
    assert!(matches!(
        err,
        EvidenceRetentionError::IllegalEvidenceRoot { .. }
    ));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn accepts_explicit_ephemeral_root_for_tests() {
    let dir = unique_dir("ok");
    let root = validate_evidence_root(&dir, &test_policy()).expect("test root ok");
    assert!(root.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn resolve_requires_explicit_root_by_default() {
    let policy = EvidenceRootPolicy {
        repo_root: repo_root(),
        allow_ephemeral: false,
        require_explicit: true,
    };
    let err = resolve_evidence_root(None, None, &policy).unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::MissingEvidenceRoot));
}

// ── Unique run id + layout ──

#[test]
fn allocate_run_id_is_unique_and_namespaced() {
    let root = unique_dir("ids");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let a = allocate_run_id(&validated, "canary").unwrap();
    let b = allocate_run_id(&validated, "canary").unwrap();
    assert_ne!(a, b);
    assert!(a.starts_with("run-canary-"));
    assert!(is_controlled_run_dirname(&run_dirname_for_id(&a)));
    // Creating both run dirs must succeed without collision.
    let da = prepare_run_dir(&validated, &a).unwrap();
    let db = prepare_run_dir(&validated, &b).unwrap();
    assert!(da.exists());
    assert!(db.exists());
    assert_ne!(da, db);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn prepare_run_dir_fails_closed_on_duplicate_run_id() {
    let root = unique_dir("dup");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let id = allocate_run_id(&validated, "full").unwrap();
    prepare_run_dir(&validated, &id).unwrap();
    let err = prepare_run_dir(&validated, &id).unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::DuplicateRunId { .. }));
    let _ = fs::remove_dir_all(root);
}

// ── Seal / verify / state machine ──

#[test]
fn seal_completed_run_writes_atomic_manifest_with_hashes() {
    let root = unique_dir("seal");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    let summary = BudgetSummary {
        max_calls: 30,
        max_turns: 3,
        timeout_secs: 120,
        max_tokens: None,
    };
    let manifest = seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock-model".into(),
            budget: summary,
            commit: "deadbeef".into(),
            branch: "codex/m5-evidence-retention".into(),
        },
    )
    .expect("seal");

    assert_eq!(manifest.schema_version, RETENTION_SCHEMA_VERSION);
    assert_eq!(manifest.run_id, run_id);
    assert_eq!(manifest.status, RunStatus::Completed);
    assert!(!manifest.files.is_empty());
    for f in &manifest.files {
        assert!(!f.relative_path.contains('\\') || f.relative_path.contains('/'));
        assert!(!Path::new(&f.relative_path).is_absolute());
        assert_eq!(f.sha256.len(), 64);
        // No absolute host paths in relative path field.
        assert!(!f.relative_path.contains(":\\"));
        assert!(!f.relative_path.starts_with('/'));
    }

    let on_disk = run_dir.join(RUN_MANIFEST_FILE);
    assert!(on_disk.exists());
    let raw = fs::read_to_string(&on_disk).unwrap();
    assert!(!contains_forbidden_evidence_payload(&raw));
    assert!(!raw.contains("sk-"));
    assert!(!raw.to_ascii_lowercase().contains("api_key"));

    let verified = verify_run(&run_dir).expect("verify");
    assert_eq!(verified.run_id, run_id);
    assert_eq!(verified.status, RunStatus::Completed);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn interrupted_and_resume_state_transitions_without_replaying_accepts() {
    let root = unique_dir("resume");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    let summary = BudgetSummary {
        max_calls: 700,
        max_turns: 100,
        timeout_secs: 180,
        max_tokens: Some(512),
    };
    seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Interrupted,
            stage: "full".into(),
            model_label: "mock-model".into(),
            budget: summary.clone(),
            commit: "cafebabe".into(),
            branch: "codex/m5-evidence-retention".into(),
        },
    )
    .unwrap();

    let resume = load_resume_context(&run_dir, &run_id).expect("resume context");
    assert_eq!(resume.run_id, run_id);
    assert_eq!(resume.accepted_turn_number, 1);
    assert_eq!(resume.next_turn, 2);
    assert_eq!(resume.status, RunStatus::Interrupted);

    // Mark completed after simulated continuation without replaying turn 1.
    let manifest = seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "full".into(),
            model_label: "mock-model".into(),
            budget: summary,
            commit: "cafebabe".into(),
            branch: "codex/m5-evidence-retention".into(),
        },
    )
    .unwrap();
    assert_eq!(manifest.status, RunStatus::Completed);
    let _ = fs::remove_dir_all(root);
}

// ── Fail closed ──

#[test]
fn verify_fails_closed_on_missing_checkpoint() {
    let root = unique_dir("misscp");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 30,
                max_turns: 3,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap();
    fs::remove_file(run_dir.join("endurance_checkpoint.jsonl")).unwrap();
    let err = verify_run(&run_dir).unwrap_err();
    assert!(matches!(
        err,
        EvidenceRetentionError::MissingRequiredFile { .. }
            | EvidenceRetentionError::HashMismatch { .. }
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_fails_closed_on_tampered_hash() {
    let root = unique_dir("tamp");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 30,
                max_turns: 3,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap();
    let calls = run_dir.join("endurance_calls.jsonl");
    let mut body = fs::read_to_string(&calls).unwrap();
    body.push('\n');
    fs::write(&calls, body).unwrap();
    let err = verify_run(&run_dir).unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::HashMismatch { .. }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_fails_closed_on_mixed_run_ids() {
    let root = unique_dir("mix");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    // Append a second call with a different run_id.
    let calls =
        EvidenceWriter::open_append(run_dir.join("endurance_calls.jsonl"), "other-run").unwrap();
    calls
        .write_call(EvidenceCallRecord {
            schema_version: EVIDENCE_SCHEMA_VERSION.into(),
            run_id: "other-run".into(),
            suite: "endurance".into(),
            turn_index: 2,
            role: "editor".into(),
            tag: "t2".into(),
            streaming: false,
            request_fp16: "aaaaaaaaaaaaaaaa".into(),
            system_hash16: "bbbbbbbbbbbbbbbb".into(),
            history_hash16: "cccccccccccccccc".into(),
            tail_hash16: "dddddddddddddddd".into(),
            history_len: 1,
            tail_parts: 1,
            msg_count: 2,
            prompt_tokens: 1,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            completion_tokens: 1,
            elapsed_ms: 1,
            outcome: "ok".into(),
            assertion_results: vec![],
            model_label: "mock".into(),
            recorded_at_unix_ms: 9,
        })
        .unwrap();
    let err = seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 30,
                max_turns: 3,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::MixedRunId { .. }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_fails_closed_on_schema_drift() {
    let root = unique_dir("schema");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 30,
                max_turns: 3,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap();
    let manifest_path = run_dir.join(RUN_MANIFEST_FILE);
    let mut v: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    v["schema_version"] = serde_json::Value::String("not-a-real-schema".into());
    fs::write(&manifest_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
    let err = verify_run(&run_dir).unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::SchemaMismatch { .. }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resume_fails_closed_when_expected_run_id_mismatches() {
    let root = unique_dir("resmix");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Interrupted,
            stage: "full".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 700,
                max_turns: 100,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap();
    let err = load_resume_context(&run_dir, "run-full-someone-else").unwrap_err();
    assert!(matches!(
        err,
        EvidenceRetentionError::MixedRunId { .. } | EvidenceRetentionError::RunIdMismatch { .. }
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn seal_refuses_secret_payload() {
    let root = unique_dir("secret");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    fs::write(
        run_dir.join("leaky.txt"),
        "api_key=sk-this-must-never-be-archived",
    )
    .unwrap();
    let err = seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 30,
                max_turns: 3,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap_err();
    assert!(matches!(
        err,
        EvidenceRetentionError::ForbiddenPayload { .. }
    ));
    assert!(!err.to_string().contains("sk-this-must-never"));
    let _ = fs::remove_dir_all(root);
}

// ── Archive / restore ──

#[test]
fn archive_restore_and_rehash_roundtrip() {
    let root = unique_dir("arch");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    let sealed = seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 30,
                max_turns: 3,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: "abc".into(),
            branch: "codex/m5-evidence-retention".into(),
        },
    )
    .unwrap();

    let archive_root = validated.join("archives");
    let archived = archive_run(&run_dir, &archive_root).expect("archive");
    assert!(archived.exists());
    // Archive must not embed absolute host paths or secrets.
    let archived_manifest = fs::read_to_string(archived.join(RUN_MANIFEST_FILE)).unwrap();
    assert!(!contains_forbidden_evidence_payload(&archived_manifest));
    assert!(!archived_manifest.contains(root.to_string_lossy().as_ref()));

    let restore_root = validated.join("restore");
    let restored = restore_run(&archived, &restore_root).expect("restore");
    let verified = verify_run(&restored).expect("re-hash after restore");
    assert_eq!(verified.run_id, sealed.run_id);
    assert_eq!(verified.files.len(), sealed.files.len());
    for (a, b) in sealed.files.iter().zip(verified.files.iter()) {
        assert_eq!(a.relative_path, b.relative_path);
        assert_eq!(a.sha256, b.sha256);
    }
    let _ = fs::remove_dir_all(root);
}

// ── Retention ──

#[test]
fn retention_only_deletes_completed_namespaced_runs_and_protects_active() {
    let root = unique_dir("ret");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();

    let mut completed = Vec::new();
    for stage in ["canary", "coverage", "stability"] {
        let id = allocate_run_id(&validated, stage).unwrap();
        let dir = prepare_run_dir(&validated, &id).unwrap();
        write_sanitized_fixture(&dir, &id);
        seal_run(
            &dir,
            SealOptions {
                run_id: id.clone(),
                status: RunStatus::Completed,
                stage: stage.into(),
                model_label: "mock".into(),
                budget: BudgetSummary {
                    max_calls: 10,
                    max_turns: 1,
                    timeout_secs: 10,
                    max_tokens: None,
                },
                commit: String::new(),
                branch: String::new(),
            },
        )
        .unwrap();
        // Stagger mtime ordering via tiny rewrite of marker.
        std::thread::sleep(std::time::Duration::from_millis(15));
        completed.push(dir);
    }

    let active_id = allocate_run_id(&validated, "full").unwrap();
    let active_dir = prepare_run_dir(&validated, &active_id).unwrap();
    write_sanitized_fixture(&active_dir, &active_id);
    seal_run(
        &active_dir,
        SealOptions {
            run_id: active_id.clone(),
            status: RunStatus::Running,
            stage: "full".into(),
            model_label: "mock".into(),
            budget: BudgetSummary {
                max_calls: 700,
                max_turns: 100,
                timeout_secs: 60,
                max_tokens: None,
            },
            commit: String::new(),
            branch: String::new(),
        },
    )
    .unwrap();

    // Unknown / non-namespace directory must never be selected.
    let unknown = validated.join("scratch-notes");
    fs::create_dir_all(&unknown).unwrap();
    fs::write(unknown.join("note.txt"), "keep me").unwrap();

    let plan = plan_retention_cleanup(
        &validated,
        RetentionOptions {
            keep: 1,
            protect_run_ids: vec![active_id.clone()],
            only_statuses: vec![RunStatus::Completed],
        },
    )
    .expect("plan");
    assert!(
        !plan.targets.is_empty(),
        "expected older completed runs to be cleanup candidates: {plan:?}"
    );
    for t in &plan.targets {
        assert!(is_controlled_run_dirname(
            t.file_name().and_then(|s| s.to_str()).unwrap_or("")
        ));
        assert_ne!(t, &active_dir);
        assert_ne!(t, &unknown);
    }

    let removed = apply_retention_cleanup(&validated, &plan).expect("apply");
    assert!(!removed.is_empty());
    assert!(active_dir.exists(), "active run must be protected");
    assert!(unknown.exists(), "unknown dirs must not be deleted");
    assert!(
        unknown.join("note.txt").exists(),
        "unknown dir content must remain"
    );

    // Path traversal target rejection (explicit raw + PathBuf forms).
    let sneaky_raw = format!("{}{}..", validated.display(), std::path::MAIN_SEPARATOR);
    let err = plan_retention_cleanup(
        Path::new(&sneaky_raw),
        RetentionOptions {
            keep: 0,
            protect_run_ids: vec![],
            only_statuses: vec![RunStatus::Completed],
        },
    );
    assert!(
        err.is_err(),
        "retention must reject parent traversal root, got ok: {sneaky_raw}"
    );
    let err2 = plan_retention_cleanup(
        &validated.join("..").join("escape-target"),
        RetentionOptions {
            keep: 0,
            protect_run_ids: vec![],
            only_statuses: vec![RunStatus::Completed],
        },
    );
    assert!(
        err2.is_err(),
        "joined ParentDir retention root must fail closed"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn retention_refuses_path_outside_namespace() {
    let root = unique_dir("ret2");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let outside = unique_dir("outside");
    let plan = RetentionPlan {
        root: validated.clone(),
        targets: vec![outside.clone()],
    };
    let err = apply_retention_cleanup(&validated, &plan).unwrap_err();
    assert!(matches!(
        err,
        EvidenceRetentionError::PathOutsideRoot { .. }
            | EvidenceRetentionError::IllegalEvidenceRoot { .. }
            | EvidenceRetentionError::UncontrolledPath { .. }
    ));
    assert!(outside.exists());
    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn error_messages_never_echo_credentials() {
    let err = EvidenceRetentionError::ForbiddenPayload {
        relative_path: "calls.jsonl".into(),
    };
    let s = err.to_string();
    assert!(!s.contains("sk-"));
    assert!(!s.to_ascii_lowercase().contains("api_key"));
    assert!(!s.contains("Bearer "));
}
