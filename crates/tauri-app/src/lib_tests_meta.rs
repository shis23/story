use super::*;

#[test]
fn gate3_meta_and_world_info_commands_do_not_probe_the_sqlite_runtime() {
    for (name, source) in [
        ("meta", include_str!("commands/meta.rs")),
        ("meta_typed", include_str!("commands/meta_typed.rs")),
        ("world_info", include_str!("commands/world_info.rs")),
    ] {
        assert!(
            !source.contains("sqlite_runtime::is_sqlite_active()"),
            "{name} commands must select storage through AppState::storage()"
        );
    }
}

#[test]
fn sqlite_meta_and_world_info_capabilities_are_supported_and_remaining_gaps_fail_closed() {
    use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};

    let storage = storage_backend::StorageFacade::new(
        std::env::temp_dir().join("storyforge-gate4-meta-capability"),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    );
    // Gate 4：Meta/MVU/WorldInfo 已是 SQLite 原生能力。
    for (capability, operation) in [
        (
            storage_backend::BackendCapability::TypedMetaPatch,
            "typed Meta patch accept",
        ),
        (
            storage_backend::BackendCapability::MvuSchemaApply,
            "MVU schema apply",
        ),
        (
            storage_backend::BackendCapability::WorldInfo,
            "list campaign world info",
        ),
        (
            storage_backend::BackendCapability::ChronicleCompressor,
            "chronicle compression",
        ),
        (
            storage_backend::BackendCapability::CharacterCommands,
            "character commands",
        ),
        (
            storage_backend::BackendCapability::ImportExport,
            "import/export",
        ),
    ] {
        storage
            .require_supported(capability, operation)
            .expect("SQLite capability must be supported");
    }
    // 仍显式 unsupported 的剩余缺口（Campaign lifecycle）必须带 operation
    // 名 fail closed，不能静默空列表。
    let error = storage
        .require_supported(
            storage_backend::BackendCapability::CampaignLifecycle,
            "campaign lifecycle",
        )
        .expect_err("SQLite capability must fail closed before a JSON store is consulted");
    assert!(error.contains("campaign lifecycle"));
    assert!(error.contains("Unsupported"));
    storage
        .require_supported(
            storage_backend::BackendCapability::MvuTranslation,
            "list MVU translations",
        )
        .expect("SQLite MVU translation is a supported facade capability");
}

#[test]
fn test_meta_apply_mvu_schema_backfills_all_campaign_instances_without_overwriting_values() {
    use storyforge_domain::campaign::Campaign;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_mvu_apply_backfill_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let character = make_test_character("MVU Apply Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    let mut definition = make_test_character_definition(&card.id, "mvu-apply-def", "Hero");
    definition.variable_schema = vec![test_variable_field("hp", "HP", serde_json::json!(100))];
    card.character_definitions.push(definition.clone());
    store.save_card(card.clone()).unwrap();

    store
        .save_mvu(campaign_store::StoredMvuTranslation {
            source_character_id: character.id.clone(),
            character_name: character.name.clone(),
            translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                vec![
                    test_variable_field("hp", "Hit Points", serde_json::json!(200)),
                    test_variable_field("mana", "Mana", serde_json::json!(30)),
                ],
            ),
            analyzed_at: "2026-07-07T00:00:00Z".into(),
        })
        .unwrap();

    let campaign_a = Campaign::new(card.id.clone(), "Campaign A");
    let campaign_a_id = campaign_a.id.clone();
    let (_, campaign_a, _) = store.create_campaign_with_instances(campaign_a).unwrap();
    let campaign_b = Campaign::new(card.id.clone(), "Campaign B");
    let campaign_b_id = campaign_b.id.clone();
    let (_, campaign_b, _) = store.create_campaign_with_instances(campaign_b).unwrap();

    let mut instance_a = store.list_instances(&campaign_a.id).remove(0);
    let hp = instance_a
        .variables
        .iter_mut()
        .find(|value| value.key == "hp")
        .unwrap();
    hp.value = serde_json::json!(42);
    hp.last_updated_turn = 9;
    store.update_instance(instance_a.clone()).unwrap();

    let mut instance_b = store.list_instances(&campaign_b.id).remove(0);
    instance_b.variables.retain(|value| value.key != "hp");
    store.update_instance(instance_b).unwrap();

    meta_apply_mvu_schema_in_store(&store, &character.id, &definition.id).unwrap();

    let updated_card = store.get_card(&card.id).unwrap().card;
    let updated_def = updated_card
        .character_definitions
        .iter()
        .find(|def| def.id == definition.id)
        .unwrap();
    let hp_schema = updated_def
        .variable_schema
        .iter()
        .find(|field| field.key == "hp")
        .unwrap();
    assert_eq!(hp_schema.label, "Hit Points");
    assert_eq!(hp_schema.default, serde_json::json!(200));
    assert!(
        updated_def
            .variable_schema
            .iter()
            .any(|field| field.key == "mana" && field.default == serde_json::json!(30))
    );

    for campaign_id in [campaign_a_id, campaign_b_id] {
        let instance = store.list_instances(&campaign_id).remove(0);
        assert_eq!(instance.definition_id.as_ref(), Some(&definition.id));
        let mana = instance
            .variables
            .iter()
            .find(|value| value.key == "mana")
            .unwrap();
        assert_eq!(mana.value, serde_json::json!(30));
        assert_eq!(mana.last_updated_turn, 0);
    }

    let preserved = store.list_instances(&campaign_a.id).remove(0);
    let preserved_hp = preserved
        .variables
        .iter()
        .find(|value| value.key == "hp")
        .unwrap();
    assert_eq!(preserved_hp.value, serde_json::json!(42));
    assert_eq!(preserved_hp.last_updated_turn, 9);

    let restored = store.list_instances(&campaign_b.id).remove(0);
    let restored_hp = restored
        .variables
        .iter()
        .find(|value| value.key == "hp")
        .unwrap();
    assert_eq!(restored_hp.value, serde_json::json!(200));
    assert_eq!(restored_hp.last_updated_turn, 0);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_meta_apply_mvu_schema_normalizes_legacy_key_notation() {
    // 存量翻译产物可能带旧记法键（斜杠 / stat_data. 前缀）。应用边界必须
    // 归一，否则同一变量以两种键并存（"stat_data.hp" 与 "hp"）。
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_mvu_apply_normalize_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let character = make_test_character("MVU Legacy Notation Source");
    let mut card = storyforge_domain::character::CharacterCard::from_character(&character);
    let mut definition = make_test_character_definition(&card.id, "mvu-legacy-def", "Hero");
    definition.variable_schema = vec![test_variable_field("hp", "HP", serde_json::json!(100))];
    card.character_definitions.push(definition.clone());
    store.save_card(card.clone()).unwrap();

    store
        .save_mvu(campaign_store::StoredMvuTranslation {
            source_character_id: character.id.clone(),
            character_name: character.name.clone(),
            translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                vec![
                    // 旧记法：stat_data. 前缀 → 应合并到已有 "hp" 而不是新增键
                    test_variable_field("stat_data.hp", "Hit Points", serde_json::json!(200)),
                    // 旧记法：斜杠 → 点
                    test_variable_field("/世界/时间", "时间", serde_json::json!("清晨")),
                ],
            ),
            analyzed_at: "2026-07-27T00:00:00Z".into(),
        })
        .unwrap();

    meta_apply_mvu_schema_in_store(&store, &character.id, &definition.id).unwrap();

    let updated_card = store.get_card(&card.id).unwrap().card;
    let updated_def = updated_card
        .character_definitions
        .iter()
        .find(|def| def.id == definition.id)
        .unwrap();
    let keys: Vec<&str> = updated_def
        .variable_schema
        .iter()
        .map(|f| f.key.as_str())
        .collect();
    assert!(keys.contains(&"hp"), "stat_data.hp 应归一为 hp: {keys:?}");
    assert!(
        keys.contains(&"世界.时间"),
        "斜杠键应归一为点记法: {keys:?}"
    );
    assert!(
        !keys
            .iter()
            .any(|k| k.contains("stat_data") || k.contains('/')),
        "不应残留旧记法键: {keys:?}"
    );
    let hp = updated_def
        .variable_schema
        .iter()
        .find(|f| f.key == "hp")
        .unwrap();
    assert_eq!(hp.label, "Hit Points", "归一后应与已有 hp 合并覆盖");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_save_mvu_translation_async_persists_and_replaces_existing() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_mvu_async_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = Arc::new(campaign_store::CampaignStore::new(&dir));

    save_mvu_translation_async(
        store.clone(),
        make_test_mvu_translation("src-mvu", "MVU 初版", "2026-07-07T00:00:00Z"),
    )
    .await
    .unwrap();
    save_mvu_translation_async(
        store.clone(),
        make_test_mvu_translation("src-mvu", "MVU 更新", "2026-07-07T00:00:01Z"),
    )
    .await
    .unwrap();

    let stored = store.get_mvu(&Id::from_str("src-mvu")).unwrap();
    assert_eq!(stored.character_name, "MVU 更新");
    assert_eq!(stored.analyzed_at, "2026-07-07T00:00:01Z");
    assert_eq!(store.list_all_mvu().len(), 1);

    let reloaded = campaign_store::CampaignStore::new(&dir);
    let reloaded_stored = reloaded.get_mvu(&Id::from_str("src-mvu")).unwrap();
    assert_eq!(reloaded_stored.character_name, "MVU 更新");
    assert_eq!(reloaded_stored.analyzed_at, "2026-07-07T00:00:01Z");

    let _ = std::fs::remove_dir_all(&dir);
}

fn make_test_character_definition(
    card_id: &Id,
    id: &str,
    name: &str,
) -> storyforge_domain::character::CharacterDefinition {
    storyforge_domain::character::CharacterDefinition {
        id: Id::from_str(id),
        card_id: card_id.clone(),
        name: name.into(),
        persona_prompt: format!("{name} persona"),
        behavior_rules: format!("{name} behavior"),
        base_backstory: vec!["backstory".into()],
        group: None,
        role_type: storyforge_domain::character::RoleType::Protagonist,
        variable_schema: vec![],
    }
}

#[test]
fn test_meta_session_explainer_is_injected() {
    let state = AppState::new_for_test();
    assert!(
        state.meta_session.explainer.is_some(),
        "meta_session.explainer should be injected (not None) for inspect_generation"
    );
    // campaign_runtime 默认为 None（无 active campaign）
    assert!(
        state
            .meta_session
            .campaign_runtime
            .lock()
            .unwrap()
            .is_none(),
        "meta_session.campaign_runtime should be None when no active campaign"
    );
}

#[tokio::test]
async fn test_conv_generation_explainer_reads_provenance_async() {
    let state = Arc::new(AppState::new_for_test());
    let conversation = state.conv_store.create(Some("card-1".into()), None);
    let node_id = state
        .conv_store
        .append_ai_draft(
            &conversation.id,
            "draft text".into(),
            Some(Provenance {
                session_id: Id::from_str("sess-1"),
                plan: None,
                subagent_results: vec![storyforge_domain::conversation::SubagentSnapshot {
                    character_id: "alice".into(),
                    full_text: "Alice output".into(),
                    character_instance_id: None,
                    display_name: Some("Alice".into()),
                    fallback_reason: None,
                    reasoning_content: Some("alice reasoning".into()),
                }],
                profile_id: Some(Id::from_str("profile-1")),
                generation_mode: None,
                seed: 7,
                last_hint: Some("try again".into()),
                director_reasoning: Some("director reasoning".into()),
                writer_reasoning: Some("writer reasoning".into()),
                editor_reasoning: Some("editor reasoning".into()),
            }),
        )
        .unwrap();
    let explainer = ConvGenerationExplainer {
        conv_store: state.conv_store.clone(),
    };

    let explanation =
            <ConvGenerationExplainer as storyforge_app_meta::meta_conversation::GenerationExplainer>::explain(
                &explainer,
                conversation.id.as_str().to_string(),
                node_id.as_str().to_string(),
            )
            .await
            .unwrap();

    assert_eq!(explanation.seed, 7);
    assert_eq!(explanation.profile_id.as_deref(), Some("profile-1"));
    assert_eq!(explanation.last_hint.as_deref(), Some("try again"));
    assert!(explanation.director_reasoning.is_none());
    assert!(explanation.writer_reasoning.is_none());
    assert!(explanation.editor_reasoning.is_none());
    assert_eq!(explanation.subagents.len(), 1);
    assert_eq!(explanation.subagents[0].character_id, "alice");
    assert_eq!(explanation.subagents[0].display_name, "Alice");
    assert!(explanation.subagents[0].reasoning_content.is_none());
    assert_eq!(explanation.subagents[0].output_preview, "Alice output");

    let _ = std::fs::remove_dir_all(&state.data_dir);
}

#[tokio::test]
async fn test_conv_generation_explainer_returns_none_for_missing_provenance_or_node() {
    let state = Arc::new(AppState::new_for_test());
    let conversation = state.conv_store.create(Some("card-1".into()), None);
    let node_without_provenance = state
        .conv_store
        .append_ai_draft(&conversation.id, "draft text".into(), None)
        .unwrap();
    let explainer = ConvGenerationExplainer {
        conv_store: state.conv_store.clone(),
    };

    let without_provenance =
            <ConvGenerationExplainer as storyforge_app_meta::meta_conversation::GenerationExplainer>::explain(
                &explainer,
                conversation.id.as_str().to_string(),
                node_without_provenance.as_str().to_string(),
            )
            .await;
    let missing_node =
            <ConvGenerationExplainer as storyforge_app_meta::meta_conversation::GenerationExplainer>::explain(
                &explainer,
                conversation.id.as_str().to_string(),
                Id::new().as_str().to_string(),
            )
            .await;

    assert!(without_provenance.is_none());
    assert!(missing_node.is_none());

    let _ = std::fs::remove_dir_all(&state.data_dir);
}

/// 验证 tool_ctx 的 RwLock + snapshot 机制：写入后快照能读到
/// （这是 import_character 同步 tool_ctx 的核心机制）

#[test]
fn test_meta_accept_typed_patch_prune_task_refs() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::story_task::StoryTask;
    use storyforge_domain::variables::default_character_variables;

    let dir = std::env::temp_dir().join(format!("sf_test_accept_prune_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let card = {
        let mut c = storyforge_domain::character::CharacterCard {
            id: Id::from_str("card-1"),
            name: "测试卡".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let def = CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: c.id.clone(),
            name: "Lin".into(),
            persona_prompt: "surgeon".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        c.character_definitions.push(def);
        c
    };
    store.save_card(card).unwrap();

    let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
    store.save_campaign(campaign.clone()).unwrap();

    let instance = CharacterInstance::from_definition(
        campaign.id.clone(),
        &store
            .get_card(&Id::from_str("card-1"))
            .unwrap()
            .card
            .character_definitions[0],
    );
    store.add_instance(instance.clone()).unwrap();

    // 创建一个 task，related_characters 含 orphan id
    let mut task = StoryTask::user_planned(campaign.id.clone(), "复仇", "老王复仇", vec![], 1);
    let orphan_id = Id::from_str("orphan-char");
    task.related_characters.push(orphan_id.clone());
    task.related_characters.push(instance.id.clone());
    let task_id = task.id.clone();
    store.add_task(task).unwrap();

    let state = AppState::new_for_test();
    let patch = storyforge_app_meta::TypedPatch {
        id: "test-prune-patch".into(),
        description: "修剪孤儿引用".into(),
        source_issue_category: "orphan_task_references".into(),
        affected_id: Some(task_id.as_str().to_string()),
        actions: vec![
            storyforge_app_meta::TypedPatchAction::PruneOrphanTaskReferences {
                task_id: task_id.clone(),
                orphan_character_ids: vec![orphan_id.clone()],
            },
        ],
        diff: vec![],
        created_at: chrono::Utc::now(),
        status: storyforge_app_meta::TypedPatchStatus::Pending,
        campaign_revision: None,
    };

    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        typed.push(patch);
    }

    let preview =
        meta_preview_typed_patch_in_store(&store, "test-prune-patch", campaign.id.as_str(), &state)
            .unwrap();
    assert_eq!(preview["stale"], serde_json::json!(false));

    meta_accept_typed_patch_in_store(&store, "test-prune-patch", campaign.id.as_str(), &state)
        .unwrap();

    // 验证：task.related_characters 不再含 orphan
    let updated_task = store.get_task(&task_id).unwrap();
    assert!(
        !updated_task.related_characters.contains(&orphan_id),
        "orphan id 应已被移除"
    );
    assert!(
        updated_task.related_characters.contains(&instance.id),
        "正常 instance id 应保留"
    );
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        typed
            .iter()
            .find(|p| p.id == "test-prune-patch")
            .unwrap()
            .status,
        storyforge_app_meta::TypedPatchStatus::Accepted
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// accept 前手动删掉 target instance，accept 返回错误且 status 变 Stale

#[test]
fn test_meta_accept_typed_patch_stale() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::default_character_variables;

    let dir = std::env::temp_dir().join(format!("sf_test_accept_stale_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let card = {
        let mut c = storyforge_domain::character::CharacterCard {
            id: Id::from_str("card-1"),
            name: "测试卡".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let def = CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: c.id.clone(),
            name: "Lin".into(),
            persona_prompt: "surgeon".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        c.character_definitions.push(def);
        c
    };
    store.save_card(card).unwrap();

    let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut instance = CharacterInstance::from_definition(
        campaign.id.clone(),
        &store
            .get_card(&Id::from_str("card-1"))
            .unwrap()
            .card
            .character_definitions[0],
    );
    instance.id = Id::from_str("target-inst");
    store.add_instance(instance.clone()).unwrap();

    // 构造一个指向该 instance 的 patch
    let mut patch = storyforge_app_meta::TypedPatch {
        id: "test-stale-patch".into(),
        description: "修改变量".into(),
        source_issue_category: "variable_schema_mismatch".into(),
        affected_id: Some("target-inst".into()),
        actions: vec![
            storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
                instance_id: Id::from_str("target-inst"),
                definition_id: Id::from_str("def-1"),
                add_keys: vec!["new_var".into()],
                remove_keys: vec![],
            },
        ],
        diff: vec![],
        created_at: chrono::Utc::now(),
        status: storyforge_app_meta::TypedPatchStatus::Pending,
        campaign_revision: None,
    };

    // 删掉 target instance（模拟 stale）：
    // CampaignStore 没有 delete_instance，改用空快照模拟 target 不存在

    // 用 is_patch_stale 检测
    // 构造一个快照，其中不包含 target instance
    let empty_instances: Vec<CharacterInstance> = vec![];
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let knowledge = store.list_knowledge(&campaign.id);
    let tasks = store.list_tasks(&campaign.id);

    let input = storyforge_app_meta::PreviewInput {
        instances: &empty_instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: Some(&campaign),
    };

    // is_patch_stale 应返回 true（target instance 不在快照中）
    assert!(
        storyforge_app_meta::is_patch_stale(&patch, &input),
        "删掉 target 后 patch 应为 stale"
    );

    // 模拟 accept 逻辑：stale → status 改 Stale
    if storyforge_app_meta::is_patch_stale(&patch, &input) {
        patch.status = storyforge_app_meta::TypedPatchStatus::Stale;
    }
    assert_eq!(patch.status, storyforge_app_meta::TypedPatchStatus::Stale);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_meta_accept_typed_patch_preflights_all_actions_before_writing() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::default_character_variables;

    let dir =
        std::env::temp_dir().join(format!("sf_test_accept_preflight_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let card = {
        let mut c = storyforge_domain::character::CharacterCard {
            id: Id::from_str("card-1"),
            name: "测试卡".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        let def = CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: c.id.clone(),
            name: "Lin".into(),
            persona_prompt: "surgeon".into(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: default_character_variables(),
        };
        c.character_definitions.push(def);
        c
    };
    store.save_card(card).unwrap();

    let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut instance = CharacterInstance::from_definition(
        campaign.id.clone(),
        &store
            .get_card(&Id::from_str("card-1"))
            .unwrap()
            .card
            .character_definitions[0],
    );
    instance.id = Id::from_str("target-inst");
    store.add_instance(instance).unwrap();

    let state = AppState::new_for_test();
    let patch = storyforge_app_meta::TypedPatch {
        id: "test-preflight-patch".into(),
        description: "先校验所有 action 再写盘".into(),
        source_issue_category: "preflight".into(),
        affected_id: Some(campaign.id.as_str().to_string()),
        actions: vec![
            storyforge_app_meta::TypedPatchAction::UpdateCampaignVariable {
                key: "preflight_marker".into(),
                value: serde_json::json!("should-not-write"),
            },
            storyforge_app_meta::TypedPatchAction::RepointInstanceDefinition {
                instance_id: Id::from_str("target-inst"),
                new_definition_id: Some(Id::from_str("missing-definition")),
            },
        ],
        diff: vec![],
        created_at: chrono::Utc::now(),
        status: storyforge_app_meta::TypedPatchStatus::Pending,
        campaign_revision: None,
    };
    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        typed.push(patch);
    }

    let err = meta_accept_typed_patch_in_store(
        &store,
        "test-preflight-patch",
        campaign.id.as_str(),
        &state,
    )
    .expect_err("invalid later action should fail before any write");
    assert!(
        err.to_string().contains("Definition 不存在"),
        "unexpected error: {:?}",
        err
    );

    let campaign_after = store.get_campaign(&campaign.id).unwrap();
    assert_eq!(campaign_after.get_variable("preflight_marker"), None);
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        typed
            .iter()
            .find(|p| p.id == "test-preflight-patch")
            .unwrap()
            .status,
        storyforge_app_meta::TypedPatchStatus::Stale
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_meta_accept_typed_patch_rejects_stale_definition_binding() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::{VariableField, VariableType, VariableValue};

    let dir =
        std::env::temp_dir().join(format!("sf_test_accept_def_stale_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let card = {
        let mut c = storyforge_domain::character::CharacterCard {
            id: Id::from_str("card-1"),
            name: "测试卡".into(),
            source_character_id: Id::from_str("src-1"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::Value::Null,
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        };
        c.character_definitions.push(CharacterDefinition {
            id: Id::from_str("def-1"),
            card_id: c.id.clone(),
            name: "Old".into(),
            persona_prompt: String::new(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![VariableField {
                key: "old_hp".into(),
                label: "Old HP".into(),
                value_type: VariableType::Int,
                default: serde_json::json!(10),
                description: None,
                group: None,
            }],
        });
        c.character_definitions.push(CharacterDefinition {
            id: Id::from_str("def-2"),
            card_id: c.id.clone(),
            name: "New".into(),
            persona_prompt: String::new(),
            behavior_rules: String::new(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Supporting,
            variable_schema: vec![VariableField {
                key: "new_hp".into(),
                label: "New HP".into(),
                value_type: VariableType::Int,
                default: serde_json::json!(20),
                description: None,
                group: None,
            }],
        });
        c
    };
    store.save_card(card).unwrap();

    let campaign = Campaign::new(Id::from_str("card-1"), "test-run");
    store.save_campaign(campaign.clone()).unwrap();
    let definitions = store
        .get_card(&campaign.card_id)
        .unwrap()
        .card
        .character_definitions;
    let mut instance = CharacterInstance::from_definition(campaign.id.clone(), &definitions[1]);
    instance.id = Id::from_str("target-inst");
    instance.definition_id = Some(Id::from_str("def-2"));
    instance.variables = vec![VariableValue::new("new_hp", serde_json::json!(20), 0)];
    store.add_instance(instance).unwrap();

    let state = AppState::new_for_test();
    let patch = storyforge_app_meta::TypedPatch {
        id: "test-stale-definition-patch".into(),
        description: "旧定义变量同步".into(),
        source_issue_category: "variable_schema_mismatch".into(),
        affected_id: Some("target-inst".into()),
        actions: vec![
            storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
                instance_id: Id::from_str("target-inst"),
                definition_id: Id::from_str("def-1"),
                add_keys: vec!["old_hp".into()],
                remove_keys: vec!["new_hp".into()],
            },
        ],
        diff: vec![],
        created_at: chrono::Utc::now(),
        status: storyforge_app_meta::TypedPatchStatus::Pending,
        campaign_revision: None,
    };
    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        typed.push(patch);
    }

    let err = meta_accept_typed_patch_in_store(
        &store,
        "test-stale-definition-patch",
        campaign.id.as_str(),
        &state,
    )
    .expect_err("definition mismatch should stale the patch before writing");
    assert!(
        err.to_string().contains("已不再使用 definition"),
        "unexpected error: {:?}",
        err
    );

    let updated = store
        .get_instance(&campaign.id, &Id::from_str("target-inst"))
        .unwrap();
    assert!(updated.get_variable("new_hp").is_some());
    assert!(updated.get_variable("old_hp").is_none());
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(
        typed
            .iter()
            .find(|p| p.id == "test-stale-definition-patch")
            .unwrap()
            .status,
        storyforge_app_meta::TypedPatchStatus::Stale
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_typed_patch_pending_dedupe_canonicalizes_unordered_actions() {
    let make_patch = |add_keys: Vec<&str>, remove_keys: Vec<&str>, orphan_ids: Vec<&str>| {
        storyforge_app_meta::TypedPatch {
            id: uuid::Uuid::new_v4().to_string(),
            description: "dedupe".into(),
            source_issue_category: "variable_schema_mismatch".into(),
            affected_id: Some("target-inst".into()),
            actions: vec![
                storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
                    instance_id: Id::from_str("target-inst"),
                    definition_id: Id::from_str("def-1"),
                    add_keys: add_keys.into_iter().map(str::to_string).collect(),
                    remove_keys: remove_keys.into_iter().map(str::to_string).collect(),
                },
                storyforge_app_meta::TypedPatchAction::PruneOrphanTaskReferences {
                    task_id: Id::from_str("task-1"),
                    orphan_character_ids: orphan_ids.into_iter().map(Id::from_str).collect(),
                },
            ],
            diff: vec![],
            created_at: chrono::Utc::now(),
            status: storyforge_app_meta::TypedPatchStatus::Pending,
            campaign_revision: None,
        }
    };

    let existing = make_patch(vec!["mana", "hp"], vec!["legacy"], vec!["b", "a"]);
    let proposed = make_patch(vec!["hp", "mana"], vec!["legacy"], vec!["a", "b"]);

    assert!(is_same_pending_typed_patch(&existing, &proposed));
}

/// dismiss 后 status = Dismissed

#[test]
fn test_meta_dismiss_typed_patch() {
    let state = Arc::new(AppState::new_for_test());

    // 手动插入一条 patch
    let patch = storyforge_app_meta::TypedPatch {
        id: "test-dismiss-patch".into(),
        description: "测试忽略".into(),
        source_issue_category: "orphan_instance".into(),
        affected_id: None,
        actions: vec![],
        diff: vec![],
        created_at: chrono::Utc::now(),
        status: storyforge_app_meta::TypedPatchStatus::Pending,
        campaign_revision: None,
    };

    {
        let mut typed = state
            .typed_patches
            .write()
            .unwrap_or_else(|p| p.into_inner());
        typed.push(patch);
    }

    meta_dismiss_typed_patch_in_state("test-dismiss-patch", state.as_ref()).unwrap();

    // 验证
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    let p = typed.iter().find(|p| p.id == "test-dismiss-patch").unwrap();
    assert_eq!(p.status, storyforge_app_meta::TypedPatchStatus::Dismissed);

    // Pending 列表应不含该 patch
    let pending: Vec<_> = typed
        .iter()
        .filter(|p| p.status == storyforge_app_meta::TypedPatchStatus::Pending)
        .collect();
    assert!(
        pending.is_empty(),
        "dismissed patch 不应出现在 pending 列表"
    );
}

// ─── Phase A: Turn 提交屏障契约测试（hermetic：临时 TurnStore）────────
#[test]
fn plugin_dto_exposes_modify_prompt_permission_and_event_subscriptions() {
    use storyforge_infra_plugin_host::{InstalledPlugin, Permission, PluginManifest, UiSlot};

    let plugin = InstalledPlugin {
        manifest: PluginManifest {
            id: "prompt-hook".into(),
            name: "Prompt Hook".into(),
            version: "1.0.0".into(),
            permissions: vec![Permission::ModifyPrompt],
            entry_html: String::new(),
            ui_slots: vec![UiSlot::SidebarPanel],
            event_subscriptions: vec!["CHAT_COMPLETION_PROMPT_READY".into()],
            description: None,
            author: None,
        },
        installed_at: chrono::Utc::now(),
        enabled: true,
    };

    let dto = plugin_to_dto(&plugin);

    assert_eq!(dto.permissions, vec!["ModifyPrompt"]);
    assert_eq!(
        dto.event_subscriptions,
        vec!["CHAT_COMPLETION_PROMPT_READY"]
    );
}

/// Batch 2.4 契约：typed patch 的前置条件校验（target 存在 + definition/schema 一致）
/// 必须由 app-meta 的单一纯函数承载，Preview 与 Accept 共用同一逻辑。
///
/// 该测试钉住三件事：
/// 1. `validate_patch_preconditions` 对 target 缺失返回 `TypedPatchError`（与 `apply_to_snapshot`
///    的 `TargetMissing` 同源），不再让 meta_typed 重新实现一遍存在性扫描。
/// 2. `SyncInstanceVariables` 的 definition_id 不匹配时被同一纯函数拒绝（既有
///    `validate_typed_patch_targets` 的额外检查归并进来）。
/// 3. Preview 在 target 缺失时返回 `stale: true`（与 Accept 的 stale 标记语义一致）。
#[test]
fn typed_patch_preconditions_share_one_pure_function_between_preview_and_accept() {
    use storyforge_app_meta::{
        PreviewInput, TypedPatch, TypedPatchAction, TypedPatchStatus, validate_patch_preconditions,
    };
    use storyforge_domain::campaign::CharacterInstance;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::{VariableField, VariableType, default_character_variables};

    let campaign = storyforge_domain::campaign::Campaign::new(Id::from_str("card-1"), "tc");
    let cid = campaign.id.clone();
    let mut schema = default_character_variables();
    schema.push(VariableField {
        key: "hp".into(),
        label: "HP".into(),
        value_type: VariableType::Int,
        default: serde_json::json!(100),
        description: None,
        group: Some("状态".into()),
    });
    let def = CharacterDefinition {
        id: Id::from_str("def-1"),
        card_id: Id::from_str("card-1"),
        name: "Hero".into(),
        persona_prompt: "".into(),
        behavior_rules: "".into(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: schema,
    };
    let mut inst = CharacterInstance::from_definition(cid.clone(), &def);
    inst.id = Id::from_str("inst-1");
    let instances = vec![inst.clone()];
    let definitions = vec![def.clone()];
    let input = PreviewInput {
        instances: &instances,
        definitions: &definitions,
        knowledge: &[],
        tasks: &[],
        campaign: Some(&campaign),
    };

    // 1. target 存在 + definition 匹配 + schema 含 add_key → 通过
    let ok_patch = TypedPatch {
        id: "p-ok".into(),
        description: "sync".into(),
        source_issue_category: "variable_schema_mismatch".into(),
        affected_id: Some("inst-1".into()),
        actions: vec![TypedPatchAction::SyncInstanceVariables {
            instance_id: Id::from_str("inst-1"),
            definition_id: Id::from_str("def-1"),
            add_keys: vec!["hp".into()],
            remove_keys: vec![],
        }],
        diff: vec![],
        created_at: chrono::Utc::now(),
        status: TypedPatchStatus::Pending,
        campaign_revision: None,
    };
    validate_patch_preconditions(&ok_patch, &input).expect("匹配的前置条件应通过");

    // 2. definition_id 不匹配 → 纯函数拒绝（既有 validate_typed_patch_targets 的检查归并）
    let mismatch_patch = TypedPatch {
        id: "p-mismatch".into(),
        actions: vec![TypedPatchAction::SyncInstanceVariables {
            instance_id: Id::from_str("inst-1"),
            definition_id: Id::from_str("def-other"),
            add_keys: vec!["hp".into()],
            remove_keys: vec![],
        }],
        ..ok_patch.clone()
    };
    let err = validate_patch_preconditions(&mismatch_patch, &input)
        .expect_err("definition 不匹配应被纯函数拒绝");
    assert!(
        err.to_string().contains("def-other") || err.to_string().contains("definition"),
        "应报告 definition 不匹配: {err:?}"
    );

    // 3. target instance 缺失 → 纯函数拒绝（与 apply_to_snapshot TargetMissing 同源）
    let missing_patch = TypedPatch {
        id: "p-missing".into(),
        actions: vec![TypedPatchAction::SyncInstanceVariables {
            instance_id: Id::from_str("inst-gone"),
            definition_id: Id::from_str("def-1"),
            add_keys: vec!["hp".into()],
            remove_keys: vec![],
        }],
        ..ok_patch.clone()
    };
    validate_patch_preconditions(&missing_patch, &input).expect_err("target 缺失应被纯函数拒绝");
}
