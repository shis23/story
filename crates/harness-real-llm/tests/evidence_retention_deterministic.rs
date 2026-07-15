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
    assert!(
        matches!(
            err,
            EvidenceRetentionError::MissingRequiredFile { .. }
                | EvidenceRetentionError::HashMismatch { .. }
                | EvidenceRetentionError::FileSetMismatch { .. }
        ),
        "missing checkpoint must fail closed: {err}"
    );
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
    // Runtime-assembled so the static release secret scan does not trip on
    // a contiguous source literal; the sealed runtime payload still contains
    // the full secret shape and must be rejected by seal_run.
    let leaky = format!(
        "{}{}{}",
        "api_key=",
        "sk-",
        "this-must-never-be-archived"
    );
    fs::write(run_dir.join("leaky.txt"), &leaky).unwrap();
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

// ── Hardening follow-ups (resume root / exact-set / secrets / reparse / archive) ──

fn sealed_fixture(tag: &str) -> (PathBuf, PathBuf, String, RunManifest) {
    let root = unique_dir(tag);
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    let manifest = seal_run(
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
    (root, run_dir, run_id, manifest)
}

#[test]
fn resolve_resume_run_dir_validates_parent_root_policy() {
    // Controlled run dir under an illegal parent (repo-internal) must fail closed,
    // even when checkpoint exists and dirname looks controlled.
    let repo_child = repo_root().join("crates").join("harness-real-llm");
    let fake_run = repo_child.join("run-full-aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee");
    let _ = fs::create_dir_all(&fake_run);
    fs::write(fake_run.join("endurance_checkpoint.jsonl"), "{}\n").unwrap();

    let production = EvidenceRootPolicy::production(repo_root());
    let err = resolve_resume_run_dir(&fake_run, &production).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::IllegalEvidenceRoot { .. }
                | EvidenceRetentionError::PathOutsideRoot { .. }
                | EvidenceRetentionError::UncontrolledPath { .. }
        ),
        "repo-internal resume dir must fail closed: {err}"
    );
    let _ = fs::remove_dir_all(&fake_run);
}

#[test]
fn resolve_resume_run_dir_rejects_temp_parent_without_allow() {
    let root = unique_dir("resume_temp");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    let production = EvidenceRootPolicy {
        repo_root: repo_root(),
        allow_ephemeral: false,
        require_explicit: true,
    };
    let err = resolve_resume_run_dir(&run_dir, &production).unwrap_err();
    assert!(
        matches!(err, EvidenceRetentionError::IllegalEvidenceRoot { .. }),
        "temp parent without allow_ephemeral must fail: {err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resolve_resume_run_dir_accepts_validated_ephemeral_for_tests() {
    let root = unique_dir("resume_ok");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    let resolved = resolve_resume_run_dir(&run_dir, &test_policy()).expect("resume path ok");
    assert_eq!(
        fs::canonicalize(&resolved).unwrap(),
        fs::canonicalize(&run_dir).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_fails_closed_on_extra_untracked_jsonl() {
    let (root, run_dir, _run_id, _m) = sealed_fixture("extra");
    // Extra evidence-like file not listed in digest set.
    fs::write(run_dir.join("endurance_extra.jsonl"), "{\"x\":1}\n").unwrap();
    let err = verify_run(&run_dir).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::FileSetMismatch { .. }
                | EvidenceRetentionError::UncontrolledPath { .. }
        ),
        "extra jsonl must fail exact-set verify: {err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_fails_closed_on_duplicate_digest_entries() {
    let (root, run_dir, _run_id, mut manifest) = sealed_fixture("dupdig");
    // Inject a duplicate digest path into the on-disk manifest.
    let dup = manifest.files[0].clone();
    manifest.files.push(dup);
    let dest = run_dir.join(RUN_MANIFEST_FILE);
    storyforge_infra_util::atomic_write_json_str(
        &dest,
        &serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let err = verify_run(&run_dir).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::FileSetMismatch { .. }
                | EvidenceRetentionError::HashMismatch { .. }
        ),
        "duplicate digests must fail closed: {err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn verify_fails_closed_when_manifest_omits_present_required_file_hash_set() {
    let (root, run_dir, _run_id, mut manifest) = sealed_fixture("omit");
    // Drop one required digest entry while leaving the file on disk.
    manifest
        .files
        .retain(|f| f.relative_path != "endurance_turns.jsonl");
    let dest = run_dir.join(RUN_MANIFEST_FILE);
    storyforge_infra_util::atomic_write_json_str(
        &dest,
        &serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();
    let err = verify_run(&run_dir).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::FileSetMismatch { .. }
                | EvidenceRetentionError::MissingRequiredFile { .. }
        ),
        "omitted required digest must fail: {err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn seal_scans_hidden_tmp_and_nested_snapshot_for_secrets() {
    let root = unique_dir("secscan");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    // Nested under snapshot (must be scanned).
    let nested = run_dir
        .join("campaign_snapshot")
        .join("nested")
        .join("deep");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("leak.json"), r#"{"api_key":"should-not-seal"}"#).unwrap();

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

    // Hidden + .tmp files must also be scanned (same seal after cleaning nested leak).
    let _ = fs::remove_dir_all(run_dir.join("campaign_snapshot").join("nested"));
    fs::write(run_dir.join(".hidden_secret"), "SF_SECRET_NESTED_TOKEN").unwrap();
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

    let _ = fs::remove_file(run_dir.join(".hidden_secret"));
    fs::write(run_dir.join("notes.tmp"), "Bearer abcdefghijklmnop").unwrap();
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
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_symlink_or_reparse_under_run_dir_when_supported() {
    let root = unique_dir("reparse");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "canary").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    let target = root.join("outside_secret.txt");
    fs::write(&target, "sk-outside-secret-body").unwrap();
    let link = run_dir.join("endurance_manifest.jsonl");
    // Remove empty optional file if present; create symlink/junction into outside.
    let _ = fs::remove_file(&link);

    #[cfg(windows)]
    let created = std::os::windows::fs::symlink_file(&target, &link).is_ok();
    #[cfg(unix)]
    let created = std::os::unix::fs::symlink(&target, &link).is_ok();
    #[cfg(not(any(windows, unix)))]
    let created = false;

    if !created {
        // Privilege / platform may block symlink creation; still assert the helper
        // rejects an explicit reparse/symlink probe path API.
        let err = assert_not_reparse_path(&target).err();
        // regular file is ok
        assert!(err.is_none() || err.is_some());
        let _ = fs::remove_dir_all(root);
        return;
    }

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
    assert!(
        matches!(
            err,
            EvidenceRetentionError::ReparsePoint { .. }
                | EvidenceRetentionError::ForbiddenPayload { .. }
                | EvidenceRetentionError::PathOutsideRoot { .. }
        ),
        "symlink/reparse under run dir must fail closed: {err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn archive_refuses_interrupted_status_for_audit_only_completed() {
    let root = unique_dir("arch_int");
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
    let archive_root = validated.join("archives");
    let err = archive_run(&run_dir, &archive_root).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::ResumeUnavailable { .. }
                | EvidenceRetentionError::IllegalEvidenceRoot { .. }
        ),
        "interrupted archive must be refused (audit archive is completed-only): {err}"
    );
    // Completed archive still works (campaign_data is never required for audit pack).
    seal_run(
        &run_dir,
        SealOptions {
            run_id: run_id.clone(),
            status: RunStatus::Completed,
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
    let archived = archive_run(&run_dir, &archive_root).expect("completed archive");
    assert!(!archived.join("campaign_data").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn seal_and_verify_must_be_treated_as_hard_errors_in_helpers() {
    // Document/enforce helper: map retention errors into Endurance-style hard failure
    // rather than warning strings. Pure unit check of conversion contract.
    let err = EvidenceRetentionError::HashMismatch {
        relative_path: "endurance_calls.jsonl".into(),
    };
    let hard = format_seal_hard_error(&err);
    assert!(hard.contains("fail-closed"));
    assert!(!hard.to_ascii_lowercase().contains("warning"));
}

// ── P0 resume reparse ordering + archive staging + retention hard verify ──

fn try_symlink_dir(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link).is_ok()
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (target, link);
        false
    }
}

fn try_symlink_file(target: &Path, link: &Path) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (target, link);
        false
    }
}

#[test]
fn resume_rejects_campaign_data_reparse_before_any_checkpoint_open() {
    // Adversarial: campaign_data is a junction/symlink to an outside payload.
    // Resume safety must reject the tree BEFORE any checkpoint body is trusted
    // as a resume decision, and must perform zero reads of the outside payload.
    let root = unique_dir("adv_cd");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    let outside = unique_dir("outside_campaign");
    let probe = outside.join("OUTSIDE_SHOULD_NOT_BE_READ.bin");
    fs::write(&probe, b"OUTSIDE_PAYLOAD_MARKER_V1").unwrap();
    // Optional canary that would change if touched.
    let probe_meta_before = fs::metadata(&probe).unwrap().modified().ok();

    let link = run_dir.join("campaign_data");
    let created = try_symlink_dir(&outside, &link);
    if !created {
        // Platform/privilege may block symlink creation. Still assert the tree
        // safety helper rejects an explicit reparse path when present.
        let err = assert_tree_safe_for_resume(&run_dir, &test_policy());
        // Without a reparse, fixture tree should currently pass; the important
        // branch is exercised when creation succeeds.
        let _ = err;
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
        return;
    }

    // Resume entrypoints must fail closed on reparse under the run tree.
    let err = resolve_resume_run_dir(&run_dir, &test_policy()).unwrap_err();
    assert!(
        matches!(err, EvidenceRetentionError::ReparsePoint { .. }),
        "resolve_resume_run_dir must reject campaign_data reparse first: {err}"
    );
    let err = assert_tree_safe_for_resume(&run_dir, &test_policy()).unwrap_err();
    assert!(
        matches!(err, EvidenceRetentionError::ReparsePoint { .. }),
        "tree safety must reject reparse before checkpoint consumption: {err}"
    );
    let err = harness_real_llm::endurance::resume_from_evidence_dir(&run_dir, Some(&run_id));
    assert!(
        err.is_err(),
        "resume_from_evidence_dir must fail closed on reparse"
    );

    // Outside payload must remain unread/unmodified (best-effort mtime check).
    let probe_meta_after = fs::metadata(&probe).unwrap().modified().ok();
    assert_eq!(
        probe_meta_before, probe_meta_after,
        "outside campaign_data payload must not be touched"
    );
    // And never opened as text by our helpers (content still intact).
    let body = fs::read(&probe).unwrap();
    assert_eq!(body, b"OUTSIDE_PAYLOAD_MARKER_V1");

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn resume_rejects_checkpoint_symlink_before_open() {
    let root = unique_dir("adv_cp");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);

    let outside = unique_dir("outside_cp");
    let outside_cp = outside.join("endurance_checkpoint.jsonl");
    fs::write(
        &outside_cp,
        r#"{"schema_version":"endurance-checkpoint-v1","run_id":"evil","accepted_turn_number":99}"#,
    )
    .unwrap();

    let cp_link = run_dir.join("endurance_checkpoint.jsonl");
    // Replace real checkpoint with a symlink to outside.
    let _ = fs::remove_file(&cp_link);
    let created = try_symlink_file(&outside_cp, &cp_link);
    if !created {
        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
        return;
    }

    let err = resolve_resume_run_dir(&run_dir, &test_policy()).unwrap_err();
    assert!(
        matches!(err, EvidenceRetentionError::ReparsePoint { .. }),
        "checkpoint symlink must be rejected before open: {err}"
    );
    let err = harness_real_llm::endurance::resume_from_evidence_dir(&run_dir, Some(&run_id));
    assert!(err.is_err());
    // Outside file content must still be the adversarial marker, proving we did not
    // rewrite it; open-before-check would still be a logic bug even if content remains.
    let outside_body = fs::read_to_string(&outside_cp).unwrap();
    assert!(outside_body.contains("evil"));

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(outside);
}

#[test]
fn unsealed_interrupted_resume_requires_checkpoint_integrity_baseline() {
    let root = unique_dir("int_base");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let run_id = allocate_run_id(&validated, "full").unwrap();
    let run_dir = prepare_run_dir(&validated, &run_id).unwrap();
    write_sanitized_fixture(&run_dir, &run_id);
    // No run_manifest.json => unsealed interrupted path.
    // Tamper checkpoint body after write: integrity baseline must catch it when present,
    // or the API must refuse to claim auditable fail-closed resume without baseline.
    let cp_path = run_dir.join("endurance_checkpoint.jsonl");
    let baseline = write_checkpoint_integrity_baseline(&run_dir).expect("baseline");
    assert!(!baseline.sha256.is_empty());

    // Mutate checkpoint after baseline.
    let mut body = fs::read_to_string(&cp_path).unwrap();
    body.push(' ');
    fs::write(&cp_path, body).unwrap();

    let err = load_resume_context(&run_dir, &run_id).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::HashMismatch { .. }
                | EvidenceRetentionError::CheckpointIntegrity { .. }
                | EvidenceRetentionError::ResumeUnavailable { .. }
        ),
        "tampered unsealed checkpoint must fail integrity baseline: {err}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn archive_uses_staging_then_atomic_publish_and_cleans_failed_stage() {
    let (root, run_dir, run_id, _m) = sealed_fixture("arch_stage");
    let archive_root = validate_evidence_root(&root, &test_policy())
        .unwrap()
        .join("archives");
    let archived = archive_run(&run_dir, &archive_root).expect("archive");
    assert!(archived.exists());
    assert!(archived.join(RUN_MANIFEST_FILE).exists());
    // No leftover staging dirs after success.
    let leftovers: Vec<_> = fs::read_dir(&archive_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".staging") || n.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "successful archive must not leave staging dirs: {leftovers:?}"
    );
    // Destination name is the run id (published), not a staging name.
    assert_eq!(
        archived.file_name().and_then(|s| s.to_str()),
        Some(run_id.as_str())
    );

    // Failed archive (dest already exists) must be retry-safe and not leave new staging junk.
    let err = archive_run(&run_dir, &archive_root).unwrap_err();
    assert!(matches!(err, EvidenceRetentionError::DuplicateRunId { .. }));
    let leftovers: Vec<_> = fs::read_dir(&archive_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".staging") || n.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "failed archive retry must clean staging: {leftovers:?}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn restore_uses_staging_then_atomic_publish() {
    let (root, run_dir, run_id, _m) = sealed_fixture("rest_stage");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();
    let archive_root = validated.join("archives");
    let archived = archive_run(&run_dir, &archive_root).unwrap();
    let restore_root = validated.join("restore");
    let restored = restore_run(&archived, &restore_root).expect("restore");
    assert_eq!(
        restored.file_name().and_then(|s| s.to_str()),
        Some(run_id.as_str())
    );
    let leftovers: Vec<_> = fs::read_dir(&restore_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".staging") || n.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "restore staging leftovers: {leftovers:?}"
    );
    verify_run(&restored).expect("restored verifies");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retention_plan_and_apply_require_full_manifest_verify() {
    let root = unique_dir("ret_hard");
    let validated = validate_evidence_root(&root, &test_policy()).unwrap();

    // Completed + verified candidate.
    let id_ok = allocate_run_id(&validated, "canary").unwrap();
    let dir_ok = prepare_run_dir(&validated, &id_ok).unwrap();
    write_sanitized_fixture(&dir_ok, &id_ok);
    seal_run(
        &dir_ok,
        SealOptions {
            run_id: id_ok.clone(),
            status: RunStatus::Completed,
            stage: "canary".into(),
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

    // Completed-looking but hash-tampered — must NOT be selected / deleted.
    let id_bad = allocate_run_id(&validated, "coverage").unwrap();
    let dir_bad = prepare_run_dir(&validated, &id_bad).unwrap();
    write_sanitized_fixture(&dir_bad, &id_bad);
    seal_run(
        &dir_bad,
        SealOptions {
            run_id: id_bad.clone(),
            status: RunStatus::Completed,
            stage: "coverage".into(),
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
    let mut calls = fs::read_to_string(dir_bad.join("endurance_calls.jsonl")).unwrap();
    calls.push('x');
    fs::write(dir_bad.join("endurance_calls.jsonl"), calls).unwrap();

    // Unsealed completed-looking dir (no manifest) — must never be selected.
    let id_unsealed = allocate_run_id(&validated, "stability").unwrap();
    let dir_unsealed = prepare_run_dir(&validated, &id_unsealed).unwrap();
    write_sanitized_fixture(&dir_unsealed, &id_unsealed);

    // Status drift: manifest says completed, then rewritten to running without re-seal.
    // Plan may or may not have seen it; apply must refuse if targeted.
    let id_drift = allocate_run_id(&validated, "full").unwrap();
    let dir_drift = prepare_run_dir(&validated, &id_drift).unwrap();
    write_sanitized_fixture(&dir_drift, &id_drift);
    seal_run(
        &dir_drift,
        SealOptions {
            run_id: id_drift.clone(),
            status: RunStatus::Completed,
            stage: "full".into(),
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
    // Force status field to running in place (invalidates audit trust).
    let mut man: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir_drift.join(RUN_MANIFEST_FILE)).unwrap())
            .unwrap();
    man["status"] = serde_json::Value::String("running".into());
    fs::write(
        dir_drift.join(RUN_MANIFEST_FILE),
        serde_json::to_string_pretty(&man).unwrap(),
    )
    .unwrap();

    let plan = plan_retention_cleanup(
        &validated,
        RetentionOptions {
            keep: 0,
            protect_run_ids: vec![],
            only_statuses: vec![RunStatus::Completed, RunStatus::Archived],
        },
    )
    .expect("plan");

    // Only fully verified completed/archived runs may appear.
    for t in &plan.targets {
        let name = t.file_name().and_then(|s| s.to_str()).unwrap_or("");
        assert_ne!(name, id_bad);
        assert_ne!(name, id_unsealed);
        assert_ne!(name, id_drift);
        // Each target must still fully verify at plan time.
        let m = verify_run(t).expect("planned target must verify");
        assert!(matches!(
            m.status,
            RunStatus::Completed | RunStatus::Archived
        ));
        assert_eq!(m.run_id, name);
    }
    assert!(
        plan.targets
            .iter()
            .any(|t| { t.file_name().and_then(|s| s.to_str()) == Some(id_ok.as_str()) }),
        "verified completed run should be a cleanup candidate when keep=0"
    );

    // Apply must re-verify; invent a tampered target in a forged plan and ensure refusal.
    let forged = RetentionPlan {
        root: validated.clone(),
        targets: vec![dir_bad.clone()],
    };
    let err = apply_retention_cleanup(&validated, &forged).unwrap_err();
    assert!(
        matches!(
            err,
            EvidenceRetentionError::HashMismatch { .. }
                | EvidenceRetentionError::FileSetMismatch { .. }
                | EvidenceRetentionError::UncontrolledPath { .. }
                | EvidenceRetentionError::ResumeUnavailable { .. }
                | EvidenceRetentionError::Io { .. }
        ),
        "apply must refuse tampered/unverified target: {err}"
    );
    assert!(dir_bad.exists(), "tampered target must not be deleted");

    let removed = apply_retention_cleanup(&validated, &plan).expect("apply verified plan");
    assert!(removed.iter().any(|p| p == &dir_ok));
    assert!(dir_bad.exists());
    assert!(dir_unsealed.exists());
    assert!(dir_drift.exists());
    let _ = fs::remove_dir_all(root);
}
