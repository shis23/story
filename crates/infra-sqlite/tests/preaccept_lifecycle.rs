//! Pre-accept lifecycle repository: draft / attempt intermediate state /
//! autofix+postprocess writeback / recovery / fault injection.
//!
//! This is the SQLite authority for Accept-previous writes. Callers must not
//! dual-write JSON when using these APIs.

use std::sync::{Arc, Barrier};

use storyforge_domain::Id;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::conversation::{
    Conversation, Provenance, Role, SubagentSnapshot, VariantStatus,
};
use storyforge_domain::turn::{
    AttemptStatus, DerivationComponents, DerivationStatus, Mutation, MutationBatch, QualityReport,
    QualitySeverity, QualityWarning, QualityWarningCode, TurnStatus,
};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::export_sqlite_to_json;
use storyforge_infra_sqlite::migrations::{builtin_migrations, current_version, migrate};
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptRequest, PostprocessApplyOutcome, PostprocessApplyRequest,
    PreacceptFault, PreacceptOutboxKind, PreacceptOutboxStatus, RegenerateAttemptRequest,
    SqlitePreacceptRepository,
};
use storyforge_infra_sqlite::production::{SqliteProductionRepository, compute_draft_hash};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    db: Database,
    campaign_id: Id,
    conversation_id: Id,
    turn_id: Id,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let mut db = Database::open(dir.path().join("preaccept.sqlite3")).unwrap();

    let campaign_id = Id::from_str("camp-preaccept");
    let conversation_id = Id::from_str("conv-preaccept");
    let mut campaign = Campaign::new(Id::from_str("card-preaccept"), "Preaccept");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());

    let mut conversation = Conversation::new(None, Some(campaign_id.clone()));
    conversation.id = conversation_id.clone();
    let input_node_id = conversation.append_message(Role::User, "继续写这一幕".into());

    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    let mut turn = storyforge_domain::turn::TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        input_node_id.clone(),
        0,
    );
    turn.turn_id = Id::from_str("turn-preaccept");
    turn.status = TurnStatus::Generating;
    SqliteProductionRepository::save_turn(&mut db, &turn).unwrap();

    let _ = input_node_id;
    Fixture {
        _dir: dir,
        db,
        campaign_id,
        conversation_id,
        turn_id: turn.turn_id,
    }
}

fn quality_report_with_error() -> QualityReport {
    QualityReport {
        warnings: vec![QualityWarning {
            code: QualityWarningCode::TooShort { char_count: 3 },
            message: "too short".into(),
            severity: QualitySeverity::Error,
        }],
    }
}

fn force_committing(db: &mut Database, turn_id: &Id, attempt_id: &Id) {
    let mut turn = SqliteProductionRepository::get_turn(db, turn_id)
        .unwrap()
        .unwrap();
    turn.status = TurnStatus::Committing;
    turn.find_attempt_mut(attempt_id).unwrap().status = AttemptStatus::Committing;
    turn.touch();
    SqliteProductionRepository::save_turn(db, &turn).unwrap();
}

fn snapshot_preaccept(
    db: &Database,
    _campaign_id: &Id,
    conversation_id: &Id,
    turn_id: &Id,
) -> (String, String, usize) {
    let conversation = SqliteProductionRepository::get_conversation(db, conversation_id)
        .unwrap()
        .unwrap();
    let turn = SqliteProductionRepository::get_turn(db, turn_id)
        .unwrap()
        .unwrap();
    let outbox = SqlitePreacceptRepository::list_outbox_for_turn(db, turn_id).unwrap();
    (
        serde_json::to_string(&conversation).unwrap(),
        serde_json::to_string(&turn).unwrap(),
        outbox.len(),
    )
}

#[test]
fn create_draft_attempt_atomically_persists_conversation_and_attempt() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-1");
    let draft = "第一版草稿正文";
    let outcome = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: draft,
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    assert_eq!(outcome.attempt_id, attempt_id);
    assert_eq!(outcome.draft_hash, compute_draft_hash(draft));

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    let node = conversation
        .find_node(&outcome.variant_id)
        .expect("draft node");
    assert_eq!(node.active().unwrap().content, draft);
    assert_eq!(node.active().unwrap().status, VariantStatus::Draft);

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::DraftReady);
    assert_eq!(turn.attempts.len(), 1);
    let attempt = turn.find_attempt(&attempt_id).unwrap();
    assert_eq!(attempt.status, AttemptStatus::DraftReady);
    assert_eq!(attempt.draft_hash, compute_draft_hash(draft));
    assert_eq!(attempt.variant_id, outcome.variant_id);

    let outbox = SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id).unwrap();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].kind, PreacceptOutboxKind::DraftReady);
    assert_eq!(outbox[0].status, PreacceptOutboxStatus::Applied);
    assert_eq!(outbox[0].draft_hash, compute_draft_hash(draft));
}

#[test]
fn create_draft_attempt_fault_rolls_back_conversation_turn_and_outbox() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-fault-draft");
    let err = SqlitePreacceptRepository::create_draft_attempt_with_fault(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "will roll back",
            pending_temporary_instances: vec![],
            provenance: None,
        },
        PreacceptFault::BeforeCommit,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected"));

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(conversation.nodes.len(), 1, "only user input remains");
    assert!(
        conversation
            .nodes
            .iter()
            .all(|n| n.active().is_some_and(|v| v.role == Role::User))
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Generating);
    assert!(turn.attempts.is_empty());
    assert!(
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
            .unwrap()
            .is_empty()
    );
    assert!(
        SqliteProductionRepository::get_attempt(&f.db, &attempt_id)
            .unwrap()
            .is_none()
    );
}

#[test]
fn create_draft_rejects_campaign_conversation_scope_mismatch() {
    let mut f = fixture();
    let other_campaign = Id::from_str("camp-other");
    let err = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &other_campaign,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &Id::from_str("attempt-scope"),
            draft_text: "nope",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap_err();
    assert!(err.to_string().contains("scope") || err.to_string().contains("campaign"));

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert!(turn.attempts.is_empty());
}

#[test]
fn autofix_sync_rewrites_conversation_and_attempt_hash_atomically() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-autofix");
    let original = "original draft";
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: original,
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let fixed = "auto-fixed draft body";
    let report = quality_report_with_error();
    let autofix_provenance = Provenance {
        session_id: Id::from_str("autofix-session"),
        plan: None,
        subagent_results: vec![SubagentSnapshot {
            character_id: "lin".into(),
            full_text: "performance".into(),
            character_instance_id: None,
            display_name: None,
            fallback_reason: None,
            reasoning_content: Some("subagent reasoning".into()),
        }],
        profile_id: None,
        generation_mode: None,
        seed: 1,
        last_hint: None,
        director_reasoning: Some("director reasoning".into()),
        writer_reasoning: None,
        editor_reasoning: Some("autofix editor reasoning".into()),
    };
    SqlitePreacceptRepository::sync_autofix(
        &mut f.db,
        AutofixSyncRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            final_text: fixed,
            quality_report: report.clone(),
            provenance: Some(autofix_provenance.clone()),
        },
    )
    .unwrap();

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation
            .find_node(&created.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .content,
        fixed
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    let attempt = turn.find_attempt(&attempt_id).unwrap();
    assert_eq!(attempt.draft_hash, compute_draft_hash(fixed));
    assert_eq!(
        attempt.quality_report.as_ref().unwrap().error_count(),
        report.error_count()
    );
    assert_eq!(attempt.status, AttemptStatus::DraftReady);
    assert_eq!(
        attempt
            .provenance
            .as_ref()
            .and_then(|p| p.editor_reasoning.as_deref()),
        Some("autofix editor reasoning")
    );
    assert_eq!(turn.status, TurnStatus::DraftReady);

    assert_eq!(
        conversation
            .find_node(&created.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .provenance
            .as_ref()
            .and_then(|p| p.editor_reasoning.as_deref()),
        Some("autofix editor reasoning")
    );

    // content hash must match active conversation text
    assert_eq!(
        compute_draft_hash(
            conversation
                .find_node(&created.variant_id)
                .unwrap()
                .active()
                .unwrap()
                .content
                .as_str()
        ),
        attempt.draft_hash
    );
}

#[test]
fn autofix_fault_before_commit_leaves_original_draft_intact() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-autofix-fault");
    let original = "keep me";
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: original,
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let err = SqlitePreacceptRepository::sync_autofix_with_fault(
        &mut f.db,
        AutofixSyncRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            final_text: "should not stick",
            quality_report: QualityReport::default(),
            provenance: None,
        },
        PreacceptFault::BeforeCommit,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected"));

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation
            .find_node(&created.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .content,
        original
    );
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        turn.find_attempt(&attempt_id).unwrap().draft_hash,
        compute_draft_hash(original)
    );
}

#[test]
fn postprocess_apply_is_atomic_and_idempotent_on_replay() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-pp");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "ready for postprocess",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let mut batch = MutationBatch::new(Id::from_str("commit-pp"), 0);
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: created.variant_id.clone(),
    });
    let derivation = DerivationComponents {
        summary_derivation: DerivationStatus::Succeeded,
        state_derivation: DerivationStatus::Succeeded,
    };

    let request = PostprocessApplyRequest {
        campaign_id: &f.campaign_id,
        conversation_id: &f.conversation_id,
        turn_id: &f.turn_id,
        attempt_id: &attempt_id,
        batch: Some(batch.clone()),
        derivation: derivation.clone(),
    };
    let first = SqlitePreacceptRepository::apply_postprocess(&mut f.db, request.clone()).unwrap();
    assert_eq!(first, PostprocessApplyOutcome::Applied);
    let outbox_after_first =
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id).unwrap();
    let pp_rows_after_first = outbox_after_first
        .iter()
        .filter(|r| r.kind == PreacceptOutboxKind::PostprocessApply)
        .count();
    assert_eq!(pp_rows_after_first, 1);

    // Idempotent replay with identical payload: no new outbox rows.
    let second = SqlitePreacceptRepository::apply_postprocess(&mut f.db, request).unwrap();
    assert_eq!(second, PostprocessApplyOutcome::AlreadyApplied);
    let outbox_after_replay =
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id).unwrap();
    let pp_rows_after_replay = outbox_after_replay
        .iter()
        .filter(|r| r.kind == PreacceptOutboxKind::PostprocessApply)
        .count();
    assert_eq!(
        pp_rows_after_replay, pp_rows_after_first,
        "same-payload postprocess replay must not append outbox rows"
    );
    assert!(
        !outbox_after_replay.iter().any(|r| {
            r.kind == PreacceptOutboxKind::PostprocessApply
                && r.status == PreacceptOutboxStatus::Skipped
        }),
        "successful postprocess replay must not record Skipped"
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::AwaitingAcceptance);
    let attempt = turn.find_attempt(&attempt_id).unwrap();
    assert_eq!(attempt.status, AttemptStatus::AwaitingAcceptance);
    assert!(attempt.pending_state_changes.is_some());
    assert_eq!(
        attempt.pending_state_changes.as_ref().unwrap().commit_id,
        batch.commit_id
    );
    assert_eq!(
        attempt.derivation.as_ref().unwrap().summary_derivation,
        DerivationStatus::Succeeded
    );

    // Campaign must remain untouched pre-accept (no dual authority / premature writes).
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 0);
    assert_eq!(
        SqliteProductionRepository::count_commit_ledger(&f.db).unwrap(),
        0
    );
}

#[test]
fn postprocess_skips_when_attempt_no_longer_current() {
    let mut f = fixture();
    let first = Id::from_str("attempt-old");
    let second = Id::from_str("attempt-new");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &first,
            draft_text: "old",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    let regenerated = SqlitePreacceptRepository::append_regenerate_attempt(
        &mut f.db,
        RegenerateAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            previous_variant_id: &created.variant_id,
            attempt_id: &second,
            draft_text: "new regenerate",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let mut batch = MutationBatch::new(Id::from_str("commit-late"), 0);
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: created.variant_id.clone(),
    });
    let applied = SqlitePreacceptRepository::apply_postprocess(
        &mut f.db,
        PostprocessApplyRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &first,
            batch: Some(batch),
            derivation: DerivationComponents {
                summary_derivation: DerivationStatus::Succeeded,
                state_derivation: DerivationStatus::Succeeded,
            },
        },
    )
    .unwrap();
    assert_eq!(
        applied,
        PostprocessApplyOutcome::SkippedLate,
        "late postprocess must not revive superseded attempt"
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        turn.find_attempt(&first).unwrap().status,
        AttemptStatus::Superseded
    );
    assert_eq!(
        turn.find_attempt(&second).unwrap().status,
        AttemptStatus::DraftReady
    );
    assert!(
        turn.find_attempt(&first)
            .unwrap()
            .pending_state_changes
            .is_none()
    );
    assert_eq!(turn.active_attempt().unwrap().attempt_id, second);
    assert_eq!(
        turn.active_attempt().unwrap().variant_id,
        regenerated.variant_id
    );
}

#[test]
fn regenerate_supersedes_previous_attempt_atomically() {
    let mut f = fixture();
    let first = Id::from_str("attempt-a");
    let second = Id::from_str("attempt-b");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &first,
            draft_text: "v1",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    let regen = SqlitePreacceptRepository::append_regenerate_attempt(
        &mut f.db,
        RegenerateAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            previous_variant_id: &created.variant_id,
            attempt_id: &second,
            draft_text: "v2",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::DraftReady);
    assert_eq!(
        turn.find_attempt(&first).unwrap().status,
        AttemptStatus::Superseded
    );
    assert_eq!(
        turn.find_attempt(&second).unwrap().status,
        AttemptStatus::DraftReady
    );
    assert_eq!(turn.active_attempt().unwrap().attempt_id, second);

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    // Same node, new active draft variant.
    let node = conversation.find_node(&created.variant_id).unwrap();
    assert_eq!(node.id, regen.variant_id);
    assert_eq!(node.active().unwrap().content, "v2");
    assert_eq!(node.active().unwrap().status, VariantStatus::Draft);
    assert!(
        node.variants
            .iter()
            .filter(|v| v.status == VariantStatus::Draft)
            .count()
            >= 1
    );
}

#[test]
fn mark_stale_after_edit_keeps_hash_and_content_consistent() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-edit");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "before edit",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    SqlitePreacceptRepository::mark_stale_after_edit(
        &mut f.db,
        &f.campaign_id,
        &f.conversation_id,
        &f.turn_id,
        &attempt_id,
        "after manual edit",
    )
    .unwrap();

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    let content = conversation
        .find_node(&created.variant_id)
        .unwrap()
        .active()
        .unwrap()
        .content
        .clone();
    assert_eq!(content, "after manual edit");

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    let attempt = turn.find_attempt(&attempt_id).unwrap();
    assert_eq!(attempt.status, AttemptStatus::Stale);
    // Stale keeps the original draft_hash so Accept can detect mismatch.
    assert_eq!(attempt.draft_hash, compute_draft_hash("before edit"));
    assert_ne!(compute_draft_hash(&content), attempt.draft_hash);
}

#[test]
fn recovery_lists_active_preaccept_state_and_fail_incomplete_is_atomic() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-recover");
    SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "recover me",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let snapshot = SqlitePreacceptRepository::recover_active_state(&f.db, &f.campaign_id).unwrap();
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.turns[0].turn_id, f.turn_id);
    assert_eq!(
        snapshot.turns[0].active_attempt().unwrap().attempt_id,
        attempt_id
    );
    assert!(!snapshot.outbox.is_empty());

    let failed = SqlitePreacceptRepository::fail_incomplete_preaccept(&mut f.db).unwrap();
    assert_eq!(failed, 1);
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Failed);
    assert_eq!(
        turn.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::Failed
    );
    let outbox = SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id).unwrap();
    assert!(
        outbox
            .iter()
            .all(|row| row.status == PreacceptOutboxStatus::Failed
                || row.status == PreacceptOutboxStatus::Applied)
    );
}

#[test]
fn concurrent_draft_create_serializes_to_single_active_attempt() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("race.sqlite3");
    {
        let mut db = Database::open(&path).unwrap();
        let mut campaign = Campaign::new(Id::from_str("card-race"), "Race");
        campaign.id = Id::from_str("camp-race");
        let mut conversation = Conversation::new(None, Some(campaign.id.clone()));
        conversation.id = Id::from_str("conv-race");
        conversation.append_message(Role::User, "start".into());
        campaign.conversation_id = Some(conversation.id.clone());
        SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();
        let mut turn = storyforge_domain::turn::TurnRecord::new(
            campaign.id.clone(),
            conversation.id.clone(),
            Id::from_str("input"),
            0,
        );
        turn.turn_id = Id::from_str("turn-race");
        turn.status = TurnStatus::Generating;
        SqliteProductionRepository::save_turn(&mut db, &turn).unwrap();
    }

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|i| {
            let path = path.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut db = Database::open(path).unwrap();
                barrier.wait();
                SqlitePreacceptRepository::create_draft_attempt(
                    &mut db,
                    DraftAttemptRequest {
                        campaign_id: &Id::from_str("camp-race"),
                        conversation_id: &Id::from_str("conv-race"),
                        turn_id: &Id::from_str("turn-race"),
                        attempt_id: &Id::from_str(format!("attempt-race-{i}")),
                        draft_text: &format!("race-{i}"),
                        pending_temporary_instances: vec![],
                        provenance: None,
                    },
                )
            })
        })
        .collect();

    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let err = results.iter().filter(|r| r.is_err()).count();
    assert_eq!(ok, 1, "exactly one draft create must win: {results:?}");
    assert_eq!(err, 1, "the loser must fail closed: {results:?}");

    let db = Database::open(path).unwrap();
    let turn = SqliteProductionRepository::get_turn(&db, &Id::from_str("turn-race"))
        .unwrap()
        .unwrap();
    assert_eq!(turn.attempts.len(), 1);
    assert_eq!(turn.status, TurnStatus::DraftReady);
}

#[test]
fn postprocess_rejects_attempt_owned_by_another_turn_with_zero_outbox() {
    let mut f = fixture();
    let attempt_a = Id::from_str("attempt-cross-a");
    SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_a,
            draft_text: "turn A draft",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    let failed = SqlitePreacceptRepository::fail_incomplete_preaccept(&mut f.db).unwrap();
    assert_eq!(failed, 1);

    let mut turn_b = storyforge_domain::turn::TurnRecord::new(
        f.campaign_id.clone(),
        f.conversation_id.clone(),
        Id::from_str("input-b"),
        0,
    );
    turn_b.turn_id = Id::from_str("turn-preaccept-b");
    turn_b.status = TurnStatus::Generating;
    SqliteProductionRepository::save_turn(&mut f.db, &turn_b).unwrap();
    let attempt_b = Id::from_str("attempt-cross-b");
    let created_b = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &turn_b.turn_id,
            attempt_id: &attempt_b,
            draft_text: "turn B draft",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let outbox_before =
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &turn_b.turn_id).unwrap();
    let before_count = outbox_before.len();

    let mut batch = MutationBatch::new(Id::from_str("commit-cross"), 0);
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: created_b.variant_id.clone(),
    });
    let err = SqlitePreacceptRepository::apply_postprocess(
        &mut f.db,
        PostprocessApplyRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &turn_b.turn_id,
            attempt_id: &attempt_a,
            batch: Some(batch),
            derivation: DerivationComponents {
                summary_derivation: DerivationStatus::Succeeded,
                state_derivation: DerivationStatus::Succeeded,
            },
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("attempt") || err.to_string().contains("turn"),
        "cross-turn attempt must fail closed: {err}"
    );

    let outbox_after =
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &turn_b.turn_id).unwrap();
    assert_eq!(
        outbox_after.len(),
        before_count,
        "cross-turn postprocess must write zero outbox rows"
    );
    assert!(
        !outbox_after
            .iter()
            .any(|r| r.kind == PreacceptOutboxKind::PostprocessApply),
        "must not record Skipped/Applied outbox for foreign attempt"
    );

    let turn_b_loaded = SqliteProductionRepository::get_turn(&f.db, &turn_b.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn_b_loaded.status, TurnStatus::DraftReady);
    assert!(
        turn_b_loaded
            .find_attempt(&attempt_b)
            .unwrap()
            .pending_state_changes
            .is_none()
    );
}

#[test]
fn postprocess_same_payload_replay_is_idempotent_without_extra_outbox() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-pp-idem");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "idempotent postprocess",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    let mut batch = MutationBatch::new(Id::from_str("commit-pp-idem"), 0);
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: created.variant_id.clone(),
    });
    let derivation = DerivationComponents {
        summary_derivation: DerivationStatus::Succeeded,
        state_derivation: DerivationStatus::Disabled,
    };
    let request = PostprocessApplyRequest {
        campaign_id: &f.campaign_id,
        conversation_id: &f.conversation_id,
        turn_id: &f.turn_id,
        attempt_id: &attempt_id,
        batch: Some(batch.clone()),
        derivation: derivation.clone(),
    };
    assert_eq!(
        SqlitePreacceptRepository::apply_postprocess(&mut f.db, request.clone()).unwrap(),
        PostprocessApplyOutcome::Applied
    );
    let rows_after_apply = SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
        .unwrap()
        .len();

    assert_eq!(
        SqlitePreacceptRepository::apply_postprocess(&mut f.db, request.clone()).unwrap(),
        PostprocessApplyOutcome::AlreadyApplied
    );
    assert_eq!(
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
            .unwrap()
            .len(),
        rows_after_apply
    );

    let mut other = batch.clone();
    other.commit_id = Id::from_str("commit-pp-different");
    let err = SqlitePreacceptRepository::apply_postprocess(
        &mut f.db,
        PostprocessApplyRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            batch: Some(other),
            derivation,
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("different payload")
            || err.to_string().contains("conflict")
            || err.to_string().contains("already applied"),
        "different postprocess payload must fail closed: {err}"
    );
    assert_eq!(
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
            .unwrap()
            .len(),
        rows_after_apply,
        "conflicting payload must not append outbox"
    );
}

#[test]
fn regenerate_rejects_previous_variant_not_owned_by_active_attempt() {
    let mut f = fixture();
    let first = Id::from_str("attempt-reg-active");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &first,
            draft_text: "active draft",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    let foreign_node = conversation.nodes[0].id.clone();
    assert_ne!(foreign_node, created.variant_id);

    let outbox_before = SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id).unwrap();
    let before_len = outbox_before.len();

    let err = SqlitePreacceptRepository::append_regenerate_attempt(
        &mut f.db,
        RegenerateAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            previous_variant_id: &foreign_node,
            attempt_id: &Id::from_str("attempt-reg-bad"),
            draft_text: "should not land",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("variant") || err.to_string().contains("active"),
        "regenerate must require active attempt variant: {err}"
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.attempts.len(), 1);
    assert_eq!(turn.active_attempt().unwrap().attempt_id, first);
    assert_eq!(
        SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
            .unwrap()
            .len(),
        before_len
    );
}

#[test]
fn autofix_idempotency_fingerprint_covers_full_quality_report() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-autofix-fp");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "original autofix body",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let final_text = "fixed body shared";
    let report_a = QualityReport {
        warnings: vec![QualityWarning {
            code: QualityWarningCode::TooShort { char_count: 4 },
            message: "report A".into(),
            severity: QualitySeverity::Error,
        }],
    };
    let report_b = QualityReport {
        warnings: vec![QualityWarning {
            code: QualityWarningCode::MetaDescription {
                snippet: "leak".into(),
            },
            message: "report B".into(),
            severity: QualitySeverity::Error,
        }],
    };
    assert_eq!(report_a.error_count(), report_b.error_count());

    SqlitePreacceptRepository::sync_autofix(
        &mut f.db,
        AutofixSyncRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            final_text,
            quality_report: report_a.clone(),
            provenance: None,
        },
    )
    .unwrap();
    let autofix_rows_a = SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
        .unwrap()
        .iter()
        .filter(|r| r.kind == PreacceptOutboxKind::AutofixSync)
        .count();
    assert_eq!(autofix_rows_a, 1);

    SqlitePreacceptRepository::sync_autofix(
        &mut f.db,
        AutofixSyncRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            final_text,
            quality_report: report_a.clone(),
            provenance: None,
        },
    )
    .unwrap();
    let autofix_rows_replay = SqlitePreacceptRepository::list_outbox_for_turn(&f.db, &f.turn_id)
        .unwrap()
        .iter()
        .filter(|r| r.kind == PreacceptOutboxKind::AutofixSync)
        .count();
    assert_eq!(
        autofix_rows_replay, autofix_rows_a,
        "identical quality report replay must not add outbox rows"
    );

    SqlitePreacceptRepository::sync_autofix(
        &mut f.db,
        AutofixSyncRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            final_text,
            quality_report: report_b.clone(),
            provenance: None,
        },
    )
    .unwrap();
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    let stored = turn
        .find_attempt(&attempt_id)
        .unwrap()
        .quality_report
        .as_ref()
        .unwrap();
    assert_eq!(stored.warnings[0].message, "report B");
    assert!(matches!(
        stored.warnings[0].code,
        QualityWarningCode::MetaDescription { .. }
    ));

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation
            .find_node(&created.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .content,
        final_text
    );
}

#[test]
fn regenerate_rejects_committing_turn_with_zero_writes() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-commit-regen");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "pre-commit draft",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    force_committing(&mut f.db, &f.turn_id, &attempt_id);
    let before = snapshot_preaccept(&f.db, &f.campaign_id, &f.conversation_id, &f.turn_id);

    let err = SqlitePreacceptRepository::append_regenerate_attempt(
        &mut f.db,
        RegenerateAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            previous_variant_id: &created.variant_id,
            attempt_id: &Id::from_str("attempt-commit-regen-new"),
            draft_text: "must not land",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("Committing") || err.to_string().contains("committing"),
        "regenerate during Committing must fail closed: {err}"
    );

    let after = snapshot_preaccept(&f.db, &f.campaign_id, &f.conversation_id, &f.turn_id);
    assert_eq!(before, after, "zero writes to conversation/turn/outbox");
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Committing);
    assert_eq!(turn.attempts.len(), 1);
    assert_eq!(
        turn.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::Committing
    );
}

#[test]
fn mark_stale_rejects_committing_turn_or_attempt_with_zero_writes() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-commit-stale");
    let created = SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "pre-commit draft",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    force_committing(&mut f.db, &f.turn_id, &attempt_id);
    let before = snapshot_preaccept(&f.db, &f.campaign_id, &f.conversation_id, &f.turn_id);

    let err = SqlitePreacceptRepository::mark_stale_after_edit(
        &mut f.db,
        &f.campaign_id,
        &f.conversation_id,
        &f.turn_id,
        &attempt_id,
        "must not overwrite committing draft",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("Committing") || err.to_string().contains("committing"),
        "mark_stale during Committing must fail closed: {err}"
    );

    let after = snapshot_preaccept(&f.db, &f.campaign_id, &f.conversation_id, &f.turn_id);
    assert_eq!(before, after, "zero writes to conversation/turn/outbox");
    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation
            .find_node(&created.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .content,
        "pre-commit draft"
    );
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Committing);
    assert_eq!(
        turn.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::Committing
    );
}

#[test]
fn sync_autofix_rejects_stale_attempt_instead_of_dead_reactivation() {
    let mut f = fixture();
    let attempt_id = Id::from_str("attempt-stale-autofix");
    SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            draft_text: "before stale",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();
    SqlitePreacceptRepository::mark_stale_after_edit(
        &mut f.db,
        &f.campaign_id,
        &f.conversation_id,
        &f.turn_id,
        &attempt_id,
        "edited stale body",
    )
    .unwrap();
    let before = snapshot_preaccept(&f.db, &f.campaign_id, &f.conversation_id, &f.turn_id);

    let err = SqlitePreacceptRepository::sync_autofix(
        &mut f.db,
        AutofixSyncRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &attempt_id,
            final_text: "should not reactivate stale",
            quality_report: QualityReport::default(),
            provenance: None,
        },
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("active")
            || err.to_string().contains("Stale")
            || err.to_string().contains("stale")
            || err.to_string().contains("writable"),
        "stale attempt must not be silently reactivated by autofix: {err}"
    );

    let after = snapshot_preaccept(&f.db, &f.campaign_id, &f.conversation_id, &f.turn_id);
    assert_eq!(
        before, after,
        "rejected autofix on stale must be zero-write"
    );
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        turn.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::Stale
    );
    assert_eq!(
        turn.find_attempt(&attempt_id).unwrap().draft_hash,
        compute_draft_hash("before stale")
    );
}

#[test]
fn reverse_export_marks_preaccept_outbox_as_unsupported_without_silent_loss() {
    let mut f = fixture();
    SqlitePreacceptRepository::create_draft_attempt(
        &mut f.db,
        DraftAttemptRequest {
            campaign_id: &f.campaign_id,
            conversation_id: &f.conversation_id,
            turn_id: &f.turn_id,
            attempt_id: &Id::from_str("attempt-export"),
            draft_text: "export me",
            pending_temporary_instances: vec![],
            provenance: None,
        },
    )
    .unwrap();

    let export_dir = f._dir.path().join("export-out");
    let result = export_sqlite_to_json(&f.db, &export_dir).unwrap();
    assert!(
        result
            .report
            .unsupported_fields
            .iter()
            .any(|f| f.contains("preaccept_outbox")),
        "expected preaccept_outbox unsupported classification: {:?}",
        result.report.unsupported_fields
    );
    // Turn payload still exports (supported via turns.json).
    assert!(export_dir.join("turns.json").exists());
}

#[test]
fn schema_upgrades_to_v4_preaccept_outbox() {
    let mut db = Database::open_in_memory().unwrap();
    let migrations = builtin_migrations();
    assert!(migrations.iter().any(|m| m.version == 4));
    let v1 = migrations.iter().find(|m| m.version == 1).unwrap().clone();
    storyforge_infra_sqlite::migrations::migrate_with(&mut db, &[v1]).unwrap();
    assert_eq!(migrate(&mut db).unwrap(), vec![2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(current_version(&db).unwrap(), 8);
    let exists: i64 = db
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='preaccept_outbox'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(exists, 1);
}
