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
use storyforge_infra_sqlite::cutover::{CutoverPlan, CutoverRequest, recover_or_verify};
use storyforge_infra_sqlite::production::SqliteProductionRepository;
use storyforge_lib::sqlite_runtime;
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

    let outcome =
        sqlite_runtime::accept_by_variant(&campaign_id, &conversation_id, &draft_node, true)
            .expect("force accept uses the regenerated SQLite Attempt");
    assert!(outcome.commit_as_degraded);
    assert_eq!(outcome.turn_status, TurnStatus::Degraded);

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
}
