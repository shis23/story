//! SQLite-native Meta lifecycle coverage (Gate 4).
//!
//! `sqlite_runtime::activate` is process-global, so this integration binary
//! intentionally contains one test that walks: health reads from the SQLite
//! authority → repair proposal from the SQLite snapshot (same pure functions
//! as JSON) → typed patch atomic apply with rollback fault injection.

use std::fs;
use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{
    CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, PropagationPolicy};
use storyforge_domain::story_task::{StoryTask, TaskTrigger};
use storyforge_infra_sqlite::backend::{BackendSource, PinnedBackend, StorageBackend};
use storyforge_lib::campaign_store::StoredCard;
use storyforge_lib::{backend_workflows, meta_backend, sqlite_runtime, storage_backend};

#[test]
fn sqlite_meta_health_proposal_and_atomic_patch_apply() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db_path = temp.path().join("storyforge.sqlite3");
    sqlite_runtime::activate(&db_path).expect("activate SQLite authority");

    let card_id = Id::from_str("meta-card-1");
    let campaign_id = Id::from_str("meta-campaign-1");
    let definition_id = Id::from_str("meta-definition-1");

    let definition = CharacterDefinition {
        id: definition_id.clone(),
        card_id: card_id.clone(),
        name: "Auditor".into(),
        persona_prompt: "Checks state consistently".into(),
        behavior_rules: "Never invent storage results".into(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Supporting,
        variable_schema: vec![storyforge_domain::variables::VariableField {
            key: "hp".into(),
            label: "HP".into(),
            value_type: storyforge_domain::variables::VariableType::Int,
            default: serde_json::json!(100),
            description: None,
            group: None,
        }],
    };
    let stored_card = StoredCard {
        card: CharacterCard {
            id: card_id.clone(),
            name: "SQLite Meta Fixture".into(),
            source_character_id: Id::from_str("meta-source-1"),
            character_definitions: vec![definition],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({"fixture": true}),
            extraction_status: CharacterExtractionStatus::Extracted,
            extraction_message: None,
        },
        imported_at: "2026-07-16T00:00:00Z".into(),
    };

    let mut campaign = Campaign::new(card_id.clone(), "SQLite Meta Campaign");
    campaign.id = campaign_id.clone();
    sqlite_runtime::save_campaign(&campaign).expect("save campaign");
    sqlite_runtime::save_card_payload(
        &card_id,
        "SQLite Meta Fixture",
        Some("meta-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::to_value(&stored_card).expect("serialize stored card"),
    )
    .expect("save card payload");

    // Deliberately orphan the instance from the one valid definition.
    let instance = CharacterInstance {
        id: Id::from_str("meta-instance-1"),
        campaign_id: campaign_id.clone(),
        definition_id: Some(Id::from_str("missing-definition")),
        name: "Orphan".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };
    sqlite_runtime::save_instance(&instance).expect("save orphan instance");
    let mut private = CharacterKnowledgeEntry::backstory(
        campaign_id.clone(),
        instance.id.clone(),
        "synthetic owner-only fixture fact",
    );
    private.set_propagation(PropagationPolicy::Private);
    sqlite_runtime::save_knowledge(&private).expect("save private knowledge");
    let mut task = StoryTask::user_planned(
        campaign_id.clone(),
        "Audit task",
        "Synthetic task for SQLite Meta inspection",
        vec![TaskTrigger::Manual],
        0,
    );
    task.id = Id::from_str("meta-task-1");
    sqlite_runtime::save_task(&task).expect("save task");
    assert_eq!(
        sqlite_runtime::list_knowledge(&campaign_id).unwrap().len(),
        1
    );
    assert_eq!(sqlite_runtime::list_tasks(&campaign_id).unwrap().len(), 1);

    // A legacy JSON decoy is removed before the query. A fallback cannot pass.
    let legacy_campaigns = temp.path().join("campaigns.json");
    fs::write(&legacy_campaigns, b"[]").expect("write JSON decoy");
    fs::remove_file(&legacy_campaigns).expect("remove JSON decoy");

    // 1. 健康检查必须读 SQLite 权威。
    let issues = meta_backend::sqlite_campaign_health_issues(&campaign_id)
        .expect("health must read SQLite authority");
    assert!(
        issues
            .iter()
            .any(|issue| issue.category == "orphan_instance"),
        "SQLite orphan instance must be reported"
    );
    assert!(!legacy_campaigns.exists(), "health must not recreate JSON");

    // 2. 修复提案：SQLite 快照喂给与 JSON 相同的纯函数管线。
    let facade = storage_backend::StorageFacade::new(
        temp.path().to_path_buf(),
        PinnedBackend::new(StorageBackend::Sqlite, BackendSource::Env),
    );
    facade
        .validate_runtime_authority()
        .expect("facade/runtime authority must match");
    let snapshot = backend_workflows::load_meta_snapshot_for_backend(&facade, &campaign_id)
        .expect("load SQLite Meta snapshot");
    assert_eq!(snapshot.campaign.id, campaign_id);
    assert_eq!(snapshot.instances.len(), 1);
    assert_eq!(snapshot.knowledge.len(), 1);
    assert_eq!(snapshot.tasks.len(), 1);
    assert_eq!(snapshot.definitions.len(), 1);

    let input = storyforge_app_meta::PreviewInput {
        instances: &snapshot.instances,
        definitions: &snapshot.definitions,
        knowledge: &snapshot.knowledge,
        tasks: &snapshot.tasks,
        campaign: Some(&snapshot.campaign),
    };
    let health =
        storyforge_app_meta::check_campaign_health(&storyforge_app_meta::CampaignHealthSnapshot {
            instances: &snapshot.instances,
            definitions: &snapshot.definitions,
            knowledge: &snapshot.knowledge,
            tasks: &snapshot.tasks,
        });
    let proposals: Vec<storyforge_app_meta::TypedPatch> = health
        .iter()
        .filter_map(|issue| storyforge_app_meta::build_patch_for_issue(issue, &input))
        .collect();
    assert!(
        proposals
            .iter()
            .any(|p| p.source_issue_category == "orphan_instance"),
        "orphan-instance repair proposal must be produced from SQLite snapshot"
    );

    // 3. 原子 apply：repoint orphan instance 到有效 definition + 更新变量。
    let actions = vec![
        storyforge_app_meta::TypedPatchAction::RepointInstanceDefinition {
            instance_id: instance.id.clone(),
            new_definition_id: Some(definition_id.clone()),
        },
        storyforge_app_meta::TypedPatchAction::SyncInstanceVariables {
            instance_id: instance.id.clone(),
            definition_id: definition_id.clone(),
            add_keys: vec!["hp".into(), "mana".into()],
            remove_keys: vec![],
        },
    ];
    sqlite_runtime::meta_apply_typed_patch_actions(&campaign_id, &actions, None)
        .expect("atomic Meta patch apply must work under SQLite");

    let instances = sqlite_runtime::list_instances(&campaign_id).unwrap();
    assert_eq!(instances[0].definition_id.as_ref(), Some(&definition_id));
    assert_eq!(
        instances[0].get_variable("hp"),
        Some(&serde_json::json!(100))
    );
    assert_eq!(
        instances[0].get_variable("mana"),
        Some(&serde_json::Value::Null)
    );

    // 4. 故障注入回滚：AfterFirstAction 后失败 → 变量与 definition 都不留痕迹。
    let actions2 = vec![
        storyforge_app_meta::TypedPatchAction::UpdateInstanceVariable {
            instance_id: instance.id.clone(),
            key: "hp".into(),
            value: serde_json::json!(1),
        },
        storyforge_app_meta::TypedPatchAction::UpdateTaskStatus {
            task_id: task.id.clone(),
            new_status: storyforge_domain::story_task::TaskStatus::Completed,
        },
    ];
    sqlite_runtime::fail_meta_patch_uow_for_test(
        storyforge_lib::sqlite_meta_repo::MetaPatchFault::AfterFirstAction,
    );
    let err = sqlite_runtime::meta_apply_typed_patch_actions(&campaign_id, &actions2, None)
        .expect_err("fault injection must fail the UoW");
    assert!(err.contains("injected failure after first meta action"));
    sqlite_runtime::fail_meta_patch_uow_for_test(
        storyforge_lib::sqlite_meta_repo::MetaPatchFault::None,
    );

    let instances_after = sqlite_runtime::list_instances(&campaign_id).unwrap();
    assert_eq!(
        instances_after[0].get_variable("hp"),
        Some(&serde_json::json!(100)),
        "rolled-back instance variable must stay at schema default"
    );
    let tasks_after = sqlite_runtime::list_tasks(&campaign_id).unwrap();
    assert_eq!(
        tasks_after[0].status,
        storyforge_domain::story_task::TaskStatus::Pending,
        "rolled-back task status must stay pending"
    );

    // 5. scope 校验：外部 campaign 的 patch 必须被拒绝（fail closed）。
    let foreign_campaign_id = Id::from_str("other-campaign");
    let err = sqlite_runtime::meta_apply_typed_patch_actions(&foreign_campaign_id, &actions2, None)
        .expect_err("foreign-campaign patch must fail closed");
    assert!(err.contains("other-campaign"));

    // 6. 损坏卡 payload 仍是存储完整性错误，不是空回退。
    sqlite_runtime::save_card_payload(
        &card_id,
        "SQLite Meta Fixture",
        Some("meta-source-1"),
        Some("2026-07-16T00:00:00Z"),
        &serde_json::json!({"card": {"id": "broken"}}),
    )
    .expect("replace card payload");
    assert!(meta_backend::sqlite_campaign_health_issues(&campaign_id).is_err());

    // 7. 能力矩阵：SQLite 下 Meta/MVU/Chronicle 均 supported。
    assert_eq!(
        facade.capability(storage_backend::BackendCapability::TypedMetaPatch),
        storage_backend::CapabilityStatus::Supported
    );
    assert_eq!(
        facade.capability(storage_backend::BackendCapability::MvuSchemaApply),
        storage_backend::CapabilityStatus::Supported
    );
    assert_eq!(
        facade.capability(storage_backend::BackendCapability::ChronicleCompressor),
        storage_backend::CapabilityStatus::Supported
    );
    assert_eq!(
        facade.capability(storage_backend::BackendCapability::WorldInfo),
        storage_backend::CapabilityStatus::Supported
    );

    // 8. WorldInfo SQLite：本局世界书读写 + 条目变更经 facade 持久化。
    let book = storyforge_domain::world_info::WorldInfoBook {
        entries: vec![storyforge_domain::world_info::WorldInfoEntry {
            st_id: None,
            keys: vec!["castle".into()],
            secondary_keys: vec![],
            content: "A stone keep.".into(),
            constant: false,
            selective: true,
            selective_logic: storyforge_domain::world_info::SelectiveLogic::And,
            disabled: false,
            position: 0,
            depth: 2,
            order: 100,
            route: storyforge_domain::world_info::LoreRoute::Selective,
            extensions: serde_json::json!({ "sf_source": "user" }),
            extra: Default::default(),
        }],
        source: storyforge_domain::Source::Native,
        metadata: Default::default(),
    };
    facade
        .set_world_info(&campaign_id, &book)
        .expect("set world info under SQLite");
    let loaded_book = facade
        .get_world_info(&campaign_id)
        .expect("get world info under SQLite");
    assert_eq!(loaded_book.entries.len(), 1);
    assert_eq!(loaded_book.entries[0].content, "A stone keep.");

    facade
        .set_world_info_route(
            &campaign_id,
            0,
            storyforge_domain::world_info::LoreRoute::Constant,
        )
        .expect("set route under SQLite");
    let routed = facade
        .get_world_info(&campaign_id)
        .expect("get routed world info");
    assert_eq!(
        routed.entries[0].route,
        storyforge_domain::world_info::LoreRoute::Constant
    );

    facade
        .set_world_info_entry_enabled(&campaign_id, 0, false)
        .expect("toggle enabled under SQLite");
    let toggled = facade
        .get_world_info(&campaign_id)
        .expect("get toggled world info");
    assert!(toggled.entries[0].disabled);

    facade
        .delete_world_info_entry(&campaign_id, 0)
        .expect("delete entry under SQLite");
    let emptied = facade
        .get_world_info(&campaign_id)
        .expect("get emptied world info");
    assert!(emptied.entries.is_empty());

    let _ = Arc::new(facade);
}
