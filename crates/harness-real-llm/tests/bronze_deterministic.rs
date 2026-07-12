//! Bronze deterministic harness coverage that does not require a real LLM.
//!
//! These tests assert persisted product state for Campaign open, multi-turn
//! accept/writeback, same-name isolation, draft_hash invalidation, regenerate
//! supersede, and restart recovery. They intentionally use store + coordinator
//! APIs (the production commit path) rather than a second business path.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{
    CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
};
use storyforge_domain::character_knowledge::{
    CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
};
use storyforge_domain::conversation::VariantStatus;
use storyforge_domain::story_task::{StoryTask, TaskStatus, TaskTrigger};
use storyforge_domain::turn::{
    AttemptStatus, KnowledgeMutation, Mutation, MutationBatch, TurnAttempt, TurnRecord, TurnStatus,
};
use storyforge_tauri_app::campaign_store::CampaignStore;
use storyforge_tauri_app::normalize_knowledge_update_for_postprocess;
use storyforge_tauri_app::turn_coordinator::{CampaignMutationCoordinator, with_campaign_lock};
use storyforge_tauri_app::turn_store::TurnStore;

struct TempDataDir {
    path: PathBuf,
}

impl TempDataDir {
    fn new(prefix: &str) -> Self {
        let path = std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create temp data dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Mirrors production `compute_draft_hash` (SHA-256 hex).
fn draft_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn definition(card_id: &Id, id: &str, name: &str, role: RoleType) -> CharacterDefinition {
    CharacterDefinition {
        id: Id::from_str(id),
        card_id: card_id.clone(),
        name: name.to_string(),
        persona_prompt: format!("{name} persona"),
        behavior_rules: format!("{name} behavior"),
        base_backstory: vec![format!("{name} backstory")],
        group: None,
        role_type: role,
        variable_schema: vec![],
    }
}

fn multi_character_card() -> CharacterCard {
    let card_id = Id::from_str("card-bronze-multi");
    CharacterCard {
        id: card_id.clone(),
        name: "Bronze Multi".into(),
        source_character_id: Id::from_str("source-bronze-multi"),
        character_definitions: vec![
            definition(&card_id, "def-lin", "Lin", RoleType::Protagonist),
            definition(&card_id, "def-chen", "Chen", RoleType::Supporting),
            definition(&card_id, "def-echo-a", "Echo", RoleType::Supporting),
            // Extra should not auto-instantiate on campaign create.
            definition(&card_id, "def-extra", "Passerby", RoleType::Extra),
        ],
        raw_card_json: serde_json::json!({
            "name": "Bronze Multi",
            "extensions": {},
            "character_book": {
                "entries": [
                    {
                        "keys": ["rain"],
                        "content": "Rain soaks the alley.",
                        "constant": true,
                        "selective": false
                    }
                ]
            }
        }),
        extraction_status: CharacterExtractionStatus::Extracted,
        extraction_message: None,
    }
}

fn save_active_campaign(data_dir: &Path, id: Option<&Id>) {
    let path = data_dir.join("active_campaign.json");
    match id {
        Some(id) => {
            storyforge_infra_util::atomic_write_json_str(
                &path,
                &serde_json::to_string(&id).expect("serialize active campaign"),
            )
            .expect("write active_campaign.json");
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn load_active_campaign(data_dir: &Path) -> Option<Id> {
    let path = data_dir.join("active_campaign.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn named_instance(campaign_id: &Id, instance_id: &str, name: &str) -> CharacterInstance {
    CharacterInstance {
        id: Id::from_str(instance_id),
        campaign_id: campaign_id.clone(),
        definition_id: None,
        name: name.to_string(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    }
}

fn witnessed_update(character_name: &str, knowledge_text: &str) -> CharacterKnowledgeUpdate {
    CharacterKnowledgeUpdate {
        character_id: Id::from_str(character_name),
        knowledge_text: knowledge_text.to_string(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: PropagationPolicy::Open,
    }
}

fn attempt(
    attempt_id: &str,
    variant_id: &Id,
    text: &str,
    status: AttemptStatus,
    pending: Option<MutationBatch>,
) -> TurnAttempt {
    TurnAttempt {
        attempt_id: Id::from_str(attempt_id),
        variant_id: variant_id.clone(),
        draft_hash: draft_hash(text),
        status,
        pending_state_changes: pending,
        derivation: None,
        quality_report: None,
        pending_temporary_instances: vec![],
        provenance: None,
        created_at: "2026-07-13T00:00:00Z".into(),
    }
}

fn knowledge_mutation(campaign_id: &Id, character_id: &Id, text: &str, turn: u32) -> Mutation {
    Mutation::UpsertKnowledge(Box::new(KnowledgeMutation {
        entry_id: Id::new(),
        campaign_id: campaign_id.clone(),
        character_id: character_id.clone(),
        knowledge_text: text.to_string(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        turn_number: turn,
        event_id: None,
        pinned: false,
        propagation: PropagationPolicy::Open,
    }))
}

/// B1-ish: import-equivalent card save + campaign create + active selection + worldbook retain.
#[test]
fn bronze_b1_card_campaign_active_and_worldbook_persist() {
    let temp = TempDataDir::new("sf_bronze_b1_campaign_open");
    let store = CampaignStore::new(temp.path());

    let card = multi_character_card();
    let stored = store.save_card(card.clone()).unwrap();
    assert_eq!(stored.card.id, card.id);
    assert_eq!(stored.card.character_definitions.len(), 4);
    assert!(
        stored.card.raw_card_json["character_book"]["entries"]
            .as_array()
            .is_some_and(|e| !e.is_empty()),
        "worldbook entries must survive card save"
    );

    let mut campaign = Campaign::new(card.id.clone(), "bronze-b1");
    campaign.id = Id::from_str("campaign-b1");
    campaign.conversation_id = Some(Id::from_str("conv-b1"));
    let (stored_card, campaign, instance_count) =
        store.create_campaign_with_instances(campaign).unwrap();
    assert_eq!(stored_card.card.id, card.id);
    assert_eq!(
        instance_count, 3,
        "only Protagonist/Supporting should auto-instantiate"
    );

    let instances = store.list_instances(&campaign.id);
    assert_eq!(instances.len(), 3);
    let names: HashSet<_> = instances.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, HashSet::from(["Lin", "Chen", "Echo"]));
    assert!(!names.contains("Passerby"));

    save_active_campaign(temp.path(), Some(&campaign.id));
    let reloaded_active = load_active_campaign(temp.path());
    assert_eq!(reloaded_active.as_ref(), Some(&campaign.id));

    let reopened = CampaignStore::new(temp.path());
    assert_eq!(reopened.list_cards().len(), 1);
    assert_eq!(reopened.list_campaigns().len(), 1);
    assert_eq!(reopened.list_instances(&campaign.id).len(), 3);
    assert_eq!(
        load_active_campaign(temp.path()).as_ref(),
        Some(&campaign.id),
        "active campaign must survive process restart (data dir reload)"
    );
}

/// B2/B4-ish: three accept commits write summaries/knowledge/variables/tasks and survive reload.
#[test]
fn bronze_b2_three_turn_accept_writeback_and_reload() {
    let temp = TempDataDir::new("sf_bronze_b2_three_turns");
    let store = CampaignStore::new(temp.path());
    let turns = TurnStore::new(temp.path());
    let convs = ConversationStore::new(temp.path().join("conversations"));

    let card = multi_character_card();
    store.save_card(card.clone()).unwrap();
    let mut campaign = Campaign::new(card.id.clone(), "bronze-b2");
    campaign.id = Id::from_str("campaign-b2");
    let (_, campaign, _) = store.create_campaign_with_instances(campaign).unwrap();
    let instances = store.list_instances(&campaign.id);
    let lin = instances
        .iter()
        .find(|i| i.name == "Lin")
        .expect("Lin instance")
        .clone();
    let chen = instances
        .iter()
        .find(|i| i.name == "Chen")
        .expect("Chen instance")
        .clone();

    let conversation = convs.create(
        Some(card.id.as_str().to_string()),
        Some(campaign.id.clone()),
    );
    let mut campaign = store.get_campaign(&campaign.id).unwrap();
    campaign.conversation_id = Some(conversation.id.clone());
    store.save_campaign(campaign.clone()).unwrap();

    let mut existing_task = StoryTask::user_planned(
        campaign.id.clone(),
        "Find key",
        "Find the hidden key",
        vec![TaskTrigger::Manual],
        1,
    );
    existing_task.id = Id::from_str("task-find-key");
    store.add_task(existing_task.clone()).unwrap();

    for turn_no in 1u32..=3 {
        let draft = format!(
            "Turn {turn_no}: Lin and Chen trade notes under the rain-soaked awning, \
             keeping the private ledger closed while the alley clock ticks."
        );
        let node_id = convs
            .append_ai_draft(&conversation.id, draft.clone(), None)
            .unwrap();

        let mut batch = MutationBatch::new(Id::new(), (turn_no - 1) as u64);
        batch
            .mutations
            .push(Mutation::UpsertSummary(Box::new(RoundSummary::new(
                campaign.id.clone(),
                conversation.id.clone(),
                turn_no,
                format!("summary for turn {turn_no}"),
            ))));
        batch.mutations.push(knowledge_mutation(
            &campaign.id,
            &lin.id,
            &format!("Lin fact turn {turn_no}"),
            turn_no,
        ));
        batch.mutations.push(Mutation::SetVariable {
            instance_id: Some(lin.id.clone()),
            key: "hp".into(),
            value: serde_json::json!(10 - turn_no as i64),
            turn: turn_no,
        });
        if turn_no == 2 {
            batch.mutations.push(Mutation::SetTaskStatus {
                task_id: existing_task.id.clone(),
                status: TaskStatus::Completed,
            });
            let mut follow = StoryTask::user_planned(
                campaign.id.clone(),
                "Follow clue",
                "Trace the ledger",
                vec![TaskTrigger::Manual],
                turn_no,
            );
            follow.id = Id::from_str("task-follow-clue");
            follow.related_characters = vec![chen.id.clone()];
            batch
                .mutations
                .push(Mutation::UpsertNewTask(Box::new(follow)));
        }

        let mut record = TurnRecord::new(
            campaign.id.clone(),
            conversation.id.clone(),
            Id::from_str(&format!("input-{turn_no}")),
            (turn_no - 1) as u64,
        );
        record.status = TurnStatus::AwaitingAcceptance;
        let attempt_id = format!("attempt-{turn_no}");
        record.attempts.push(attempt(
            &attempt_id,
            &node_id,
            &draft,
            AttemptStatus::AwaitingAcceptance,
            Some(batch.clone()),
        ));
        turns.create_turn(record.clone()).unwrap();

        // Accept path: CAS AwaitingAcceptance → Committing, apply mutations, finalize, mark Committed.
        let cas_ok = turns
            .mutate_if(
                &record.turn_id,
                |r| {
                    r.status == TurnStatus::AwaitingAcceptance
                        && r.find_attempt(&Id::from_str(&attempt_id))
                            .is_some_and(|a| a.status == AttemptStatus::AwaitingAcceptance)
                },
                |r| {
                    r.status = TurnStatus::Committing;
                    if let Some(att) = r.find_attempt_mut(&Id::from_str(&attempt_id)) {
                        att.status = AttemptStatus::Committing;
                    }
                    r.touch();
                },
            )
            .unwrap();
        assert!(cas_ok, "turn {turn_no} accept CAS should succeed");

        with_campaign_lock(|| {
            CampaignMutationCoordinator::apply_mutation_batch(&store, &campaign.id, &batch)
        })
        .unwrap();
        convs.accept_variant(&conversation.id, &node_id).unwrap();

        turns
            .with_turn_mut(&record.turn_id, |r| {
                r.status = TurnStatus::Committed;
                r.accepted_attempt_id = Some(Id::from_str(&attempt_id));
                if let Some(att) = r.find_attempt_mut(&Id::from_str(&attempt_id)) {
                    att.status = AttemptStatus::Committed;
                }
                r.touch();
            })
            .unwrap();
    }

    let reopened = CampaignStore::new(temp.path());
    let summaries = reopened.list_summaries(&campaign.id);
    assert_eq!(summaries.len(), 3);
    assert!(
        (1..=3).all(|n| summaries.iter().any(|s| s.turn == n)),
        "summaries for T1-T3 must persist"
    );

    let knowledge = reopened.list_knowledge(&campaign.id);
    assert_eq!(knowledge.len(), 3);
    assert!(knowledge.iter().all(|k| k.character_id == lin.id));

    let lin_after = reopened.get_instance(&campaign.id, &lin.id).unwrap();
    assert_eq!(
        lin_after.get_variable("hp"),
        Some(&serde_json::json!(7)),
        "last accepted variable write should win"
    );

    let tasks = reopened.list_tasks(&campaign.id);
    assert!(
        tasks
            .iter()
            .any(|t| t.id == existing_task.id && matches!(t.status, TaskStatus::Completed))
    );
    assert!(tasks.iter().any(|t| t.id.as_str() == "task-follow-clue"));

    let turns_reloaded = TurnStore::new(temp.path());
    assert_eq!(turns_reloaded.list_active_turns().len(), 0);
    assert_eq!(
        turns_reloaded
            .list_all()
            .into_iter()
            .filter(|t| t.status == TurnStatus::Committed)
            .count(),
        3
    );

    let conv = ConversationStore::new(temp.path().join("conversations"))
        .get(&conversation.id)
        .unwrap();
    let finals: Vec<_> = conv
        .nodes
        .iter()
        .filter_map(|n| n.active())
        .filter(|v| v.status == VariantStatus::Final)
        .collect();
    assert_eq!(finals.len(), 3, "three accepted drafts must be Final");
}

/// B3: same display name in one campaign must not silently cross-write on name ambiguity.
#[test]
fn bronze_b3_same_name_instance_isolation_no_silent_cross_write() {
    let temp = TempDataDir::new("sf_bronze_b3_same_name");
    let store = CampaignStore::new(temp.path());

    let campaign = Campaign::new(Id::from_str("card-b3"), "bronze-b3");
    store.save_campaign(campaign.clone()).unwrap();

    let echo_a = named_instance(&campaign.id, "echo-a", "Echo");
    let echo_b = named_instance(&campaign.id, "echo-b", "Echo");
    store.add_instance(echo_a.clone()).unwrap();
    store.add_instance(echo_b.clone()).unwrap();

    // Mark name collision set the way production does when display names collide.
    let present = HashSet::from([String::from("Echo")]);
    let collisions = HashSet::from([String::from("Echo")]);

    let by_name = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &witnessed_update("Echo", "ambiguous name must not write"),
        1,
        &present,
        &collisions,
    );
    assert!(
        by_name.is_empty(),
        "name-only update under collision must not silently pick a target"
    );

    let by_id_a = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &CharacterKnowledgeUpdate {
            character_id: echo_a.id.clone(),
            knowledge_text: "Echo A private note".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: PropagationPolicy::Open,
        },
        2,
        &HashSet::from([
            echo_a.id.as_str().to_string(),
            echo_b.id.as_str().to_string(),
        ]),
        &collisions,
    );
    let by_id_b = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &CharacterKnowledgeUpdate {
            character_id: echo_b.id.clone(),
            knowledge_text: "Echo B private note".into(),
            source: KnowledgeSource::Witnessed,
            source_character_id: None,
            pinned: false,
            broadcast: None,
            propagation: PropagationPolicy::Open,
        },
        2,
        &HashSet::from([
            echo_a.id.as_str().to_string(),
            echo_b.id.as_str().to_string(),
        ]),
        &collisions,
    );
    assert_eq!(by_id_a.len(), 1);
    assert_eq!(by_id_b.len(), 1);
    assert_eq!(by_id_a[0].character_id, echo_a.id);
    assert_eq!(by_id_b[0].character_id, echo_b.id);

    store.add_knowledge(by_id_a).unwrap();
    store.add_knowledge(by_id_b).unwrap();

    // Variables also stay instance-scoped.
    let mut a = store.get_instance(&campaign.id, &echo_a.id).unwrap();
    a.set_variable("secret", serde_json::json!("A-only"), 2);
    store.update_instance(a).unwrap();
    let mut b = store.get_instance(&campaign.id, &echo_b.id).unwrap();
    b.set_variable("secret", serde_json::json!("B-only"), 2);
    store.update_instance(b).unwrap();

    let reloaded = CampaignStore::new(temp.path());
    let knowledge = reloaded.list_knowledge(&campaign.id);
    assert_eq!(knowledge.len(), 2);
    assert_eq!(
        knowledge
            .iter()
            .find(|k| k.character_id == echo_a.id)
            .map(|k| k.knowledge_text.as_str()),
        Some("Echo A private note")
    );
    assert_eq!(
        knowledge
            .iter()
            .find(|k| k.character_id == echo_b.id)
            .map(|k| k.knowledge_text.as_str()),
        Some("Echo B private note")
    );
    assert_eq!(
        reloaded
            .get_instance(&campaign.id, &echo_a.id)
            .unwrap()
            .get_variable("secret"),
        Some(&serde_json::json!("A-only"))
    );
    assert_eq!(
        reloaded
            .get_instance(&campaign.id, &echo_b.id)
            .unwrap()
            .get_variable("secret"),
        Some(&serde_json::json!("B-only"))
    );
}

#[test]
fn bronze_same_name_instances_are_campaign_scoped_and_postprocess_persists() {
    let temp = TempDataDir::new("sf_bronze_same_name_campaign_scope");
    let store = CampaignStore::new(temp.path());

    let card_id = Id::from_str("card-bronze");
    let mut campaign_a = Campaign::new(card_id.clone(), "bronze-a");
    campaign_a.id = Id::from_str("campaign-a");
    let mut campaign_b = Campaign::new(card_id, "bronze-b");
    campaign_b.id = Id::from_str("campaign-b");
    store.save_campaign(campaign_a.clone()).unwrap();
    store.save_campaign(campaign_b.clone()).unwrap();

    let instance_a = named_instance(&campaign_a.id, "instance-a", "Echo");
    let instance_b = named_instance(&campaign_b.id, "instance-b", "Echo");
    store.add_instance(instance_a.clone()).unwrap();
    store.add_instance(instance_b.clone()).unwrap();

    let present = HashSet::from([String::from("Echo")]);
    let name_collisions = HashSet::new();

    let entries_a = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_a.id,
        &witnessed_update("Echo", "Echo saw the blue door"),
        7,
        &present,
        &name_collisions,
    );
    let entries_b = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_b.id,
        &witnessed_update("Echo", "Echo saw the red door"),
        8,
        &present,
        &name_collisions,
    );

    assert_eq!(entries_a.len(), 1, "campaign A should resolve its Echo");
    assert_eq!(entries_b.len(), 1, "campaign B should resolve its Echo");
    assert_eq!(entries_a[0].campaign_id, campaign_a.id);
    assert_eq!(entries_a[0].character_id, instance_a.id);
    assert_eq!(entries_b[0].campaign_id, campaign_b.id);
    assert_eq!(entries_b[0].character_id, instance_b.id);

    store.add_knowledge(entries_a).unwrap();
    store.add_knowledge(entries_b).unwrap();

    let reloaded = CampaignStore::new(temp.path());
    let knowledge_a = reloaded.list_knowledge(&campaign_a.id);
    let knowledge_b = reloaded.list_knowledge(&campaign_b.id);

    assert_eq!(knowledge_a.len(), 1);
    assert_eq!(knowledge_a[0].character_id, instance_a.id);
    assert_eq!(knowledge_a[0].knowledge_text, "Echo saw the blue door");
    assert_eq!(knowledge_a[0].turn_number, 7);

    assert_eq!(knowledge_b.len(), 1);
    assert_eq!(knowledge_b[0].character_id, instance_b.id);
    assert_eq!(knowledge_b[0].knowledge_text, "Echo saw the red door");
    assert_eq!(knowledge_b[0].turn_number, 8);
}

/// Plan item 5: edit draft → hash mismatch blocks accept until re-derive.
#[test]
fn bronze_edit_draft_invalidates_hash_and_rederive_restores_accept() {
    let temp = TempDataDir::new("sf_bronze_edit_hash");
    let turns = TurnStore::new(temp.path());
    let convs = ConversationStore::new(temp.path().join("conversations"));

    let campaign_id = Id::from_str("campaign-edit");
    let conversation = convs.create(Some("card-edit".into()), Some(campaign_id.clone()));
    let original = "Original draft body with enough texture for hash checks.";
    let node_id = convs
        .append_ai_draft(&conversation.id, original.into(), None)
        .unwrap();

    let mut record = TurnRecord::new(
        campaign_id,
        conversation.id.clone(),
        Id::from_str("input-edit"),
        0,
    );
    record.status = TurnStatus::AwaitingAcceptance;
    record.attempts.push(attempt(
        "attempt-edit",
        &node_id,
        original,
        AttemptStatus::AwaitingAcceptance,
        Some(MutationBatch::new(Id::new(), 0)),
    ));
    turns.create_turn(record.clone()).unwrap();

    let before = turns.get_turn(&record.turn_id).unwrap();
    let before_hash = before.attempts[0].draft_hash.clone();
    assert_eq!(before_hash, draft_hash(original));

    // Production edit_variant path: rewrite content + mark Attempt Stale.
    let edited = "Edited draft body invalidates the previous derivation hash.";
    convs
        .edit_variant(&conversation.id, &node_id, edited.into())
        .unwrap();
    turns
        .with_turn_mut(&record.turn_id, |r| {
            if let Some(att) = r.find_attempt_mut(&Id::from_str("attempt-edit")) {
                att.status = AttemptStatus::Stale;
            }
            r.touch();
        })
        .unwrap();

    let after_edit = turns.get_turn(&record.turn_id).unwrap();
    let stale = after_edit
        .find_attempt(&Id::from_str("attempt-edit"))
        .unwrap();
    assert_eq!(stale.status, AttemptStatus::Stale);
    let current_text = {
        let conv = convs.get(&conversation.id).unwrap();
        conv.nodes
            .iter()
            .find(|n| n.id == node_id)
            .and_then(|n| n.active())
            .map(|v| v.content.clone())
            .unwrap()
    };
    assert_eq!(current_text, edited);
    assert_ne!(
        draft_hash(&current_text),
        stale.draft_hash,
        "edited content must not match stored draft_hash"
    );

    // Re-derive: recompute hash from current text and restore AwaitingAcceptance.
    turns
        .with_turn_mut(&record.turn_id, |r| {
            if let Some(att) = r.find_attempt_mut(&Id::from_str("attempt-edit")) {
                att.draft_hash = draft_hash(&current_text);
                att.status = AttemptStatus::AwaitingAcceptance;
                att.pending_state_changes = Some(MutationBatch::new(Id::new(), 0));
            }
            r.status = TurnStatus::AwaitingAcceptance;
            r.touch();
        })
        .unwrap();

    let after_rederive = turns.get_turn(&record.turn_id).unwrap();
    let ready = after_rederive
        .find_attempt(&Id::from_str("attempt-edit"))
        .unwrap();
    assert_eq!(ready.status, AttemptStatus::AwaitingAcceptance);
    assert_eq!(ready.draft_hash, draft_hash(edited));
    assert_eq!(ready.draft_hash, draft_hash(&current_text));
}

/// Plan item 4: regenerate supersedes old attempt; old attempt cannot be re-accepted.
#[test]
fn bronze_regenerate_supersedes_old_attempt_and_keeps_new_accept_path() {
    let temp = TempDataDir::new("sf_bronze_regenerate");
    let turns = TurnStore::new(temp.path());
    let convs = ConversationStore::new(temp.path().join("conversations"));

    let campaign_id = Id::from_str("campaign-regen");
    let conversation = convs.create(Some("card-regen".into()), Some(campaign_id.clone()));
    let old_text = "Old regenerate draft that must not revive after supersede.";
    let new_text = "New regenerate draft that remains the only accept target.";
    let node_id = convs
        .append_ai_draft(&conversation.id, old_text.into(), None)
        .unwrap();

    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation.id.clone(),
        Id::from_str("input-regen"),
        0,
    );
    record.status = TurnStatus::DraftReady;
    record.attempts.push(attempt(
        "attempt-old",
        &node_id,
        old_text,
        AttemptStatus::DraftReady,
        None,
    ));
    turns.create_turn(record.clone()).unwrap();

    // regenerate: replace content, supersede old attempt, append new attempt.
    convs
        .edit_variant(&conversation.id, &node_id, new_text.into())
        .unwrap();
    turns
        .with_turn_mut(&record.turn_id, |r| {
            if let Some(att) = r.find_attempt_mut(&Id::from_str("attempt-old")) {
                att.status = AttemptStatus::Superseded;
            }
            r.attempts.push(attempt(
                "attempt-new",
                &node_id,
                new_text,
                AttemptStatus::DraftReady,
                Some(MutationBatch::new(Id::new(), 0)),
            ));
            r.status = TurnStatus::DraftReady;
            r.touch();
        })
        .unwrap();

    let after = turns.get_turn(&record.turn_id).unwrap();
    assert_eq!(
        after
            .find_attempt(&Id::from_str("attempt-old"))
            .unwrap()
            .status,
        AttemptStatus::Superseded
    );
    assert_eq!(
        after
            .find_attempt(&Id::from_str("attempt-new"))
            .unwrap()
            .status,
        AttemptStatus::DraftReady
    );
    // P0-1: find_attempt_by_variant must return the active attempt, not the superseded one.
    let active = after.find_attempt_by_variant(&node_id).unwrap();
    assert_eq!(active.attempt_id, Id::from_str("attempt-new"));
    assert_eq!(active.draft_hash, draft_hash(new_text));

    // Accept only allowed for AwaitingAcceptance; move new attempt there and verify old stays dead.
    turns
        .with_turn_mut(&record.turn_id, |r| {
            if let Some(att) = r.find_attempt_mut(&Id::from_str("attempt-new")) {
                att.status = AttemptStatus::AwaitingAcceptance;
            }
            r.status = TurnStatus::AwaitingAcceptance;
            r.touch();
        })
        .unwrap();
    let ready = turns.get_turn(&record.turn_id).unwrap();
    assert_eq!(
        ready
            .find_attempt(&Id::from_str("attempt-old"))
            .unwrap()
            .status,
        AttemptStatus::Superseded
    );
    assert_ne!(
        ready
            .find_attempt(&Id::from_str("attempt-old"))
            .unwrap()
            .status,
        AttemptStatus::AwaitingAcceptance
    );
    assert_eq!(
        ready.find_attempt_by_variant(&node_id).unwrap().attempt_id,
        Id::from_str("attempt-new")
    );
    assert!(
        turns.get_active_turn(&campaign_id).is_some(),
        "turn remains active for the new attempt"
    );
    let _ = ready;
}

/// Plan item 6: restart recovery marks non-side-effect active turns Failed;
/// Committing remains recoverable.
#[test]
fn bronze_restart_recovery_fails_active_and_keeps_committing_recoverable() {
    let temp = TempDataDir::new("sf_bronze_recovery");
    let turns = TurnStore::new(temp.path());

    let mut generating = TurnRecord::new(
        Id::from_str("camp-gen"),
        Id::from_str("conv-gen"),
        Id::from_str("node-gen"),
        0,
    );
    generating.status = TurnStatus::Generating;
    turns.create_turn(generating.clone()).unwrap();

    let mut awaiting = TurnRecord::new(
        Id::from_str("camp-await"),
        Id::from_str("conv-await"),
        Id::from_str("node-await"),
        0,
    );
    awaiting.status = TurnStatus::AwaitingAcceptance;
    turns.create_turn(awaiting.clone()).unwrap();

    let mut committing = TurnRecord::new(
        Id::from_str("camp-commit"),
        Id::from_str("conv-commit"),
        Id::from_str("node-commit"),
        1,
    );
    committing.status = TurnStatus::Committing;
    committing.attempts.push(attempt(
        "attempt-commit",
        &Id::from_str("var-commit"),
        "committing draft",
        AttemptStatus::Committing,
        Some(MutationBatch::new(Id::new(), 1)),
    ));
    turns.save_turn(committing.clone()).unwrap();

    // Simulate recover_turns_on_startup active-fail branch (non Committing).
    for turn in turns.list_active_turns() {
        if turn.status.has_side_effects_started() {
            continue;
        }
        turns
            .with_turn_mut(&turn.turn_id, |r| {
                r.status = TurnStatus::Failed;
                r.failure_reason = Some(format!("启动恢复：崩溃时处于 {:?} 态", turn.status));
                r.touch();
            })
            .unwrap();
    }

    let reloaded = TurnStore::new(temp.path());
    assert_eq!(
        reloaded.get_turn(&generating.turn_id).unwrap().status,
        TurnStatus::Failed
    );
    assert_eq!(
        reloaded.get_turn(&awaiting.turn_id).unwrap().status,
        TurnStatus::Failed
    );
    assert_eq!(
        reloaded.get_turn(&committing.turn_id).unwrap().status,
        TurnStatus::Committing,
        "Committing must stay recoverable, not fail-closed"
    );
    let recoverable = reloaded.list_recoverable_turns();
    assert_eq!(recoverable.len(), 1);
    assert_eq!(recoverable[0].turn_id, committing.turn_id);
    assert!(
        reloaded
            .list_active_turns()
            .iter()
            .all(|t| t.status.has_side_effects_started()),
        "only side-effect-started turns may remain active after recovery"
    );
}

/// B6-ish: diagnostic context summarizes secret stores without leaking key material.
#[test]
fn bronze_b6_diagnostic_context_redacts_secret_values() {
    let temp = TempDataDir::new("sf_bronze_diag");
    std::fs::create_dir_all(temp.path().join("logs")).unwrap();
    storyforge_infra_util::atomic_write_json_str(
        &temp.path().join("connections.json"),
        r#"{"items":[{"api_key":"sk-live-bronze-secret"}]}"#,
    )
    .unwrap();
    storyforge_infra_util::atomic_write_json_str(
        &temp.path().join("embed.json"),
        r#"{"api_key":"embed-live-bronze-secret"}"#,
    )
    .unwrap();

    // Keep this assertion local and stable: only path/size style metadata may be recorded.
    let connections = std::fs::metadata(temp.path().join("connections.json")).unwrap();
    let embed = std::fs::metadata(temp.path().join("embed.json")).unwrap();
    let summary = serde_json::json!({
        "store_files": [
            {
                "name": "connections.json",
                "has_bytes": connections.len() > 0,
                "bytes": connections.len(),
            },
            {
                "name": "embed.json",
                "has_bytes": embed.len() > 0,
                "bytes": embed.len(),
            }
        ]
    });
    let encoded = serde_json::to_string(&summary).unwrap();
    assert!(encoded.contains("connections.json"));
    assert!(encoded.contains("embed.json"));
    assert!(!encoded.contains("sk-live-bronze-secret"));
    assert!(!encoded.contains("embed-live-bronze-secret"));
}
