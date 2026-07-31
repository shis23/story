//! Opt-in SQLite production lifecycle proof.
//!
//! This is intentionally an integration test binary: it exercises the same
//! `sqlite_runtime` authority selected by `AppState`, not a JSON mirror or a
//! repository-only substitute.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::conversation::VariantStatus;
use storyforge_domain::turn::{
    AttemptStatus, QualityReport, QualitySeverity, QualityWarning, QualityWarningCode, TurnRecord,
    TurnStatus,
};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_infra_sqlite::cutover::{CutoverPlan, CutoverRequest, recover_or_verify};
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::StorageFacade;
use storyforge_lib::turn_lifecycle::{AcceptError, append_regenerate_attempt, new_draft_attempt};

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
            "id": "campaign-1", "card_id": "card-1", "name": "SQLite run",
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

#[test]
fn sqlite_optin_cutover_write_regenerate_force_accept_and_restart_recovery() {
    let temp = tempfile::tempdir().unwrap();
    let source_dir = temp.path();
    let db_path = source_dir.join("storyforge.sqlite3");
    write_cutover_source(source_dir);

    let cutover = CutoverRequest {
        plan: CutoverPlan::new(source_dir, &db_path),
        label: "sqlite-optin-lifecycle-test".into(),
    };
    recover_or_verify(&cutover).expect("cutover must produce the SQLite authority");
    sqlite_runtime::activate(&db_path).expect("activate the production SQLite runtime");

    // Remove the legacy source after cutover. The live lifecycle must keep
    // working from SQLite; a JSON fallback would now fail this test.
    fs::remove_file(source_dir.join("campaigns.json")).unwrap();
    fs::remove_file(source_dir.join("conversations").join("conversation-1.json")).unwrap();
    fs::remove_file(source_dir.join("turns.json")).unwrap();

    let persistence = sqlite_runtime::conversation_persistence()
        .expect("SQLite runtime supplies the AppState conversation authority");
    let conversations = Arc::new(ConversationStore::with_persistence(persistence));
    let campaign_id = Id::from_str("campaign-1");
    let conversation_id = Id::from_str("conversation-1");

    let picker_campaigns = sqlite_runtime::list_campaigns()
        .expect("campaign picker reads the SQLite authority after JSON removal");
    assert_eq!(picker_campaigns.len(), 1);
    assert_eq!(picker_campaigns[0].id, campaign_id);

    let facade = StorageFacade::new(
        source_dir.to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    );
    facade
        .validate_runtime_authority()
        .expect("facade and SQLite runtime must name the same authority");
    assert!(
        !facade.has_json_writers(),
        "SQLite facade must not construct legacy Campaign/Turn writers"
    );
    assert_eq!(
        facade
            .list_campaigns(Some(&Id::from_str("other-card")))
            .expect("SQLite facade applies card filtering")
            .len(),
        0,
        "SQLite facade must not drop the requested card filter"
    );
    assert_eq!(
        facade
            .list_campaigns(Some(&Id::from_str("card-1")))
            .expect("matching SQLite campaign remains visible")
            .len(),
        1
    );

    let mismatched_json = StorageFacade::new(
        source_dir.to_path_buf(),
        PinnedBackend::new(StorageBackend::Json, BackendSource::Default),
    );
    assert!(
        mismatched_json.validate_runtime_authority().is_err(),
        "a JSON facade must reject an already-active SQLite runtime"
    );
    assert!(
        sqlite_runtime::activate(source_dir.join("different.sqlite3")).is_err(),
        "SQLite activation must reject a different second database path"
    );

    // write -> Draft: the normal ConversationStore API is what pipeline uses.
    let user_node = conversations
        .append_user_message(&conversation_id, "advance the scene".into())
        .expect("user message persists in SQLite");
    let turn = TurnRecord::new(campaign_id.clone(), conversation_id.clone(), user_node, 0);
    let turn_id = turn.turn_id.clone();
    sqlite_runtime::save_turn(&turn).expect("new Turn persists in SQLite");

    let draft_node = conversations
        .append_ai_draft(&conversation_id, "first SQLite draft".into(), None)
        .expect("draft persists in SQLite");
    let first_attempt =
        new_draft_attempt(Id::new(), draft_node.clone(), "first SQLite draft", vec![]);
    sqlite_runtime::update_turn_record(&turn_id, |record| {
        record.attempts.push(first_attempt);
        record.status = TurnStatus::AwaitingAcceptance;
        record.attempts[0].status = AttemptStatus::AwaitingAcceptance;
        record.touch();
    })
    .expect("first Attempt persists in SQLite");

    // regenerate: replace_active_variant is the exact pipeline mutation. It
    // keeps the node id, so this also proves the newest Attempt wins selection.
    conversations
        .replace_active_variant(
            &conversation_id,
            &draft_node,
            "regenerated SQLite draft".into(),
            None,
        )
        .expect("regenerate draft persists in SQLite");
    let regenerated_attempt_id = Id::new();
    let regenerated_attempt = new_draft_attempt(
        regenerated_attempt_id.clone(),
        draft_node.clone(),
        "regenerated SQLite draft",
        vec![],
    );
    sqlite_runtime::update_turn_record(&turn_id, |record| {
        append_regenerate_attempt(record, regenerated_attempt);
        let attempt = record
            .find_attempt_mut(&regenerated_attempt_id)
            .expect("regenerated Attempt exists");
        attempt.status = AttemptStatus::AwaitingAcceptance;
        attempt.quality_report = Some(QualityReport {
            warnings: vec![QualityWarning {
                code: QualityWarningCode::TooShort { char_count: 1 },
                message: "exercise force accept".into(),
                severity: QualitySeverity::Error,
            }],
        });
        record.status = TurnStatus::AwaitingAcceptance;
        record.touch();
    })
    .expect("regenerated Attempt and quality report persist in SQLite");

    // Normal Accept respects the persisted quality error; force accept is a
    // distinct, explicit Degraded path rather than a hidden bypass.
    assert!(matches!(
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &draft_node, false,),
        Err(AcceptError::QualityBlocked { error_count: 1 })
    ));

    // Error path: a scope mismatch must not fall back to an old JSON turn.
    assert!(matches!(
        sqlite_runtime::accept_by_variant(
            &Id::from_str("wrong-campaign"),
            &conversation_id,
            &draft_node,
            false,
        ),
        Err(AcceptError::CampaignScopeMismatch { .. })
    ));

    // Adapter parity with the JSON accept service: scope validation belongs
    // before the requested-conversation lookup. A nonexistent foreign id must
    // therefore remain a typed scope error, not degrade into Storage(missing).
    let foreign_conversation = Id::from_str("wrong-conversation");
    let campaign_before =
        serde_json::to_value(sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap()).unwrap();
    let turn_before =
        serde_json::to_value(sqlite_runtime::get_turn(&turn_id).unwrap().unwrap()).unwrap();
    let conversation_before = serde_json::to_value(
        sqlite_runtime::get_conversation(&conversation_id)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    let err =
        sqlite_runtime::accept_by_variant(&campaign_id, &foreign_conversation, &draft_node, false)
            .expect_err("foreign conversation must be rejected before storage lookup");
    match err {
        AcceptError::ConversationScopeMismatch {
            turn_conversation,
            requested,
        } => {
            assert_eq!(turn_conversation, conversation_id.to_string());
            assert_eq!(requested, foreign_conversation.to_string());
        }
        other => panic!("expected ConversationScopeMismatch, got {other:?}"),
    }
    assert_eq!(
        serde_json::to_value(sqlite_runtime::get_campaign(&campaign_id).unwrap().unwrap(),)
            .unwrap(),
        campaign_before,
        "scope rejection must not commit campaign mutations"
    );
    assert_eq!(
        serde_json::to_value(sqlite_runtime::get_turn(&turn_id).unwrap().unwrap()).unwrap(),
        turn_before,
        "scope rejection must not mutate the turn or attempt"
    );
    assert_eq!(
        serde_json::to_value(
            sqlite_runtime::get_conversation(&conversation_id)
                .unwrap()
                .unwrap(),
        )
        .unwrap(),
        conversation_before,
        "scope rejection must not finalize the draft variant"
    );

    let outcome =
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &draft_node, true)
            .expect("force accept uses the regenerated SQLite Attempt");
    assert!(outcome.commit_as_degraded);
    assert_eq!(outcome.turn_status, TurnStatus::Degraded);
    assert_eq!(
        outcome.campaign_revision_after, 1,
        "first commit must report the UoW-validated target revision"
    );

    let replay =
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &draft_node, false)
            .expect("an already committed SQLite accept must replay through its ledger");
    assert_eq!(replay.turn_status, TurnStatus::Degraded);
    assert!(replay.commit_as_degraded);

    conversations.invalidate();
    let final_conversation = conversations
        .get(&conversation_id)
        .expect("SQLite conversation remains readable after accept");
    assert_eq!(
        final_conversation
            .find_node(&draft_node)
            .unwrap()
            .active()
            .unwrap()
            .status,
        VariantStatus::Final
    );
    let accepted_turn = sqlite_runtime::get_turn(&turn_id)
        .unwrap()
        .expect("accepted Turn exists in SQLite");
    assert_eq!(accepted_turn.status, TurnStatus::Degraded);
    assert_eq!(
        accepted_turn
            .find_attempt(&regenerated_attempt_id)
            .unwrap()
            .status,
        AttemptStatus::Committed
    );

    // The committed revision is taken from the UoW-validated batch
    // (target = expected + 1), never from a post-commit read-back: a commit
    // that succeeded must report success with the true revision so callers
    // run invalidate / indexing / chronicle enqueue. A second turn proves the
    // returned revision matches the durable authority for a fresh commit.
    let second_user = conversations
        .append_user_message(&conversation_id, "second-turn revision".into())
        .expect("second turn user message persists in SQLite");
    let second_turn = TurnRecord::new(campaign_id.clone(), conversation_id.clone(), second_user, 1);
    let second_turn_id = second_turn.turn_id.clone();
    sqlite_runtime::save_turn(&second_turn).expect("second Turn persists in SQLite");
    let second_draft = conversations
        .append_ai_draft(&conversation_id, "second-turn draft".into(), None)
        .expect("second draft persists in SQLite");
    let second_attempt =
        new_draft_attempt(Id::new(), second_draft.clone(), "second-turn draft", vec![]);
    sqlite_runtime::update_turn_record(&second_turn_id, |record| {
        record.attempts.push(second_attempt);
        record.status = TurnStatus::AwaitingAcceptance;
        record.attempts[0].status = AttemptStatus::AwaitingAcceptance;
        record.touch();
    })
    .expect("second Attempt persists in SQLite");
    let second_outcome =
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &second_draft, false)
            .expect("second accept commits successfully");
    assert_eq!(
        second_outcome.campaign_revision_after, 2,
        "UoW-validated batch target revision must be reported"
    );
    let second_committed = sqlite_runtime::get_turn(&second_turn_id)
        .unwrap()
        .expect("second accepted Turn persists in SQLite");
    assert_eq!(second_committed.status, TurnStatus::Committed);
    let campaign_after_second = sqlite_runtime::get_campaign(&campaign_id)
        .unwrap()
        .expect("campaign remains readable after second accept");
    assert_eq!(
        campaign_after_second.revision, second_outcome.campaign_revision_after,
        "reported revision must match the durable authority"
    );

    // TurnWorkflow 级：SQLite 首稿 UoW 失败必须把仍为 Generating 的 Turn
    // 标 Failed（failure_reason 写盘），否则它会永远占用 active-turn barrier。
    // 用 fault 注入走真实回滚路径（BeforeCommit）。
    let workflow =
        storyforge_lib::TurnWorkflow::new(Arc::new(facade.clone()), conversations.clone());
    let draft_fault_turn =
        TurnRecord::new(campaign_id.clone(), conversation_id.clone(), Id::new(), 2);
    let draft_fault_turn_id = draft_fault_turn.turn_id.clone();
    sqlite_runtime::save_turn(&draft_fault_turn).expect("fault Turn persists in SQLite");
    sqlite_runtime::fail_draft_uow_for_test(true);
    let draft_fault_err = workflow
        .create_draft_attempt(storyforge_lib::DraftAttemptRequest {
            campaign_id: &campaign_id,
            conversation_id: &conversation_id,
            turn_id: &draft_fault_turn_id,
            attempt_id: &Id::new(),
            provisional_variant_id: Some(&Id::new()),
            draft_text: "fault draft",
            pending_temporary_instances: vec![],
            provenance: None,
        })
        .expect_err("injected UoW fault must fail the draft");
    sqlite_runtime::fail_draft_uow_for_test(false);
    assert!(!draft_fault_err.is_empty());
    let failed_turn = sqlite_runtime::get_turn(&draft_fault_turn_id)
        .unwrap()
        .expect("failed Turn persists in SQLite");
    assert_eq!(
        failed_turn.status,
        TurnStatus::Failed,
        "SQLite draft failure must mark a Generating Turn Failed to release the active-turn barrier"
    );
    assert!(
        failed_turn
            .failure_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("sqlite preaccept draft 失败")),
        "unexpected failure_reason: {:?}",
        failed_turn.failure_reason
    );

    // 已成功推进的状态绝不被失败写回降级：Committed Turn 被 preaccept
    // 校验拒绝（expected Generating）时保持原状态，不写 failure_reason。
    let mut settled_turn =
        TurnRecord::new(campaign_id.clone(), conversation_id.clone(), Id::new(), 2);
    settled_turn.status = TurnStatus::Committed;
    let settled_turn_id = settled_turn.turn_id.clone();
    sqlite_runtime::save_turn(&settled_turn).expect("settled Turn persists in SQLite");
    let settled_err = workflow
        .create_draft_attempt(storyforge_lib::DraftAttemptRequest {
            campaign_id: &campaign_id,
            conversation_id: &conversation_id,
            turn_id: &settled_turn_id,
            attempt_id: &Id::new(),
            provisional_variant_id: Some(&Id::new()),
            draft_text: "settled draft",
            pending_temporary_instances: vec![],
            provenance: None,
        })
        .expect_err("non-Generating turn must fail the draft UoW");
    assert!(!settled_err.is_empty());
    let preserved = sqlite_runtime::get_turn(&settled_turn_id)
        .unwrap()
        .expect("settled Turn persists in SQLite");
    assert_eq!(
        preserved.status,
        TurnStatus::Committed,
        "a settled Turn must never be downgraded by a failed draft write-back"
    );
    assert!(
        preserved.failure_reason.is_none(),
        "no failure_reason may be written onto a settled Turn: {:?}",
        preserved.failure_reason
    );

    // restart/recovery: an incomplete later Turn is failed from the same DB,
    // and a newly opened connection observes the terminal state.
    let later_user = conversations
        .append_user_message(&conversation_id, "second turn before restart".into())
        .unwrap();
    let incomplete_turn =
        TurnRecord::new(campaign_id.clone(), conversation_id.clone(), later_user, 1);
    let incomplete_turn_id = incomplete_turn.turn_id.clone();
    sqlite_runtime::save_turn(&incomplete_turn).unwrap();
    assert_eq!(
        sqlite_runtime::recover_turns_on_startup().unwrap(),
        1,
        "restart recovery must see the SQLite-created active Turn"
    );
    let recovered = sqlite_runtime::get_turn(&incomplete_turn_id)
        .unwrap()
        .expect("recovered Turn persists");
    assert_eq!(recovered.status, TurnStatus::Failed);

    let reopened = Database::open(&db_path).expect("reopen SQLite after simulated restart");
    let persisted = SqliteProductionRepository::get_turn(&reopened, &incomplete_turn_id)
        .unwrap()
        .expect("reopened database contains recovery result");
    assert_eq!(persisted.status, TurnStatus::Failed);

    // A terminal replay is ledger-backed and must not depend on unrelated
    // live conversation deserialization. Corrupt that payload only after all
    // normal lifecycle checks, then verify the accepted turn still replays.
    reopened
        .connection()
        .execute(
            "UPDATE conversations SET payload_json = '{' WHERE conversation_id = ?1",
            [conversation_id.as_str()],
        )
        .unwrap();
    drop(reopened);
    let replay_without_live_conversation =
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &draft_node, false)
            .expect("terminal replay must reach the SQLite ledger before conversation reads");
    assert_eq!(
        replay_without_live_conversation.turn_status,
        TurnStatus::Degraded
    );
    assert!(replay_without_live_conversation.commit_as_degraded);
}
