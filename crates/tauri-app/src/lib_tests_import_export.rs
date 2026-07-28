use super::*;
use std::collections::HashMap;
use std::sync::Mutex;

fn valid_summary_graph_bundle() -> CampaignBundle {
    use storyforge_domain::agent::RoundSummary;
    use storyforge_domain::campaign::Campaign;

    let card_id = Id::from_str("graph-card");
    let campaign_id = Id::from_str("graph-campaign");
    let conversation_id = Id::from_str("graph-conversation");
    let a1_id = Id::from_str("graph-a1");
    let a2_id = Id::from_str("graph-a2");
    let b_id = Id::from_str("graph-b1");
    let mut campaign = Campaign::new(card_id, "Graph Campaign");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    let lineage_id = campaign.lineage_id.clone().unwrap();

    let mut a1 = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        1,
        "leaf one".into(),
    );
    a1.id = a1_id.clone();
    a1.code = Some("A0001".into());
    a1.lineage_id = Some(lineage_id.clone());
    a1.covered_by = Some(b_id.clone());

    let mut a2 = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        2,
        "leaf two".into(),
    );
    a2.id = a2_id.clone();
    a2.code = Some("A0002".into());
    a2.lineage_id = Some(lineage_id.clone());
    a2.covered_by = Some(b_id.clone());

    let mut b = RoundSummary::new(campaign_id, conversation_id, 1, "band".into());
    b.id = b_id;
    b.code = Some("B0001".into());
    b.lineage_id = Some(lineage_id);
    b.level = 1;
    b.turn_end = 2;
    b.covers = vec![a1_id, a2_id];

    CampaignBundle {
        format_version: BUNDLE_FORMAT_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        card: None,
        campaign,
        instances: vec![],
        definitions: vec![],
        knowledge: vec![],
        tasks: vec![],
        summaries: vec![a1, a2, b],
    }
}

#[derive(Default)]
pub(super) struct MemorySecretStore {
    secrets: Mutex<HashMap<String, String>>,
    deleted: Mutex<Vec<String>>,
}

impl SecretStore for MemorySecretStore {
    fn put_secret(&self, secret_ref: &str, secret: &str) -> Result<(), String> {
        self.secrets
            .lock()
            .unwrap()
            .insert(secret_ref.to_string(), secret.to_string());
        Ok(())
    }

    fn get_secret(&self, secret_ref: &str) -> Result<String, String> {
        self.secrets
            .lock()
            .unwrap()
            .get(secret_ref)
            .cloned()
            .ok_or_else(|| format!("missing secret {secret_ref}"))
    }

    fn delete_secret(&self, secret_ref: &str) -> Result<(), String> {
        self.secrets.lock().unwrap().remove(secret_ref);
        self.deleted.lock().unwrap().push(secret_ref.to_string());
        Ok(())
    }
}

pub(super) fn make_test_llm_connection(id: &str, api_key: &str) -> LlmConnection {
    LlmConnection {
        id: Id::from_str(id),
        name: id.into(),
        base_url: "https://api.example.com/v1/chat/completions".into(),
        api_key: api_key.into(),
        model: "test-model".into(),
        protocol: LlmProtocol::OpenAi,
        params: SamplingParams::default(),
        tool_mode: ToolMode::Native,
    }
}

#[test]
#[ignore = "requires a local real ST card fixture; run scripts/run-real-card-smoke.ps1"]
fn test_real_complex_card_fixture_can_create_campaign_and_roundtrip_bundle() {
    use storyforge_domain::character::{
        CharacterCard, CharacterDefinition, CharacterExtractionStatus,
    };

    let fixture_path = std::env::var_os("SF_COMPLEX_CARD_FIXTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("test-card.png")
        });
    let bytes = std::fs::read(&fixture_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_path.display()));
    let character =
        storyforge_infra_import::import_character(&bytes).expect("complex card should import");

    assert_eq!(character.alternate_greetings.len(), 6);
    assert_complex_card_raw_extensions(&character.raw_card_json);
    assert_complex_card_raw_world_book(&character.raw_card_json);

    let dir = TempDirGuard::new("storyforge_test_real_complex_bundle");
    let store = campaign_store::CampaignStore::new(dir.path());
    let conv_store = ConversationStore::new(dir.path().join("conversations"));

    let mut card = CharacterCard::from_character(&character);
    card.extraction_status = CharacterExtractionStatus::Extracted;
    let mut definition = CharacterDefinition::fallback_from_character(&character, &[]);
    definition.card_id = card.id.clone();
    card.character_definitions = vec![definition];

    let stored = save_character_card_to_store(&store, card).unwrap();
    let campaign = create_campaign_in_store(
        &store,
        &conv_store,
        stored.card.id.as_str().to_string(),
        "Complex Fixture Campaign".into(),
        None,
    )
    .unwrap();

    let campaign_id = Id::from_str(&campaign.id);
    let instances = store.list_instances(&campaign_id);
    let expected_instance_count = instances.len();
    assert!(
        expected_instance_count > 0,
        "campaign should create instances"
    );

    let bundle_json = export_campaign_bundle_from_store(&store, campaign_id).unwrap();
    let bundle: CampaignBundle = serde_json::from_str(&bundle_json).unwrap();
    let exported_card = bundle.card.as_ref().expect("bundle should include card");
    let (_, exported_alternate_greetings) = raw_card_greetings(&exported_card.raw_card_json);

    assert_complex_card_raw_extensions(&exported_card.raw_card_json);
    assert_complex_card_raw_world_book(&exported_card.raw_card_json);
    assert_eq!(exported_alternate_greetings.len(), 6);
    assert_eq!(bundle.instances.len(), expected_instance_count);

    let import_dir = TempDirGuard::new("storyforge_test_real_complex_bundle_import");
    let import_store = campaign_store::CampaignStore::new(import_dir.path());
    let import_conv_store = ConversationStore::new(import_dir.path().join("conversations"));
    let result =
        import_campaign_bundle_into_store(&import_store, &import_conv_store, bundle).unwrap();

    assert_eq!(result.instance_count, expected_instance_count);
    let imported_card = import_store
        .get_card(&Id::from_str(&result.card_id))
        .expect("imported bundle card should exist");
    let (_, imported_alternate_greetings) = raw_card_greetings(&imported_card.card.raw_card_json);
    let imported_instances = import_store.list_instances(&Id::from_str(&result.campaign_id));

    assert_complex_card_raw_extensions(&imported_card.card.raw_card_json);
    assert_complex_card_raw_world_book(&imported_card.card.raw_card_json);
    assert_eq!(imported_alternate_greetings.len(), 6);
    assert_eq!(imported_instances.len(), expected_instance_count);
}

#[test]
fn export_campaign_bundle_includes_complete_campaign_state() {
    use storyforge_domain::agent::RoundSummary;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_export_bundle_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let card_id = Id::from_str("export-card");
    let source_id = Id::from_str("export-source");
    let campaign_id = Id::from_str("export-campaign");
    let conversation_id = Id::from_str("export-conversation");
    let def_a = Id::from_str("export-def-a");
    let def_b = Id::from_str("export-def-b");
    let instance_a = Id::from_str("export-instance-a");
    let instance_b = Id::from_str("export-instance-b");
    let knowledge_a = Id::from_str("export-knowledge-a");

    let definitions = vec![
        CharacterDefinition {
            id: def_a.clone(),
            card_id: card_id.clone(),
            name: "Alpha".into(),
            persona_prompt: "alpha persona".into(),
            behavior_rules: "protect the key".into(),
            base_backstory: vec!["Alpha found the sealed door.".into()],
            group: Some("party".into()),
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        },
        CharacterDefinition {
            id: def_b.clone(),
            card_id: card_id.clone(),
            name: "Beta".into(),
            persona_prompt: "beta persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: Some("party".into()),
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        },
    ];
    let card = CharacterCard {
        id: card_id.clone(),
        name: "Export Bundle Card".into(),
        source_character_id: source_id,
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({
            "first_mes": "hello from export",
            "alternate_greetings": ["alt export"]
        }),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: Some("ok".into()),
    };
    store.save_card(card).unwrap();

    let mut campaign = Campaign::new(card_id.clone(), "Export Campaign");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    campaign.set_variable("story_clock", serde_json::json!("Day 9 - dusk"), 7);
    store.save_campaign(campaign).unwrap();

    let instances = vec![
        CharacterInstance {
            id: instance_a.clone(),
            campaign_id: campaign_id.clone(),
            definition_id: Some(def_a.clone()),
            name: "Alpha".into(),
            persona_override: Some("alpha override".into()),
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        },
        CharacterInstance {
            id: instance_b.clone(),
            campaign_id: campaign_id.clone(),
            definition_id: Some(def_b.clone()),
            name: "Beta".into(),
            persona_override: None,
            behavior_override: Some("beta override".into()),
            variables: vec![],
            is_temporary: false,
        },
    ];
    for instance in instances {
        store.add_instance(instance).unwrap();
    }

    let mut knowledge = CharacterKnowledgeEntry::witnessed(
        campaign_id.clone(),
        instance_a.clone(),
        "Alpha knows the door code",
        4,
    );
    knowledge.id = knowledge_a.clone();
    knowledge.pinned = true;
    store.add_knowledge(vec![knowledge]).unwrap();

    let mut task = StoryTask::user_planned(
        campaign_id.clone(),
        "Open the sealed door",
        "Use the code later",
        vec![TaskTrigger::TurnReminder { at_turn: 6 }],
        5,
    );
    task.related_characters = vec![instance_a.clone(), instance_b.clone()];
    store.add_task(task).unwrap();

    store
        .add_summary(RoundSummary {
            id: Id::from_str("export-summary"),
            campaign_id: campaign_id.clone(),
            conversation_id: conversation_id.clone(),
            turn: 5,
            content: "Round five reached the sealed door.".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            code: Some("A0005".into()),
            headline: Some("\u{5bc6}\u{5c01}\u{95e8}".into()),
            lineage_id: None,
            covered_by: None,
            level: 0,
            turn_end: 5,
            covers: vec![],
        })
        .unwrap();

    let bundle_json = export_campaign_bundle_from_store(&store, campaign_id.clone()).unwrap();
    let bundle: CampaignBundle = serde_json::from_str(&bundle_json).unwrap();

    assert_eq!(bundle.format_version, BUNDLE_FORMAT_VERSION);
    assert_eq!(bundle.campaign.id, campaign_id);
    assert_eq!(bundle.campaign.card_id, card_id);
    assert_eq!(
        bundle.campaign.conversation_id,
        Some(conversation_id.clone())
    );
    assert_eq!(
        bundle.campaign.get_variable("story_clock").unwrap(),
        &serde_json::json!("Day 9 - dusk")
    );

    let exported_card = bundle.card.as_ref().unwrap();
    assert_eq!(exported_card.id, card_id);
    assert_eq!(
        exported_card.source_character_id,
        Id::from_str("export-source")
    );
    assert_eq!(
        exported_card.extraction_status,
        storyforge_domain::character::CharacterExtractionStatus::Extracted
    );
    assert_eq!(exported_card.extraction_message.as_deref(), Some("ok"));
    let (first_mes, alternate_greetings) = raw_card_greetings(&exported_card.raw_card_json);
    assert_eq!(first_mes, "hello from export");
    assert_eq!(alternate_greetings, vec!["alt export"]);
    assert_eq!(exported_card.character_definitions.len(), 2);
    assert_eq!(bundle.definitions.len(), 2);
    assert_eq!(bundle.definitions[0].id, def_a);
    assert_eq!(
        bundle.definitions[0].base_backstory[0],
        "Alpha found the sealed door."
    );

    assert_eq!(bundle.instances.len(), 2);
    assert_eq!(bundle.instances[0].definition_id, Some(def_a));
    assert_eq!(
        bundle.instances[0].persona_override.as_deref(),
        Some("alpha override")
    );
    assert_eq!(bundle.instances[1].definition_id, Some(def_b));
    assert_eq!(
        bundle.instances[1].behavior_override.as_deref(),
        Some("beta override")
    );
    assert_eq!(bundle.knowledge.len(), 1);
    assert_eq!(bundle.knowledge[0].id, knowledge_a);
    assert_eq!(bundle.knowledge[0].character_id, instance_a);
    assert_eq!(
        bundle.knowledge[0].knowledge_text,
        "Alpha knows the door code"
    );
    assert_eq!(bundle.knowledge[0].turn_number, 4);
    assert_eq!(bundle.knowledge[0].source, KnowledgeSource::Witnessed);
    assert!(bundle.knowledge[0].pinned);
    assert_eq!(bundle.tasks.len(), 1);
    assert_eq!(bundle.tasks[0].title, "Open the sealed door");
    assert_eq!(bundle.tasks[0].description, "Use the code later");
    assert_eq!(
        bundle.tasks[0].triggers,
        vec![TaskTrigger::TurnReminder { at_turn: 6 }]
    );
    assert_eq!(
        bundle.tasks[0].related_characters,
        vec![Id::from_str("export-instance-a"), instance_b]
    );
    assert_eq!(bundle.summaries.len(), 1);
    assert_eq!(bundle.summaries[0].conversation_id, conversation_id);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_campaign_bundle_reports_missing_campaign() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_export_bundle_missing_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let err = export_campaign_bundle_from_store(&store, Id::from_str("missing-campaign"))
        .expect_err("missing campaign should return not_found");

    match err {
        TauriCommandError::NotFound { message } => {
            assert!(message.contains("missing-campaign"));
        }
        other => panic!("expected not_found error, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_rewrites_ids_and_references() {
    use storyforge_domain::agent::RoundSummary;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_bundle_{}",
        uuid::Uuid::new_v4()
    ));
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let old_card_id = Id::from_str("old-card");
    let old_source_id = Id::from_str("old-source");
    let old_campaign_id = Id::from_str("old-campaign");
    let old_conversation_id = Id::from_str("old-conversation");
    let old_def_a = Id::from_str("old-def-a");
    let old_def_b = Id::from_str("old-def-b");
    let old_instance_a = Id::from_str("old-instance-a");
    let old_instance_b = Id::from_str("old-instance-b");
    let old_knowledge_a = Id::from_str("old-knowledge-a");
    let old_knowledge_b = Id::from_str("old-knowledge-b");

    let definitions = vec![
        CharacterDefinition {
            id: old_def_a.clone(),
            card_id: old_card_id.clone(),
            name: "Alpha".into(),
            persona_prompt: "alpha persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: Some("party".into()),
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        },
        CharacterDefinition {
            id: old_def_b.clone(),
            card_id: old_card_id.clone(),
            name: "Beta".into(),
            persona_prompt: "beta persona".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: Some("party".into()),
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        },
    ];
    let card = CharacterCard {
        id: old_card_id.clone(),
        name: "Bundle Card".into(),
        source_character_id: old_source_id,
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({
            "first_mes": "hello from raw",
            "alternate_greetings": ["alt one", "alt two"]
        }),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let mut campaign = Campaign::new(old_card_id.clone(), "Bundle Campaign");
    campaign.id = old_campaign_id.clone();
    campaign.conversation_id = Some(old_conversation_id.clone());
    let campaign_lineage = campaign.lineage_id.clone().unwrap();

    let instances = vec![
        CharacterInstance {
            id: old_instance_a.clone(),
            campaign_id: old_campaign_id.clone(),
            definition_id: Some(old_def_a.clone()),
            name: "Alpha".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        },
        CharacterInstance {
            id: old_instance_b.clone(),
            campaign_id: old_campaign_id.clone(),
            definition_id: Some(old_def_b.clone()),
            name: "Beta".into(),
            persona_override: None,
            behavior_override: None,
            variables: vec![],
            is_temporary: false,
        },
    ];
    let knowledge = vec![
        CharacterKnowledgeEntry {
            id: old_knowledge_a.clone(),
            campaign_id: old_campaign_id.clone(),
            character_id: old_instance_a.clone(),
            knowledge_text: "Alpha knows the door code".into(),
            source: KnowledgeSource::Backstory,
            source_character_id: None,
            source_knowledge_id: None,
            turn_number: 0,
            event_id: None,
            pinned: true,
            propagation: Default::default(),
        },
        CharacterKnowledgeEntry {
            id: old_knowledge_b,
            campaign_id: old_campaign_id.clone(),
            character_id: old_instance_b.clone(),
            knowledge_text: "Beta heard the door code".into(),
            source: KnowledgeSource::ToldByOther,
            source_character_id: Some(old_instance_a.clone()),
            source_knowledge_id: Some(old_knowledge_a.clone()),
            turn_number: 1,
            event_id: None,
            pinned: false,
            propagation: Default::default(),
        },
    ];
    let tasks = vec![StoryTask::user_planned(
        old_campaign_id.clone(),
        "Open the sealed door",
        "Use the code later",
        vec![TaskTrigger::TurnReminder { at_turn: 2 }],
        1,
    )];
    let mut tasks = tasks;
    // Keep only valid related characters; broken refs are rejected by a dedicated test.
    tasks[0].related_characters = vec![old_instance_a.clone()];
    let summaries = vec![RoundSummary {
        id: Id::from_str("old-summary"),
        campaign_id: old_campaign_id.clone(),
        conversation_id: old_conversation_id,
        turn: 1,
        content: "Round one happened.".into(),
        created_at: chrono::Utc::now().to_rfc3339(),
        code: Some("A0001".into()),
        headline: None,
        lineage_id: Some(campaign_lineage),
        covered_by: None,
        level: 0,
        turn_end: 0,
        covers: vec![],
    }];

    let result = import_campaign_bundle_into_store(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: Some(card),
            campaign,
            instances,
            definitions,
            knowledge,
            tasks,
            summaries,
        },
    )
    .unwrap();

    assert_ne!(result.card_id, "old-card");
    assert_ne!(result.campaign_id, "old-campaign");
    assert_eq!(result.instance_count, 2);
    assert_eq!(result.knowledge_count, 2);
    assert_eq!(result.task_count, 1);
    assert_eq!(result.summary_count, 1);

    let new_campaign_id = Id::from_str(&result.campaign_id);
    let imported_campaign = store.get_campaign(&new_campaign_id).unwrap();
    assert_eq!(imported_campaign.card_id.as_str(), result.card_id);
    assert_eq!(
        imported_campaign.conversation_id.as_ref().unwrap().as_str(),
        result.conversation_id
    );
    assert!(conv_store.find_by_campaign(&new_campaign_id).is_some());

    let imported_card = store.get_card(&Id::from_str(&result.card_id)).unwrap();
    let (first_mes, alternate_greetings) = raw_card_greetings(&imported_card.card.raw_card_json);
    assert_eq!(first_mes, "hello from raw");
    assert_eq!(alternate_greetings, vec!["alt one", "alt two"]);
    assert!(
        imported_card
            .card
            .character_definitions
            .iter()
            .all(|def| def.card_id.as_str() == result.card_id)
    );
    assert!(
        imported_card
            .card
            .character_definitions
            .iter()
            .all(|def| def.id.as_str() != "old-def-a" && def.id.as_str() != "old-def-b")
    );

    let imported_instances = store.list_instances(&new_campaign_id);
    assert_eq!(imported_instances.len(), 2);
    assert!(
        imported_instances
            .iter()
            .all(|inst| inst.campaign_id == new_campaign_id)
    );
    assert!(
        imported_instances.iter().all(
            |inst| inst.id.as_str() != "old-instance-a" && inst.id.as_str() != "old-instance-b"
        )
    );

    let imported_knowledge = store.list_knowledge(&new_campaign_id);
    assert_eq!(imported_knowledge.len(), 2);
    let told = imported_knowledge
        .iter()
        .find(|entry| entry.source == KnowledgeSource::ToldByOther)
        .unwrap();
    assert!(told.source_character_id.is_some());
    assert_ne!(
        told.source_character_id.as_ref().unwrap().as_str(),
        "old-instance-a"
    );
    assert!(told.source_knowledge_id.is_some());
    assert_ne!(
        told.source_knowledge_id.as_ref().unwrap().as_str(),
        "old-knowledge-a"
    );

    let imported_tasks = store.list_tasks(&new_campaign_id);
    assert_eq!(imported_tasks.len(), 1);
    assert_eq!(imported_tasks[0].related_characters.len(), 1);
    assert_ne!(
        imported_tasks[0].related_characters[0].as_str(),
        "old-instance-a"
    );

    let imported_summaries = store.list_summaries(&new_campaign_id);
    assert_eq!(imported_summaries.len(), 1);
    assert_eq!(
        imported_summaries[0].conversation_id.as_str(),
        result.conversation_id
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_is_atomic_on_mid_write_failure() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_bundle_atomic_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let old_card_id = Id::from_str("atomic-card");
    let old_campaign_id = Id::from_str("atomic-campaign");
    let old_def = Id::from_str("atomic-def");
    let old_instance = Id::from_str("atomic-instance");

    let definitions = vec![CharacterDefinition {
        id: old_def.clone(),
        card_id: old_card_id.clone(),
        name: "Atomic".into(),
        persona_prompt: "persona".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: vec![],
    }];
    let card = CharacterCard {
        id: old_card_id.clone(),
        name: "Atomic Card".into(),
        source_character_id: Id::from_str("atomic-source"),
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({"first_mes": "hi"}),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let mut campaign = Campaign::new(old_card_id.clone(), "Atomic Campaign");
    campaign.id = old_campaign_id.clone();
    campaign.set_variable("story_clock", serde_json::json!("Day 3"), 2);
    let instances = vec![CharacterInstance {
        id: old_instance,
        campaign_id: old_campaign_id,
        definition_id: Some(old_def),
        name: "Atomic".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![storyforge_domain::variables::VariableValue::new(
            "hp",
            serde_json::json!(77),
            2,
        )],
        is_temporary: false,
    }];

    let err = import_campaign_bundle_into_store_with_after_campaign(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: Some(card),
            campaign,
            instances,
            definitions,
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![],
        },
        || Err(TauriCommandError::storage("injected after campaign save")),
    )
    .expect_err("post-campaign failure should fail the import");

    match err {
        TauriCommandError::Storage { .. } | TauriCommandError::Internal { .. } => {}
        other => panic!("expected storage/internal error, got {other:?}"),
    }

    assert!(
        store.list_cards().is_empty(),
        "failed import must not leave a partial card"
    );
    assert!(
        store.list_campaigns().is_empty(),
        "failed import must not leave a partial campaign"
    );
    assert!(
        store.list_all_instances().is_empty(),
        "failed import must not leave partial instances"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn campaign_bundle_roundtrip_preserves_variables_and_multi_character_semantics() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};
    use storyforge_domain::variables::VariableValue;

    let export_dir = std::env::temp_dir().join(format!(
        "storyforge_test_bundle_vars_export_{}",
        uuid::Uuid::new_v4()
    ));
    let import_dir = std::env::temp_dir().join(format!(
        "storyforge_test_bundle_vars_import_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&export_dir).unwrap();
    std::fs::create_dir_all(&import_dir).unwrap();
    let export_store = campaign_store::CampaignStore::new(&export_dir);
    let import_store = campaign_store::CampaignStore::new(&import_dir);
    let import_conv = ConversationStore::new(import_dir.join("conversations"));

    let card_id = Id::from_str("vars-card");
    let campaign_id = Id::from_str("vars-campaign");
    let def_a = Id::from_str("vars-def-a");
    let def_b = Id::from_str("vars-def-b");
    let inst_a = Id::from_str("vars-inst-a");
    let inst_b = Id::from_str("vars-inst-b");
    let knowledge_a = Id::from_str("vars-know-a");
    let knowledge_b = Id::from_str("vars-know-b");

    let definitions = vec![
        CharacterDefinition {
            id: def_a.clone(),
            card_id: card_id.clone(),
            name: "Alpha".into(),
            persona_prompt: "alpha persona".into(),
            behavior_rules: "alpha rules".into(),
            base_backstory: vec!["alpha backstory".into()],
            group: Some("party".into()),
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        },
        CharacterDefinition {
            id: def_b.clone(),
            card_id: card_id.clone(),
            name: "Beta".into(),
            persona_prompt: "beta persona".into(),
            behavior_rules: "beta rules".into(),
            base_backstory: vec![],
            group: Some("party".into()),
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        },
    ];
    let card = CharacterCard {
        id: card_id.clone(),
        name: "Multi Card".into(),
        source_character_id: Id::from_str("vars-source"),
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({
            "first_mes": "multi-open",
            "alternate_greetings": ["m1", "m2"]
        }),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: Some("two characters".into()),
    };
    export_store.save_card(card).unwrap();

    let mut campaign = Campaign::new(card_id.clone(), "Vars Campaign");
    campaign.id = campaign_id.clone();
    campaign.revision = 4;
    campaign.chronicle_revision = 2;
    campaign.set_variable("story_clock", serde_json::json!("Day 12 - night"), 8);
    campaign.set_variable("weather", serde_json::json!("storm"), 8);
    export_store.save_campaign(campaign).unwrap();

    for instance in [
        CharacterInstance {
            id: inst_a.clone(),
            campaign_id: campaign_id.clone(),
            definition_id: Some(def_a.clone()),
            name: "Alpha".into(),
            persona_override: Some("alpha live".into()),
            behavior_override: None,
            variables: vec![VariableValue::new("hp", serde_json::json!(88), 8)],
            is_temporary: false,
        },
        CharacterInstance {
            id: inst_b.clone(),
            campaign_id: campaign_id.clone(),
            definition_id: Some(def_b.clone()),
            name: "Beta".into(),
            persona_override: None,
            behavior_override: Some("beta live".into()),
            variables: vec![VariableValue::new("mood", serde_json::json!("wary"), 8)],
            is_temporary: false,
        },
    ] {
        export_store.add_instance(instance).unwrap();
    }

    let mut witnessed = CharacterKnowledgeEntry::witnessed(
        campaign_id.clone(),
        inst_a.clone(),
        "Alpha saw the seal break",
        7,
    );
    witnessed.id = knowledge_a.clone();
    witnessed.pinned = true;
    let mut told = CharacterKnowledgeEntry {
        id: knowledge_b,
        campaign_id: campaign_id.clone(),
        character_id: inst_b.clone(),
        knowledge_text: "Beta was told about the seal".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: Some(inst_a.clone()),
        source_knowledge_id: Some(knowledge_a),
        turn_number: 8,
        event_id: None,
        pinned: false,
        propagation: Default::default(),
    };
    let _ = &mut told;
    export_store.add_knowledge(vec![witnessed, told]).unwrap();

    let mut task = StoryTask::user_planned(
        campaign_id.clone(),
        "Repair the seal",
        "Both characters involved",
        vec![TaskTrigger::TurnReminder { at_turn: 10 }],
        8,
    );
    task.related_characters = vec![inst_a, inst_b];
    export_store.add_task(task).unwrap();

    let bundle_json = export_campaign_bundle_from_store(&export_store, campaign_id).unwrap();
    let bundle: CampaignBundle = serde_json::from_str(&bundle_json).unwrap();
    assert_eq!(bundle.definitions.len(), 2);
    assert_eq!(bundle.instances.len(), 2);
    assert!(
        bundle.card.as_ref().unwrap().character_definitions.len() == 2,
        "bundle must keep multi-character definitions instead of flattening"
    );

    let imported = import_campaign_bundle_into_store(&import_store, &import_conv, bundle).unwrap();
    let new_campaign_id = Id::from_str(&imported.campaign_id);
    let imported_campaign = import_store.get_campaign(&new_campaign_id).unwrap();
    assert_eq!(
        imported_campaign.get_variable("story_clock").unwrap(),
        &serde_json::json!("Day 12 - night")
    );
    assert_eq!(
        imported_campaign.get_variable("weather").unwrap(),
        &serde_json::json!("storm")
    );
    // revision/chronicle_revision are Campaign fields and must survive exportximport.
    assert_eq!(imported_campaign.revision, 4);
    assert_eq!(imported_campaign.chronicle_revision, 2);

    let imported_card = import_store
        .get_card(&Id::from_str(&imported.card_id))
        .unwrap();
    assert_eq!(imported_card.card.character_definitions.len(), 2);
    assert!(
        imported_card
            .card
            .character_definitions
            .iter()
            .any(|d| d.name == "Alpha")
            && imported_card
                .card
                .character_definitions
                .iter()
                .any(|d| d.name == "Beta"),
        "multi-character definitions must both survive"
    );

    let imported_instances = import_store.list_instances(&new_campaign_id);
    assert_eq!(imported_instances.len(), 2);
    let alpha = imported_instances
        .iter()
        .find(|i| i.name == "Alpha")
        .unwrap();
    let beta = imported_instances
        .iter()
        .find(|i| i.name == "Beta")
        .unwrap();
    assert_eq!(
        alpha
            .variables
            .iter()
            .find(|v| v.key == "hp")
            .map(|v| &v.value),
        Some(&serde_json::json!(88))
    );
    assert_eq!(
        beta.variables
            .iter()
            .find(|v| v.key == "mood")
            .map(|v| &v.value),
        Some(&serde_json::json!("wary"))
    );
    assert_eq!(alpha.persona_override.as_deref(), Some("alpha live"));
    assert_eq!(beta.behavior_override.as_deref(), Some("beta live"));

    let knowledge = import_store.list_knowledge(&new_campaign_id);
    assert_eq!(knowledge.len(), 2);
    let told = knowledge
        .iter()
        .find(|k| k.source == KnowledgeSource::ToldByOther)
        .unwrap();
    let source_instance = told.source_character_id.as_ref().unwrap();
    assert_eq!(source_instance, &alpha.id);
    assert!(
        knowledge
            .iter()
            .any(|k| k.id == *told.source_knowledge_id.as_ref().unwrap()),
        "knowledge provenance must point at rewritten source knowledge id"
    );

    let tasks = import_store.list_tasks(&new_campaign_id);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].related_characters.len(), 2);

    let _ = std::fs::remove_dir_all(&export_dir);
    let _ = std::fs::remove_dir_all(&import_dir);
}

#[test]
fn import_campaign_bundle_rejects_unsupported_version_without_mutation() {
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::character::CharacterCard;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_bundle_bad_version_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let card = CharacterCard {
        id: Id::from_str("v-card"),
        name: "V".into(),
        source_character_id: Id::from_str("v-source"),
        character_definitions: vec![],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::Value::Null,
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Unknown,
        extraction_message: None,
    };
    let campaign = Campaign::new(card.id.clone(), "V");
    let err = import_campaign_bundle_into_store(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: 99,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: Some(card),
            campaign,
            instances: vec![],
            definitions: vec![],
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![],
        },
    )
    .expect_err("unsupported version must fail");
    match err {
        TauriCommandError::Validation { .. } => {}
        other => panic!("expected validation error, got {other:?}"),
    }
    assert!(store.list_cards().is_empty());
    assert!(store.list_campaigns().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_rejects_malformed_summary_graphs_before_writes() {
    let cases = [
        "duplicate-id",
        "asymmetric-edge",
        "cycle",
        "wrong-level",
        "wrong-span",
        "scope-drift",
        "campaign-lineage-missing",
        "lineage-missing",
        "lineage-drift",
        "code-level-mismatch",
        "duplicate-code",
        "duplicate-cover",
    ];

    for case in cases {
        let dir = std::env::temp_dir().join(format!(
            "storyforge_test_import_bad_graph_{case}_{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = campaign_store::CampaignStore::new(&dir);
        let conv_store = ConversationStore::new(dir.join("conversations"));
        let mut bundle = valid_summary_graph_bundle();

        match case {
            "duplicate-id" => bundle.summaries.push(bundle.summaries[0].clone()),
            "asymmetric-edge" => bundle.summaries[0].covered_by = None,
            "cycle" => {
                let parent_id = bundle.summaries[2].id.clone();
                let child_id = bundle.summaries[0].id.clone();
                bundle.summaries[0].covers = vec![parent_id];
                bundle.summaries[2].covered_by = Some(child_id);
            }
            "wrong-level" => bundle.summaries[2].level = 2,
            "wrong-span" => bundle.summaries[2].turn_end = 1,
            "scope-drift" => bundle.summaries[1].campaign_id = Id::from_str("other-campaign"),
            "campaign-lineage-missing" => bundle.campaign.lineage_id = None,
            "lineage-missing" => bundle.summaries[0].lineage_id = None,
            "lineage-drift" => bundle.summaries[0].lineage_id = Some(Id::from_str("other-lineage")),
            "code-level-mismatch" => bundle.summaries[2].code = Some("A9999".into()),
            "duplicate-code" => bundle.summaries[1].code = bundle.summaries[0].code.clone(),
            "duplicate-cover" => {
                let child = bundle.summaries[2].covers[0].clone();
                bundle.summaries[2].covers.push(child);
            }
            _ => unreachable!(),
        }

        let error = import_campaign_bundle_into_store(&store, &conv_store, bundle)
            .expect_err("malformed summary graph must fail closed");
        assert!(
            matches!(error, TauriCommandError::Validation { .. }),
            "case={case}, unexpected={error:?}"
        );
        assert!(store.list_cards().is_empty(), "case={case}");
        assert!(store.list_campaigns().is_empty(), "case={case}");
        assert!(store.list_all_summaries().is_empty(), "case={case}");
        assert!(conv_store.list().is_empty(), "case={case}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn import_campaign_bundle_clears_nonportable_pending_compress_marker() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_pending_marker_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));
    let mut bundle = valid_summary_graph_bundle();
    let parent = bundle.summaries[2].id.clone();
    let children: Vec<_> = bundle.summaries[..2]
        .iter()
        .map(|summary| (summary.id.clone(), parent.clone()))
        .collect();
    bundle.campaign.pending_compress_publication = Some(
        storyforge_domain::chronicle::PendingCompressPublication::new(
            bundle.campaign.chronicle_revision,
            vec![parent],
            children,
        ),
    );

    let imported = import_campaign_bundle_into_store(&store, &conv_store, bundle).unwrap();
    let campaign = store
        .get_campaign(&Id::from_str(&imported.campaign_id))
        .unwrap();
    assert!(
        campaign.pending_compress_publication.is_none(),
        "an in-flight source publication cannot be resumed with rewritten ids"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_rewrites_summary_covers_and_preserves_bc_graph() {
    use storyforge_domain::agent::RoundSummary;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_bundle_chronicle_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let card_id = Id::from_str("chr-card");
    let campaign_id = Id::from_str("chr-campaign");
    let def_id = Id::from_str("chr-def");
    let instance_id = Id::from_str("chr-inst");
    let leaf_a1 = Id::from_str("leaf-a1");
    let leaf_a2 = Id::from_str("leaf-a2");
    let parent_b = Id::from_str("parent-b");
    let conversation_id = Id::from_str("chr-conversation");

    let definitions = vec![CharacterDefinition {
        id: def_id.clone(),
        card_id: card_id.clone(),
        name: "Chron".into(),
        persona_prompt: "persona".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: vec![],
    }];
    let card = CharacterCard {
        id: card_id.clone(),
        name: "Chronicle Card".into(),
        source_character_id: Id::from_str("chr-source"),
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({"first_mes": "hi"}),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let mut campaign = Campaign::new(card_id.clone(), "Chronicle Campaign");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    campaign.chronicle_revision = 3;
    let campaign_lineage = campaign.lineage_id.clone().unwrap();

    let instances = vec![CharacterInstance {
        id: instance_id,
        campaign_id: campaign_id.clone(),
        definition_id: Some(def_id),
        name: "Chron".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    }];

    let mut a1 = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        1,
        "leaf one".into(),
    );
    a1.id = leaf_a1.clone();
    a1.code = Some("A0001".into());
    a1.lineage_id = Some(campaign_lineage.clone());
    a1.level = 0;
    a1.turn_end = 1;
    a1.covered_by = Some(parent_b.clone());

    let mut a2 = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        2,
        "leaf two".into(),
    );
    a2.id = leaf_a2.clone();
    a2.code = Some("A0002".into());
    a2.lineage_id = Some(campaign_lineage.clone());
    a2.level = 0;
    a2.turn_end = 2;
    a2.covered_by = Some(parent_b.clone());

    // B-level parent shares turn span start with a1; add_summary-by-turn would clobber it.
    let mut b = RoundSummary::new(
        campaign_id.clone(),
        conversation_id,
        1,
        "band covering leaves".into(),
    );
    b.id = parent_b.clone();
    b.code = Some("B0001".into());
    b.lineage_id = Some(campaign_lineage);
    b.level = 1;
    b.turn_end = 2;
    b.covers = vec![leaf_a1.clone(), leaf_a2.clone()];
    b.covered_by = None;

    let imported = import_campaign_bundle_into_store(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: Some(card),
            campaign,
            instances,
            definitions,
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![a1, a2, b],
        },
    )
    .expect("chronicle graph bundle should import");

    let new_campaign_id = Id::from_str(&imported.campaign_id);
    let summaries = store.list_summaries(&new_campaign_id);
    assert_eq!(
        summaries.len(),
        3,
        "A leaves and B parent must all survive import"
    );
    assert_eq!(imported.summary_count, 3);

    let parent = summaries
        .iter()
        .find(|s| s.level == 1)
        .expect("B parent should exist");
    assert_eq!(parent.covers.len(), 2);
    assert!(
        !parent.covers.contains(&leaf_a1) && !parent.covers.contains(&leaf_a2),
        "covers must be rewritten to new leaf ids, not keep old ids"
    );

    let leaves: Vec<_> = summaries.iter().filter(|s| s.level == 0).collect();
    assert_eq!(leaves.len(), 2);
    for leaf in leaves {
        assert_eq!(
            leaf.covered_by.as_ref(),
            Some(&parent.id),
            "covered_by must point at rewritten parent id"
        );
        assert!(
            parent.covers.contains(&leaf.id),
            "parent.covers must include rewritten leaf id {}",
            leaf.id
        );
        assert_ne!(leaf.id, leaf_a1);
        assert_ne!(leaf.id, leaf_a2);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_rejects_broken_internal_references() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_bundle_broken_refs_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let card_id = Id::from_str("brk-card");
    let campaign_id = Id::from_str("brk-campaign");
    let def_id = Id::from_str("brk-def");
    let instance_id = Id::from_str("brk-inst");

    let definitions = vec![CharacterDefinition {
        id: def_id,
        card_id: card_id.clone(),
        name: "Broken".into(),
        persona_prompt: "persona".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: vec![],
    }];
    let card = CharacterCard {
        id: card_id.clone(),
        name: "Broken Card".into(),
        source_character_id: Id::from_str("brk-source"),
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({}),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let mut campaign = Campaign::new(card_id, "Broken Campaign");
    campaign.id = campaign_id.clone();

    // Instance points at a definition id that is not present after rewrite map.
    let instances = vec![CharacterInstance {
        id: instance_id.clone(),
        campaign_id: campaign_id.clone(),
        definition_id: Some(Id::from_str("missing-def")),
        name: "Broken".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    }];
    let knowledge = vec![CharacterKnowledgeEntry {
        id: Id::from_str("k1"),
        campaign_id: campaign_id.clone(),
        character_id: instance_id.clone(),
        knowledge_text: "orphan source".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: Some(Id::from_str("ghost-instance")),
        source_knowledge_id: Some(Id::from_str("ghost-knowledge")),
        turn_number: 1,
        event_id: None,
        pinned: false,
        propagation: Default::default(),
    }];
    let mut task = StoryTask::user_planned(
        campaign_id,
        "broken task",
        "refs ghost",
        vec![TaskTrigger::TurnReminder { at_turn: 2 }],
        1,
    );
    task.related_characters = vec![instance_id, Id::from_str("ghost-instance")];

    let err = import_campaign_bundle_into_store(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: Some(card),
            campaign,
            instances,
            definitions,
            knowledge,
            tasks: vec![task],
            summaries: vec![],
        },
    )
    .expect_err("broken internal references must fail closed");

    match err {
        TauriCommandError::Validation { message } => {
            assert!(
                message.contains("definition")
                    || message.contains("knowledge")
                    || message.contains("related_characters")
                    || message.contains("\u{5f15}\u{7528}"),
                "validation message should mention broken refs: {message}"
            );
        }
        other => panic!("expected validation error, got {other:?}"),
    }
    assert!(store.list_cards().is_empty());
    assert!(store.list_campaigns().is_empty());
    assert!(store.list_all_instances().is_empty());
    assert!(store.list_all_knowledge().is_empty());
    assert!(store.list_all_tasks().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_rollback_is_verified_on_disk() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_bundle_verified_rollback_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let card_id = Id::from_str("vr-card");
    let campaign_id = Id::from_str("vr-campaign");
    let def_id = Id::from_str("vr-def");
    let definitions = vec![CharacterDefinition {
        id: def_id.clone(),
        card_id: card_id.clone(),
        name: "Verified".into(),
        persona_prompt: "persona".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: vec![],
    }];
    let card = CharacterCard {
        id: card_id.clone(),
        name: "Verified Card".into(),
        source_character_id: Id::from_str("vr-source"),
        character_definitions: definitions.clone(),
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({}),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let mut campaign = Campaign::new(card_id, "Verified Campaign");
    campaign.id = campaign_id.clone();
    let instances = vec![CharacterInstance {
        id: Id::from_str("vr-inst"),
        campaign_id,
        definition_id: Some(def_id),
        name: "Verified".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    }];

    let instances_path = dir.join("instances.json");
    let instances_sentinel = instances_path.join("pre-existing-after-baseline.txt");
    let error = import_campaign_bundle_into_store_with_after_campaign(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: Some(card),
            campaign,
            instances,
            definitions,
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![],
        },
        || {
            // Corrupt a collection only after the strict preflight so this
            // exercises rollback verification rather than baseline rejection.
            std::fs::create_dir_all(&instances_path).map_err(|error| {
                TauriCommandError::storage(format!("inject rollback fault: {error}"))
            })?;
            std::fs::write(&instances_sentinel, b"must survive").map_err(|error| {
                TauriCommandError::storage(format!("inject sentinel fault: {error}"))
            })?;
            Ok(())
        },
    )
    .expect_err("instance write failure should fail import");
    let message = match error {
        TauriCommandError::Storage { message } => message,
        other => panic!("expected storage error, got {other:?}"),
    };
    assert!(
        message.contains("\u{56de}\u{6eda}\u{672a}\u{5b8c}\u{5168}\u{9a8c}\u{8bc1}")
            && message.contains("strict disk read failed")
            && message.contains("instances.json"),
        "rollback verification must report the preserved non-file path: {message}"
    );

    // Reload store from disk x in-memory empty is not enough.
    let reloaded = campaign_store::CampaignStore::new(&dir);
    assert!(
        reloaded.list_cards().is_empty(),
        "rollback must clear cards on disk"
    );
    assert!(
        reloaded.list_campaigns().is_empty(),
        "rollback must clear campaigns on disk"
    );
    assert!(
        reloaded.list_all_instances().is_empty(),
        "rollback must clear instances on disk"
    );
    assert_eq!(std::fs::read(&instances_sentinel).unwrap(), b"must survive");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn strict_bundle_disk_snapshot_rejects_unreadable_collection() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_strict_snapshot_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(dir.join("instances.json")).unwrap();
    let conversations_dir = dir.join("conversations");
    std::fs::create_dir_all(&conversations_dir).unwrap();

    let error = read_bundle_disk_snapshot_strict(&dir, &conversations_dir)
        .expect_err("a collection path that is a directory must not deserialize as empty");

    assert!(error.contains("instances.json"), "unexpected: {error}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_rejects_preexisting_corrupt_store_before_any_write() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_corrupt_baseline_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let corrupt = br#"[{"card":"truncated"}"#;
    std::fs::write(dir.join("cards.json"), corrupt).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));
    let campaign = storyforge_domain::campaign::Campaign::new(
        Id::from_str("corrupt-baseline-card"),
        "Corrupt Baseline",
    );

    let error = import_campaign_bundle_into_store(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: None,
            campaign,
            instances: vec![],
            definitions: vec![],
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![],
        },
    )
    .expect_err("corrupt pre-import disk state must fail before writes");

    assert!(matches!(error, TauriCommandError::Storage { .. }));
    assert_eq!(
        std::fs::read(dir.join("cards.json")).unwrap(),
        corrupt,
        "preflight must not overwrite a corrupt source file"
    );
    assert!(store.list_cards().is_empty());
    assert!(store.list_campaigns().is_empty());
    assert!(conv_store.list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_campaign_bundle_conversation_create_failure_leaves_no_card_or_campaign() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_import_conversation_failure_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let conversations_path = dir.join("conversations");
    std::fs::write(&conversations_path, b"blocked").unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(conversations_path);
    let campaign = storyforge_domain::campaign::Campaign::new(
        Id::from_str("conv-fail-card"),
        "Conversation Failure",
    );

    let error = import_campaign_bundle_into_store(
        &store,
        &conv_store,
        CampaignBundle {
            format_version: BUNDLE_FORMAT_VERSION,
            exported_at: chrono::Utc::now().to_rfc3339(),
            card: None,
            campaign,
            instances: vec![],
            definitions: vec![],
            knowledge: vec![],
            tasks: vec![],
            summaries: vec![],
        },
    )
    .expect_err("conversation create must fail closed");

    assert!(matches!(error, TauriCommandError::Storage { .. }));
    assert!(store.list_cards().is_empty());
    assert!(store.list_campaigns().is_empty());
    assert!(conv_store.list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
#[ignore = "requires a local real ST card fixture; run scripts/run-real-card-smoke.ps1"]
async fn test_real_complex_card_offline_mvu_plumbing_smoke() {
    // This is an offline plumbing smoke. It uses the real complex PNG fixture
    // for import/campaign wiring, but feeds a deterministic MVU tool response
    // and a synthetic postprocess update so it can run without live LLM creds.
    use storyforge_domain::agent::VariableUpdate;
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character::{
        CharacterCard, CharacterDefinition, CharacterExtractionStatus,
    };
    use storyforge_domain::llm::{ChatResponse, FunctionCall, ToolCall};
    use storyforge_domain::mvu_translation::MvuRouting;

    let fixture_path = std::env::var_os("SF_COMPLEX_CARD_FIXTURE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("..")
                .join("test-card.png")
        });
    let bytes = std::fs::read(&fixture_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", fixture_path.display()));
    let character =
        storyforge_infra_import::import_character(&bytes).expect("complex card should import");

    let dir = TempDirGuard::new("storyforge_test_real_complex_offline_mvu_plumbing");
    let store = campaign_store::CampaignStore::new(dir.path());
    let conv_store = ConversationStore::new(dir.path().join("conversations"));

    let mut card = CharacterCard::from_character(&character);
    card.extraction_status = CharacterExtractionStatus::Extracted;
    let mut definition = CharacterDefinition::fallback_from_character(&character, &[]);
    definition.card_id = card.id.clone();
    card.character_definitions = vec![definition];

    let stored = save_character_card_to_store(&store, card).unwrap();
    let campaign = create_campaign_in_store(
        &store,
        &conv_store,
        stored.card.id.as_str().to_string(),
        "Complex Fixture MVU Fallback Campaign".into(),
        None,
    )
    .unwrap();

    let campaign_id = Id::from_str(&campaign.id);
    let initial_instances = store.list_instances(&campaign_id);
    let mut initial_instance = initial_instances
        .first()
        .expect("campaign should create at least one instance")
        .clone();
    let initial_variable_count = initial_instance.variables.len();
    let initial_had_mana = initial_instance
        .variables
        .iter()
        .any(|value| value.key == "mana");
    initial_instance
        .variables
        .retain(|value| value.key != "mana");
    store.update_instance(initial_instance).unwrap();

    let mvu_fixture_json = serde_json::json!({
        "variable_schema": [
            {"key": "hp", "label": "HP", "value_type": "int", "default": 100},
            {"key": "mana", "label": "Mana", "value_type": "int", "default": 30}
        ],
        "ui_bindings": [
            {"element": "hp_bar", "variable_key": "hp", "display": {"kind": "bar", "max": 100}},
            {"element": "mana_text", "variable_key": "mana", "display": {"kind": "text"}}
        ],
        "update_rules": ["damage reduces hp"],
        "interactions": [],
        "fallback_fragments": [
            {
                "description": "complex card fallback probe",
                "js_snippet": "variables.__complex_card_probe = true;",
                "reason": "offline MVU plumbing smoke"
            },
            {
                "description": "empty fallback fragments are ignored",
                "js_snippet": "",
                "reason": "filter coverage"
            }
        ],
        "routing": {"kind": "hybrid", "webview_reason": "offline fixture contains JS fallback"},
        "analysis_confidence": 0.92,
        "notes": ["offline fixture: this smoke validates plumbing, not live LLM analysis"]
    })
    .to_string();
    let mvu_response = ChatResponse {
        content: String::new(),
        reasoning_content: None,
        tool_calls: vec![ToolCall {
            id: "mvu-smoke-tool-call".into(),
            call_type: "function".into(),
            function: FunctionCall {
                name: "emit_mvu_translation".into(),
                arguments: mvu_fixture_json,
            },
        }],
        finish_reason: Some("tool_calls".into()),
        usage: None,
    };
    let translation =
        storyforge_app_meta::mvu_import::parse_mvu_translation_from_response(&mvu_response, &[])
            .expect("offline MVU plumbing path should parse emit_mvu_translation");
    assert!(
        matches!(translation.routing, MvuRouting::Hybrid { .. }),
        "expected hybrid routing, got {:?}; fallback_count={}, schema_count={}",
        translation.routing,
        translation.fallback_fragments.len(),
        translation.variable_schema.len()
    );
    assert_eq!(translation.fallback_fragments.len(), 2);
    assert!(
        translation
            .variable_schema
            .iter()
            .any(|field| field.key == "mana")
    );

    save_mvu_translation_to_store(
        &store,
        campaign_store::StoredMvuTranslation {
            source_character_id: character.id.clone(),
            character_name: character.name.clone(),
            translation,
            analyzed_at: "2026-07-07T00:00:00Z".into(),
        },
    )
    .unwrap();
    meta_apply_mvu_schema_in_store(
        &store,
        character.id.as_str().to_string(),
        stored.card.character_definitions[0].id.as_str().to_string(),
    )
    .unwrap();

    let campaign = store
        .get_campaign(&campaign_id)
        .expect("campaign should exist");
    let instances = store.list_instances(&campaign_id);
    let present_instance_id = instances
        .first()
        .expect("campaign should create at least one instance")
        .id
        .as_str()
        .to_string();
    let instance_after_apply = instances
        .first()
        .expect("campaign should create at least one instance");
    assert!(
        instance_after_apply
            .variables
            .iter()
            .any(|value| value.key == "mana" && value.value == serde_json::json!(30)),
        "MVU apply should backfill existing campaign instances"
    );
    let expected_variable_count_after_apply = if initial_had_mana {
        initial_variable_count
    } else {
        initial_variable_count + 1
    };
    assert!(
        instance_after_apply.variables.len() >= expected_variable_count_after_apply,
        "MVU apply should preserve existing variables while backfilling missing fields"
    );
    let ctx = WritingContext {
        characters: vec![],
        world_info: None,
        conversation_id: campaign.conversation_id.clone().unwrap_or_default(),
        campaign_id: Some(campaign_id.clone()),
        turn: 1,
        pending_tasks: vec![],
        story_clock: String::new(),
        profile: None,
        modules: vec![],
        regex_scripts: vec![],
        campaign_runtime: Some(std::sync::Arc::new(CampaignRuntimeContext {
            campaign,
            instances,
            definitions_by_id: std::collections::HashMap::new(),
            knowledge: vec![],
            tasks: vec![],
            turn: 1,
        })),
        agent_profile_config: None,
        recent_summaries: vec![],
        chronicle_prompt_catalog: vec![],
        far_memory_hits: vec![],
        template_random_seed: None,
        context_epoch: None,
        chronicle_revision: 0,
    };

    let fragments =
        collect_mvu_fallback_fragments(&ctx, &store, std::slice::from_ref(&present_instance_id));
    assert_eq!(fragments.len(), 1);
    assert_eq!(
        fragments[0].js_snippet,
        "variables.__complex_card_probe = true;"
    );

    let rules = collect_mvu_update_rules(&ctx, &store, std::slice::from_ref(&present_instance_id));
    assert_eq!(rules, vec!["damage reduces hp".to_string()]);
    let rules_dedup = collect_mvu_update_rules(
        &ctx,
        &store,
        &[present_instance_id.clone(), character.name.clone()],
    );
    assert_eq!(
        rules_dedup.len(),
        1,
        "\u{540c}\u{4e00}\u{5f20}\u{6e90}\u{5361}\u{7684}\u{591a}\u{4e2a}\u{5728}\u{573a}\u{89d2}\u{8272}\u{53ea}\u{5e94}\u{8d21}\u{732e}\u{4e00}\u{6b21}\u{89c4}\u{5219}"
    );

    let fragments_by_name = collect_mvu_fallback_fragments(&ctx, &store, &[character.name]);
    assert_eq!(fragments_by_name.len(), 1);

    let persist_ctx = PostprocessPersistContext {
        campaign_id: campaign_id.clone(),
        conversation_id: ctx.conversation_id.clone(),
        turn: 2,
    };
    let outcome = storyforge_app_agent::PostProcessOutcome {
        summary: Some("offline MVU plumbing smoke summary".into()),
        summary_attempted: true,
        post_process_attempted: true,
        post_process: Some(storyforge_domain::agent::PostProcessResult {
            knowledge_updates: vec![],
            variable_updates: vec![VariableUpdate {
                instance_id: Some(Id::from_str(&present_instance_id)),
                key: "mana".into(),
                value: serde_json::json!(64),
            }],
            task_updates: vec![],
            parse_succeeded: true,
        }),
    };
    persist_postprocess_outcome_to_store(
        &store,
        &persist_ctx,
        &outcome,
        std::slice::from_ref(&present_instance_id),
    );

    let updated_instance = store
        .list_instances(&campaign_id)
        .into_iter()
        .find(|inst| inst.id.as_str() == present_instance_id)
        .expect("updated instance should still exist");
    let mana = updated_instance
        .variables
        .iter()
        .find(|value| value.key == "mana")
        .expect("mana variable should exist after MVU apply");
    assert_eq!(mana.value, serde_json::json!(64));
    assert_eq!(mana.last_updated_turn, 2);
}
