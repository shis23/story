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
use storyforge_infra_sqlite::production::{
    AcceptFault, AcceptOutcome, AcceptTurnRequest, SqliteProductionRepository, compute_draft_hash,
};
use storyforge_infra_sqlite::{Database, SqliteError};
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

fn persist_request_batch(f: &mut Fixture) {
    let mut turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    turn.find_attempt_mut(&f.attempt_id)
        .unwrap()
        .pending_state_changes = Some(f.batch.clone());
    SqliteProductionRepository::save_turn(&mut f.db, &turn).unwrap();
}

fn assert_accept_unchanged(f: &Fixture) {
    let campaign = SqliteProductionRepository::get_campaign(&f.db, &f.campaign_id)
        .unwrap()
        .unwrap();
    assert_eq!(campaign.revision, 0);
    assert_eq!(campaign.chronicle_revision, 0);
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
    assert!(
        SqliteProductionRepository::list_summaries(&f.db, &f.campaign_id)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        SqliteProductionRepository::count_commit_ledger(&f.db).unwrap(),
        0
    );
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
fn accept_rejects_batch_without_finalize_variant_with_zero_side_effects() {
    let mut f = fixture();
    f.batch
        .mutations
        .retain(|mutation| !matches!(mutation, Mutation::FinalizeVariant { .. }));
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("FinalizeVariant"));
    assert_accept_unchanged(&f);
}

#[test]
fn accept_rejects_duplicate_finalize_variant_with_zero_side_effects() {
    let mut f = fixture();
    f.batch.mutations.push(Mutation::FinalizeVariant {
        variant_id: f.node_id.clone(),
    });
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("FinalizeVariant"));
    assert_accept_unchanged(&f);
}

#[test]
fn accept_rejects_wrong_finalize_variant_with_zero_side_effects() {
    let mut f = fixture();
    for mutation in &mut f.batch.mutations {
        if let Mutation::FinalizeVariant { variant_id } = mutation {
            *variant_id = Id::from_str("wrong-variant");
        }
    }
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("FinalizeVariant"));
    assert_accept_unchanged(&f);
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
fn ledger_replay_rejects_campaign_turn_attempt_ownership_drift() {
    let mut f = fixture();
    SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap();

    let other_campaign_id = Id::from_str("camp-ledger-other");
    let other_conversation_id = Id::from_str("conv-ledger-other");
    let mut other_campaign = Campaign::new(Id::from_str("card-ledger-other"), "Other");
    other_campaign.id = other_campaign_id.clone();
    other_campaign.conversation_id = Some(other_conversation_id.clone());
    let mut other_conversation = Conversation::new(None, Some(other_campaign_id.clone()));
    other_conversation.id = other_conversation_id;
    SqliteProductionRepository::bootstrap_campaign(&mut f.db, &other_campaign, &other_conversation)
        .unwrap();
    f.db.connection()
        .execute(
            "UPDATE mutation_commits SET campaign_id = ?1 WHERE commit_id = ?2",
            rusqlite::params![other_campaign_id.as_str(), f.batch.commit_id.as_str()],
        )
        .unwrap();

    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("ledger ownership"));
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
fn get_turn_by_variant_preserves_unique_and_missing_lookup_paths() {
    let f = fixture();

    let found = SqliteProductionRepository::get_turn_by_variant(&f.db, &f.node_id)
        .unwrap()
        .unwrap();
    assert_eq!(found.turn_id, f.turn_id);
    assert!(
        SqliteProductionRepository::get_turn_by_variant(&f.db, &Id::from_str("missing-variant"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn get_turn_by_variant_allows_regenerate_attempts_on_the_same_turn() {
    let mut f = fixture();
    let mut turn = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    let mut duplicate = turn.attempts[0].clone();
    turn.attempts[0].status = AttemptStatus::Superseded;
    duplicate.attempt_id = Id::from_str("attempt-duplicate-variant");
    duplicate.status = AttemptStatus::AwaitingAcceptance;
    turn.attempts.push(duplicate);
    SqliteProductionRepository::save_turn(&mut f.db, &turn).unwrap();

    let resolved = SqliteProductionRepository::get_turn_by_variant(&f.db, &f.node_id)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.turn_id, f.turn_id);
    assert_eq!(resolved.attempts.len(), 2);
    assert_eq!(
        resolved
            .find_attempt_by_variant(&f.node_id)
            .unwrap()
            .attempt_id,
        Id::from_str("attempt-duplicate-variant")
    );
}

#[test]
fn get_turn_by_variant_rejects_matches_owned_by_different_turns() {
    let mut f = fixture();
    let original_attempt = SqliteProductionRepository::get_attempt(&f.db, &f.attempt_id)
        .unwrap()
        .unwrap();
    let mut second = TurnRecord::new(
        f.campaign_id.clone(),
        f.conversation_id.clone(),
        Id::from_str("input-duplicate-variant"),
        0,
    );
    second.turn_id = Id::from_str("turn-duplicate-variant");
    second.status = TurnStatus::Failed;
    let mut duplicate = original_attempt;
    duplicate.attempt_id = Id::from_str("attempt-cross-turn-duplicate-variant");
    duplicate.status = AttemptStatus::Stale;
    duplicate.pending_state_changes = None;
    second.attempts.push(duplicate);
    SqliteProductionRepository::save_turn(&mut f.db, &second).unwrap();

    let err = SqliteProductionRepository::get_turn_by_variant(&f.db, &f.node_id).unwrap_err();
    assert!(matches!(
        err,
        SqliteError::Conflict(message)
            if message.contains("variant_id") && message.contains("multiple turns")
    ));
}

#[test]
fn get_turn_rejects_payload_id_drift_instead_of_rehanging_record() {
    let f = fixture();
    let mut payload: serde_json::Value =
        f.db.connection()
            .query_row(
                "SELECT payload_json FROM turns WHERE turn_id = ?1",
                [f.turn_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .map(|raw| serde_json::from_str(&raw).unwrap())
            .unwrap();
    payload["turn_id"] = serde_json::json!("rehung-turn");
    f.db.connection()
        .execute(
            "UPDATE turns SET payload_json = ?1 WHERE turn_id = ?2",
            rusqlite::params![serde_json::to_string(&payload).unwrap(), f.turn_id.as_str()],
        )
        .unwrap();
    let err = SqliteProductionRepository::get_turn(&f.db, &f.turn_id).unwrap_err();
    assert!(err.to_string().contains("turn_id"));
}

#[test]
fn get_turn_rejects_structured_scope_status_and_base_revision_drift() {
    let mut f = fixture();
    let other_campaign_id = Id::from_str("camp-other-scope");
    let other_conversation_id = Id::from_str("conv-other-scope");
    let mut other_campaign = Campaign::new(Id::from_str("card-other-scope"), "Other");
    other_campaign.id = other_campaign_id.clone();
    other_campaign.conversation_id = Some(other_conversation_id.clone());
    let mut other_conversation = Conversation::new(None, Some(other_campaign_id.clone()));
    other_conversation.id = other_conversation_id.clone();
    SqliteProductionRepository::bootstrap_campaign(&mut f.db, &other_campaign, &other_conversation)
        .unwrap();
    f.db.connection()
        .execute(
            "UPDATE turns SET campaign_id = ?1, conversation_id = ?2, status = 'committed', base_campaign_revision = 9 WHERE turn_id = ?3",
            rusqlite::params![
                other_campaign_id.as_str(),
                other_conversation_id.as_str(),
                f.turn_id.as_str()
            ],
        )
        .unwrap();
    let err = SqliteProductionRepository::get_turn(&f.db, &f.turn_id).unwrap_err();
    assert!(err.to_string().contains("structured"));
}

#[test]
fn get_attempt_rejects_payload_id_drift() {
    let f = fixture();
    let mut payload: serde_json::Value =
        f.db.connection()
            .query_row(
                "SELECT payload_json FROM turn_attempts WHERE attempt_id = ?1",
                [f.attempt_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .map(|raw| serde_json::from_str(&raw).unwrap())
            .unwrap();
    payload["attempt_id"] = serde_json::json!("rehung-attempt");
    f.db.connection()
        .execute(
            "UPDATE turn_attempts SET payload_json = ?1 WHERE attempt_id = ?2",
            rusqlite::params![
                serde_json::to_string(&payload).unwrap(),
                f.attempt_id.as_str()
            ],
        )
        .unwrap();
    let err = SqliteProductionRepository::get_attempt(&f.db, &f.attempt_id).unwrap_err();
    assert!(err.to_string().contains("attempt_id"));
}

#[test]
fn get_attempt_and_parent_turn_reject_structured_ownership_and_field_drift() {
    let mut f = fixture();
    let mut second = TurnRecord::new(
        f.campaign_id.clone(),
        f.conversation_id.clone(),
        Id::from_str("input-terminal"),
        0,
    );
    second.turn_id = Id::from_str("turn-terminal-owner");
    second.status = TurnStatus::Failed;
    SqliteProductionRepository::save_turn(&mut f.db, &second).unwrap();
    f.db.connection()
        .execute(
            "UPDATE turn_attempts SET turn_id = ?1, variant_id = 'wrong-variant', draft_hash = 'wrong-hash', status = 'committed' WHERE attempt_id = ?2",
            rusqlite::params![second.turn_id.as_str(), f.attempt_id.as_str()],
        )
        .unwrap();
    let attempt_err = SqliteProductionRepository::get_attempt(&f.db, &f.attempt_id).unwrap_err();
    assert!(attempt_err.to_string().contains("structured"));
    let turn_err = SqliteProductionRepository::get_turn(&f.db, &f.turn_id).unwrap_err();
    assert!(turn_err.to_string().contains("attempt"));
}

#[test]
fn save_turn_rejects_rehanging_an_existing_attempt_id() {
    let mut f = fixture();
    let existing_attempt = SqliteProductionRepository::get_attempt(&f.db, &f.attempt_id)
        .unwrap()
        .unwrap();
    let mut second = TurnRecord::new(
        f.campaign_id.clone(),
        f.conversation_id.clone(),
        Id::from_str("input-rehang"),
        0,
    );
    second.turn_id = Id::from_str("turn-rehang-target");
    second.status = TurnStatus::Failed;
    second.attempts.push(existing_attempt);

    let err = SqliteProductionRepository::save_turn(&mut f.db, &second).unwrap_err();
    assert!(err.to_string().contains("rehang"));
    let original = SqliteProductionRepository::get_turn(&f.db, &f.turn_id)
        .unwrap()
        .unwrap();
    assert!(original.find_attempt(&f.attempt_id).is_some());
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
fn accept_rejects_knowledge_owned_by_another_existing_campaign() {
    let mut f = fixture();
    let other_campaign_id = Id::from_str("camp-other-knowledge");
    let other_conversation_id = Id::from_str("conv-other-knowledge");
    let mut other_campaign = Campaign::new(Id::from_str("card-other-knowledge"), "Other");
    other_campaign.id = other_campaign_id.clone();
    other_campaign.conversation_id = Some(other_conversation_id.clone());
    let mut other_conversation = Conversation::new(None, Some(other_campaign_id.clone()));
    other_conversation.id = other_conversation_id;
    SqliteProductionRepository::bootstrap_campaign(&mut f.db, &other_campaign, &other_conversation)
        .unwrap();
    f.batch.mutations.insert(
        0,
        Mutation::UpsertKnowledge(Box::new(KnowledgeMutation {
            entry_id: Id::from_str("knowledge-wrong-campaign"),
            campaign_id: other_campaign_id,
            character_id: Id::from_str("character-other"),
            knowledge_text: "不应跨 Campaign 写入".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: PropagationPolicy::Open,
        })),
    );
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("knowledge"));
    assert_accept_unchanged(&f);
}

#[test]
fn accept_rejects_chronicle_b_in_turn_batch() {
    let mut f = fixture();
    for mutation in &mut f.batch.mutations {
        if let Mutation::UpsertSummary(summary) = mutation {
            summary.level = 1;
        }
    }
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("Chronicle A"));
    assert_accept_unchanged(&f);
}

#[test]
fn accept_rejects_leaf_summary_with_covers_or_covered_by() {
    for use_covered_by in [false, true] {
        let mut f = fixture();
        for mutation in &mut f.batch.mutations {
            if let Mutation::UpsertSummary(summary) = mutation {
                if use_covered_by {
                    summary.covered_by = Some(summary.id.clone());
                } else {
                    summary.covers.push(summary.id.clone());
                }
            }
        }
        persist_request_batch(&mut f);
        let err = SqliteProductionRepository::accept_turn(
            &mut f.db,
            request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
        )
        .unwrap_err();
        assert!(err.to_string().contains("Chronicle A"));
        assert_accept_unchanged(&f);
    }
}

#[test]
fn accept_rejects_chronicle_a_with_wrong_source_scope() {
    let mut f = fixture();
    for mutation in &mut f.batch.mutations {
        if let Mutation::UpsertSummary(summary) = mutation {
            summary.conversation_id = Id::from_str("wrong-conversation");
        }
    }
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("scope"));
    assert_accept_unchanged(&f);
}

#[test]
fn accept_rejects_chronicle_a_with_wrong_lineage() {
    let mut f = fixture();
    for mutation in &mut f.batch.mutations {
        if let Mutation::UpsertSummary(summary) = mutation {
            summary.lineage_id = Some(Id::from_str("wrong-lineage"));
        }
    }
    persist_request_batch(&mut f);
    let err = SqliteProductionRepository::accept_turn(
        &mut f.db,
        request(&f.turn_id, &f.attempt_id, &f.batch, &f.draft_hash),
    )
    .unwrap_err();
    assert!(err.to_string().contains("lineage"));
    assert_accept_unchanged(&f);
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

/// #22：MVU 翻译 payload 的 save/get/list/delete 往返（V005 表）。
#[test]
fn mvu_payload_roundtrip_overwrite_and_delete() {
    let mut db = Database::open_in_memory().unwrap();
    let src = Id::from_str("char-mvu");
    let payload = serde_json::json!({
        "source_character_id": "char-mvu",
        "character_name": "Alice",
        "analyzed_at": "2026-07-27T00:00:00Z",
        "translation": { "update_rules": ["damage reduces hp"] }
    });
    SqliteProductionRepository::save_mvu_payload(&mut db, &src, "Alice", &payload).unwrap();

    let loaded = SqliteProductionRepository::get_mvu_payload(&db, &src)
        .unwrap()
        .unwrap();
    assert_eq!(loaded["character_name"], "Alice");
    assert_eq!(
        loaded["translation"]["update_rules"][0],
        "damage reduces hp"
    );

    // 覆盖：同 source_character_id upsert 不产生第二行
    let mut updated = payload.clone();
    updated["character_name"] = serde_json::json!("Alice-改");
    SqliteProductionRepository::save_mvu_payload(&mut db, &src, "Alice-改", &updated).unwrap();
    let all = SqliteProductionRepository::list_mvu_payloads(&db).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0]["character_name"], "Alice-改");

    // 删除：首删 true、再删 false、读回空
    assert!(SqliteProductionRepository::delete_mvu_payload(&mut db, &src).unwrap());
    assert!(!SqliteProductionRepository::delete_mvu_payload(&mut db, &src).unwrap());
    assert!(
        SqliteProductionRepository::get_mvu_payload(&db, &src)
            .unwrap()
            .is_none()
    );
}
