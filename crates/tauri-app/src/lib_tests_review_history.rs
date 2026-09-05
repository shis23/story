use super::*;
use storyforge_domain::campaign::Campaign;
use storyforge_domain::character::CharacterCard;
use storyforge_domain::turn::{AttemptStatus, TurnRecord, TurnStatus};

struct Fixture {
    _dir: tempfile::TempDir,
    state: Arc<AppState>,
    campaign: Campaign,
    first: Id,
    draft: Id,
    turn: TurnRecord,
}

fn fixture(finalized: bool) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let storage = Arc::new(storage_backend::StorageFacade::new(
        dir.path().to_path_buf(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Json,
            storyforge_infra_sqlite::backend::BackendSource::Env,
        ),
    ));
    let state = Arc::new(AppState::new_with_backend(dir.path().to_path_buf(), storage).unwrap());
    let mut character = make_test_character("Review");
    character.first_mes = "Opening".into();
    let stored = state
        .storage()
        .save_character(CharacterInfo::from(&character))
        .unwrap();
    let card_id = Id::new();
    let definition = make_test_character_definition(&card_id, "review-def", "Alice");
    state
        .storage()
        .save_card(CharacterCard {
            id: card_id.clone(),
            name: "Review".into(),
            source_character_id: character.id,
            character_definitions: vec![definition],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({}),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        })
        .unwrap();
    let (_, mut campaign, _) = state
        .storage()
        .create_campaign_with_instances(Campaign::new(card_id, "Review campaign"))
        .unwrap();
    let conversation = state
        .conv_store
        .create_persisted(Some(stored.id.to_string()), Some(campaign.id.clone()))
        .unwrap();
    let first = state
        .conv_store
        .append_user_message(&conversation.id, "First intent".into())
        .unwrap();
    let draft = state
        .conv_store
        .append_ai_draft(&conversation.id, "Draft body".into(), None)
        .unwrap();
    campaign.conversation_id = Some(conversation.id.clone());
    campaign.set_variable("weather", serde_json::json!("rainy"), 1);
    campaign.revision = u64::from(finalized);
    state.storage().save_campaign(&campaign).unwrap();
    let mut turn = TurnRecord::new(
        campaign.id.clone(),
        conversation.id.clone(),
        first.clone(),
        0,
    );
    let mut attempt =
        turn_lifecycle::new_draft_attempt(Id::new(), draft.clone(), "Draft body", vec![]);
    attempt.status = if finalized {
        AttemptStatus::Committed
    } else {
        AttemptStatus::AwaitingAcceptance
    };
    turn.status = if finalized {
        state
            .conv_store
            .accept_variant(&conversation.id, &draft)
            .unwrap();
        turn.accepted_attempt_id = Some(attempt.attempt_id.clone());
        TurnStatus::Committed
    } else {
        TurnStatus::AwaitingAcceptance
    };
    turn.attempts.push(attempt);
    state.storage().save_turn(&turn).unwrap();
    Fixture {
        _dir: dir,
        state,
        campaign,
        first,
        draft,
        turn,
    }
}

#[test]
fn review_delete_draft_ends_its_turn() {
    let f = fixture(false);
    crate::commands::turns::delete_message_from(
        f.campaign.conversation_id.as_ref().unwrap().to_string(),
        f.draft.to_string(),
        tauri_state_for_test(&f.state),
    )
    .unwrap();
    assert!(
        f.state
            .storage()
            .get_active_turn(&f.campaign.id)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.state
            .storage()
            .get_turn(&f.turn.turn_id)
            .unwrap()
            .unwrap()
            .status,
        TurnStatus::Abandoned,
    );
}

#[test]
fn review_cardstudio_uses_each_app_state_data_directory() {
    let first = fixture(false);
    let second = fixture(false);
    let project = crate::card_studio_api::cardstudio_create_project(
        "Isolated".into(),
        "Local".into(),
        tauri_state_for_test(&first.state),
    )
    .unwrap();
    assert_eq!(
        crate::card_studio_api::cardstudio_list_projects(tauri_state_for_test(&first.state),).len(),
        1
    );
    assert!(
        crate::card_studio_api::cardstudio_get_project(
            project.id,
            tauri_state_for_test(&second.state),
        )
        .is_err()
    );
    let reopened = crate::card_studio_store::CardStudioStore::new(first._dir.path());
    assert_eq!(reopened.list().len(), 1);
}

#[test]
fn review_delete_committed_history_is_rejected_without_mutation() {
    let f = fixture(true);
    let conv_id = f.campaign.conversation_id.as_ref().unwrap();
    let before = serde_json::to_value(f.state.conv_store.get(conv_id)).unwrap();
    assert!(
        crate::commands::turns::delete_message_from(
            conv_id.to_string(),
            f.draft.to_string(),
            tauri_state_for_test(&f.state),
        )
        .is_err()
    );
    assert_eq!(
        serde_json::to_value(f.state.conv_store.get(conv_id)).unwrap(),
        before
    );
}

#[test]
fn review_fork_rejects_historical_node() {
    let f = fixture(true);
    assert!(
        crate::commands::campaigns::fork_campaign_for_backend(
            f.state.storage(),
            &f.state.conv_store,
            f.campaign.id,
            f.first,
            "Historical".into(),
        )
        .is_err()
    );
}

#[test]
fn review_fork_preserves_memory_and_remaps_instances() {
    let f = fixture(true);
    let source_instance = f
        .state
        .storage()
        .list_instances(&f.campaign.id)
        .unwrap()
        .remove(0);
    f.state
        .storage()
        .add_knowledge(&[
            storyforge_domain::character_knowledge::CharacterKnowledgeEntry::witnessed(
                f.campaign.id.clone(),
                source_instance.id.clone(),
                "Observed the rain",
                1,
            ),
        ])
        .unwrap();
    let mut task = storyforge_domain::story_task::StoryTask::user_planned(
        f.campaign.id.clone(),
        "Wait",
        "Wait for the visitor",
        vec![],
        1,
    );
    task.related_characters.push(source_instance.id.clone());
    f.state.storage().add_task(&task).unwrap();
    let mut summary = storyforge_domain::agent::RoundSummary::new(
        f.campaign.id.clone(),
        f.campaign.conversation_id.clone().unwrap(),
        1,
        "Rain began".into(),
    );
    summary.lineage_id = f.campaign.lineage_id.clone();
    f.state
        .storage()
        .json_campaign_store(
            storage_backend::BackendCapability::ImportExport,
            "review fixture",
        )
        .unwrap()
        .add_summary(summary)
        .unwrap();
    let store = f
        .state
        .storage()
        .json_campaign_store(
            storage_backend::BackendCapability::ImportExport,
            "review snapshot",
        )
        .unwrap();
    assert_eq!(
        bundle_store_snapshot_in_memory(store, &f.state.conv_store).unwrap(),
        read_bundle_disk_snapshot_strict(f._dir.path(), f.state.conv_store.data_dir()).unwrap(),
    );
    let fork = crate::commands::campaigns::fork_campaign_for_backend(
        f.state.storage(),
        &f.state.conv_store,
        f.campaign.id.clone(),
        f.draft,
        "Branch".into(),
    )
    .unwrap();
    let fork_id = Id::from_str(&fork.id);
    let knowledge = f.state.storage().list_knowledge(&fork_id).unwrap();
    assert_eq!(knowledge.len(), 1);
    let instances = f.state.storage().list_instances(&fork_id).unwrap();
    assert_ne!(instances[0].id, source_instance.id);
    assert_eq!(knowledge[0].character_id, instances[0].id);
    assert_eq!(f.state.storage().list_tasks(&fork_id).unwrap().len(), 1);
    assert_eq!(f.state.storage().list_summaries(&fork_id).unwrap().len(), 1);
}

#[test]
fn review_bundle_roundtrip_preserves_text_and_source_character() {
    let f = fixture(true);
    let bundle = f
        .state
        .storage()
        .export_campaign_bundle(&f.campaign.id)
        .unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let target = storage_backend::StorageFacade::new(
        target_dir.path().to_path_buf(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Json,
            storyforge_infra_sqlite::backend::BackendSource::Env,
        ),
    );
    let conversations = ConversationStore::new(target_dir.path().join("conversations"));
    let imported = target
        .import_campaign_bundle(serde_json::from_str(&bundle).unwrap(), &conversations)
        .unwrap();
    let conversation = conversations
        .get(&Id::from_str(&imported.conversation_id))
        .unwrap();
    assert_eq!(conversation.nodes.len(), 2);
    assert_eq!(
        conversation.nodes[1].active().unwrap().content,
        "Draft body"
    );
    let card = target
        .get_card(&Id::from_str(&imported.card_id))
        .unwrap()
        .unwrap();
    assert!(
        target
            .get_character(card.card.source_character_id.as_str())
            .unwrap()
            .is_some()
    );
    let turns = target
        .json_turn_store("verify imported history")
        .unwrap()
        .list_for_campaign(&Id::from_str(&imported.campaign_id));
    assert_eq!(turns.len(), 1);
    assert_ne!(turns[0].turn_id, f.turn.turn_id);
    assert_eq!(turns[0].input_node_id, conversation.nodes[0].id);
    assert_eq!(turns[0].attempts[0].variant_id, conversation.nodes[1].id);
    assert_eq!(
        turns[0].accepted_attempt_id.as_ref(),
        Some(&turns[0].attempts[0].attempt_id)
    );
}

#[test]
fn review_bundle_rejects_active_turn_and_invalid_scope_without_writes() {
    let f = fixture(false);
    assert!(
        f.state
            .storage()
            .export_campaign_bundle(&f.campaign.id)
            .is_err()
    );
    let f = fixture(true);
    let json = f
        .state
        .storage()
        .export_campaign_bundle(&f.campaign.id)
        .unwrap();
    let mut bundle: serde_json::Value = serde_json::from_str(&json).unwrap();
    bundle["runtime"]["turns"][0]["campaign_id"] = serde_json::json!("wrong-campaign");
    let before = f.state.storage().list_campaigns(None).unwrap().len();
    assert!(
        f.state
            .storage()
            .import_campaign_bundle(serde_json::from_value(bundle).unwrap(), &f.state.conv_store,)
            .is_err()
    );
    assert_eq!(
        f.state.storage().list_campaigns(None).unwrap().len(),
        before
    );
}

#[test]
fn review_bundle_json_failure_compensates_source_history_and_turns() {
    let f = fixture(true);
    let bundle = f
        .state
        .storage()
        .export_campaign_bundle(&f.campaign.id)
        .unwrap();
    let target = fixture(true);
    let storage = target.state.storage();
    let before_characters = storage.list_characters().unwrap().len();
    let before_campaigns = storage.list_campaigns(None).unwrap().len();
    let before_conversations = target.state.conv_store.list().len();
    let fence = target._dir.path().join("turns.json");
    storyforge_infra_util::write_fence::freeze(&fence);
    let result = storage.import_campaign_bundle(
        serde_json::from_str(&bundle).unwrap(),
        &target.state.conv_store,
    );
    storyforge_infra_util::write_fence::unfreeze(&fence);
    assert!(result.is_err());
    assert_eq!(storage.list_characters().unwrap().len(), before_characters);
    assert_eq!(
        storage.list_campaigns(None).unwrap().len(),
        before_campaigns
    );
    assert_eq!(target.state.conv_store.list().len(), before_conversations);
    assert_eq!(
        storage
            .json_turn_store("verify rollback")
            .unwrap()
            .list_all()
            .len(),
        1
    );
}
