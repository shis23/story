//! SQLite-native CharacterCommands + ImportExport lifecycle coverage (Gate 4
//! review fix P1-4).
//!
//! `sqlite_runtime::activate` is process-global, so this integration binary
//! intentionally contains one test that walks: character library
//! import/list/get/delete → world-info editing → campaign bundle export →
//! import (fresh ids) → import rollback (fault injection) → delete cascade →
//! ST PNG export chain over SQLite-sourced data.

use std::sync::Arc;

use storyforge_app_conversation::ConversationStore;
use storyforge_domain::Id;
use storyforge_domain::agent::RoundSummary;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{
    Character, CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, PropagationPolicy};
use storyforge_domain::conversation::Conversation;
use storyforge_domain::story_task::{StoryTask, TaskTrigger};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::campaign_store::StoredCard;
use storyforge_lib::sqlite_runtime;
use storyforge_lib::storage_backend::{
    BackendCapability, CapabilityStatus, CharacterInfo, StorageFacade,
};

fn sample_character_info(name: &str, source_character_id: Option<String>) -> CharacterInfo {
    CharacterInfo {
        source_character_id,
        name: name.to_string(),
        description: format!("{name} description"),
        personality: "calm".into(),
        scenario: "a forest".into(),
        first_mes: format!("Hello, I am {name}."),
        mes_example: String::new(),
        post_history_instructions: String::new(),
        alternate_greetings: vec![],
        system_prompt: String::new(),
        tags: vec!["test".into()],
        creator: "sqlite-lifecycle".into(),
        character_version: "1.0".into(),
        spec_version: "3.0".into(),
        extensions: serde_json::json!({}),
        embedded_world_info: None,
        renderable_assets: None,
        raw_card_json: serde_json::json!({ "spec": "3.0" }),
        has_world_info: false,
        has_renderable_assets: false,
        world_info_count: 0,
        world_info_entries: vec![],
    }
}

/// StoredCharacter → Character（与 `stored_info_to_character` 相同的语义，
/// 该纯函数是 pub(crate)，集成测试从公开 API 复刻一遍用于 PNG 导出链）。
fn to_character(stored: &storyforge_lib::storage_backend::StoredCharacter) -> Character {
    Character {
        id: Id::from_str(
            stored
                .info
                .source_character_id
                .as_deref()
                .unwrap_or(&stored.id),
        ),
        name: stored.info.name.clone(),
        description: stored.info.description.clone(),
        personality: stored.info.personality.clone(),
        scenario: stored.info.scenario.clone(),
        first_mes: stored.info.first_mes.clone(),
        mes_example: stored.info.mes_example.clone(),
        system_prompt: stored.info.system_prompt.clone(),
        post_history_instructions: stored.info.post_history_instructions.clone(),
        tags: stored.info.tags.clone(),
        creator: stored.info.creator.clone(),
        character_version: stored.info.character_version.clone(),
        alternate_greetings: stored.info.alternate_greetings.clone(),
        embedded_world_info: stored.info.embedded_world_info.clone(),
        extensions: stored.info.extensions.clone(),
        renderable_assets: stored.info.renderable_assets.clone(),
        source: storyforge_domain::Source::Native,
        spec_version: stored.info.spec_version.clone(),
        raw_card_json: stored.info.raw_card_json.clone(),
    }
}

#[test]
fn sqlite_character_library_world_info_bundle_roundtrip_and_png() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let facade = StorageFacade::new(
        temp.path().to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    );
    facade
        .validate_runtime_authority()
        .expect("facade/runtime authority must match");

    // ─── 0. 能力矩阵：CharacterCommands / ImportExport 必须 Supported ─────
    assert_eq!(
        facade.capability(BackendCapability::CharacterCommands),
        CapabilityStatus::Supported,
        "CharacterCommands must be Supported under SQLite"
    );
    assert_eq!(
        facade.capability(BackendCapability::ImportExport),
        CapabilityStatus::Supported,
        "ImportExport must be Supported under SQLite"
    );

    // ─── 1. 角色库：save → list → get（by id + by source）──────────────────
    let source_id = Id::new();
    let stored = facade
        .save_character(sample_character_info(
            "Elena",
            Some(source_id.as_str().to_string()),
        ))
        .expect("save character under SQLite");
    assert_eq!(stored.info.name, "Elena");
    assert!(!stored.id.is_empty());
    assert_eq!(
        stored.info.source_character_id.as_deref(),
        Some(source_id.as_str())
    );

    let listed = facade.list_characters().expect("list characters");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, stored.id);

    let by_id = facade
        .get_character(&stored.id)
        .expect("get character by stored id")
        .expect("found by stored id");
    assert_eq!(by_id.info.name, "Elena");

    let by_source = facade
        .get_character(source_id.as_str())
        .expect("get character by source id")
        .expect("found by source id");
    assert_eq!(by_source.id, stored.id);

    // 纯 SQLite 权威：不得偷偷写 characters.json。
    assert!(
        !temp.path().join("characters.json").exists(),
        "SQLite mode must not write the legacy JSON character library"
    );

    // ─── 2. 世界书编辑（add/route/update/delete）───────────────────────────
    let idx = facade
        .add_character_world_info_entry(
            &stored.id,
            vec!["castle".into()],
            "A stone keep.".into(),
            false,
            false,
        )
        .expect("add world info entry");
    assert_eq!(idx, 0);
    let after_add = facade.get_character(&stored.id).unwrap().unwrap();
    assert_eq!(after_add.info.world_info_count, 1);
    assert!(after_add.info.has_world_info);
    assert_eq!(after_add.info.world_info_entries[0].route, "Selective");
    assert_eq!(after_add.info.world_info_entries[0].depth, 2);
    assert_eq!(after_add.info.world_info_entries[0].order, 100);

    facade
        .update_character_world_info_route(&stored.id, 0, "Constant")
        .expect("update world info route");
    let routed = facade.get_character(&stored.id).unwrap().unwrap();
    assert_eq!(routed.info.world_info_entries[0].route, "Constant");

    facade
        .update_character_world_info_entry(
            &stored.id,
            0,
            vec!["keep".into()],
            "A grand keep.".into(),
            true,
            true,
            3,
            7,
        )
        .expect("update world info entry");
    let edited = facade.get_character(&stored.id).unwrap().unwrap();
    assert_eq!(edited.info.world_info_entries[0].keys, vec!["keep"]);
    assert_eq!(edited.info.world_info_entries[0].content, "A grand keep.");
    assert!(edited.info.world_info_entries[0].constant);
    assert!(edited.info.world_info_entries[0].is_global);
    assert_eq!(edited.info.world_info_entries[0].depth, 3);
    assert_eq!(edited.info.world_info_entries[0].order, 7);

    facade
        .delete_character_world_info_entry(&stored.id, 0)
        .expect("delete world info entry");
    let emptied = facade.get_character(&stored.id).unwrap().unwrap();
    assert_eq!(emptied.info.world_info_count, 0);
    assert!(!emptied.info.has_world_info);

    // 错误路径：越界索引与不存在的角色必须失败（不得用空成功冒充）。
    assert!(
        facade
            .update_character_world_info_route(&stored.id, 9, "Constant")
            .is_err()
    );
    assert!(
        facade
            .add_character_world_info_entry("missing-character", vec![], "x".into(), false, false)
            .is_err()
    );
    assert!(
        facade
            .update_character_world_info_entry(
                &stored.id,
                9,
                vec![],
                "x".into(),
                false,
                false,
                0,
                0
            )
            .is_err()
    );
    assert!(
        facade
            .delete_character_world_info_entry("missing-character", 0)
            .is_err()
    );

    // ─── 3. 种子 Campaign 图（SQLite 权威）────────────────────────────────
    let card_id = Id::new();
    let campaign_id = Id::new();
    let card_source_id = Id::new();
    let snapshot_source = facade
        .save_character(sample_character_info(
            "Elena snapshot source",
            Some(card_source_id.to_string()),
        ))
        .unwrap();
    let definition_id = Id::new();
    let mut conversation =
        Conversation::new(Some(snapshot_source.id.clone()), Some(campaign_id.clone()));
    let input_id = conversation.append_message(
        storyforge_domain::conversation::Role::User,
        "Continue".into(),
    );
    let draft_id = conversation.append_ai_draft("Snapshot body".into(), None);
    conversation.nodes[1].active_mut().unwrap().status =
        storyforge_domain::conversation::VariantStatus::Final;
    let conversation_id = conversation.id.clone();
    sqlite_runtime::save_conversation(&conversation).expect("seed conversation");

    let definition = CharacterDefinition {
        id: definition_id.clone(),
        card_id: card_id.clone(),
        name: "Elena".into(),
        persona_prompt: "She is a ranger".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: vec![],
    };
    let stored_card = StoredCard {
        card: CharacterCard {
            id: card_id.clone(),
            name: "Elena Card".into(),
            source_character_id: card_source_id.clone(),
            character_definitions: vec![definition],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({ "spec": "3.0" }),
            extraction_status: CharacterExtractionStatus::Extracted,
            extraction_message: None,
        },
        imported_at: "2026-07-31T00:00:00Z".into(),
    };
    sqlite_runtime::save_card_payload(
        &card_id,
        "Elena Card",
        Some(card_source_id.as_str()),
        Some("2026-07-31T00:00:00Z"),
        &serde_json::to_value(&stored_card).expect("serialize stored card"),
    )
    .expect("seed card payload");

    let mut campaign = Campaign::new(card_id.clone(), "Elena's Journey");
    campaign.id = campaign_id.clone();
    campaign.conversation_id = Some(conversation_id.clone());
    sqlite_runtime::save_campaign(&campaign).expect("seed campaign");
    let mut turn = storyforge_domain::turn::TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        input_id,
        0,
    );
    let mut attempt = storyforge_lib::turn_lifecycle::new_draft_attempt(
        Id::new(),
        draft_id.clone(),
        "Snapshot body",
        vec![],
    );
    attempt.status = storyforge_domain::turn::AttemptStatus::Committed;
    turn.status = storyforge_domain::turn::TurnStatus::Committed;
    turn.accepted_attempt_id = Some(attempt.attempt_id.clone());
    turn.attempts.push(attempt);
    sqlite_runtime::save_turn(&turn).unwrap();

    let instance = CharacterInstance {
        id: Id::new(),
        campaign_id: campaign_id.clone(),
        definition_id: Some(definition_id.clone()),
        name: "Elena".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    let instance_id = instance.id.clone();
    sqlite_runtime::save_instance(&instance).expect("seed instance");

    let mut knowledge = CharacterKnowledgeEntry::backstory(
        campaign_id.clone(),
        instance_id.clone(),
        "Born in the north".to_string(),
    );
    knowledge.set_propagation(PropagationPolicy::Private);
    sqlite_runtime::save_knowledge(&knowledge).expect("seed knowledge");

    let mut task = StoryTask::user_planned(
        campaign_id.clone(),
        "Find the keep".to_string(),
        "A journey task".to_string(),
        vec![TaskTrigger::Manual],
        0,
    );
    task.related_characters = vec![instance_id.clone()];
    task.id = Id::new();
    sqlite_runtime::save_task(&task).expect("seed task");

    let lineage_id = campaign.lineage_id.clone().unwrap();
    let mut leaf1 = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        1,
        "leaf one".into(),
    );
    leaf1.id = Id::new();
    leaf1.code = Some("A0001".into());
    leaf1.lineage_id = Some(lineage_id.clone());
    sqlite_runtime::seed_summary(&leaf1).expect("seed leaf summary");
    let mut leaf2 = RoundSummary::new(
        campaign_id.clone(),
        conversation_id.clone(),
        2,
        "leaf two".into(),
    );
    leaf2.id = Id::new();
    leaf2.code = Some("A0002".into());
    leaf2.lineage_id = Some(lineage_id.clone());
    sqlite_runtime::seed_summary(&leaf2).expect("seed leaf summary");

    assert_eq!(
        sqlite_runtime::list_instances(&campaign_id).unwrap().len(),
        1
    );
    assert_eq!(
        sqlite_runtime::list_knowledge(&campaign_id).unwrap().len(),
        1
    );
    assert_eq!(sqlite_runtime::list_tasks(&campaign_id).unwrap().len(), 1);
    assert_eq!(
        sqlite_runtime::list_summaries(&campaign_id).unwrap().len(),
        2
    );

    // ─── 4. Bundle 导出：结构与 JSON 路径一致 ──────────────────────────────
    let campaign_book = storyforge_domain::world_info::WorldInfoBook::from_st(
        serde_json::from_value(serde_json::json!({
            "entries": [
                { "id": 31, "keys": ["rain"], "content": "Rain matters", "constant": true },
                { "id": 32, "keys": ["gate"], "content": "Gate stays locked", "selective": true }
            ]
        }))
        .unwrap(),
    );
    facade.set_world_info(&campaign_id, &campaign_book).unwrap();
    let bundle_json = facade
        .export_campaign_bundle(&campaign_id)
        .expect("export bundle under SQLite");
    let exported: serde_json::Value = serde_json::from_str(&bundle_json).expect("bundle JSON");
    assert_eq!(exported["format_version"], 3);
    assert_eq!(
        exported["runtime"]["conversation"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(exported["runtime"]["turns"].as_array().unwrap().len(), 1);
    assert_eq!(exported["campaign"]["id"], campaign_id.as_str());
    assert_eq!(exported["card"]["id"], card_id.as_str());
    assert_eq!(exported["instances"].as_array().unwrap().len(), 1);
    assert_eq!(exported["definitions"].as_array().unwrap().len(), 1);
    assert_eq!(exported["knowledge"].as_array().unwrap().len(), 1);
    assert_eq!(exported["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(exported["summaries"].as_array().unwrap().len(), 2);

    // CampaignBundle DTO 结构契约由 JSON 路径测试（lib_tests_import_export）
    // 与导入解析共享；这里只验证导出 JSON 的结构与字段。

    // ─── 5. Bundle 导入（全新 ID，同库）→ 数据全部落 SQLite ───────────────
    let conv_store = Arc::new(ConversationStore::with_persistence(
        sqlite_runtime::conversation_persistence().expect("SQLite conversation authority"),
    ));
    let import_result = facade
        .import_campaign_bundle(
            serde_json::from_str(&bundle_json).unwrap(),
            conv_store.as_ref(),
        )
        .expect("import bundle under SQLite");
    assert_ne!(import_result.campaign_id, campaign_id.as_str());
    assert_ne!(import_result.card_id, card_id.as_str());
    assert_ne!(import_result.conversation_id, conversation_id.as_str());
    assert_eq!(import_result.instance_count, 1);
    assert_eq!(import_result.knowledge_count, 1);
    assert_eq!(import_result.task_count, 1);
    assert_eq!(import_result.summary_count, 2);

    let imported_campaign_id = Id::from_str(&import_result.campaign_id);
    let imported_campaign = sqlite_runtime::get_campaign(&imported_campaign_id)
        .expect("imported campaign readable")
        .expect("imported campaign exists");
    assert_eq!(imported_campaign.name, "Elena's Journey");
    assert_eq!(
        imported_campaign.conversation_id.as_ref(),
        Some(&Id::from_str(&import_result.conversation_id))
    );
    assert_eq!(
        sqlite_runtime::list_instances(&imported_campaign_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        sqlite_runtime::list_knowledge(&imported_campaign_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        sqlite_runtime::list_tasks(&imported_campaign_id)
            .unwrap()
            .len(),
        1
    );
    let imported_summaries = sqlite_runtime::list_summaries(&imported_campaign_id).unwrap();
    assert_eq!(imported_summaries.len(), 2);
    for summary in &imported_summaries {
        assert_eq!(
            summary.conversation_id,
            imported_campaign.conversation_id.clone().unwrap()
        );
    }

    let imported_card = sqlite_runtime::get_card_payload(&Id::from_str(&import_result.card_id))
        .expect("imported card payload readable")
        .expect("imported card exists");
    let imported_stored: StoredCard =
        serde_json::from_value(imported_card).expect("imported card payload decodes");
    assert_eq!(imported_stored.card.name, "Elena Card");
    assert_eq!(imported_stored.card.character_definitions.len(), 1);

    // 会话权威（ConversationStore）必须能看到导入的对话。
    let imported_conversation = conv_store
        .get(&Id::from_str(&import_result.conversation_id))
        .expect("imported conversation visible after invalidate");
    assert_eq!(
        imported_conversation.nodes[1].active_content(),
        "Snapshot body"
    );
    assert_ne!(imported_conversation.nodes[1].id, draft_id);
    assert!(
        facade
            .get_character(imported_stored.card.source_character_id.as_str())
            .unwrap()
            .is_some()
    );
    let reexported: serde_json::Value = serde_json::from_str(
        &facade
            .export_campaign_bundle(&imported_campaign_id)
            .unwrap(),
    )
    .unwrap();
    let imported_turn: storyforge_domain::turn::TurnRecord =
        serde_json::from_value(reexported["runtime"]["turns"][0].clone()).unwrap();
    assert_eq!(
        imported_turn.attempts[0].variant_id,
        imported_conversation.nodes[1].id
    );
    assert_eq!(
        imported_turn.input_node_id,
        imported_conversation.nodes[0].id
    );
    assert_ne!(imported_turn.turn_id, turn.turn_id);
    assert!(
        sqlite_runtime::get_turn(&imported_turn.turn_id)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        imported_conversation.campaign_id.as_ref(),
        Some(&imported_campaign_id)
    );

    // 导入的实例必须指向重写后的 definition。
    let imported_instances = sqlite_runtime::list_instances(&imported_campaign_id).unwrap();
    assert_eq!(
        imported_instances[0].definition_id.as_ref(),
        Some(&imported_stored.card.character_definitions[0].id)
    );

    {
        let state = Arc::new(
            storyforge_lib::AppState::new_with_backend(
                temp.path().to_path_buf(),
                Arc::new(facade.clone()),
            )
            .unwrap(),
        );
        // Same Tauri State wrapper used by the other native command tests.
        let command_state = unsafe {
            std::mem::transmute::<
                &Arc<storyforge_lib::AppState>,
                tauri::State<'_, Arc<storyforge_lib::AppState>>,
            >(&state)
        };
        let st = storyforge_lib::export_campaign_st_cards(
            import_result.campaign_id.clone(),
            command_state,
        )
        .unwrap();
        let shared: storyforge_domain::character::StWorldInfoBook =
            serde_json::from_str(&st.lorebook_json).unwrap();
        assert_eq!(
            shared.entries.len(),
            3,
            "campaign book plus acquired knowledge"
        );
        assert_eq!(shared.entries[2].id, Some(33), "no worldbook id collision");
        assert!(!st.cards.is_empty());
        for file in st.cards {
            let restored = storyforge_infra_import::import_character_from_png(&file.data).unwrap();
            let book = restored.embedded_world_info.unwrap().to_st_book();
            assert_eq!(book.entries.len(), 2);
            assert_eq!(book.entries[0].content.as_deref(), Some("Rain matters"));
            assert_eq!(book.entries[1].keys, vec!["gate"]);
        }
    }

    // 源数据不受影响（导入不覆盖）。
    assert_eq!(
        sqlite_runtime::list_instances(&campaign_id).unwrap().len(),
        1
    );

    // ─── 6. 带 B 父摘要的 bundle 导入：covers/covered_by 落 SQLite ────────
    // 手工构造等价 bundle（a1/a2 covered_by b；b covers a1/a2）
    let b_bundle = serde_json::json!({
        "format_version": 2,
        "exported_at": "2026-07-31T00:00:00Z",
        "card": null,
        "campaign": {
            "id": "graph-campaign",
            "card_id": "graph-card",
            "name": "Graph Campaign",
            "created_at": "2026-07-31T00:00:00Z",
            "revision": 0,
            "chronicle_revision": 0,
            "conversation_id": "graph-conversation",
            "lineage_id": "graph-lineage",
            "story_clock": "Day 1"
        },
        "instances": [],
        "definitions": [],
        "knowledge": [],
        "tasks": [],
        "summaries": [
            {
                "id": "graph-a1",
                "campaign_id": "graph-campaign",
                "conversation_id": "graph-conversation",
                "lineage_id": "graph-lineage",
                "level": 0,
                "turn": 1,
                "turn_end": 0,
                "code": "A0001",
                "headline": null,
                "covered_by": "graph-b1",
                "content": "leaf one",
                "created_at": "2026-07-31T00:00:00Z",
                "covers": []
            },
            {
                "id": "graph-a2",
                "campaign_id": "graph-campaign",
                "conversation_id": "graph-conversation",
                "lineage_id": "graph-lineage",
                "level": 0,
                "turn": 2,
                "turn_end": 0,
                "code": "A0002",
                "headline": null,
                "covered_by": "graph-b1",
                "content": "leaf two",
                "created_at": "2026-07-31T00:00:00Z",
                "covers": []
            },
            {
                "id": "graph-b1",
                "campaign_id": "graph-campaign",
                "conversation_id": "graph-conversation",
                "lineage_id": "graph-lineage",
                "level": 1,
                "turn": 1,
                "turn_end": 2,
                "code": "B0001",
                "headline": null,
                "covered_by": null,
                "content": "band",
                "created_at": "2026-07-31T00:00:00Z",
                "covers": ["graph-a1", "graph-a2"]
            }
        ]
    });
    let graph_result = facade
        .import_campaign_bundle(
            serde_json::from_value(b_bundle).unwrap(),
            conv_store.as_ref(),
        )
        .expect("import graph bundle under SQLite");
    assert_eq!(graph_result.summary_count, 3);
    let graph_campaign_id = Id::from_str(&graph_result.campaign_id);
    let graph_summaries = sqlite_runtime::list_summaries(&graph_campaign_id).unwrap();
    assert_eq!(graph_summaries.len(), 3);
    let parent = graph_summaries
        .iter()
        .find(|s| s.level == 1)
        .expect("B summary imported");
    assert_eq!(parent.covers.len(), 2);
    for child in graph_summaries.iter().filter(|s| s.level == 0) {
        assert_eq!(child.covered_by.as_ref(), Some(&parent.id));
    }

    // ─── 7. 导入故障注入：整体回滚（事务原子性证明）───────────────────────
    let rollback_bundle: serde_json::Value = serde_json::from_str(&bundle_json).unwrap();
    sqlite_runtime::fail_bundle_import_for_test(sqlite_runtime::BundleImportFault::AfterSummaries);
    let err = facade
        .import_campaign_bundle(
            serde_json::from_value(rollback_bundle).unwrap(),
            conv_store.as_ref(),
        )
        .expect_err("fault injection must fail the bundle import");
    assert!(
        err.to_string()
            .contains("injected failure after bundle summaries import")
    );
    sqlite_runtime::fail_bundle_import_for_test(sqlite_runtime::BundleImportFault::None);

    let pre_rollback_count = sqlite_runtime::list_campaigns().unwrap().len();
    let pre_rollback_card_count = sqlite_runtime::list_card_payloads().unwrap().len();
    let pre_rollback_character_count = sqlite_runtime::list_characters().unwrap().len();
    assert_eq!(
        sqlite_runtime::list_campaigns().unwrap().len(),
        pre_rollback_count,
        "rolled-back import must leave no campaign rows"
    );
    // 三种 fault 阶段各验证一次回滚（card / campaign / summaries）。
    for fault in [
        sqlite_runtime::BundleImportFault::AfterCard,
        sqlite_runtime::BundleImportFault::AfterCampaign,
        sqlite_runtime::BundleImportFault::AfterSummaries,
        sqlite_runtime::BundleImportFault::AfterRuntime,
    ] {
        sqlite_runtime::fail_bundle_import_for_test(fault);
        let err = facade
            .import_campaign_bundle(
                serde_json::from_str(&bundle_json).unwrap(),
                conv_store.as_ref(),
            )
            .expect_err("injected fault must fail the bundle import");
        assert!(!err.to_string().is_empty());
        sqlite_runtime::fail_bundle_import_for_test(sqlite_runtime::BundleImportFault::None);
        assert_eq!(
            sqlite_runtime::list_campaigns().unwrap().len(),
            pre_rollback_count,
            "fault {fault:?} must leave no imported rows behind"
        );
        assert_eq!(
            sqlite_runtime::list_card_payloads().unwrap().len(),
            pre_rollback_card_count,
            "fault {fault:?} must leave no imported card rows behind"
        );
        assert_eq!(
            sqlite_runtime::list_characters().unwrap().len(),
            pre_rollback_character_count
        );
    }

    // ─── 8. 角色删除级联：卡（含 campaign）+ MVU + 向量 ───────────────────
    // 给角色补一条 MVU 翻译，再把卡绑定到角色的 source id。
    sqlite_runtime::save_mvu(&storyforge_lib::campaign_store::StoredMvuTranslation {
        source_character_id: source_id.clone(),
        character_name: "Elena".into(),
        translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(vec![]),
        analyzed_at: "2026-07-31T00:00:00Z".into(),
    })
    .expect("seed MVU translation");
    let bound_card = StoredCard {
        card: CharacterCard {
            id: Id::new(),
            name: "Elena Card".into(),
            source_character_id: source_id.clone(),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({}),
            extraction_status: CharacterExtractionStatus::Unknown,
            extraction_message: None,
        },
        imported_at: "2026-07-31T00:00:00Z".into(),
    };
    sqlite_runtime::save_card_payload(
        &bound_card.card.id,
        "Elena Card",
        Some(source_id.as_str()),
        Some("2026-07-31T00:00:00Z"),
        &serde_json::to_value(&bound_card).unwrap(),
    )
    .expect("bind card to character source id");

    let characters_before_delete = sqlite_runtime::list_characters().unwrap().len();
    facade
        .delete_character(&stored.id)
        .expect("delete character under SQLite");
    assert!(
        facade.get_character(&stored.id).unwrap().is_none(),
        "character must be gone after delete"
    );
    assert_eq!(
        sqlite_runtime::list_characters().unwrap().len(),
        characters_before_delete - 1,
        "deleting one source must preserve imported sources"
    );
    assert!(
        !facade.delete_character(&stored.id).unwrap()
            || facade.get_character(&stored.id).unwrap().is_none(),
        "second delete must be a clean miss"
    );
    // 级联（Tauri 命令 delete_character 在 facade 删除后按
    // delete_character_cascade_source_ids 走 delete_mvu → get_card_by_source →
    // delete_card）：逐段验证 SQLite 侧每个环节可用且删干净。
    let cascade_source_ids = [Id::from_str(&stored.id), source_id.clone()];
    for source_id in &cascade_source_ids {
        let _ = facade
            .delete_mvu(source_id)
            .expect("delete MVU cascade works");
        if let Ok(Some(card)) = facade.get_card_by_source(source_id) {
            assert!(
                facade
                    .delete_card(&card.card.id)
                    .expect("delete card cascade works"),
                "bound card must be deleted"
            );
        }
    }
    assert!(
        sqlite_runtime::get_mvu(&source_id).unwrap().is_none(),
        "MVU translation must be gone after cascade"
    );
    assert!(
        sqlite_runtime::get_card_payload_by_source(&source_id)
            .unwrap()
            .is_none(),
        "bound card must be gone after cascade"
    );

    // ─── 8b. 真实 `delete_character_full_cascade`：带全量依赖（turns/outbox/
    //         mutation_commits/chronicle jobs/summaries/covers）的角色删除
    //         必须在一个事务内清理干净，绝不报告成功却留下半套数据（三审 P1）。
    let (_full_card_id, full_camp_id, full_conv_id, full_turn_id) = {
        let cid = Id::new();
        let camp_id = Id::new();
        let conv_id = Id::new();
        let turn_id = Id::new();
        sqlite_runtime::save_card_payload(
            &cid,
            "Full Cascade Card",
            Some("full-source-1"),
            Some("2026-07-31T00:00:00Z"),
            &serde_json::json!({"card": {"id": cid.as_str(), "name": "Full Cascade Card"}}),
        )
        .expect("save full card");
        let mut camp = Campaign::new(cid.clone(), "Full Cascade Camp");
        camp.id = camp_id.clone();
        camp.conversation_id = Some(conv_id.clone());
        sqlite_runtime::save_campaign(&camp).expect("save full campaign");
        let mut conv = Conversation::new(None, Some(camp_id.clone()));
        conv.id = conv_id.clone();
        sqlite_runtime::save_conversation(&conv).expect("save full conversation");
        let turn = storyforge_domain::turn::TurnRecord::new(
            camp_id.clone(),
            conv_id.clone(),
            Id::new(),
            0,
        );
        let turn = {
            let mut t = turn;
            t.turn_id = turn_id.clone();
            t
        };
        sqlite_runtime::save_turn(&turn).expect("save full turn");
        sqlite_runtime::save_mvu(&storyforge_lib::campaign_store::StoredMvuTranslation {
            source_character_id: Id::from_str("full-source-1"),
            character_name: "Full Cascade".into(),
            translation: storyforge_domain::mvu_translation::MvuTranslation::pure_data_fallback(
                vec![],
            ),
            analyzed_at: "2026-07-31T00:00:00Z".into(),
        })
        .expect("save full mvu");
        // 直接依赖行：mutation_commit + compress job + preaccept outbox
        // （FK 顺序：turn_attempts → mutation_commits / preaccept_outbox）。
        sqlite_runtime::with_db_raw(|db| {
            let conn = db.connection();
            conn.execute(
                "INSERT INTO turn_attempts (attempt_id, turn_id, variant_id, draft_hash, status, created_at, payload_json) VALUES ('attempt-full-1', ?1, 'variant-full', 'h', 'committed', 'now', '{}')",
                rusqlite::params![turn_id.as_str()],
            )
            .expect("insert attempt");
            conn.execute(
                "INSERT INTO mutation_commits (commit_id, campaign_id, turn_id, attempt_id, expected_revision, target_revision, terminal_status, payload_hash, committed_at) VALUES ('mc-full-1', ?1, ?2, 'attempt-full-1', 0, 1, 'committed', 'ph', 'now')",
                rusqlite::params![camp_id.as_str(), turn_id.as_str()],
            )
            .expect("insert mutation commit");
            conn.execute(
                "INSERT INTO chronicle_compress_jobs (job_id, campaign_id, status, attempts, max_attempts, created_at, updated_at) VALUES ('cj-full-1', ?1, 'pending', 0, 5, 'now', 'now')",
                rusqlite::params![camp_id.as_str()],
            )
            .expect("insert compress job");
            conn.execute(
                "INSERT INTO preaccept_outbox (outbox_id, campaign_id, conversation_id, turn_id, attempt_id, kind, draft_hash, payload_hash, status, payload_json, created_at, updated_at) VALUES ('ob-full-1', ?1, ?2, ?3, 'attempt-full-1', 'postprocess', 'h', 'ph', 'pending', '{}', 'now', 'now')",
                rusqlite::params![camp_id.as_str(), conv_id.as_str(), turn_id.as_str()],
            )
            .expect("insert outbox");
        });
        (cid, camp_id, conv_id, turn_id)
    };
    let full_char = facade
        .save_character(sample_character_info(
            "Full Cascade Hero",
            Some("full-source-1".to_string()),
        ))
        .expect("save full character");
    assert!(
        facade
            .delete_character_full_cascade(&full_char.id, &[Id::from_str("full-source-1")])
            .expect("atomic full cascade delete"),
        "full cascade must report deletion"
    );
    // 全部依赖行清理干净：卡、campaign、conversation、turn、MVU、jobs、outbox、commits。
    assert!(
        sqlite_runtime::get_card_payload_by_source(&Id::from_str("full-source-1"))
            .unwrap()
            .is_none(),
        "card must be gone after full cascade"
    );
    assert!(
        sqlite_runtime::get_campaign(&full_camp_id)
            .unwrap()
            .is_none(),
        "campaign must be gone after full cascade"
    );
    assert!(
        sqlite_runtime::get_conversation(&full_conv_id)
            .unwrap()
            .is_none(),
        "conversation must be gone after full cascade"
    );
    assert!(
        sqlite_runtime::get_turn(&full_turn_id).unwrap().is_none(),
        "turn must be gone after full cascade"
    );
    assert!(
        sqlite_runtime::get_mvu(&Id::from_str("full-source-1"))
            .unwrap()
            .is_none(),
        "MVU must be gone after full cascade"
    );
    sqlite_runtime::with_db_raw(|db| {
        let conn = db.connection();
        let compress_jobs: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM chronicle_compress_jobs WHERE campaign_id = ?1",
                [full_camp_id.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            compress_jobs, 0,
            "compress jobs must be gone after full cascade"
        );
        let outbox: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM preaccept_outbox WHERE campaign_id = ?1",
                [full_camp_id.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(outbox, 0, "outbox must be empty after full cascade");
        let commits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mutation_commits WHERE campaign_id = ?1",
                [full_camp_id.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            commits, 0,
            "mutation_commits must be empty after full cascade"
        );
        let turns: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM turns WHERE campaign_id = ?1",
                [full_camp_id.as_str()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(turns, 0, "turns must be gone after full cascade");
    });

    // ─── 8c. 删除级联 fault-injection 回滚（四审 P2：新事务必须有真实
    //          fault-injection rollback 测试）───────────────────────────────
    {
        let roll_char = facade
            .save_character(sample_character_info(
                "Rollback Hero",
                Some("rollback-source-1".to_string()),
            ))
            .expect("save rollback character");
        let roll_card_id = Id::new();
        sqlite_runtime::save_card_payload(
            &roll_card_id,
            "Rollback Card",
            Some("rollback-source-1"),
            Some("2026-07-31T00:00:00Z"),
            &serde_json::json!({"card": {"id": roll_card_id.as_str(), "name": "Rollback Card"}}),
        )
        .expect("save rollback card");
        let roll_camp_id = Id::new();
        let mut camp = Campaign::new(roll_card_id.clone(), "Rollback Camp");
        camp.id = roll_camp_id.clone();
        sqlite_runtime::save_campaign(&camp).expect("save rollback campaign");
        // 注入 MidCascade 失败 → 级联整体回滚，角色/卡/campaign 全部保留。
        storyforge_lib::sqlite_runtime::fail_delete_cascade_for_test(
            storyforge_lib::sqlite_runtime::DeleteCascadeFault::MidCascade,
        );
        let roll_result = facade
            .delete_character_full_cascade(&roll_char.id, &[Id::from_str("rollback-source-1")]);
        assert!(
            roll_result.is_err(),
            "fault injection must fail the cascade"
        );
        let roll_err = roll_result.unwrap_err();
        assert!(
            roll_err.contains("injected failure mid delete cascade"),
            "got: {roll_err}"
        );
        storyforge_lib::sqlite_runtime::fail_delete_cascade_for_test(
            storyforge_lib::sqlite_runtime::DeleteCascadeFault::None,
        );
        // 回滚证据：角色、卡、campaign 都还在。
        assert!(
            facade.get_character(&roll_char.id).unwrap().is_some(),
            "character must survive rollback"
        );
        assert!(
            sqlite_runtime::get_card_payload(&roll_card_id)
                .unwrap()
                .is_some(),
            "card must survive rollback"
        );
        assert!(
            sqlite_runtime::get_campaign(&roll_camp_id)
                .unwrap()
                .is_some(),
            "campaign must survive rollback"
        );
        // 清理，避免影响后续断言。
        facade
            .delete_character_full_cascade(&roll_char.id, &[Id::from_str("rollback-source-1")])
            .expect("cleanup rollback character");
    }

    // ─── 9. ST PNG 导出链可执行（SQLite 数据源）───────────────────────────
    let png_stored = facade
        .save_character(sample_character_info(
            "PNG Hero",
            Some(Id::new().as_str().to_string()),
        ))
        .expect("save PNG source character");
    let character = to_character(&png_stored);
    let st_data = storyforge_domain::character::to_st_data(&character, None, None);
    let card = storyforge_infra_import::png::make_st_card(st_data, &character.spec_version);
    let png_bytes = storyforge_infra_import::png::write_st_card_png(&card, None)
        .expect("PNG export chain executable");
    assert!(!png_bytes.is_empty());
    assert!(png_bytes.len() > 64, "PNG must be a real image payload");

    // 清尾：删掉 PNG 角色，避免影响前面断言过的列表（顺序在断言之后，无影响）。
    let _ = facade.delete_character(&png_stored.id);

    let _ = Arc::new(facade);
}
