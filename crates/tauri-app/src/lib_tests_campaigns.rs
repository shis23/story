use super::*;

fn make_test_character(name: &str) -> Character {
    use storyforge_domain::Source;
    Character {
        id: Id::new(),
        name: name.into(),
        description: String::new(),
        personality: String::new(),
        scenario: String::new(),
        first_mes: String::new(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        alternate_greetings: vec![],
        embedded_world_info: None,
        extensions: serde_json::json!({}),
        renderable_assets: None,
        source: Source::Native,
        spec_version: "3.0".into(),
        raw_card_json: serde_json::json!({}),
    }
}

#[test]
fn character_detail_resolves_the_card_source_character_id() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_character_detail_source_id_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = storage::CharacterStore::new(&dir);
    let character = make_test_character("Seraphina");
    let source_character_id = character.id.clone();
    let stored = store.save(CharacterInfo::from(&character)).unwrap();

    let by_storage_id =
        stored_character_for_id_or_source_in_store(&store, &Id::from_str(&stored.id))
            .expect("storage id should resolve");
    let by_source_id = stored_character_for_id_or_source_in_store(&store, &source_character_id)
        .expect("source character id from CardSummaryDto should resolve");

    assert_eq!(by_source_id.id, by_storage_id.id);
    assert_eq!(by_source_id.info.name, "Seraphina");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn stored_world_info_both_route_restores_constant_and_selective_semantics() {
    let mut info = CharacterInfo::from(&make_test_character("Active Card"));
    info.world_info_entries = vec![WorldInfoEntryInfo {
        keys: vec!["harbor".into()],
        content: "BOTH_ROUTE_LORE".into(),
        constant: true,
        route: "Both".into(),
        is_global: false,
        depth: 2,
        order: 100,
    }];

    let stored = storage::StoredCharacter {
        id: "active-card".into(),
        info,
        imported_at: "2026-07-07T00:00:00Z".into(),
    };
    let book = collect_world_info_for_active(&[stored], "Active Card");

    let constant_contents: Vec<_> = book
        .constant_entries()
        .into_iter()
        .map(|entry| entry.content.as_str())
        .collect();
    let triggered_contents: Vec<_> = book
        .triggered_selective_entries("sail to the harbor")
        .into_iter()
        .map(|entry| entry.content.as_str())
        .collect();

    assert_eq!(constant_contents, vec!["BOTH_ROUTE_LORE"]);
    assert_eq!(triggered_contents, vec!["BOTH_ROUTE_LORE"]);
}

#[test]
fn character_info_and_restore_preserve_alternate_greetings() {
    let mut character = make_test_character("Greeter");
    character.first_mes = "default opening".into();
    character.alternate_greetings = vec!["alternate one".into(), "alternate two".into()];

    let info = CharacterInfo::from(&character);
    assert_eq!(
        info.alternate_greetings,
        vec!["alternate one".to_string(), "alternate two".to_string()]
    );

    let stored = storage::StoredCharacter {
        id: "stored-greeter".into(),
        info,
        imported_at: "now".into(),
    };
    let restored = stored_info_to_character(&stored);
    assert_eq!(
        restored.alternate_greetings,
        vec!["alternate one".to_string(), "alternate two".to_string()]
    );
}

#[test]
fn character_info_restore_preserves_st_round_trip_fields() {
    use storyforge_domain::character::to_st_data;
    use storyforge_domain::world_info::{LoreRoute, SelectiveLogic, WorldInfoBook, WorldInfoEntry};

    let mut character = make_test_character("RoundTrip");
    character.description = "stored description".into();
    character.mes_example = "<START>\n\u{793a}\u{4f8b}\u{5bf9}\u{8bdd}".into();
    character.post_history_instructions = "\u{5386}\u{53f2}\u{540e}\u{6307}\u{4ee4}".into();
    character.character_version = "2.1".into();
    character.raw_card_json = serde_json::json!({
        "name": "RoundTrip",
        "description": "raw description",
        "group_only": true,
        "creator_notes": "keep this unknown field",
        "extensions": {
            "unknown_plugin": {"state": 7}
        }
    });
    character.embedded_world_info = Some(WorldInfoBook {
        source: storyforge_domain::Source::ImportedFromST,
        entries: vec![WorldInfoEntry {
            st_id: Some(9),
            keys: vec!["\u{94a5}\u{5319}".into()],
            secondary_keys: vec!["\u{95e8}".into()],
            content: "\u{4e16}\u{754c}\u{4e66}\u{5185}\u{5bb9}".into(),
            constant: true,
            selective: false,
            selective_logic: SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 3,
            order: 12,
            route: LoreRoute::Constant,
            extensions: serde_json::json!({"entry_extra": true}),
            extra: Default::default(),
        }],
        metadata: Default::default(),
    });

    let stored = storage::StoredCharacter {
        id: "stored-round-trip".into(),
        info: CharacterInfo::from(&character),
        imported_at: "now".into(),
    };

    let restored = stored_info_to_character(&stored);
    let exported = to_st_data(
        &restored,
        None,
        restored
            .embedded_world_info
            .as_ref()
            .map(|b| b.to_st_book()),
    );
    let exported_json = serde_json::to_value(&exported).unwrap();

    assert_eq!(
        restored.mes_example,
        "<START>\n\u{793a}\u{4f8b}\u{5bf9}\u{8bdd}"
    );
    assert_eq!(
        restored.post_history_instructions,
        "\u{5386}\u{53f2}\u{540e}\u{6307}\u{4ee4}"
    );
    assert_eq!(restored.character_version, "2.1");
    assert_eq!(exported_json["group_only"], true);
    assert_eq!(exported_json["creator_notes"], "keep this unknown field");
    assert_eq!(exported_json["extensions"]["unknown_plugin"]["state"], 7);
    assert_eq!(
        exported.mes_example,
        "<START>\n\u{793a}\u{4f8b}\u{5bf9}\u{8bdd}"
    );
    assert_eq!(
        exported.post_history_instructions,
        "\u{5386}\u{53f2}\u{540e}\u{6307}\u{4ee4}"
    );
    assert_eq!(exported.character_version, "2.1");
    assert_eq!(exported.character_book.unwrap().entries[0].id, Some(9));
}

#[test]
fn resolve_legacy_opening_message_accepts_only_card_greetings() {
    let mut character = make_test_character("Greeter");
    character.first_mes = "default opening".into();
    character.alternate_greetings = vec!["alternate one".into(), "alternate two".into()];
    let character = Arc::new(character);

    assert_eq!(
        resolve_legacy_opening_message(Some(&character), Some("alternate two".into())),
        Some("alternate two".into())
    );
    assert_eq!(
        resolve_legacy_opening_message(Some(&character), Some("not from this card".into())),
        Some("default opening".into())
    );
    assert_eq!(
        resolve_legacy_opening_message(Some(&character), Some("   ".into())),
        Some("default opening".into())
    );
}

#[test]
fn resolve_opening_message_from_parts_supports_campaign_greetings() {
    let alternates = vec!["alternate one".to_string(), "alternate two".to_string()];

    assert_eq!(
        resolve_opening_message_from_parts(
            "default opening",
            &alternates,
            Some("alternate two".into()),
            "campaign"
        ),
        Some("alternate two".into())
    );
    assert_eq!(
        resolve_opening_message_from_parts(
            "default opening",
            &alternates,
            Some("not from this card".into()),
            "campaign"
        ),
        Some("default opening".into())
    );
    assert_eq!(
        resolve_opening_message_from_parts("", &alternates, None, "campaign"),
        Some("alternate one".into())
    );
    assert_eq!(
        resolve_opening_message_from_parts("", &[], Some("missing".into()), "campaign"),
        None
    );
}

#[test]
fn test_delete_character_source_ids_include_domain_character_id() {
    let mut character = make_test_character("Lin");
    character.id = Id::from_str("domain-lin");

    let source_ids = delete_character_cascade_source_ids(
        "stored-lin",
        Some("Lin"),
        None,
        &[Arc::new(character)],
    );

    assert_eq!(
        source_ids,
        vec![Id::from_str("stored-lin"), Id::from_str("domain-lin")]
    );
}

#[test]
fn test_delete_character_source_ids_deduplicate_stored_id() {
    let mut character = make_test_character("Lin");
    character.id = Id::from_str("same-id");

    let source_ids =
        delete_character_cascade_source_ids("same-id", Some("Lin"), None, &[Arc::new(character)]);

    assert_eq!(source_ids, vec![Id::from_str("same-id")]);
}

#[test]
fn test_delete_character_source_ids_include_persisted_source_character_id() {
    let source_ids =
        delete_character_cascade_source_ids("stored-lin", Some("Lin"), Some("source-lin"), &[]);

    assert_eq!(
        source_ids,
        vec![Id::from_str("stored-lin"), Id::from_str("source-lin")]
    );
}

#[test]
fn test_stored_info_to_character_uses_persisted_source_character_id() {
    let mut character = make_test_character("Lin");
    character.id = Id::from_str("source-lin");
    let stored = storage::StoredCharacter {
        id: "stored-lin".into(),
        info: CharacterInfo::from(&character),
        imported_at: "now".into(),
    };

    let restored = stored_info_to_character(&stored);

    assert_eq!(restored.id, Id::from_str("source-lin"));
    assert_eq!(restored.name, "Lin");
}

#[test]
fn test_stored_info_to_character_preserves_extensions_for_scoped_regex() {
    let mut character = make_test_character("Regex Card");
    character.id = Id::from_str("source-regex");
    character.extensions = serde_json::json!({
        "regex_scripts": [
            {
                "id": "scoped-output",
                "scriptName": "Scoped output",
                "findRegex": "foo",
                "replaceString": "bar",
                "placement": [2],
                "disabled": false
            }
        ]
    });
    let stored = storage::StoredCharacter {
        id: "stored-regex".into(),
        info: CharacterInfo::from(&character),
        imported_at: "now".into(),
    };

    let restored = stored_info_to_character(&stored);
    let scripts = restored.scoped_regex_scripts();

    assert_eq!(restored.extensions, character.extensions);
    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].id, "scoped-output");
}

#[test]
fn campaign_world_info_dto_preserves_the_st_entry_comment_as_its_name() {
    let mut entry = storyforge_domain::world_info::WorldInfoEntry {
        st_id: Some(17),
        keys: vec!["fallback key".into()],
        secondary_keys: vec![],
        content: "lore".into(),
        constant: true,
        selective: false,
        selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
        disabled: false,
        position: 0,
        depth: 1,
        order: 100,
        route: storyforge_domain::world_info::LoreRoute::Constant,
        extensions: serde_json::json!({}),
        extra: Default::default(),
    };
    entry.extra.insert(
        "comment".into(),
        serde_json::Value::String(
            "\u{547d}\u{5b9a}\u{7cfb}\u{7edf}-\u{963f}\u{6bd4}\u{76d6}\u{5c14}\u{6838}\u{5fc3}"
                .into(),
        ),
    );

    let dto = world_info_entry_to_dto_full(7, &entry);

    assert_eq!(
        dto.name,
        "\u{547d}\u{5b9a}\u{7cfb}\u{7edf}-\u{963f}\u{6bd4}\u{76d6}\u{5c14}\u{6838}\u{5fc3}"
    );
}

#[test]
fn toggling_campaign_world_info_enabled_preserves_its_injection_route() {
    let mut entry = storyforge_domain::world_info::WorldInfoEntry {
        st_id: Some(18),
        keys: vec!["core".into()],
        secondary_keys: vec![],
        content: "lore".into(),
        constant: true,
        selective: false,
        selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
        disabled: false,
        position: 0,
        depth: 1,
        order: 100,
        route: storyforge_domain::world_info::LoreRoute::Constant,
        extensions: serde_json::json!({}),
        extra: Default::default(),
    };

    entry.set_enabled(false).unwrap();
    assert!(entry.disabled);
    assert!(matches!(
        entry.route,
        storyforge_domain::world_info::LoreRoute::Constant
    ));

    entry.set_enabled(true).unwrap();
    assert!(!entry.disabled);
    assert!(matches!(
        entry.route,
        storyforge_domain::world_info::LoreRoute::Constant
    ));
}

#[test]
fn enabling_a_disabled_route_restores_its_st_derived_injection_route() {
    let mut entry = storyforge_domain::world_info::WorldInfoEntry {
        st_id: Some(19),
        keys: vec!["core".into()],
        secondary_keys: vec![],
        content: "lore".into(),
        constant: false,
        selective: true,
        selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
        disabled: true,
        position: 0,
        depth: 1,
        order: 100,
        route: storyforge_domain::world_info::LoreRoute::Disabled,
        extensions: serde_json::json!({}),
        extra: Default::default(),
    };

    entry.set_enabled(true).unwrap();

    assert!(!entry.disabled);
    assert!(matches!(
        entry.route,
        storyforge_domain::world_info::LoreRoute::Selective
    ));
}

#[test]
fn enabling_a_disabled_both_world_info_route_restores_both_injection_paths() {
    let mut entry = storyforge_domain::world_info::WorldInfoEntry {
        st_id: Some(20),
        keys: vec!["hybrid core".into()],
        secondary_keys: vec![],
        content: "lore".into(),
        constant: true,
        selective: true,
        selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
        disabled: true,
        position: 0,
        depth: 1,
        order: 100,
        route: storyforge_domain::world_info::LoreRoute::Disabled,
        extensions: serde_json::json!({}),
        extra: Default::default(),
    };

    entry.set_enabled(true).unwrap();

    assert!(!entry.disabled);
    assert!(matches!(
        entry.route,
        storyforge_domain::world_info::LoreRoute::Both
    ));
}

#[test]
fn add_campaign_instance_from_card_definition_preserves_role_and_schema() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, RoleType};
    use storyforge_domain::variables::default_character_variables;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_add_card_instance_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let source = make_test_character("Roster");
    let mut card = CharacterCard::from_character(&source);
    card.id = Id::from_str("card-roster");
    let hero = make_test_character_definition(&card.id, "def-hero", "Hero");
    let mut extra = make_test_character_definition(&card.id, "def-extra", "Courier");
    extra.role_type = RoleType::Extra;
    extra.variable_schema = default_character_variables();
    card.character_definitions = vec![hero.clone(), extra.clone()];
    store.save_card(card).unwrap();

    let campaign = Campaign::new(Id::from_str("card-roster"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    store
        .add_instance(CharacterInstance::from_definition(
            campaign.id.clone(),
            &hero,
        ))
        .unwrap();

    let added =
        add_campaign_instance_to_store(&store, &campaign.id, Some(&extra.id), None, None, None)
            .unwrap();

    assert_eq!(added.name, "Courier");
    assert_eq!(added.definition_id.as_deref(), Some("def-extra"));
    assert_eq!(added.role_type.as_deref(), Some("extra"));
    assert!(!added.is_temporary);
    assert!(!added.variables.is_empty());

    let duplicate =
        add_campaign_instance_to_store(&store, &campaign.id, Some(&extra.id), None, None, None)
            .unwrap_err();
    assert!(duplicate.to_string().contains("\u{5df2}\u{52a0}\u{5165}"));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn add_campaign_custom_instance_creates_validated_temporary_character() {
    use storyforge_domain::campaign::Campaign;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_add_custom_instance_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-custom"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let added = add_campaign_instance_to_store(
        &store,
        &campaign.id,
        None,
        Some("  \u{6e21}\u{9e26}\u{4fe1}\u{4f7f}  ".into()),
        Some("  \u{5be1}\u{8a00}\u{800c}\u{8b66}\u{89c9}  ".into()),
        Some("  \u{53ea}\u{4ea4}\u{4ed8}\u{5bc6}\u{4fe1}  ".into()),
    )
    .unwrap();

    assert_eq!(added.name, "\u{6e21}\u{9e26}\u{4fe1}\u{4f7f}");
    assert_eq!(added.role_type.as_deref(), Some("extra"));
    assert!(added.is_temporary);
    assert_eq!(
        added.persona_override.as_deref(),
        Some("\u{5be1}\u{8a00}\u{800c}\u{8b66}\u{89c9}")
    );
    assert_eq!(
        added.behavior_override.as_deref(),
        Some("\u{53ea}\u{4ea4}\u{4ed8}\u{5bc6}\u{4fe1}")
    );

    let duplicate = add_campaign_instance_to_store(
        &store,
        &campaign.id,
        None,
        Some("\u{6e21}\u{9e26}\u{4fe1}\u{4f7f}".into()),
        None,
        None,
    )
    .unwrap_err();
    assert!(
        duplicate
            .to_string()
            .contains("\u{540c}\u{540d}\u{89d2}\u{8272}")
    );

    let blank =
        add_campaign_instance_to_store(&store, &campaign.id, None, Some("   ".into()), None, None)
            .unwrap_err();
    assert!(
        blank
            .to_string()
            .contains("\u{89d2}\u{8272}\u{540d}\u{79f0}")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_card_summary_treats_fallback_as_not_extracted() {
    let character = make_test_character("Fallback Card");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.extraction_status = storyforge_domain::character::CharacterExtractionStatus::Fallback;
    card.extraction_message = Some(FALLBACK_EXTRACTION_MESSAGE.into());
    card.character_definitions
        .push(make_test_character_definition(
            &card.id,
            "fallback-def",
            "Fallback Hero",
        ));

    let stored = campaign_store::StoredCard {
        card,
        imported_at: "2026-07-07T00:00:00Z".into(),
    };
    let dto = CardSummaryDto::from(&stored);

    assert!(!dto.extracted);
    assert_eq!(dto.extraction_status, "fallback");
    assert_eq!(dto.definition_count, 1);
    assert_eq!(dto.character_count, 1);
    assert_eq!(
        dto.extraction_message.as_deref(),
        Some(FALLBACK_EXTRACTION_MESSAGE)
    );
}

#[test]
fn test_prepare_character_extraction_force_preserves_existing_card_id() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_extract_force_preserves_id_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let character = make_test_character("Force Rerun Card");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.id = Id::from_str("existing-card-id");
    card.extraction_status = storyforge_domain::character::CharacterExtractionStatus::Fallback;
    card.character_definitions
        .push(make_test_character_definition(
            &card.id,
            "old-fallback-def",
            "Old Fallback",
        ));
    store.save_card(card).unwrap();

    let decision = prepare_character_extraction_card(&store, &character, true).unwrap();

    match decision {
        CharacterExtractionDecision::Run(card) => {
            assert_eq!(card.id, Id::from_str("existing-card-id"));
            assert_eq!(
                card.extraction_status,
                storyforge_domain::character::CharacterExtractionStatus::Fallback
            );
        }
        CharacterExtractionDecision::ReturnExisting(_) => {
            panic!("force=true should rerun instead of returning existing card")
        }
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_prepare_character_extraction_refuses_force_when_campaign_exists() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_extract_force_campaign_guard_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let character = make_test_character("Existing Campaign Card");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.id = Id::from_str("guarded-card-id");
    card.extraction_status = storyforge_domain::character::CharacterExtractionStatus::Fallback;
    let stored = store.save_card(card).unwrap();
    let campaign =
        storyforge_domain::campaign::Campaign::new(stored.card.id.clone(), "existing run");
    store.save_campaign(campaign).unwrap();

    let err = prepare_character_extraction_card(&store, &character, true).unwrap_err();

    match err {
        TauriCommandError::Validation { message } => {
            assert!(message.contains("\u{5df2}\u{6709}\u{6e38}\u{73a9}\u{6863}"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_force_rerun_commit_rejects_campaign_created_after_prepare() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_extract_force_commit_guard_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let character = make_test_character("Race Guard Card");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.id = Id::from_str("race-card-id");
    card.character_definitions
        .push(make_test_character_definition(
            &card.id,
            "old-race-def",
            "Old Race Def",
        ));
    store.save_card(card.clone()).unwrap();

    let decision = prepare_character_extraction_card(&store, &character, true).unwrap();
    let mut rerun_card = match decision {
        CharacterExtractionDecision::Run(card) => card,
        CharacterExtractionDecision::ReturnExisting(_) => {
            panic!("force=true should rerun instead of returning existing card")
        }
    };
    rerun_card.character_definitions.clear();
    rerun_card
        .character_definitions
        .push(make_test_character_definition(
            &rerun_card.id,
            "new-race-def",
            "New Race Def",
        ));

    let campaign =
        storyforge_domain::campaign::Campaign::new(card.id.clone(), "created during rerun");
    let (_, campaign, instance_count) = store.create_campaign_with_instances(campaign).unwrap();
    assert_eq!(instance_count, 1);

    let err = save_character_card_force_rerun_to_store(&store, rerun_card).unwrap_err();
    match err {
        TauriCommandError::Validation { message } => {
            assert!(message.contains("\u{5df2}\u{6709}\u{6e38}\u{73a9}\u{6863}"));
        }
        other => panic!("expected validation error, got {other:?}"),
    }

    let stored_after = store.get_card(&card.id).unwrap();
    assert!(
        stored_after
            .card
            .character_definitions
            .iter()
            .any(|def| def.id == Id::from_str("old-race-def"))
    );
    assert!(
        !stored_after
            .card
            .character_definitions
            .iter()
            .any(|def| def.id == Id::from_str("new-race-def"))
    );

    let instances = store.list_instances(&campaign.id);
    assert_eq!(instances.len(), 1);
    let definition_id = instances[0].definition_id.as_ref().unwrap();
    assert!(
        stored_after
            .card
            .character_definitions
            .iter()
            .any(|def| &def.id == definition_id)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_save_character_card_async_persists_and_replaces_source() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_card_async_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store: &'static campaign_store::CampaignStore =
        Box::leak(Box::new(campaign_store::CampaignStore::new(&dir)));

    let character = make_test_character("Async Card Source");
    let mut first = storyforge_domain::character::CharacterCard::from_character(&character);
    first
        .character_definitions
        .push(make_test_character_definition(
            &first.id,
            "def-first",
            "First",
        ));
    let mut second = storyforge_domain::character::CharacterCard::from_character(&character);
    second.name = "Async Card Updated".into();
    second
        .character_definitions
        .push(make_test_character_definition(
            &second.id,
            "def-second",
            "Second",
        ));

    save_character_card_async(store, first).await.unwrap();
    let stored = save_character_card_async(store, second).await.unwrap();

    assert_eq!(stored.card.name, "Async Card Updated");
    assert_eq!(stored.card.character_definitions.len(), 1);
    assert_eq!(store.list_cards().len(), 1);
    assert_eq!(
        store.get_card_by_source(&character.id).unwrap().card.name,
        "Async Card Updated"
    );

    let reloaded = campaign_store::CampaignStore::new(&dir);
    let reloaded_card = reloaded.get_card_by_source(&character.id).unwrap();
    assert_eq!(reloaded_card.card.name, "Async Card Updated");
    assert_eq!(reloaded_card.card.character_definitions[0].name, "Second");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_create_campaign_in_store_cleans_conversation_on_store_failure() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_create_campaign_cleanup_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let character = make_test_character("Cleanup Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.character_definitions
        .push(make_test_character_definition(
            &card.id,
            "cleanup-def",
            "Cleanup",
        ));
    let card_id = card.id.as_str().to_string();
    store.save_card(card).unwrap();
    std::fs::create_dir_all(dir.join("instances.json")).unwrap();

    let err = create_campaign_in_store(
        &store,
        &conv_store,
        card_id,
        "cleanup blocked".into(),
        Some("opening line".into()),
    )
    .unwrap_err();

    assert!(
        err.to_string().contains("instances.json"),
        "expected instances persist failure, got {err}"
    );
    assert!(store.list_campaigns().is_empty());
    assert!(store.list_all_instances().is_empty());
    assert!(conv_store.list().is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_create_campaign_in_store_initializes_card_global_variables() {
    use storyforge_domain::variables::{VariableField, VariableType};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_create_campaign_globals_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let character = make_test_character("Global Variable Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.campaign_variable_schema = vec![VariableField {
        key: "faction_tension".into(),
        label: "\u{9635}\u{8425}\u{7d27}\u{5f20}\u{5ea6}".into(),
        value_type: VariableType::Int,
        default: serde_json::json!(12),
        description: Some("\u{6574}\u{5c40}\u{5171}\u{4eab}\u{7684}\u{9635}\u{8425}\u{51b2}\u{7a81}\u{5f3a}\u{5ea6}".into()),
        group: Some("\u{4e16}\u{754c}\u{72b6}\u{6001}".into()),
    }];
    card.character_definitions
        .push(make_test_character_definition(
            &card.id,
            "global-def",
            "Hero",
        ));
    let card_id = card.id.as_str().to_string();
    store.save_card(card).unwrap();

    let dto =
        create_campaign_in_store(&store, &conv_store, card_id, "globals".into(), None).unwrap();
    let campaign = store.get_campaign(&Id::from_str(&dto.id)).unwrap();
    let field = campaign
        .variable_schema
        .iter()
        .find(|field| field.key == "faction_tension")
        .expect("card global schema should be copied into campaign");
    assert_eq!(field.label, "\u{9635}\u{8425}\u{7d27}\u{5f20}\u{5ea6}");
    assert_eq!(
        field.description.as_deref(),
        Some(
            "\u{6574}\u{5c40}\u{5171}\u{4eab}\u{7684}\u{9635}\u{8425}\u{51b2}\u{7a81}\u{5f3a}\u{5ea6}"
        )
    );
    assert_eq!(
        campaign
            .variables
            .iter()
            .find(|value| value.key == "faction_tension")
            .map(|value| &value.value),
        Some(&serde_json::json!(12))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_sync_campaign_variable_schema_adds_missing_without_overwriting_current_value() {
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::variables::{VariableField, VariableType};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_sync_campaign_globals_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let character = make_test_character("Sync Global Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    let card_id = card.id.clone();
    store.save_card(card.clone()).unwrap();
    let campaign = Campaign::new(card_id, "legacy");
    let campaign_id = campaign.id.clone();
    store.save_campaign(campaign).unwrap();

    card.campaign_variable_schema = vec![VariableField {
        key: "danger_level".into(),
        label: "\u{5371}\u{9669}\u{7b49}\u{7ea7}".into(),
        value_type: VariableType::Int,
        default: serde_json::json!(2),
        description: Some("\u{6574}\u{5c40}\u{98ce}\u{9669}".into()),
        group: Some("\u{4e16}\u{754c}\u{72b6}\u{6001}".into()),
    }];
    store.save_card(card.clone()).unwrap();

    let first = sync_campaign_variable_schema_in_store(&store, &campaign_id).unwrap();
    assert_eq!(first.added, 1);
    let mut synced = store.get_campaign(&campaign_id).unwrap();
    assert_eq!(
        synced.get_variable("danger_level"),
        Some(&serde_json::json!(2))
    );

    synced.set_variable("danger_level", serde_json::json!(77), 4);
    store.update_campaign(synced).unwrap();
    card.campaign_variable_schema[0].default = serde_json::json!(99);
    card.campaign_variable_schema[0].description =
        Some("\u{66f4}\u{65b0}\u{540e}\u{7684}\u{8bf4}\u{660e}".into());
    store.save_card(card).unwrap();

    let second = sync_campaign_variable_schema_in_store(&store, &campaign_id).unwrap();
    assert_eq!(second.added, 0);
    let resynced = store.get_campaign(&campaign_id).unwrap();
    assert_eq!(
        resynced.get_variable("danger_level"),
        Some(&serde_json::json!(77)),
        "\u{540c}\u{6b65} schema \u{4e0d}\u{5f97}\u{8986}\u{76d6}\u{6d3b}\u{52a8}\u{4e2d}\u{5df2}\u{7ecf}\u{53d8}\u{5316}\u{7684}\u{5f53}\u{524d}\u{503c}"
    );
    assert_eq!(
        resynced
            .variable_schema
            .iter()
            .find(|field| field.key == "danger_level")
            .and_then(|field| field.description.as_deref()),
        Some("\u{66f4}\u{65b0}\u{540e}\u{7684}\u{8bf4}\u{660e}")
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_apply_campaign_opening_rewrites_first_assistant_message() {
    use storyforge_domain::conversation::Role as ConvRole;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_apply_opening_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let character = make_test_character("Opening Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.character_definitions
        .push(make_test_character_definition(&card.id, "op-def", "Hero"));
    let card_id = card.id.as_str().to_string();
    store.save_card(card).unwrap();

    let dto =
        create_campaign_in_store(&store, &conv_store, card_id, "opening".into(), None).unwrap();
    let campaign_id = Id::from_str(&dto.id);
    let conv_id = Id::from_str(dto.conversation_id.as_deref().expect("bound conv"));
    // \u{6d4b}\u{8bd5}\u{73af}\u{5883}\u{65e0}\u{5168}\u{5c40} CharacterStore\u{ff1a}\u{624b}\u{52a8}\u{64ad}\u{79cd}\u{5f00}\u{573a}\u{767d}\u{ff08}\u{751f}\u{4ea7}\u{8def}\u{5f84}\u{7531} create_campaign \u{5199}\u{5165}\u{ff09}
    conv_store
        .append_final_message(&conv_id, ConvRole::Assistant, "scene-1".into())
        .unwrap();

    apply_campaign_opening_in_store(&store, &conv_store, &campaign_id, "scene-2".into()).unwrap();

    let conv = conv_store.get(&conv_id).unwrap();
    assert_eq!(conv.nodes.len(), 1);
    assert_eq!(conv.nodes[0].active_content(), "scene-2");

    // \u{91cd}\u{8f7d}\u{540e}\u{4ecd}\u{662f}\u{6539}\u{5199}\u{503c}\u{ff1a}\u{786e}\u{8ba4}\u{771f}\u{6b63}\u{843d}\u{76d8}\u{800c}\u{975e}\u{4ec5}\u{7f13}\u{5b58}
    let reloaded = ConversationStore::new(dir.join("conversations"));
    assert_eq!(
        reloaded.get(&conv_id).unwrap().nodes[0].active_content(),
        "scene-2"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_apply_campaign_opening_rejects_after_conversation_grows() {
    use storyforge_domain::conversation::Role as ConvRole;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_apply_opening_grown_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let character = make_test_character("Opening Grown Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.character_definitions
        .push(make_test_character_definition(&card.id, "og-def", "Hero"));
    let card_id = card.id.as_str().to_string();
    store.save_card(card).unwrap();

    let dto = create_campaign_in_store(&store, &conv_store, card_id, "grown".into(), None).unwrap();
    let campaign_id = Id::from_str(&dto.id);
    let conv_id = Id::from_str(dto.conversation_id.as_deref().expect("bound conv"));
    conv_store
        .append_final_message(&conv_id, ConvRole::Assistant, "scene-1".into())
        .unwrap();
    conv_store
        .append_user_message(&conv_id, "next turn".into())
        .unwrap();

    let err = apply_campaign_opening_in_store(&store, &conv_store, &campaign_id, "scene-2".into())
        .unwrap_err();
    assert!(
        err.to_string().contains("\u{540e}\u{7eed}\u{6d88}\u{606f}"),
        "expected opening-stale validation error, got {err}"
    );
    // \u{539f}\u{5f00}\u{573a}\u{672a}\u{88ab}\u{6539}\u{5199}
    let conv = conv_store.get(&conv_id).unwrap();
    assert_eq!(conv.nodes[0].active_content(), "scene-1");

    // \u{7a7a}\u{5185}\u{5bb9}\u{4e0e}\u{672a}\u{77e5} campaign \u{4e5f}\u{62d2}\u{7edd}
    let err = apply_campaign_opening_in_store(&store, &conv_store, &campaign_id, "  ".into())
        .unwrap_err();
    assert!(
        err.to_string().contains("\u{4e0d}\u{80fd}\u{4e3a}\u{7a7a}"),
        "got {err}"
    );
    let missing = Id::new();
    let err = apply_campaign_opening_in_store(&store, &conv_store, &missing, "scene-2".into())
        .unwrap_err();
    assert!(
        err.to_string().contains("\u{627e}\u{4e0d}\u{5230}"),
        "got {err}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_delete_campaign_playthrough_cascades_conversation_and_summaries() {
    use storyforge_domain::agent::RoundSummary;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_delete_playthrough_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let character = make_test_character("Delete Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.character_definitions
        .push(make_test_character_definition(&card.id, "del-def", "Hero"));
    let card_id = card.id.as_str().to_string();
    store.save_card(card).unwrap();

    let dto = create_campaign_in_store(
        &store,
        &conv_store,
        card_id,
        "playthrough-to-delete".into(),
        Some("opening".into()),
    )
    .unwrap();
    let campaign_id = Id::from_str(&dto.id);
    let conversation_id = Id::from_str(dto.conversation_id.as_deref().expect("bound conv"));

    store
        .add_summary(RoundSummary::new(
            campaign_id.clone(),
            conversation_id.clone(),
            1,
            "round one summary".into(),
        ))
        .unwrap();
    assert_eq!(store.list_summaries(&campaign_id).len(), 1);
    assert!(conv_store.get(&conversation_id).is_some());
    assert!(!store.list_instances(&campaign_id).is_empty());

    let state = AppState::new_for_test();
    *state
        .active_campaign
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = Some(campaign_id.clone());

    delete_campaign_playthrough_in_store(&store, &conv_store, &state, &campaign_id).unwrap();

    assert!(store.get_campaign(&campaign_id).is_none());
    assert!(store.list_instances(&campaign_id).is_empty());
    assert!(store.list_summaries(&campaign_id).is_empty());
    assert!(conv_store.get(&conversation_id).is_none());
    assert!(
        state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none(),
        "active campaign pointer must clear"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_delete_conversation_path_resolves_bound_campaign_and_cascades() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_delete_conv_cascades_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let character = make_test_character("C2 Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    card.character_definitions
        .push(make_test_character_definition(&card.id, "c2-def", "N"));
    let card_id = card.id.as_str().to_string();
    store.save_card(card).unwrap();

    let dto = create_campaign_in_store(&store, &conv_store, card_id, "c2".into(), None).unwrap();
    let campaign_id = Id::from_str(&dto.id);
    let conversation_id = Id::from_str(dto.conversation_id.as_deref().unwrap());
    let state = AppState::new_for_test();

    // \u{4e0e} delete_conversation \u{547d}\u{4ee4}\u{4e00}\u{81f4}\u{ff1a}\u{4ece}\u{4f1a}\u{8bdd}\u{53cd}\u{67e5} campaign \u{540e}\u{6574}\u{5c40}\u{5220}
    let camp_from_conv = conv_store
        .get(&conversation_id)
        .and_then(|c| c.campaign_id)
        .expect("conversation should bind campaign");
    assert_eq!(camp_from_conv, campaign_id);
    delete_campaign_playthrough_in_store(&store, &conv_store, &state, &camp_from_conv).unwrap();
    assert!(store.get_campaign(&campaign_id).is_none());
    assert!(conv_store.get(&conversation_id).is_none());

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_prepare_start_conversation_async_persists_legacy_and_existing_paths() {
    let state = Arc::new(AppState::new_for_test());
    let campaign_dir = std::env::temp_dir().join(format!(
        "storyforge_test_start_conversation_campaign_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&campaign_dir).unwrap();
    let campaign_store: &'static campaign_store::CampaignStore =
        Box::leak(Box::new(campaign_store::CampaignStore::new(&campaign_dir)));
    let mut legacy_character = make_test_character("Legacy Starter");
    legacy_character.first_mes = "opening line".into();
    let legacy_character = Arc::new(legacy_character);

    let created = prepare_start_conversation_async(
        state.clone(),
        campaign_store,
        None,
        Some("char-legacy".into()),
        Some(legacy_character),
        None,
        "write first scene".into(),
    )
    .await
    .unwrap();

    assert_eq!(created.regex_character_id.as_deref(), Some("char-legacy"));
    let created_conv = state.conv_store.get(&created.conversation_id).unwrap();
    assert_eq!(created_conv.character_id.as_deref(), Some("char-legacy"));
    assert_eq!(created_conv.nodes.len(), 2);
    assert_eq!(
        created_conv.nodes[0].active().unwrap().role,
        ConversationRole::Assistant
    );
    assert_eq!(
        created_conv.nodes[0].active().unwrap().status,
        VariantStatus::Final
    );
    assert_eq!(created_conv.nodes[0].active_content(), "opening line");
    assert_eq!(
        created_conv.nodes[1].active().unwrap().role,
        ConversationRole::User
    );
    assert_eq!(created_conv.nodes[1].active_content(), "write first scene");

    let reloaded = ConversationStore::new(state.data_dir.join("conversations"));
    let persisted = reloaded.get(&created.conversation_id).unwrap();
    assert_eq!(persisted.nodes.len(), 2);
    assert_eq!(persisted.nodes[0].active_content(), "opening line");
    assert_eq!(persisted.nodes[1].active_content(), "write first scene");

    let existing = state.conv_store.create(Some("char-existing".into()), None);
    let mut ignored_character = make_test_character("Ignored Starter");
    ignored_character.first_mes = "ignored opening".into();
    let reused = prepare_start_conversation_async(
        state.clone(),
        campaign_store,
        Some(existing.id.as_str().to_string()),
        None,
        Some(Arc::new(ignored_character)),
        Some("ignored opening".into()),
        "continue scene".into(),
    )
    .await
    .unwrap();

    assert_eq!(reused.conversation_id, existing.id);
    assert_eq!(reused.regex_character_id.as_deref(), Some("char-existing"));
    let reused_conv = state.conv_store.get(&existing.id).unwrap();
    assert_eq!(reused_conv.nodes.len(), 1);
    assert_eq!(
        reused_conv.nodes[0].active().unwrap().role,
        ConversationRole::User
    );
    assert_eq!(reused_conv.nodes[0].active_content(), "continue scene");

    let mut active_campaign = storyforge_domain::campaign::Campaign::new(
        Id::from_str(format!("card-{}", uuid::Uuid::new_v4())),
        "Active Campaign",
    );
    let campaign_conv = state.conv_store.create(
        Some("char-campaign".into()),
        Some(active_campaign.id.clone()),
    );
    active_campaign.conversation_id = Some(campaign_conv.id.clone());
    campaign_store
        .save_campaign(active_campaign.clone())
        .unwrap();
    {
        let mut active = state
            .active_campaign
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        *active = Some(active_campaign.id.clone());
    }

    let requested_conv = state.conv_store.create(Some("char-requested".into()), None);
    let mut ignored_campaign_character = make_test_character("Campaign Ignored");
    ignored_campaign_character.first_mes = "campaign ignored opening".into();
    let campaign_target = prepare_start_conversation_async(
        state.clone(),
        campaign_store,
        Some(requested_conv.id.as_str().to_string()),
        None,
        Some(Arc::new(ignored_campaign_character)),
        Some("campaign ignored opening".into()),
        "campaign intent".into(),
    )
    .await
    .unwrap();

    assert_eq!(campaign_target.conversation_id, campaign_conv.id);
    assert_eq!(
        campaign_target.regex_character_id.as_deref(),
        Some("char-campaign")
    );
    let campaign_conv = state.conv_store.get(&campaign_conv.id).unwrap();
    assert_eq!(campaign_conv.nodes.len(), 1);
    assert_eq!(
        campaign_conv.nodes[0].active().unwrap().role,
        ConversationRole::User
    );
    assert_eq!(campaign_conv.nodes[0].active_content(), "campaign intent");
    let requested_conv = state.conv_store.get(&requested_conv.id).unwrap();
    assert!(requested_conv.nodes.is_empty());

    let _ = std::fs::remove_dir_all(&campaign_dir);
}

#[test]
fn test_fork_campaign_in_store_records_source_and_clones_snapshot() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::variables::default_character_variables;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_campaign_fork_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(dir.join("conversations")).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let conv_store = ConversationStore::new(dir.join("conversations"));

    let mut card = CharacterCard {
        id: Id::from_str("card-1"),
        name: "Test Card".into(),
        source_character_id: Id::from_str("source-card-1"),
        character_definitions: vec![],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::Value::Null,
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    let definition = CharacterDefinition {
        id: Id::from_str("def-lin"),
        card_id: card.id.clone(),
        name: "Lin".into(),
        persona_prompt: "calm surgeon".into(),
        behavior_rules: "save first".into(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: default_character_variables(),
    };
    card.character_definitions.push(definition.clone());
    store.save_card(card).unwrap();

    let mut source_campaign = Campaign::new(Id::from_str("card-1"), "source run");
    source_campaign.set_variable("story_clock", serde_json::json!("Day 7"), 3);
    let source_conversation =
        conv_store.create(Some("card-1".into()), Some(source_campaign.id.clone()));
    let user_node = conv_store
        .append_user_message(&source_conversation.id, "first user".into())
        .unwrap();
    let fork_node = conv_store
        .append_ai_draft(&source_conversation.id, "branch point".into(), None)
        .unwrap();
    let _later = conv_store
        .append_user_message(&source_conversation.id, "later user".into())
        .unwrap();
    source_campaign.conversation_id = Some(source_conversation.id.clone());
    store.save_campaign(source_campaign.clone()).unwrap();

    let mut source_instance =
        CharacterInstance::from_definition(source_campaign.id.clone(), &definition);
    source_instance.id = Id::from_str("source-inst");
    source_instance.set_variable("hp", serde_json::json!(42), 9);
    store.add_instance(source_instance.clone()).unwrap();

    let dto = fork_campaign_in_store(
        &store,
        &conv_store,
        source_campaign.id.clone(),
        fork_node.clone(),
        "forked run".into(),
    )
    .unwrap();

    let fork_campaign = store.get_campaign(&Id::from_str(&dto.id)).unwrap();
    assert_eq!(
        fork_campaign.fork_from,
        Some((source_campaign.id.clone(), fork_node.clone()))
    );
    assert_eq!(fork_campaign.card_id, source_campaign.card_id);
    assert_eq!(fork_campaign.current_story_clock(), "Day 7");
    assert_ne!(
        fork_campaign.conversation_id,
        source_campaign.conversation_id
    );

    let fork_conversation = conv_store
        .get(fork_campaign.conversation_id.as_ref().unwrap())
        .unwrap();
    assert_eq!(
        fork_conversation.campaign_id,
        Some(fork_campaign.id.clone())
    );
    assert_eq!(fork_conversation.nodes.len(), 2);
    assert_eq!(fork_conversation.nodes[0].id, user_node);
    assert_eq!(fork_conversation.nodes[1].id, fork_node);

    let source_instances = store.list_instances(&source_campaign.id);
    assert_eq!(source_instances.len(), 1);
    assert_eq!(source_instances[0].id, Id::from_str("source-inst"));

    let fork_instances = store.list_instances(&fork_campaign.id);
    assert_eq!(fork_instances.len(), 1);
    assert_ne!(fork_instances[0].id, source_instance.id);
    assert_eq!(fork_instances[0].name, "Lin");
    assert_eq!(
        fork_instances[0].get_variable("hp"),
        Some(&serde_json::json!(42))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn campaign_variable_input_validation_accepts_unicode_keys_and_bounds_user_text() {
    assert!(
        validate_campaign_variable_input(
            "\u{4e16}\u{754c}.\u{9635}\u{8425}_\u{7d27}\u{5f20}\u{5ea6}-1",
            "\u{9635}\u{8425}\u{7d27}\u{5f20}\u{5ea6}",
            Some("\u{6574}\u{5c40}\u{5171}\u{4eab}"),
            &serde_json::json!(12),
        )
        .is_ok()
    );
    assert!(
        validate_campaign_variable_input(
            "bad key",
            "\u{574f}\u{952e}",
            None,
            &serde_json::Value::Null,
        )
        .is_err()
    );
    assert!(
        validate_campaign_variable_input(
            "world.__internal",
            "\u{5185}\u{90e8}\u{952e}",
            None,
            &serde_json::Value::Null,
        )
        .is_err()
    );
    assert!(
        validate_campaign_variable_input(
            "valid",
            &"\u{540d}".repeat(81),
            None,
            &serde_json::Value::Null,
        )
        .is_err()
    );
    assert!(
        validate_campaign_variable_input(
            "valid",
            "\u{540d}\u{79f0}",
            Some(&"\u{8bf4}".repeat(501)),
            &serde_json::Value::Null,
        )
        .is_err()
    );
}
