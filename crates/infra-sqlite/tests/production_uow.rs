use std::sync::{Arc, Barrier};

use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character_knowledge::{KnowledgeSource, PropagationPolicy};
use storyforge_domain::conversation::{Conversation, VariantStatus};
use storyforge_domain::story_task::{StoryTask, TaskStatus};
use storyforge_domain::turn::{
    AttemptStatus, KnowledgeMutation, Mutation, MutationBatch, TurnAttempt, TurnRecord, TurnStatus,
};
use storyforge_infra_sqlite::Database;
use storyforge_infra_sqlite::production::{
    AcceptFault, AcceptOutcome, AcceptTurnRequest, SqliteProductionRepository, compute_draft_hash,
};
use tempfile::TempDir;

struct Fixture {
    _dir: TempDir,
    db: Database,
    campaign_id: Id,
    conversation_id: Id,
    node_id: Id,
    turn_id: Id,
    attempt_id: Id,
    draft_hash: String,
    batch: MutationBatch,
}

fn fixture() -> Fixture {
    let dir = TempDir::new().unwrap();
    let mut db = Database::open(dir.path().join("storyforge.sqlite3")).unwrap();

    let campaign_id = Id::from_str("camp-production");
    let conversation_id = Id::from_str("conv-production");
    let mut campaign = Campaign::new(Id::from_str("card-production"), "Production UoW");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());

    let mut conversation = Conversation::new(None, Some(campaign_id.clone()));
    conversation.id = conversation_id.clone();
    conversation.append_message(
        storyforge_domain::conversation::Role::User,
        "继续故事".into(),
    );
    let node_id = conversation.append_ai_draft("草稿正文".into(), None);

    SqliteProductionRepository::bootstrap_campaign(&mut db, &campaign, &conversation).unwrap();

    let mut batch = MutationBatch::new(Id::from_str("commit-production"), 0);
    batch.mutations.push(Mutation::SetVariable {
        instance_id: None,
        key: "weather".into(),
        value: serde_json::json!("rain"),
        turn: 1,
    });
    batch
        .mutations
        .push(Mutation::UpsertSummary(Box::new(RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            1,
            "雾港落雨，主角继续前进。".into(),
        ))));
    batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: node_id.clone(),
    });

    let mut turn = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        Id::from_str("input-node"),
        0,
    );
    turn.turn_id = Id::from_str("turn-production");
    turn.status = TurnStatus::AwaitingAcceptance;
    let attempt_id = Id::from_str("attempt-production");
    let draft_hash = compute_draft_hash("草稿正文");
    turn.attempts.push(TurnAttempt {
        attempt_id: attempt_id.clone(),
        variant_id: node_id.clone(),
        draft_hash: draft_hash.clone(),
        status: AttemptStatus::AwaitingAcceptance,
        pending_state_changes: Some(batch.clone()),
        derivation: None,
        quality_report: None,
        pending_temporary_instances: vec![],
        provenance: None,
        created_at: "2026-07-13T00:00:00Z".into(),
    });
    SqliteProductionRepository::save_turn(&mut db, &turn).unwrap();

    Fixture {
        _dir: dir,
        db,
        campaign_id,
        conversation_id,
        node_id,
        turn_id: turn.turn_id,
        attempt_id,
        draft_hash,
        batch,
    }
}

fn request<'a>(
    turn_id: &'a Id,
    attempt_id: &'a Id,
    batch: &'a MutationBatch,
    draft_hash: &'a str,
) -> AcceptTurnRequest<'a> {
    AcceptTurnRequest {
        turn_id,
        attempt_id,
        draft_hash,
        batch,
        terminal_status: TurnStatus::Committed,
    }
}

#[test]
fn accept_commits_turn_campaign_chronicle_and_variant_atomically() {
    let mut f = fixture();
    let outcome = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap();
    assert_eq!(outcome, AcceptOutcome::Applied);

    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 1);
    assert_eq!(campaign.chronicle_revision, 1);
    assert_eq!(
        campaign.get_variable("weather"),
        Some(&serde_json::json!("rain"))
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::Committed);
    assert_eq!(turn.accepted_attempt_id.as_ref(), Some(&f.attempt_id));
    assert_eq!(
        turn.find_attempt(&f.attempt_id).unwrap().status,
        AttemptStatus::Committed
    );

    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation
            .find_node(&f.node_id)
            .unwrap()
            .active()
            .unwrap()
            .status,
        VariantStatus::Final
    );
    assert_eq!(
        SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn repeated_commit_id_is_idempotent_without_revision_bump() {
    let mut f = fixture();
    assert_eq!(
        SqliteProductionRepository::accept_turn(
            &mut f.db,
            request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
        )
        .unwrap(),
        AcceptOutcome::Applied
    );
    assert_eq!(
        SqliteProductionRepository::accept_turn(
            &mut f.db,
            request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
        )
        .unwrap(),
        AcceptOutcome::AlreadyCommitted
    );
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 1);
    assert_eq!(campaign.chronicle_revision, 1);
    assert_eq!(
        SqliteProductionRepository::count_commit_ledger(&f.db).unwrap(),
        1
    );
}

#[test]
fn turn_and_attempt_domain_payloads_round_trip() {
    let f = fixture();
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    let attempt = SqliteProductionRepository::get_attempt(&f.db, &f.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.attempts.len(), 1);
    assert_eq!(attempt.variant_id, f.node_id);
    assert_eq!(attempt.draft_hash, f.draft_hash);
    let stored_batch = attempt.pending_state_changes.unwrap();
    assert_eq!(stored_batch.commit_id, f.batch.commit_id);
    assert_eq!(stored_batch.mutations.len(), f.batch.mutations.len());
}

#[test]
fn accept_supports_every_mutation_variant_in_one_transaction() {
    let mut f = fixture();
    let instance_id = Id::from_str("instance-production");
    let mut instance = CharacterInstance::temporary(f.campaign_id.clone(), "雨巷信使");
    instance.id = instance_id.clone();
    let knowledge_id = Id::from_str("knowledge-production");
    let task_id = Id::from_str("task-production");
    let mut task = StoryTask::user_planned(
        f.campaign_id.clone(),
        "找到钟楼",
        "沿雨巷寻找旧钟楼",
        vec![],
        1,
    );
    task.id = task_id.clone();

    f.batch.mutations.splice(
        0..0,
        [
            Mutation::UpsertInstance(Box::new(instance)),
            Mutation::SetVariable {
                instance_id: Some(instance_id.clone()),
                key: "mood".into(),
                value: serde_json::json!("alert"),
                turn: 1,
            },
            Mutation::UpsertKnowledge(Box::new(KnowledgeMutation {
                entry_id: knowledge_id.clone(),
                campaign_id: f.campaign_id.clone(),
                character_id: instance_id.clone(),
                knowledge_text: "钟楼入口藏在雨巷尽头".into(),
                source: KnowledgeSource::Witnessed,
                source_character_id: None,
                turn_number: 1,
                event_id: None,
                pinned: false,
                propagation: PropagationPolicy::Open,
            })),
            Mutation::UpsertNewTask(Box::new(task)),
            Mutation::SetTaskStatus {
                task_id: task_id.clone(),
                status: TaskStatus::Completed,
            },
        ],
    );
    let mut turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    turn.find_attempt_mut(&f.attempt_id)
        .unwrap()
        .pending_state_changes = Some(f.batch.clone());
    SqliteProductionRepository::save_turn(&mut f.db, &turn).unwrap();

    SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap();

    let instance = SqliteProductionRepository::get_instance(&f.db, &instance_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        instance.get_variable("mood"),
        Some(&serde_json::json!("alert"))
    );
    assert_eq!(
        SqliteProductionRepository::get_task(&f.db, &task_id)
            .unwrap()
            .unwrap()
            .status,
        TaskStatus::Completed
    );
    assert_eq!(
        SqliteProductionRepository::get_knowledge(&f.db, &knowledge_id)
            .unwrap()
            .unwrap()
            .knowledge_text,
        "钟楼入口藏在雨巷尽头"
    );
}

#[test]
fn only_one_active_turn_per_campaign_is_allowed() {
    let mut f = fixture();
    let mut second = TurnRecord::new(
        f.campaign_id.clone(),
        f.conversation_id.clone(),
        Id::from_str("input-second"),
        0,
    );
    second.turn_id = Id::from_str("turn-second");
    let err = SqliteProductionRepository::save_turn(&mut f.db, &second).unwrap_err();
    assert!(err.to_string().contains("active turn"));

    let mut first = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    first.status = TurnStatus::Failed;
    SqliteProductionRepository::save_turn(&mut f.db, &first).unwrap();
    SqliteProductionRepository::save_turn(&mut f.db, &second).unwrap();
}

#[test]
fn injected_failure_rolls_back_every_accept_side_effect() {
    let mut f = fixture();
    let err = SqliteProductionRepository::accept_turn_with_fault(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
        AcceptFault::AfterMutations,
    )
    .unwrap_err();
    assert!(err.to_string().contains("injected"));

    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 0);
    assert_eq!(campaign.chronicle_revision, 0);
    assert_ne!(
        campaign.get_variable("weather"),
        Some(&serde_json::json!("rain"))
    );
    assert!(
        SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        SqliteProductionRepository::count_commit_ledger(&f.db).unwrap(),
        0
    );

    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::AwaitingAcceptance);
    assert_eq!(
        turn.find_attempt(&f.attempt_id).unwrap().status,
        AttemptStatus::AwaitingAcceptance
    );
    let conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        conversation
            .find_node(&f.node_id)
            .unwrap()
            .active()
            .unwrap()
            .status,
        VariantStatus::Draft
    );
}

#[test]
fn revision_conflict_has_no_side_effects() {
    let mut f = fixture();
    f.db.connection()
        .execute(
            "UPDATE campaigns SET revision = 9 WHERE campaign_id = ?1",
            [f.campaign_id.as_str()],
        )
        .unwrap();
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("revision"));
    assert!(
        SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
            .unwrap()
            .unwrap()
            .status,
        TurnStatus::AwaitingAcceptance
    );
}

#[test]
fn request_batch_must_match_the_persisted_attempt_batch() {
    let mut f = fixture();
    if let Mutation::SetVariable { value, .. } = &mut f.batch.mutations[0] {
        *value = serde_json::json!("tampered");
    }
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("persisted"));
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 0);
    assert_ne!(
        campaign.get_variable("weather"),
        Some(&serde_json::json!("tampered"))
    );
}

#[test]
fn edited_conversation_content_invalidates_the_persisted_draft_hash() {
    let mut f = fixture();
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    let mut conversation = SqliteProductionRepository::get_conversation(&f.db, &f.conversation_id)
        .unwrap()
        .unwrap();
    conversation
        .find_node_mut(&f.node_id)
        .unwrap()
        .active_mut()
        .unwrap()
        .content = "已编辑但未重新推导的正文".into();
    SqliteProductionRepository::bootstrap_campaign(&mut f.db, &campaign, &conversation).unwrap();

    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("draft_hash"));
    assert_eq!(
        SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
            .unwrap()
            .unwrap()
            .status,
        TurnStatus::AwaitingAcceptance
    );
    assert_eq!(
        SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
            .unwrap()
            .unwrap()
            .revision,
        0
    );
}

#[test]
fn failure_after_turn_finalization_still_rolls_back_everything() {
    let mut f = fixture();
    let err = SqliteProductionRepository::accept_turn_with_fault(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
        AcceptFault::BeforeLedger,
    )
    .unwrap_err();
    assert!(err.to_string().contains("before ledger"));
    let turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(turn.status, TurnStatus::AwaitingAcceptance);
    assert_eq!(
        turn.find_attempt(&f.attempt_id).unwrap().status,
        AttemptStatus::AwaitingAcceptance
    );
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 0);
    assert_eq!(campaign.chronicle_revision, 0);
    assert_eq!(
        SqliteProductionRepository::count_commit_ledger(&f.db).unwrap(),
        0
    );
}

#[test]
fn concurrent_accepts_serialize_to_one_apply_and_one_replay() {
    let f = fixture();
    let path = f.db.path().to_path_buf();
    let turn_id = f.turn_id.clone();
    let attempt_id = f.attempt_id.clone();
    let batch = f.batch.clone();
    let draft_hash = f.draft_hash.clone();
    drop(f.db);

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let path = path.clone();
        let turn_id = turn_id.clone();
        let attempt_id = attempt_id.clone();
        let batch = batch.clone();
        let draft_hash = draft_hash.clone();
        let barrier = barrier.clone();
        handles.push(std::thread::spawn(move || {
            let mut db = Database::open(path).unwrap();
            barrier.wait();
            SqliteProductionRepository::accept_turn(
                &mut db,
                AcceptTurnRequest {
                    turn_id: &turn_id,
                    attempt_id: &attempt_id,
                    draft_hash: &draft_hash,
                    batch: &batch,
                    terminal_status: TurnStatus::Committed,
                },
            )
            .unwrap()
        }));
    }
    let mut outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    outcomes.sort();
    assert_eq!(
        outcomes,
        vec![AcceptOutcome::Applied, AcceptOutcome::AlreadyCommitted]
    );

    let db = Database::open(path).unwrap();
    let campaign = SqliteProductionRepository::get_campaign(&db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 1);
    assert_eq!(campaign.chronicle_revision, 1);
}
