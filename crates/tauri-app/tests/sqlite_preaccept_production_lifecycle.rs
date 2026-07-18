//! Gate A: production-path SQLite pre-accept lifecycle proof.
//!
//! Enters through the shared `sqlite_runtime` gateway (the same surface Tauri
//! commands and the endurance adapter must use), not the repository unit API
//! alone. After cutover, legacy JSON sources are removed so any fallback fails.
//!
//! NOTE: `sqlite_runtime::activate` pins a process-global OnceLock, so this
//! binary keeps a single `#[test]` (same pattern as `sqlite_optin_lifecycle`).

use std::fs;
use std::path::Path;
use std::sync::Arc;

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::conversation::VariantStatus;
use storyforge_domain::turn::{
    AttemptStatus, DerivationComponents, DerivationStatus, MutationBatch, QualityReport,
    QualitySeverity, QualityWarning, QualityWarningCode, TurnRecord, TurnStatus,
};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::cutover::{CutoverPlan, CutoverRequest, recover_or_verify};
use storyforge_infra_sqlite::preaccept::{
    AutofixSyncRequest, DraftAttemptRequest, PostprocessApplyOutcome, PostprocessApplyRequest,
    PreacceptOutboxKind, PreacceptOutboxStatus, RegenerateAttemptRequest,
    SqlitePreacceptRepository,
};
use storyforge_infra_sqlite::production::{SqliteProductionRepository, compute_draft_hash};
use storyforge_lib::sqlite_runtime;

fn write_json(path: &Path, value: serde_json::Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn write_cutover_source(dir: &Path) {
    write_json(
        &dir.join("cards.json"),
        serde_json::json!([{
            "id": "card-1", "source_character_id": "source-1", "name": "Hero"
        }]),
    );
    write_json(
        &dir.join("campaigns.json"),
        serde_json::json!([{
            "id": "campaign-1", "card_id": "card-1", "name": "SQLite preaccept",
            "created_at": "2026-07-14T00:00:00Z", "revision": 0,
            "chronicle_revision": 0, "conversation_id": "conversation-1",
            "lineage_id": "lineage-1"
        }]),
    );
    write_json(
        &dir.join("conversations").join("conversation-1.json"),
        serde_json::json!({
            "id": "conversation-1", "campaign_id": "campaign-1", "character_id": null,
            "created_at": "2026-07-14T00:00:00Z", "updated_at": "2026-07-14T00:00:00Z",
            "nodes": [], "archived_upto": 0
        }),
    );
    for name in [
        "instances.json",
        "knowledge.json",
        "tasks.json",
        "round_summaries.json",
        "turns.json",
    ] {
        write_json(&dir.join(name), serde_json::json!([]));
    }
}

fn empty_derivation() -> DerivationComponents {
    DerivationComponents {
        summary_derivation: DerivationStatus::Succeeded,
        state_derivation: DerivationStatus::Succeeded,
    }
}

#[test]
fn production_gateway_preaccept_lifecycle_full_matrix() {
    let temp = tempfile::tempdir().unwrap();
    let source_dir = temp.path();
    let db_path = source_dir.join("storyforge.sqlite3");
    write_cutover_source(source_dir);

    let cutover = CutoverRequest {
        plan: CutoverPlan::new(source_dir, &db_path),
        label: "sqlite-preaccept-production-lifecycle".into(),
    };
    recover_or_verify(&cutover).expect("cutover");
    sqlite_runtime::activate(&db_path).expect("activate");

    // Fail closed: remove legacy JSON so any silent fallback breaks this test.
    fs::remove_file(source_dir.join("campaigns.json")).unwrap();
    fs::remove_file(source_dir.join("conversations").join("conversation-1.json")).unwrap();
    fs::remove_file(source_dir.join("turns.json")).unwrap();

    let persistence = sqlite_runtime::conversation_persistence().expect("persistence");
    let conversations = Arc::new(ConversationStore::with_persistence(persistence));
    let campaign_id = Id::from_str("campaign-1");
    let conversation_id = Id::from_str("conversation-1");

    // ── 1. first draft → autofix → postprocess → Accept → reopen ──────────
    let user_node = conversations
        .append_user_message(&conversation_id, "open the scene".into())
        .unwrap();
    let turn = TurnRecord::new(campaign_id.clone(), conversation_id.clone(), user_node, 0);
    let turn_id = turn.turn_id.clone();
    sqlite_runtime::save_turn(&turn).unwrap();

    let attempt_id = Id::new();
    let draft_text = "first production draft";
    let draft = sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn_id,
        attempt_id: &attempt_id,
        draft_text,
        pending_temporary_instances: vec![],
        provenance: None,
    })
    .expect("gateway create_draft_attempt");
    assert_eq!(draft.attempt_id, attempt_id);
    assert_eq!(draft.draft_hash, compute_draft_hash(draft_text));

    conversations.invalidate();
    assert!(
        conversations
            .get(&conversation_id)
            .unwrap()
            .find_node(&draft.variant_id)
            .is_some(),
        "draft node must be durable via preaccept UoW"
    );
    let outbox = sqlite_runtime::list_preaccept_outbox_for_turn(&turn_id).unwrap();
    assert!(outbox.iter().any(|r| {
        r.kind == PreacceptOutboxKind::DraftReady && r.status == PreacceptOutboxStatus::Applied
    }));

    let fixed = "first production draft (autofixed)";
    let report = QualityReport {
        warnings: vec![QualityWarning {
            code: QualityWarningCode::TooShort { char_count: 10 },
            message: "was short".into(),
            severity: QualitySeverity::Warning,
        }],
    };
    sqlite_runtime::sync_autofix(AutofixSyncRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn_id,
        attempt_id: &attempt_id,
        final_text: fixed,
        quality_report: report.clone(),
        provenance: None,
    })
    .expect("gateway sync_autofix");
    conversations.invalidate();
    assert_eq!(
        conversations
            .get(&conversation_id)
            .unwrap()
            .find_node(&draft.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .content,
        fixed
    );
    let att = sqlite_runtime::get_turn(&turn_id)
        .unwrap()
        .unwrap()
        .find_attempt(&attempt_id)
        .unwrap()
        .clone();
    assert_eq!(att.draft_hash, compute_draft_hash(fixed));
    let stored_report = att.quality_report.expect("quality report persisted");
    assert_eq!(stored_report.warnings.len(), report.warnings.len());
    assert_eq!(
        stored_report.warnings[0].message,
        report.warnings[0].message
    );
    assert_eq!(
        stored_report.warnings[0].severity,
        report.warnings[0].severity
    );

    let batch = MutationBatch::new(Id::new(), 0);
    let outcome = sqlite_runtime::apply_postprocess(PostprocessApplyRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn_id,
        attempt_id: &attempt_id,
        batch: Some(batch),
        derivation: empty_derivation(),
    })
    .expect("gateway apply_postprocess");
    assert!(matches!(outcome, PostprocessApplyOutcome::Applied));
    let awaiting = sqlite_runtime::get_turn(&turn_id).unwrap().unwrap();
    assert_eq!(awaiting.status, TurnStatus::AwaitingAcceptance);
    assert_eq!(
        awaiting.find_attempt(&attempt_id).unwrap().status,
        AttemptStatus::AwaitingAcceptance
    );

    let accept =
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &draft.variant_id, false)
            .expect("accept after preaccept path");
    assert_eq!(accept.turn_status, TurnStatus::Committed);
    // Accept mutates conversation in SQLite outside ConversationStore's cache.
    conversations.invalidate();

    {
        let reopened = Database::open(&db_path).unwrap();
        let persisted = SqliteProductionRepository::get_turn(&reopened, &turn_id)
            .unwrap()
            .unwrap();
        assert_eq!(persisted.status, TurnStatus::Committed);
        assert_eq!(
            persisted.find_attempt(&attempt_id).unwrap().status,
            AttemptStatus::Committed
        );
        let conv_now = SqliteProductionRepository::get_conversation(&reopened, &conversation_id)
            .unwrap()
            .unwrap();
        let active = conv_now
            .find_node(&draft.variant_id)
            .unwrap()
            .active()
            .unwrap();
        assert_eq!(active.status, VariantStatus::Final);
        let reopened_outbox =
            SqlitePreacceptRepository::list_outbox_for_turn(&reopened, &turn_id).unwrap();
        assert!(reopened_outbox.iter().any(|r| {
            r.kind == PreacceptOutboxKind::PostprocessApply
                && r.status == PreacceptOutboxStatus::Applied
        }));
    }
    assert!(!source_dir.join("turns.json").exists());
    assert!(!source_dir.join("campaigns.json").exists());

    // ── 2. regenerate + edit-stale + late postprocess skip ────────────────
    let user2 = conversations
        .append_user_message(&conversation_id, "continue".into())
        .unwrap();
    // revision after first Accept is 1
    let turn2 = TurnRecord::new(campaign_id.clone(), conversation_id.clone(), user2, 1);
    let turn2_id = turn2.turn_id.clone();
    sqlite_runtime::save_turn(&turn2).unwrap();

    let attempt_a = Id::new();
    let first = sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn2_id,
        attempt_id: &attempt_a,
        draft_text: "draft a",
        pending_temporary_instances: vec![],
        provenance: None,
    })
    .unwrap();

    let attempt_b = Id::new();
    let regen = sqlite_runtime::append_regenerate_attempt(RegenerateAttemptRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn2_id,
        previous_variant_id: &first.variant_id,
        attempt_id: &attempt_b,
        draft_text: "draft b",
        pending_temporary_instances: vec![],
        provenance: None,
    })
    .expect("gateway regenerate");
    assert_eq!(regen.variant_id, first.variant_id);

    let after_regen = sqlite_runtime::get_turn(&turn2_id).unwrap().unwrap();
    assert_eq!(
        after_regen.find_attempt(&attempt_a).unwrap().status,
        AttemptStatus::Superseded
    );
    assert_eq!(
        after_regen.find_attempt(&attempt_b).unwrap().status,
        AttemptStatus::DraftReady
    );
    let outbox2 = sqlite_runtime::list_preaccept_outbox_for_turn(&turn2_id).unwrap();
    assert!(outbox2.iter().any(|r| {
        r.kind == PreacceptOutboxKind::Regenerate && r.status == PreacceptOutboxStatus::Applied
    }));

    // Late postprocess for superseded A → SkippedLate
    let late = sqlite_runtime::apply_postprocess(PostprocessApplyRequest {
        campaign_id: &campaign_id,
        conversation_id: &conversation_id,
        turn_id: &turn2_id,
        attempt_id: &attempt_a,
        batch: None,
        derivation: empty_derivation(),
    })
    .unwrap();
    assert!(matches!(late, PostprocessApplyOutcome::SkippedLate));
    let still = sqlite_runtime::get_turn(&turn2_id).unwrap().unwrap();
    assert_eq!(still.status, TurnStatus::DraftReady);
    assert_eq!(
        still.find_attempt(&attempt_b).unwrap().status,
        AttemptStatus::DraftReady
    );

    // edit-stale on active attempt B
    sqlite_runtime::mark_stale_after_edit(
        &campaign_id,
        &conversation_id,
        &turn2_id,
        &attempt_b,
        "user edited text",
    )
    .expect("gateway edit-stale");
    conversations.invalidate();
    assert_eq!(
        conversations
            .get(&conversation_id)
            .unwrap()
            .find_node(&first.variant_id)
            .unwrap()
            .active()
            .unwrap()
            .content,
        "user edited text"
    );
    let stale_att = sqlite_runtime::get_turn(&turn2_id)
        .unwrap()
        .unwrap()
        .find_attempt(&attempt_b)
        .unwrap()
        .clone();
    assert_eq!(stale_att.status, AttemptStatus::Stale);
    // original draft_hash preserved so Accept can detect mismatch
    assert_eq!(stale_att.draft_hash, compute_draft_hash("draft b"));

    // scope mismatch fails closed
    let err = sqlite_runtime::create_draft_attempt(DraftAttemptRequest {
        campaign_id: &Id::from_str("wrong-campaign"),
        conversation_id: &conversation_id,
        turn_id: &turn2_id,
        attempt_id: &Id::new(),
        draft_text: "nope",
        pending_temporary_instances: vec![],
        provenance: None,
    });
    assert!(err.is_err());

    // ── 3. restart recovery fails the still-active pre-accept turn2 ───────
    // (Stale attempt keeps turn non-terminal; recovery must close it.)
    assert_eq!(
        sqlite_runtime::get_turn(&turn2_id).unwrap().unwrap().status,
        TurnStatus::DraftReady
    );
    let failed = sqlite_runtime::recover_turns_on_startup().unwrap();
    assert!(
        failed >= 1,
        "recovery must fail incomplete pre-accept turns"
    );
    let recovered = sqlite_runtime::get_turn(&turn2_id).unwrap().unwrap();
    assert_eq!(recovered.status, TurnStatus::Failed);
    // Stale remains Stale (terminal-ish attempt status); recovery only forces
    // Generating/DraftReady/DerivingState/AwaitingAcceptance → Failed.
    assert_eq!(
        recovered.find_attempt(&attempt_b).unwrap().status,
        AttemptStatus::Stale
    );

    // accepted turn remains terminal after recovery
    let still_committed = sqlite_runtime::get_turn(&turn_id).unwrap().unwrap();
    assert_eq!(still_committed.status, TurnStatus::Committed);

    // Reopen authority DB (not the process cache) and confirm Accept finalized the
    // first draft node. Later ConversationStore mutations must not clobber Final.
    let reopened2 = Database::open(&db_path).unwrap();
    let conv_persisted = SqliteProductionRepository::get_conversation(&reopened2, &conversation_id)
        .unwrap()
        .unwrap();
    let final_status = conv_persisted
        .find_node(&draft.variant_id)
        .unwrap()
        .active()
        .unwrap()
        .status
        .clone();
    assert_eq!(
        final_status,
        VariantStatus::Final,
        "first accepted node must remain Final after later turns/recovery"
    );
}
