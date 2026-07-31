use super::*;

#[test]
fn test_postprocess_knowledge_name_normalizes_to_instance_id() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_knowledge_norm_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut instance = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    instance.id = Id::from_str("inst-lin");
    store.add_instance(instance).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Lin"),
        knowledge_text: "Lin found the key".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present_ids = HashSet::from([String::from("inst-lin")]);

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        7,
        &present_ids,
        &HashSet::new(),
    );
    assert_eq!(
        entries.len(),
        1,
        "name target should resolve to campaign instance"
    );
    let entry = &entries[0];
    assert_eq!(entry.character_id, Id::from_str("inst-lin"));
    assert_eq!(entry.knowledge_text, "Lin found the key");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_postprocess_knowledge_skips_non_present_instance() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_knowledge_present_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
    chen.id = Id::from_str("inst-chen");
    store.add_instance(lin).unwrap();
    store.add_instance(chen).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Chen"),
        knowledge_text: "Chen saw the key".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present_ids = HashSet::from([String::from("inst-lin")]);

    let entry = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        7,
        &present_ids,
        &HashSet::new(),
    );

    assert!(entry.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_postprocess_knowledge_normalizes_source_character_id() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_knowledge_source_norm_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();
    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
    chen.id = Id::from_str("inst-chen");
    store.add_instance(lin).unwrap();
    store.add_instance(chen).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Lin"),
        knowledge_text: "Chen told Lin about the key".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: Some(Id::from_str("Chen")),
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present_ids = HashSet::from([String::from("Lin")]);

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        7,
        &present_ids,
        &HashSet::new(),
    );
    assert_eq!(entries.len(), 1, "target should resolve");
    let entry = &entries[0];
    assert_eq!(entry.character_id, Id::from_str("inst-lin"));
    assert_eq!(entry.source_character_id, Some(Id::from_str("inst-chen")));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_postprocess_knowledge_skips_unpersisted_temporary_name() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_knowledge_temp_skip_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Ghost"),
        knowledge_text: "Ghost appeared briefly".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present_ids = HashSet::from([String::from("Ghost")]);

    let entry = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        7,
        &present_ids,
        &HashSet::new(),
    );

    assert!(entry.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_postprocess_persistence_helper_writes_all_campaign_outputs() {
    use storyforge_domain::agent::{PostProcessResult, VariableUpdate};
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{
        CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
    };
    use storyforge_domain::story_task::{
        NewTaskSpec, StoryTask, TaskStatus, TaskTrigger, TaskUpdate,
    };

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_postprocess_persist_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    store.add_instance(lin.clone()).unwrap();

    let existing_task = StoryTask::user_planned(
        campaign.id.clone(),
        "Find key",
        "Find the hidden key",
        vec![TaskTrigger::Manual],
        1,
    );
    let existing_task_id = existing_task.id.clone();
    store.add_task(existing_task).unwrap();

    let persist_ctx = PostprocessPersistContext {
        campaign_id: campaign.id.clone(),
        conversation_id: Id::from_str("conv-1"),
        turn: 3,
    };
    let outcome = storyforge_app_agent::PostProcessOutcome {
        summary: Some("Lin found a clue.".into()),
        summary_attempted: true,
        post_process_attempted: true,
        post_process: Some(PostProcessResult {
            knowledge_updates: vec![CharacterKnowledgeUpdate {
                character_id: Id::from_str("Lin"),
                knowledge_text: "The key is under the mat.".into(),
                source: KnowledgeSource::Witnessed,
                source_character_id: None,
                pinned: false,
                broadcast: None,
                propagation: PropagationPolicy::Open,
            }],
            variable_updates: vec![
                VariableUpdate {
                    instance_id: Some(Id::from_str("Lin")),
                    key: "hp".into(),
                    value: serde_json::json!(7),
                },
                VariableUpdate {
                    instance_id: None,
                    key: "story_clock".into(),
                    value: serde_json::json!("Day 2"),
                },
            ],
            task_updates: vec![
                TaskUpdate {
                    task_id: Some(existing_task_id.clone()),
                    new_status: TaskStatus::Completed,
                    new_task: None,
                },
                TaskUpdate {
                    task_id: None,
                    new_status: TaskStatus::Pending,
                    new_task: Some(NewTaskSpec {
                        title: "Follow the clue".into(),
                        description: "Trace where the key leads.".into(),
                        triggers: vec![TaskTrigger::Manual],
                        related_characters: vec![lin.id.clone()],
                    }),
                },
            ],
            parse_succeeded: true,
        }),
    };

    persist_postprocess_outcome_to_store(&store, &persist_ctx, &outcome, &[String::from("Lin")]);

    let summaries = store.list_summaries(&campaign.id);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].conversation_id, Id::from_str("conv-1"));
    assert_eq!(summaries[0].turn, 3);
    assert_eq!(summaries[0].content, "Lin found a clue.");

    let knowledge = store.list_knowledge(&campaign.id);
    assert_eq!(knowledge.len(), 1);
    assert_eq!(knowledge[0].character_id, lin.id);
    assert_eq!(knowledge[0].turn_number, 3);
    assert_eq!(knowledge[0].knowledge_text, "The key is under the mat.");

    let updated_lin = store
        .get_instance(&campaign.id, &Id::from_str("inst-lin"))
        .unwrap();
    let hp = updated_lin.get_variable("hp").unwrap();
    assert_eq!(hp, &serde_json::json!(7));

    let updated_campaign = store.get_campaign(&campaign.id).unwrap();
    assert_eq!(updated_campaign.current_story_clock(), "Day 2");

    let updated_task = store.get_task(&existing_task_id).unwrap();
    assert!(matches!(updated_task.status, TaskStatus::Completed));

    let tasks = store.list_tasks(&campaign.id);
    assert!(tasks.iter().any(|task| task.title == "Follow the clue"
        && task.description == "Trace where the key leads."
        && task.created_turn == 3));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn postprocess_skips_unknown_character_target() {
    use storyforge_domain::agent::{PostProcessResult, VariableUpdate};
    use storyforge_domain::campaign::{Campaign, CharacterInstance};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_unknown_char_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    store.add_instance(lin.clone()).unwrap();

    let persist_ctx = PostprocessPersistContext {
        campaign_id: campaign.id.clone(),
        conversation_id: Id::from_str("conv-1"),
        turn: 2,
    };

    // outcome 包含一个未知角色名 + 一个已知角色/全局变量
    let outcome = storyforge_app_agent::PostProcessOutcome {
        summary: None,
        summary_attempted: false,
        post_process_attempted: true,
        post_process: Some(PostProcessResult {
            knowledge_updates: vec![],
            variable_updates: vec![
                VariableUpdate {
                    instance_id: Some(Id::from_str("UnknownChar")),
                    key: "level".into(),
                    value: serde_json::json!(99),
                },
                VariableUpdate {
                    instance_id: None,
                    key: "story_clock".into(),
                    value: serde_json::json!("Night"),
                },
            ],
            task_updates: vec![],
            parse_succeeded: true,
        }),
    };

    persist_postprocess_outcome_to_store(&store, &persist_ctx, &outcome, &[String::from("Lin")]);

    // 未知角色变量不写入（未报错即确认静默跳过）
    let lin_check = store
        .get_instance(&campaign.id, &Id::from_str("inst-lin"))
        .unwrap();
    assert!(
        lin_check.get_variable("level").is_none(),
        "未知角色的变量不应写入任何 instance"
    );

    // 全局变量仍正常写入
    let updated_campaign = store.get_campaign(&campaign.id).unwrap();
    assert_eq!(updated_campaign.current_story_clock(), "Night");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn postprocess_validates_task_belongs_to_campaign() {
    use storyforge_domain::agent::PostProcessResult;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::story_task::{StoryTask, TaskStatus, TaskTrigger, TaskUpdate};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_task_campaign_mismatch_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    // 从当前 campaign
    let campaign = Campaign::new(Id::from_str("card-main"), "Current Campaign");
    store.save_campaign(campaign.clone()).unwrap();
    let inst = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    store.add_instance(inst).unwrap();

    // 创建一个归属于不同 campaign 的任务（模拟写回时引用了其他 campaign 的任务）
    let other_camp_id = Id::from_str("other-campaign");
    let other_task = StoryTask::user_planned(
        other_camp_id.clone(),
        "Intrude",
        "Intrude other campaign",
        vec![TaskTrigger::Manual],
        1,
    );
    let other_task_id = other_task.id.clone();
    store.add_task(other_task).unwrap();

    let persist_ctx = PostprocessPersistContext {
        campaign_id: campaign.id.clone(),
        conversation_id: Id::from_str("conv-1"),
        turn: 1,
    };

    let outcome = storyforge_app_agent::PostProcessOutcome {
        summary: None,
        summary_attempted: false,
        post_process_attempted: true,
        post_process: Some(PostProcessResult {
            knowledge_updates: vec![],
            variable_updates: vec![],
            task_updates: vec![TaskUpdate {
                task_id: Some(other_task_id.clone()),
                new_status: TaskStatus::Completed,
                new_task: None,
            }],
            parse_succeeded: true,
        }),
    };

    persist_postprocess_outcome_to_store(&store, &persist_ctx, &outcome, &[]);

    // 其他 campaign 的任务状态不应被本 campaign 的写回修改
    let stored_task = store.get_task(&other_task_id).unwrap();
    assert!(
        matches!(stored_task.status, TaskStatus::Pending),
        "其他 campaign 的任务不应被当前 campaign 的写回修改：{stored_task:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn postprocess_empty_present_chars_rejects_witnessed_knowledge() {
    use storyforge_domain::agent::PostProcessResult;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{
        CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
    };

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_empty_present_witnessed_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);

    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    store.add_instance(lin.clone()).unwrap();

    let persist_ctx = PostprocessPersistContext {
        campaign_id: campaign.id.clone(),
        conversation_id: Id::from_str("conv-1"),
        turn: 1,
    };

    // Witnessed 知识 + present_chars 空集：知识路径收紧拒绝写入
    let outcome = storyforge_app_agent::PostProcessOutcome {
        summary: None,
        summary_attempted: false,
        post_process_attempted: true,
        post_process: Some(PostProcessResult {
            knowledge_updates: vec![CharacterKnowledgeUpdate {
                character_id: Id::from_str("Lin"),
                knowledge_text: "The key is under the mat.".into(),
                source: KnowledgeSource::Witnessed,
                source_character_id: None,
                pinned: false,
                broadcast: None,
                propagation: PropagationPolicy::Open,
            }],
            variable_updates: vec![],
            task_updates: vec![],
            parse_succeeded: true,
        }),
    };

    persist_postprocess_outcome_to_store(&store, &persist_ctx, &outcome, &[]);

    let knowledge = store.list_knowledge(&campaign.id);
    assert!(
        knowledge.is_empty(),
        "present_chars 空集时 Witnessed 知识不应写入"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_postprocess_variable_keys_include_runtime_custom_schema_and_values() {
    use std::collections::HashMap;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::{
        VariableField, VariableType, VariableValue, default_character_variables,
    };

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    let mut campaign = Campaign::new(Id::from_str("card-keys"), "Key Campaign");
    campaign.variables.push(VariableValue::new(
        "alarm_level",
        serde_json::json!("red"),
        2,
    ));

    let mut schema = default_character_variables();
    schema.push(VariableField {
        key: "stress".into(),
        label: "Stress".into(),
        value_type: VariableType::Int,
        default: serde_json::json!(0),
        description: None,
        group: Some("state".into()),
    });
    let definition = CharacterDefinition {
        id: Id::from_str("def-keys"),
        card_id: Id::from_str("card-keys"),
        name: "Lin".into(),
        persona_prompt: String::new(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: schema,
    };
    let mut instance = CharacterInstance::from_definition(campaign.id.clone(), &definition);
    instance.variables.push(VariableValue::new(
        "temporary_flag",
        serde_json::json!(true),
        2,
    ));
    let mut definitions_by_id = HashMap::new();
    definitions_by_id.insert(definition.id.clone(), definition);

    ctx.campaign_runtime = Some(Arc::new(CampaignRuntimeContext {
        campaign,
        instances: vec![instance],
        definitions_by_id,
        knowledge: vec![],
        tasks: vec![],
        turn: 2,
    }));

    let keys = postprocess_variable_keys(&ctx);

    for expected in [
        "hp（角色/生命值/int）",
        "story_clock（全局/故事时间/string）",
        "weather（全局/天气/string）",
        "stress（角色/Stress/int）",
        "alarm_level（全局/alarm_level/string）",
        "temporary_flag（角色/temporary_flag/bool）",
    ] {
        assert!(
            keys.contains(&expected.to_string()),
            "missing key {expected}"
        );
    }
    assert_eq!(
        keys.iter()
            .filter(|key| key.starts_with("hp（角色/"))
            .count(),
        1
    );
}

// ─── W6 方向 1：广播分发测试 ───────────────────────────────────────────

#[test]
fn test_broadcast_all_distributes_to_all_instances() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource,
    };

    let dir = std::env::temp_dir().join(format!("sf_broadcast_all_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    // 创建 3 个 instance
    let mut a = CharacterInstance::temporary(campaign.id.clone(), "A");
    a.id = Id::from_str("inst-a");
    let mut b = CharacterInstance::temporary(campaign.id.clone(), "B");
    b.id = Id::from_str("inst-b");
    let mut c = CharacterInstance::temporary(campaign.id.clone(), "C");
    c.id = Id::from_str("inst-c");
    store.add_instance(a).unwrap();
    store.add_instance(b).unwrap();
    store.add_instance(c).unwrap();

    // 广播发起者是 A（source_character_id=A），broadcast=All
    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("A"),
        knowledge_text: "全城戒严公告".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: Some(Id::from_str("A")),
        pinned: false,
        broadcast: Some(BroadcastTarget::All),
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &HashSet::new(),
        &HashSet::new(),
    );

    // 应分发给 B 和 C（排除发起者 A 自身）
    assert_eq!(
        entries.len(),
        2,
        "broadcast All 应分发给除发起者外的所有 instance"
    );
    let target_ids: Vec<_> = entries.iter().map(|e| e.character_id.clone()).collect();
    assert!(target_ids.contains(&Id::from_str("inst-b")));
    assert!(target_ids.contains(&Id::from_str("inst-c")));
    assert!(
        !target_ids.contains(&Id::from_str("inst-a")),
        "不应分发给发起者自身"
    );

    // 每条都是 ToldByOther，source_character_id = A
    for entry in &entries {
        assert_eq!(entry.source, KnowledgeSource::ToldByOther);
        assert_eq!(entry.source_character_id, Some(Id::from_str("inst-a")));
        assert_eq!(entry.knowledge_text, "全城戒严公告");
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_broadcast_group_distributes_to_matching_group() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterCard, CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource,
    };

    let dir = std::env::temp_dir().join(format!("sf_broadcast_group_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    // 创建 card + definitions（有 group 和无 group）
    let def_guard = CharacterDefinition {
        id: Id::from_str("def-guard"),
        card_id: Id::from_str("card-1"),
        name: "Guard".into(),
        persona_prompt: "guard".into(),
        behavior_rules: "guard".into(),
        base_backstory: vec![],
        group: Some("守卫".to_string()),
        role_type: RoleType::Supporting,
        variable_schema: vec![],
    };
    let def_merchant = CharacterDefinition {
        id: Id::from_str("def-merchant"),
        card_id: Id::from_str("card-1"),
        name: "Merchant".into(),
        persona_prompt: "merchant".into(),
        behavior_rules: "merchant".into(),
        base_backstory: vec![],
        group: Some("商人".to_string()),
        role_type: RoleType::Supporting,
        variable_schema: vec![],
    };
    let def_leader = CharacterDefinition {
        id: Id::from_str("def-leader"),
        card_id: Id::from_str("card-1"),
        name: "Leader".into(),
        persona_prompt: "leader".into(),
        behavior_rules: "leader".into(),
        base_backstory: vec![],
        group: None, // 无 group
        role_type: RoleType::Protagonist,
        variable_schema: vec![],
    };
    let card = CharacterCard {
        id: Id::from_str("card-1"),
        name: "test card".into(),
        source_character_id: Id::from_str("src-1"),
        character_definitions: vec![def_guard, def_merchant, def_leader],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::Value::Null,
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    store.save_card(card).unwrap();

    // 创建 instances（link to definitions）
    let mut inst_guard = CharacterInstance::temporary(campaign.id.clone(), "Guard");
    inst_guard.id = Id::from_str("inst-guard");
    inst_guard.definition_id = Some(Id::from_str("def-guard"));
    let mut inst_merchant = CharacterInstance::temporary(campaign.id.clone(), "Merchant");
    inst_merchant.id = Id::from_str("inst-merchant");
    inst_merchant.definition_id = Some(Id::from_str("def-merchant"));
    let mut inst_leader = CharacterInstance::temporary(campaign.id.clone(), "Leader");
    inst_leader.id = Id::from_str("inst-leader");
    inst_leader.definition_id = Some(Id::from_str("def-leader"));
    store.add_instance(inst_guard).unwrap();
    store.add_instance(inst_merchant).unwrap();
    store.add_instance(inst_leader).unwrap();

    // 广播给"守卫"组
    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Leader"),
        knowledge_text: "守卫集合命令".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: Some(Id::from_str("Leader")),
        pinned: false,
        broadcast: Some(BroadcastTarget::Group("守卫".to_string())),
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &HashSet::new(),
        &HashSet::new(),
    );

    // 只有 Guard（group=守卫）收到，Merchant（group=商人）和 Leader（group=None，且是发起者）不收
    assert_eq!(entries.len(), 1, "broadcast Group('守卫') 应只分发给守卫组");
    assert_eq!(entries[0].character_id, Id::from_str("inst-guard"));
    assert_eq!(entries[0].source, KnowledgeSource::ToldByOther);
    assert_eq!(
        entries[0].source_character_id,
        Some(Id::from_str("inst-leader"))
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_broadcast_none_single_character_unaffected() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

    let dir = std::env::temp_dir().join(format!("sf_broadcast_none_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut a = CharacterInstance::temporary(campaign.id.clone(), "A");
    a.id = Id::from_str("inst-a");
    let mut b = CharacterInstance::temporary(campaign.id.clone(), "B");
    b.id = Id::from_str("inst-b");
    store.add_instance(a).unwrap();
    store.add_instance(b).unwrap();

    // broadcast=None → 单角色定向，走原有 P3 逻辑
    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("A"),
        knowledge_text: "A 看到了什么".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present_ids = HashSet::from([String::from("inst-a")]);

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &present_ids,
        &HashSet::new(),
    );

    // 单角色：只有 A 收到
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].character_id, Id::from_str("inst-a"));
    assert_eq!(entries[0].source, KnowledgeSource::Witnessed);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_private_source_knowledge_blocks_told_by_other_propagation() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{
        CharacterKnowledgeEntry, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
    };

    let dir = std::env::temp_dir().join(format!("sf_private_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "private-test");
    store.save_campaign(campaign.clone()).unwrap();

    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
    chen.id = Id::from_str("inst-chen");
    store.add_instance(lin).unwrap();
    store.add_instance(chen).unwrap();

    let mut private_entry = CharacterKnowledgeEntry::witnessed(
        campaign.id.clone(),
        Id::from_str("inst-lin"),
        "保险柜密码是 0427",
        1,
    );
    private_entry.propagation = PropagationPolicy::Private;
    store.add_knowledge(vec![private_entry]).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Chen"),
        knowledge_text: "保险柜密码是 0427".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: Some(Id::from_str("Lin")),
        pinned: false,
        broadcast: None,
        propagation: PropagationPolicy::Open,
    };

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        2,
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        entries.is_empty(),
        "private source knowledge must not propagate"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_private_knowledge_update_cannot_broadcast() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{
        BroadcastTarget, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
    };

    let dir = std::env::temp_dir().join(format!("sf_private_broadcast_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "private-broadcast-test");
    store.save_campaign(campaign.clone()).unwrap();

    let mut lin = CharacterInstance::temporary(campaign.id.clone(), "Lin");
    lin.id = Id::from_str("inst-lin");
    let mut chen = CharacterInstance::temporary(campaign.id.clone(), "Chen");
    chen.id = Id::from_str("inst-chen");
    store.add_instance(lin).unwrap();
    store.add_instance(chen).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Lin"),
        knowledge_text: "保险柜密码是 0427".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: Some(Id::from_str("Lin")),
        pinned: false,
        broadcast: Some(BroadcastTarget::All),
        propagation: PropagationPolicy::Private,
    };

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        2,
        &HashSet::new(),
        &HashSet::new(),
    );

    assert!(
        entries.is_empty(),
        "private knowledge must not be broadcast"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_told_by_other_links_matching_source_knowledge() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{
        CharacterKnowledgeEntry, CharacterKnowledgeUpdate, KnowledgeSource, PropagationPolicy,
    };

    let dir = std::env::temp_dir().join(format!("sf_relay_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "relay-test");
    store.save_campaign(campaign.clone()).unwrap();

    let mut a = CharacterInstance::temporary(campaign.id.clone(), "A");
    a.id = Id::from_str("inst-a");
    let mut b = CharacterInstance::temporary(campaign.id.clone(), "B");
    b.id = Id::from_str("inst-b");
    let mut c = CharacterInstance::temporary(campaign.id.clone(), "C");
    c.id = Id::from_str("inst-c");
    store.add_instance(a).unwrap();
    store.add_instance(b).unwrap();
    store.add_instance(c).unwrap();

    let a_entry = CharacterKnowledgeEntry::witnessed(
        campaign.id.clone(),
        Id::from_str("inst-a"),
        "地下室有尸体",
        1,
    );
    let mut b_entry = CharacterKnowledgeEntry::told_by(
        campaign.id.clone(),
        Id::from_str("inst-b"),
        "地下室有尸体",
        Id::from_str("inst-a"),
        2,
    );
    b_entry.source_knowledge_id = Some(a_entry.id.clone());
    store.add_knowledge(vec![a_entry, b_entry.clone()]).unwrap();

    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("C"),
        knowledge_text: "地下室有尸体".into(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: Some(Id::from_str("B")),
        pinned: false,
        broadcast: None,
        propagation: PropagationPolicy::Open,
    };

    let entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        3,
        &HashSet::new(),
        &HashSet::new(),
    );

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].character_id, Id::from_str("inst-c"));
    assert_eq!(
        entries[0].source_knowledge_id,
        Some(b_entry.id.clone()),
        "C 的知识应链接到 B 持有的上游知识"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_knowledge_entry_dto_resolves_provenance_names() {
    let campaign_id = Id::from_str("camp");
    let lin_id = Id::from_str("inst-lin");
    let chen_id = Id::from_str("inst-chen");
    let entry = storyforge_domain::character_knowledge::CharacterKnowledgeEntry::told_by(
        campaign_id,
        lin_id.clone(),
        "地下室有尸体",
        chen_id.clone(),
        3,
    );
    let names = std::collections::HashMap::from([
        (lin_id, "林医生".to_string()),
        (chen_id, "陈警官".to_string()),
    ]);
    let knowledge_by_id = std::collections::HashMap::from([(entry.id.clone(), &entry)]);

    let dto = knowledge_entry_to_dto(&entry, &names, &knowledge_by_id);

    assert_eq!(dto.character_name.as_deref(), Some("林医生"));
    assert_eq!(dto.source_character_name.as_deref(), Some("陈警官"));
    assert!(dto.source_knowledge_id.is_none());
    assert!(dto.relay_chain_text.is_none());
    assert_eq!(dto.provenance_text, "林医生 被 陈警官 告知");
    assert_eq!(dto.propagation, "open");
}

#[test]
fn test_knowledge_entry_dto_renders_relay_chain() {
    use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;

    let campaign_id = Id::from_str("camp");
    let a_id = Id::from_str("inst-a");
    let b_id = Id::from_str("inst-b");
    let c_id = Id::from_str("inst-c");

    let a_entry =
        CharacterKnowledgeEntry::witnessed(campaign_id.clone(), a_id.clone(), "地下室有尸体", 1);
    let mut b_entry = CharacterKnowledgeEntry::told_by(
        campaign_id.clone(),
        b_id.clone(),
        "地下室有尸体",
        a_id.clone(),
        2,
    );
    b_entry.source_knowledge_id = Some(a_entry.id.clone());
    let mut c_entry = CharacterKnowledgeEntry::told_by(
        campaign_id,
        c_id.clone(),
        "地下室有尸体",
        b_id.clone(),
        3,
    );
    c_entry.source_knowledge_id = Some(b_entry.id.clone());

    let names = std::collections::HashMap::from([
        (a_id, "A".to_string()),
        (b_id, "B".to_string()),
        (c_id, "C".to_string()),
    ]);
    let knowledge_by_id = std::collections::HashMap::from([
        (a_entry.id.clone(), &a_entry),
        (b_entry.id.clone(), &b_entry),
        (c_entry.id.clone(), &c_entry),
    ]);

    let dto = knowledge_entry_to_dto(&c_entry, &names, &knowledge_by_id);

    assert_eq!(
        dto.source_knowledge_id.as_deref(),
        Some(b_entry.id.as_str())
    );
    assert_eq!(
        dto.relay_chain_text.as_deref(),
        Some("A（轮 1） → B（轮 2） → C（轮 3）")
    );
}

#[test]
fn test_current_cancel_slot() {
    let state = AppState::new_for_test();

    // 初始无运行中的写作
    {
        let slot = state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        assert!(slot.is_none());
    }

    // 模拟 start_writing 设置 operation-owned cancel
    let (_op_a, rx_a) = begin_writing_operation(&state);
    assert!(!*rx_a.borrow());

    // 触发取消
    {
        let slot = state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let handle = slot.as_ref().unwrap();
        let _ = handle.cancel_tx.send(true);
    }
    assert!(*rx_a.borrow(), "cancel 应已触发");

    // 清理本 operation
    let op = state
        .current_cancel
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .as_ref()
        .map(|h| h.operation_id.clone())
        .unwrap();
    clear_current_cancel_if(&state, &op);
    assert!(
        state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none()
    );
}

#[test]
fn operation_owned_cancel_interleaving_preserves_active_generation() {
    let state = AppState::new_for_test();

    // Operation A starts.
    let (op_a, rx_a) = begin_writing_operation(&state);
    assert!(!*rx_a.borrow());

    // Operation B starts while A is still "postprocessing": A must observe cancel,
    // and the global slot becomes B.
    let (op_b, rx_b) = begin_writing_operation(&state);
    assert_ne!(op_a, op_b);
    assert!(*rx_a.borrow(), "starting B must cancel A");
    assert!(!*rx_b.borrow());
    {
        let slot = state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        assert_eq!(slot.as_ref().unwrap().operation_id, op_b);
    }

    // A finishing must not clear B's sender.
    clear_current_cancel_if(&state, &op_a);
    {
        let slot = state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        assert_eq!(
            slot.as_ref().map(|h| h.operation_id.clone()),
            Some(op_b.clone()),
            "A clear must not wipe B"
        );
    }
    assert!(!*rx_b.borrow());

    // cancel_writing still cancels the active generation B.
    {
        let slot = state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let _ = slot.as_ref().unwrap().cancel_tx.send(true);
    }
    assert!(*rx_b.borrow(), "active cancel must still reach B");

    // B clear succeeds; stale A clear remains a no-op.
    clear_current_cancel_if(&state, &op_b);
    clear_current_cancel_if(&state, &op_a);
    assert!(
        state
            .current_cancel
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_none()
    );
}

#[test]
fn scope_validation_errors_do_not_mark_turn_failed() {
    use production_postprocess::{
        JsonTurnAttemptSink, PostprocessIdentity, ProductionPostprocessError,
        ProductionPostprocessService,
    };
    use std::sync::Arc;
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::turn::{AttemptStatus, QualityReport, TurnRecord, TurnStatus};

    let dir = std::env::temp_dir().join(format!("sf_scope_zero_write_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let campaign_store = Arc::new(campaign_store::CampaignStore::new(&dir));
    let turn_store = Arc::new(turn_store::TurnStore::new(&dir));
    let mut campaign = Campaign::new(Id::new(), "scope-zero");
    campaign.lineage_id = Some(Id::new());
    let campaign_id = campaign.id.clone();
    campaign_store.save_campaign(campaign).unwrap();
    let conversation_id = Id::from_str("conv-scope");
    let attempt_id = Id::new();
    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        Id::from_str("input"),
        0,
    );
    record.status = TurnStatus::DraftReady;
    record.attempts.push(turn_lifecycle::new_draft_attempt(
        attempt_id.clone(),
        Id::from_str("variant"),
        "draft",
        vec![],
    ));
    let turn_id = record.turn_id.clone();
    turn_store.create_turn(record).unwrap();

    let sink = JsonTurnAttemptSink {
        turn_store: &turn_store,
    };
    let service = ProductionPostprocessService::new_json(&campaign_store, &sink);
    let (_tx, cancel_rx) = watch::channel(false);
    let before = turn_store.get_turn(&turn_id).unwrap();
    let original_hash = before.find_attempt(&attempt_id).unwrap().draft_hash.clone();

    let assert_zero_write = |label: &str| {
        let after = turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after.status, before.status, "{label}: status");
        assert_eq!(
            after.conversation_id, before.conversation_id,
            "{label}: conversation"
        );
        assert_eq!(after.campaign_id, before.campaign_id, "{label}: campaign");
        assert_eq!(after.failure_reason, before.failure_reason, "{label}: fail");
        let att = after.find_attempt(&attempt_id).unwrap();
        assert_eq!(att.draft_hash, original_hash, "{label}: draft_hash");
        assert!(att.pending_state_changes.is_none(), "{label}: batch");
        assert!(
            campaign_store.list_summaries(&campaign_id).is_empty(),
            "{label}: summaries"
        );
    };

    // apply_outcome cross-campaign
    let err = service
        .apply_outcome(
            &PostprocessIdentity {
                turn_id: turn_id.clone(),
                attempt_id: attempt_id.clone(),
                campaign_id: Id::from_str("other-campaign"),
                conversation_id: conversation_id.clone(),
                turn_number: 1,
            },
            Some(storyforge_app_agent::PostProcessOutcome {
                summary: Some("must not write".into()),
                summary_attempted: true,
                post_process_attempted: false,
                post_process: None,
            }),
            &[],
            &cancel_rx,
        )
        .expect_err("cross campaign must fail");
    assert!(matches!(
        err,
        ProductionPostprocessError::ScopeMismatch {
            field: "campaign_id",
            ..
        }
    ));
    let bad_identity = PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: attempt_id.clone(),
        campaign_id: Id::from_str("other-campaign"),
        conversation_id: conversation_id.clone(),
        turn_number: 1,
    };
    let backend = BackendTurnAttemptSink::for_json_store(&turn_store);
    let combined = service_fail_turn(&backend, &bad_identity, err);
    assert!(matches!(
        combined,
        ProductionPostprocessError::ScopeMismatch { .. }
    ));
    assert_zero_write("apply cross campaign");

    // start_writing / regenerate adapter path: sync_autofix via Backend sink.
    // Use the isolated JSON store for the backend adapter path. This keeps the test hermetic
    // even when the workspace test runner executes tests in parallel.
    let backend = BackendTurnAttemptSink::for_json_store(&turn_store);
    turn_store.save_turn(before.clone()).unwrap();
    // new_json only needs sink for autofix; campaign_store is unused on this path.
    let backend_service = ProductionPostprocessService::new_json(get_campaign_store(), &backend);

    let camp_identity = PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: attempt_id.clone(),
        campaign_id: Id::from_str("other-campaign"),
        conversation_id: conversation_id.clone(),
        turn_number: 1,
    };
    let camp_err = backend_service
        .sync_autofix_attempt(
            &camp_identity,
            "must-not-write",
            QualityReport { warnings: vec![] },
        )
        .expect_err("backend cross campaign");
    assert!(matches!(
        camp_err,
        ProductionPostprocessError::ScopeMismatch {
            field: "campaign_id",
            ..
        }
    ));
    let combined = service_fail_turn(&backend, &camp_identity, camp_err);
    assert!(matches!(
        combined,
        ProductionPostprocessError::ScopeMismatch {
            field: "campaign_id",
            ..
        }
    ));
    // zero-write on process turn store
    let after_backend = turn_store.get_turn(&turn_id).unwrap();
    assert_eq!(after_backend.status, TurnStatus::DraftReady);
    assert_eq!(after_backend.failure_reason, None);
    assert_eq!(
        after_backend.find_attempt(&attempt_id).unwrap().draft_hash,
        original_hash
    );

    let conv_identity = PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: attempt_id.clone(),
        campaign_id: campaign_id.clone(),
        conversation_id: Id::from_str("other-conversation"),
        turn_number: 1,
    };
    let conv_err = backend_service
        .sync_autofix_attempt(
            &conv_identity,
            "must-not-write",
            QualityReport { warnings: vec![] },
        )
        .expect_err("backend cross conversation");
    assert!(matches!(
        conv_err,
        ProductionPostprocessError::ScopeMismatch {
            field: "conversation_id",
            ..
        }
    ));
    let _ = service_fail_turn(&backend, &conv_identity, conv_err);
    let after_conv = turn_store.get_turn(&turn_id).unwrap();
    assert_eq!(after_conv.conversation_id, conversation_id);
    assert_eq!(after_conv.failure_reason, None);
    assert_eq!(
        after_conv.find_attempt(&attempt_id).unwrap().draft_hash,
        original_hash
    );

    let miss_identity = PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: Id::from_str("ghost-attempt"),
        campaign_id: campaign_id.clone(),
        conversation_id: conversation_id.clone(),
        turn_number: 1,
    };
    let miss_err = backend_service
        .sync_autofix_attempt(
            &miss_identity,
            "must-not-write",
            QualityReport { warnings: vec![] },
        )
        .expect_err("backend missing attempt");
    assert!(matches!(
        miss_err,
        ProductionPostprocessError::AttemptMissing { .. }
    ));
    let _ = service_fail_turn(&backend, &miss_identity, miss_err);
    let after_miss = turn_store.get_turn(&turn_id).unwrap();
    assert_eq!(after_miss.status, TurnStatus::DraftReady);
    assert_eq!(after_miss.failure_reason, None);
    assert_eq!(
        after_miss.find_attempt(&attempt_id).unwrap().draft_hash,
        original_hash
    );

    // Concurrent supersede + late Storage/BatchConstruction from old postprocess.
    let new_attempt_id = Id::new();
    turn_store
        .with_turn_mut(&turn_id, |record| {
            if let Some(att) = record.find_attempt_mut(&attempt_id) {
                att.status = AttemptStatus::Superseded;
            }
            let mut new_att = turn_lifecycle::new_draft_attempt(
                new_attempt_id.clone(),
                Id::from_str("variant-new"),
                "regenerated",
                vec![],
            );
            new_att.status = AttemptStatus::DraftReady;
            record.attempts.push(new_att);
            record.status = TurnStatus::DraftReady;
            record.failure_reason = None;
            record.touch();
        })
        .unwrap();
    let old_identity = PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id: attempt_id.clone(),
        campaign_id: campaign_id.clone(),
        conversation_id: conversation_id.clone(),
        turn_number: 1,
    };
    backend_service
        .sync_autofix_attempt(
            &old_identity,
            "superseded-must-not-write",
            QualityReport { warnings: vec![] },
        )
        .expect("superseded is non-fatal zero-write");
    // Late Storage / BatchConstruction from old background postprocess must not Fail the Turn.
    for err in [
        ProductionPostprocessError::Storage("late attach".into()),
        ProductionPostprocessError::BatchConstruction("late batch".into()),
    ] {
        let combined = service_fail_turn(&backend, &old_identity, err);
        assert!(
            matches!(
                combined,
                ProductionPostprocessError::Storage(_)
                    | ProductionPostprocessError::BatchConstruction(_)
            ),
            "unexpected: {combined}"
        );
        let after = turn_store.get_turn(&turn_id).unwrap();
        assert_eq!(after.status, TurnStatus::DraftReady);
        assert_eq!(after.failure_reason, None);
        assert_eq!(
            after.find_attempt(&new_attempt_id).unwrap().status,
            AttemptStatus::DraftReady
        );
        assert_eq!(
            after.find_attempt(&attempt_id).unwrap().status,
            AttemptStatus::Superseded
        );
    }
    let after_super = turn_store.get_turn(&turn_id).unwrap();
    assert_eq!(
        after_super.find_attempt(&attempt_id).unwrap().draft_hash,
        original_hash
    );
    assert_eq!(after_super.conversation_id, conversation_id);

    // Cleanup process store entry so other tests are not polluted.
    let _ = turn_store.with_turn_mut(&turn_id, |r| {
        r.status = TurnStatus::Failed;
        r.failure_reason = Some("test cleanup".into());
    });
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn sqlite_postprocess_failure_never_mutates_the_injected_json_turn_store() {
    use production_postprocess::{PostprocessIdentity, ProductionPostprocessError};
    use storyforge_domain::turn::{TurnRecord, TurnStatus};

    let dir = std::env::temp_dir().join(format!(
        "storyforge-sqlite-fail-closed-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let turn_store = turn_store::TurnStore::new(&dir);
    let campaign_id = Id::from_str("sqlite-fail-closed-campaign");
    let conversation_id = Id::from_str("sqlite-fail-closed-conversation");
    let attempt_id = Id::from_str("sqlite-fail-closed-attempt");
    let mut record = TurnRecord::new(
        campaign_id.clone(),
        conversation_id.clone(),
        Id::from_str("sqlite-fail-closed-input"),
        0,
    );
    record.status = TurnStatus::DraftReady;
    record.attempts.push(turn_lifecycle::new_draft_attempt(
        attempt_id.clone(),
        Id::from_str("sqlite-fail-closed-variant"),
        "draft",
        vec![],
    ));
    let turn_id = record.turn_id.clone();
    turn_store.create_turn(record).unwrap();

    let storage = Arc::new(storage_backend::StorageFacade::new(
        dir.clone(),
        storyforge_infra_sqlite::backend::PinnedBackend::new(
            storyforge_infra_sqlite::backend::StorageBackend::Sqlite,
            storyforge_infra_sqlite::backend::BackendSource::Env,
        ),
    ));
    let sink = BackendTurnAttemptSink::for_backend_store(storage, &turn_store);
    let identity = PostprocessIdentity {
        turn_id: turn_id.clone(),
        attempt_id,
        campaign_id,
        conversation_id,
        turn_number: 1,
    };

    let combined = service_fail_turn(
        &sink,
        &identity,
        ProductionPostprocessError::Storage("forced postprocess failure".into()),
    );

    assert!(matches!(
        combined,
        ProductionPostprocessError::MarkFailed { .. }
    ));
    let unchanged = turn_store.get_turn(&turn_id).unwrap();
    assert_eq!(unchanged.status, TurnStatus::DraftReady);
    assert_eq!(unchanged.failure_reason, None);

    let _ = std::fs::remove_dir_all(dir);
}

/// 生产入口必须在无连接时 fail closed，不能把开发 Mock 当成真实模型。

#[test]
fn test_persist_temporary_instances_new() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_persist_temp_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.campaign_id = Some(campaign.id.clone());
    let persist_ctx = TemporaryInstancesPersistContext::from_writing_context(&ctx).unwrap();

    let temps = vec![
        CharacterInstance::temporary(campaign.id.clone(), "Ghost"),
        CharacterInstance::temporary_with_overrides(
            campaign.id.clone(),
            "Guard",
            Some("stern guard".into()),
            Some("block the way".into()),
        ),
    ];

    persist_temporary_instances_to_store(&store, &persist_ctx, &temps);

    let instances = store.list_instances(&campaign.id);
    assert_eq!(instances.len(), 2, "应有 2 个落盘实例");

    let ghost = instances.iter().find(|i| i.name == "Ghost").unwrap();
    assert!(ghost.is_temporary);
    assert!(ghost.persona_override.is_none());

    let guard = instances.iter().find(|i| i.name == "Guard").unwrap();
    assert!(guard.is_temporary);
    assert_eq!(guard.persona_override, Some("stern guard".into()));
    assert_eq!(guard.behavior_override, Some("block the way".into()));

    let _ = std::fs::remove_dir_all(&dir);
}

/// persist_temporary_instances：同名实例不重复落盘（去重）

#[test]
fn test_persist_temporary_instances_dedup_by_name() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_persist_dedup_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    // 先手动落盘一个 "Ghost"
    let existing = CharacterInstance::temporary(campaign.id.clone(), "Ghost");
    store.add_instance(existing.clone()).unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.campaign_id = Some(campaign.id.clone());

    // 再尝试落盘同名临时 instance
    let temps = vec![CharacterInstance::temporary(campaign.id.clone(), "Ghost")];
    persist_temporary_instances_to(&store, &ctx, &temps);

    let instances = store.list_instances(&campaign.id);
    assert_eq!(instances.len(), 1, "同名不应重复落盘");
    assert_eq!(instances[0].id, existing.id, "应保留原始实例 id");

    let _ = std::fs::remove_dir_all(&dir);
}

/// persist_temporary_instances：同一批次内的同名临时实例也不重复落盘

#[test]
fn test_persist_temporary_instances_dedup_within_batch() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_persist_batch_dedup_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.campaign_id = Some(campaign.id.clone());

    let temps = vec![
        CharacterInstance::temporary(campaign.id.clone(), "Ghost"),
        CharacterInstance::temporary_with_overrides(
            campaign.id.clone(),
            "Ghost",
            Some("duplicate brief".into()),
            None,
        ),
    ];
    persist_temporary_instances_to(&store, &ctx, &temps);

    let instances = store.list_instances(&campaign.id);
    assert_eq!(instances.len(), 1, "同批同名不应重复落盘");
    assert_eq!(instances[0].name, "Ghost");

    let _ = std::fs::remove_dir_all(&dir);
}

/// persist_temporary_instances：拒绝写入不属于当前 Campaign 的临时实例

#[test]
fn test_persist_temporary_instances_skips_wrong_campaign() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_persist_wrong_campaign_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    let other_campaign = Campaign::new(Id::from_str("card-2"), "other");
    store.save_campaign(campaign.clone()).unwrap();
    store.save_campaign(other_campaign.clone()).unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.campaign_id = Some(campaign.id.clone());

    let temps = vec![CharacterInstance::temporary(
        other_campaign.id.clone(),
        "WrongCampaignGhost",
    )];
    persist_temporary_instances_to(&store, &ctx, &temps);

    assert!(store.list_instances(&campaign.id).is_empty());
    assert!(store.list_instances(&other_campaign.id).is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

/// persist_temporary_instances：无 campaign 时 no-op

#[test]
fn test_persist_temporary_instances_no_campaign() {
    use storyforge_domain::campaign::CharacterInstance;

    let ctx = WritingContext::legacy(vec![], None, Id::new());
    // campaign_id = None → 应直接返回，不 panic
    let temps = vec![CharacterInstance::temporary(Id::new(), "Ghost")];
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_persist_no_camp_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    persist_temporary_instances_to(&store, &ctx, &temps);
    let _ = std::fs::remove_dir_all(&dir);
}

/// persist_temporary_instances：空列表 no-op

#[test]
fn test_persist_temporary_instances_empty_list() {
    use storyforge_domain::campaign::Campaign;

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_persist_empty_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.campaign_id = Some(campaign.id.clone());

    persist_temporary_instances_to(&store, &ctx, &[]);

    let instances = store.list_instances(&campaign.id);
    assert!(instances.is_empty(), "空列表不应写入任何实例");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Phase 6 集成：临时 instance 落盘后，postprocess 的知识写回能找到它

#[test]
fn test_postprocess_writes_knowledge_for_persisted_temporary() {
    use std::collections::HashSet;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::{CharacterKnowledgeUpdate, KnowledgeSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_pp_temp_knowledge_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let campaign = Campaign::new(Id::from_str("card-1"), "run");
    store.save_campaign(campaign.clone()).unwrap();

    // 模拟 persist_temporary_instances：落盘一个临时 instance
    let mut ghost = CharacterInstance::temporary(campaign.id.clone(), "Ghost");
    ghost.persona_override = Some("mysterious figure".into());
    store.add_instance(ghost.clone()).unwrap();

    // postprocess 尝试写入 Ghost 的知识（之前会因为找不到 persisted instance 而跳过）
    let update = CharacterKnowledgeUpdate {
        character_id: Id::from_str("Ghost"),
        knowledge_text: "Ghost appeared in the fog".into(),
        source: KnowledgeSource::Witnessed,
        source_character_id: None,
        pinned: false,
        broadcast: None,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    let present_ids = HashSet::from([String::from("Ghost")]);

    let entry = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign.id,
        &update,
        1,
        &present_ids,
        &HashSet::new(),
    );

    assert_eq!(
        entry.len(),
        1,
        "已落盘的临时 instance 应能被 postprocess 解析"
    );
    let entry = &entry[0];
    assert_eq!(entry.character_id, ghost.id);
    assert_eq!(entry.knowledge_text, "Ghost appeared in the fog");

    let _ = std::fs::remove_dir_all(&dir);
}

// ─── 第三轮：类型化 Patch 命令测试 ────────────────────────────────────────

/// Campaign context snapshots apply the same runtime view to writing and tools.

#[test]
fn test_meta_propose_campaign_repairs_orphan_knowledge() {
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
    use storyforge_domain::variables::default_character_variables;

    let dir =
        std::env::temp_dir().join(format!("sf_test_propose_repairs_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    // 手动设置 CAMPAIGN_STORE 指向临时目录（用 get_campaign_store 的底层）
    // 注意：CAMPAIGN_STORE 是 OnceLock，测试间会互相干扰。
    // 改用 campaign_store 直接测逻辑，不走 Tauri command 层。
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

    // 添加一条指向不存在 instance 的 knowledge（orphan）
    let orphan_knowledge = CharacterKnowledgeEntry::witnessed(
        campaign.id.clone(),
        Id::from_str("nonexistent-instance"),
        "看到了什么",
        1,
    );
    store.add_knowledge(vec![orphan_knowledge]).unwrap();

    // 用 health check 找 issues
    let definitions = store
        .get_card(&campaign.card_id)
        .map(|c| c.card.character_definitions)
        .unwrap_or_default();
    let instances = store.list_instances(&campaign.id);
    let knowledge = store.list_knowledge(&campaign.id);
    let tasks = store.list_tasks(&campaign.id);

    let snapshot = storyforge_app_meta::CampaignHealthSnapshot {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
    };
    let issues = storyforge_app_meta::check_campaign_health(&snapshot);
    assert!(!issues.is_empty(), "应发现至少一个 health issue");

    let input = storyforge_app_meta::PreviewInput {
        instances: &instances,
        definitions: &definitions,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: Some(&campaign),
    };

    let mut patches = Vec::new();
    for issue in &issues {
        if let Some(patch) = storyforge_app_meta::build_patch_for_issue(issue, &input) {
            patches.push(patch);
        }
    }

    assert!(!patches.is_empty(), "应生成至少一个 patch");

    // 检查是否包含 delete_orphan_knowledge action
    let has_delete_orphan = patches.iter().any(|p| {
        p.actions.iter().any(|a| {
            matches!(
                a,
                storyforge_app_meta::TypedPatchAction::DeleteOrphanKnowledge { .. }
            )
        })
    });
    assert!(has_delete_orphan, "应包含 delete_orphan_knowledge action");

    let state = AppState::new_for_test();
    let proposals =
        meta_propose_campaign_repairs_in_store(&store, campaign.id.as_str(), &state).unwrap();
    assert!(
        !proposals.is_empty(),
        "command helper should return patches"
    );
    {
        let typed = state
            .typed_patches
            .read()
            .unwrap_or_else(|p| p.into_inner());
        assert!(
            typed.iter().any(|p| {
                p.actions.iter().any(|a| {
                    matches!(
                        a,
                        storyforge_app_meta::TypedPatchAction::DeleteOrphanKnowledge { .. }
                    )
                })
            }),
            "command helper should persist a delete orphan knowledge patch"
        );
        assert_eq!(typed.len(), proposals.len());
    }

    let repeated =
        meta_propose_campaign_repairs_in_store(&store, campaign.id.as_str(), &state).unwrap();
    let typed = state
        .typed_patches
        .read()
        .unwrap_or_else(|p| p.into_inner());
    assert_eq!(repeated.len(), proposals.len());
    assert_eq!(
        typed.len(),
        proposals.len(),
        "repeated repair proposal should return existing pending patches without duplicating them"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// accept 一个 prune task reference patch 后，task.related_characters 不再含 orphan id

#[test]
fn test_tool_ctx_snapshot_sees_writes() {
    let state = AppState::new_for_test();

    // 初始为空
    let snap0 = state.snapshot_tool_ctx();
    assert!(snap0.characters.is_empty());
    assert!(snap0.world_info.is_none());

    // 模拟 import_character 的写入逻辑：手动构造一个 Character
    let char = Arc::new(make_test_character("TestHero"));
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.characters.push(char);
    }

    // 快照应能读到
    let snap1 = state.snapshot_tool_ctx();
    assert_eq!(snap1.characters.len(), 1);
    assert_eq!(snap1.characters[0].name, "TestHero");
}

#[test]
fn test_collect_scoped_regex_scripts_is_limited_to_selected_character() {
    let mut selected = make_test_character("Selected");
    selected.id = Id::from_str("source-selected");
    selected.extensions = serde_json::json!({
        "regex_scripts": [{
            "id": "selected-regex",
            "scriptName": "Selected regex",
            "findRegex": "foo",
            "replaceString": "bar",
            "placement": [2],
            "disabled": false
        }]
    });
    let mut other = make_test_character("Other");
    other.id = Id::from_str("source-other");
    other.extensions = serde_json::json!({
        "regex_scripts": [{
            "id": "other-regex",
            "scriptName": "Other regex",
            "findRegex": "baz",
            "replaceString": "qux",
            "placement": [2],
            "disabled": false
        }]
    });
    let characters = vec![Arc::new(selected), Arc::new(other)];

    let scripts = collect_scoped_regex_scripts(Some("source-selected"), &characters, None);

    assert_eq!(scripts.len(), 1);
    assert_eq!(scripts[0].id, "selected-regex");
    assert!(collect_scoped_regex_scripts(None, &characters, None).is_empty());
    assert!(collect_scoped_regex_scripts(Some("missing"), &characters, None).is_empty());
}

#[test]
fn test_fill_regex_context_merges_active_preset_before_scoped_scripts() {
    use storyforge_domain::Source;
    use storyforge_domain::preset::{Preset, RegexScriptSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_fill_regex_context_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let preset_store = preset_store::PresetStore::new(&dir);
    let preset_id = preset_store
        .save(Preset {
            name: "runtime preset".into(),
            prompts: vec![],
            regex_scripts: vec![test_regex_script("preset-regex", RegexScriptSource::Scoped)],
            source: Source::ImportedFromST,
        })
        .unwrap();
    assert!(preset_store.set_active(&preset_id).unwrap());

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.regex_scripts = vec![test_regex_script("scoped-regex", RegexScriptSource::Preset)];

    let global_store = global_regex_store::GlobalRegexStore::new(&dir);
    fill_regex_context(&mut ctx, &preset_store, &global_store);

    let ids: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.id.as_str())
        .collect();
    assert_eq!(ids, vec!["preset-regex", "scoped-regex"]);

    let sources: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.source)
        .collect();
    assert_eq!(
        sources,
        vec![RegexScriptSource::Preset, RegexScriptSource::Scoped]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_fill_regex_context_merges_global_before_active_preset_and_scoped() {
    use storyforge_domain::Source;
    use storyforge_domain::preset::{Preset, RegexScriptSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_global_regex_context_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let global_store = global_regex_store::GlobalRegexStore::new(&dir);
    global_store
        .replace_all(vec![test_regex_script(
            "global-regex",
            RegexScriptSource::Scoped,
        )])
        .unwrap();

    let preset_store = preset_store::PresetStore::new(&dir);
    let preset_id = preset_store
        .save(Preset {
            name: "runtime preset".into(),
            prompts: vec![],
            regex_scripts: vec![test_regex_script("preset-regex", RegexScriptSource::Scoped)],
            source: Source::ImportedFromST,
        })
        .unwrap();
    assert!(preset_store.set_active(&preset_id).unwrap());

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.regex_scripts = vec![test_regex_script("scoped-regex", RegexScriptSource::Preset)];

    fill_regex_context(&mut ctx, &preset_store, &global_store);

    let ids: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.id.as_str())
        .collect();
    assert_eq!(ids, vec!["global-regex", "preset-regex", "scoped-regex"]);

    let sources: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.source)
        .collect();
    assert_eq!(
        sources,
        vec![
            RegexScriptSource::Global,
            RegexScriptSource::Preset,
            RegexScriptSource::Scoped
        ]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_fill_campaign_runtime_adds_active_card_scoped_regex_after_preset() {
    use storyforge_app_agent::tools::ToolContext;
    use storyforge_domain::Source;
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::character::CharacterCard;
    use storyforge_domain::preset::{Preset, RegexScriptSource};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_campaign_scoped_regex_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();

    let preset_store = preset_store::PresetStore::new(&dir);
    let preset_id = preset_store
        .save(Preset {
            name: "runtime preset".into(),
            prompts: vec![],
            regex_scripts: vec![test_regex_script("preset-regex", RegexScriptSource::Scoped)],
            source: Source::ImportedFromST,
        })
        .unwrap();
    assert!(preset_store.set_active(&preset_id).unwrap());

    let campaign_store = campaign_store::CampaignStore::new(&dir);
    let card = CharacterCard {
        id: Id::from_str("card-campaign"),
        name: "Campaign Card".into(),
        source_character_id: Id::from_str("source-campaign"),
        character_definitions: vec![],
        campaign_variable_schema: vec![],
        raw_card_json: serde_json::json!({
            "name": "Campaign Card",
            "extensions": {
                "regex_scripts": [{
                    "id": "campaign-scoped-regex",
                    "scriptName": "Campaign scoped regex",
                    "findRegex": "foo",
                    "replaceString": "bar",
                    "placement": [2],
                    "disabled": false
                }]
            }
        }),
        extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    campaign_store.save_card(card.clone()).unwrap();
    let campaign = Campaign::new(card.id.clone(), "Campaign runtime");
    campaign_store.save_campaign(campaign.clone()).unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.regex_scripts = vec![test_regex_script(
        "legacy-scoped-regex",
        RegexScriptSource::Preset,
    )];
    let global_store = global_regex_store::GlobalRegexStore::new(&dir);
    global_store
        .replace_all(vec![test_regex_script(
            "global-regex",
            RegexScriptSource::Scoped,
        )])
        .unwrap();
    fill_regex_context(&mut ctx, &preset_store, &global_store);
    let tool_ctx = Arc::new(RwLock::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    }));

    fill_campaign_runtime_from_store(&mut ctx, &tool_ctx, &campaign_store, &campaign.id);

    let ids: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec![
            "global-regex",
            "preset-regex",
            "legacy-scoped-regex",
            "campaign-scoped-regex"
        ]
    );

    let sources: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.source)
        .collect();
    assert_eq!(
        sources,
        vec![
            RegexScriptSource::Global,
            RegexScriptSource::Preset,
            RegexScriptSource::Scoped,
            RegexScriptSource::Scoped
        ]
    );

    // ContextEpochSnapshot 应在 fill 时创建并落盘
    assert!(
        ctx.context_epoch.is_some(),
        "fill_campaign should freeze context_epoch"
    );
    let reloaded = campaign_store.get_campaign(&campaign.id).unwrap();
    assert!(reloaded.context_epoch.is_some());
    assert_eq!(
        reloaded.context_epoch.as_ref().unwrap().epoch_id,
        ctx.context_epoch.as_ref().unwrap().epoch_id
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_context_epoch_rollover_when_live_suffix_reaches_e() {
    use storyforge_app_agent::tools::ToolContext;
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::chronicle::{DEFAULT_E, DEFAULT_H_ANCHOR, committed_turn_id};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_epoch_rollover_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let campaign_store = campaign_store::CampaignStore::new(&dir);
    let card_id = Id::from_str("card-epoch");
    campaign_store
        .save_card(storyforge_domain::character::CharacterCard {
            id: card_id.clone(),
            name: "Epoch Card".into(),
            source_character_id: Id::from_str("src"),
            character_definitions: vec![],
            campaign_variable_schema: vec![],
            raw_card_json: serde_json::json!({}),
            extraction_status: storyforge_domain::character::CharacterExtractionStatus::Extracted,
            extraction_message: None,
        })
        .unwrap();
    let campaign = Campaign::new(card_id, "epoch-camp");
    let camp_id = campaign.id.clone();
    campaign_store.save_campaign(campaign).unwrap();

    // 先写入 H_anchor 条摘要 → 创建 epoch（head=H, live=0）
    for t in 1..=DEFAULT_H_ANCHOR {
        campaign_store
            .add_summary(
                storyforge_domain::agent::RoundSummary::new(
                    camp_id.clone(),
                    Id::from_str("conv"),
                    t,
                    format!("round {t}"),
                )
                .with_code(format!("A{t:04}"))
                .with_headline(format!("h{t}")),
            )
            .unwrap();
    }
    let tool_ctx = Arc::new(RwLock::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    }));
    let mut ctx = WritingContext::legacy(vec![], None, Id::from_str("conv"));
    fill_campaign_runtime_from_store(&mut ctx, &tool_ctx, &campaign_store, &camp_id);
    let epoch1 = ctx.context_epoch.clone().expect("epoch after first fill");
    assert_eq!(
        epoch1.source_head_turn_id,
        Some(committed_turn_id(DEFAULT_H_ANCHOR))
    );
    let rev1 = ctx.chronicle_revision;

    // 再追加 E 条 → 下次 fill 应 rollover
    for t in (DEFAULT_H_ANCHOR + 1)..=(DEFAULT_H_ANCHOR + DEFAULT_E) {
        campaign_store
            .add_summary(
                storyforge_domain::agent::RoundSummary::new(
                    camp_id.clone(),
                    Id::from_str("conv"),
                    t,
                    format!("round {t}"),
                )
                .with_code(format!("A{t:04}"))
                .with_headline(format!("h{t}")),
            )
            .unwrap();
    }
    let mut ctx2 = WritingContext::legacy(vec![], None, Id::from_str("conv"));
    fill_campaign_runtime_from_store(&mut ctx2, &tool_ctx, &campaign_store, &camp_id);
    let epoch2 = ctx2
        .context_epoch
        .clone()
        .expect("epoch after rollover fill");
    assert_ne!(
        epoch1.epoch_id, epoch2.epoch_id,
        "rollover must new epoch_id"
    );
    assert_eq!(
        epoch2.source_head_turn_id,
        Some(committed_turn_id(DEFAULT_H_ANCHOR + DEFAULT_E))
    );
    assert!(
        ctx2.chronicle_revision > rev1,
        "rollover should bump chronicle_revision"
    );
    // 同 epoch 再 fill 应稳定
    let mut ctx3 = WritingContext::legacy(vec![], None, Id::from_str("conv"));
    fill_campaign_runtime_from_store(&mut ctx3, &tool_ctx, &campaign_store, &camp_id);
    assert_eq!(
        ctx3.context_epoch.as_ref().unwrap().epoch_id,
        epoch2.epoch_id
    );
    assert_eq!(
        ctx3.context_epoch.as_ref().unwrap().source_hash,
        epoch2.source_hash
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_append_campaign_scoped_regex_skips_existing_scoped_id() {
    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    ctx.regex_scripts = vec![test_regex_script("same-scoped", RegexScriptSource::Scoped)];

    append_missing_campaign_scoped_regex_scripts(
        &mut ctx,
        vec![
            test_regex_script("same-scoped", RegexScriptSource::Scoped),
            test_regex_script("new-scoped", RegexScriptSource::Scoped),
        ],
    );

    let ids: Vec<_> = ctx
        .regex_scripts
        .iter()
        .map(|script| script.id.as_str())
        .collect();
    assert_eq!(ids, vec!["same-scoped", "new-scoped"]);
}

#[tokio::test]
async fn test_load_preset_for_classification_async_returns_preset() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_preset_classify_async_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store: &'static PresetStore = Box::leak(Box::new(PresetStore::new(&dir)));
    let preset_id = store
        .save(storyforge_domain::preset::Preset {
            name: "classify preset".into(),
            prompts: vec![],
            regex_scripts: vec![test_regex_script(
                "classify-regex",
                RegexScriptSource::Preset,
            )],
            source: storyforge_domain::Source::ImportedFromST,
        })
        .unwrap();

    let stored = load_preset_for_classification_async(store, preset_id.clone())
        .await
        .unwrap();

    assert_eq!(stored.id, preset_id);
    assert_eq!(stored.preset.name, "classify preset");
    assert_eq!(stored.preset.regex_scripts[0].id, "classify-regex");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn test_load_preset_for_classification_async_missing_returns_not_found() {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_preset_classify_missing_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store: &'static PresetStore = Box::leak(Box::new(PresetStore::new(&dir)));

    let err = load_preset_for_classification_async(store, "missing-preset".into())
        .await
        .unwrap_err();

    assert!(matches!(err, TauriCommandError::NotFound { .. }));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_last_user_intent_before_finds_nearest_user() {
    let state = AppState::new_for_test();
    let conv = state.conv_store.create(None, None);
    let _u1 = state
        .conv_store
        .append_user_message(&conv.id, "第一次意图".into())
        .unwrap();
    let _a1 = state
        .conv_store
        .append_ai_draft(&conv.id, "成文1".into(), None)
        .unwrap();
    let u2 = state
        .conv_store
        .append_user_message(&conv.id, "第二次意图".into())
        .unwrap();
    let a2 = state
        .conv_store
        .append_ai_draft(&conv.id, "成文2".into(), None)
        .unwrap();
    let intent = last_user_intent_before(&state.conv_store, &conv.id, &a2);
    assert_eq!(intent.as_deref(), Some("第二次意图"));
    // before 第二条 user 节点 → 取第一次意图
    assert_eq!(
        last_user_intent_before(&state.conv_store, &conv.id, &u2).as_deref(),
        Some("第一次意图")
    );
    // before 首条 user → 无更早 user
    assert!(last_user_intent_before(&state.conv_store, &conv.id, &_u1).is_none());
    let _ = state.conv_store.delete(&conv.id);
}

/// RoundSummary accept 后索引进向量库，并可被远记忆关键词召回。

#[test]
fn test_index_round_summary_to_far_memory() {
    let store = BruteForceStore::new();
    let camp_id = Id::from_str("camp-fm");
    let conv_id = Id::from_str("conv-fm");
    let summary = storyforge_domain::agent::RoundSummary::new(
        camp_id.clone(),
        conv_id,
        7,
        "昨夜有人潜入诊所，陈警官随后上门调查。".into(),
    );
    let summary_id = summary.id.clone();
    let mut batch = storyforge_domain::turn::MutationBatch::new(Id::new(), 0);
    batch
        .mutations
        .push(storyforge_domain::turn::Mutation::UpsertSummary(Box::new(
            summary,
        )));

    index_round_summaries_to_vector(&store, &batch);

    let hits = storyforge_app_memory::recall_archived_by_query_filtered(
        &store,
        "诊所",
        5,
        Some("camp-fm"),
    )
    .unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].content.contains("潜入诊所"));
    assert_eq!(hits[0].kind, "ArchivedSummary");

    // 幂等：同一 id 再索引不复制
    index_round_summaries_to_vector(&store, &batch);
    assert_eq!(store.count(), 1);
    let _ = store.delete(&summary_id);
}

// ─── Phase 2: CampaignRuntimeContext 快照接入验证 ────────────────────────

/// 初始状态下 tool_ctx 的 campaign_runtime 应为 None（未开 Campaign）

#[test]
fn test_campaign_runtime_none_by_default() {
    let state = AppState::new_for_test();
    let snap = state.snapshot_tool_ctx();
    assert!(
        snap.campaign_runtime.is_none(),
        "初始状态 campaign_runtime 应为 None"
    );
}

/// 写入 CampaignRuntimeContext 后，快照应能读到
/// （模拟 fill_campaign_context 的同步机制）

#[test]
fn test_campaign_runtime_synced_through_rwlock() {
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

    let state = AppState::new_for_test();

    // 构造一个最小的 CampaignRuntimeContext
    let campaign = Campaign::new(Id::from_str("test-card"), "test-run");
    let runtime = Arc::new(CampaignRuntimeContext {
        campaign,
        instances: vec![],
        definitions_by_id: std::collections::HashMap::new(),
        knowledge: vec![],
        tasks: vec![],
        turn: 1,
    });

    // 写入 tool_ctx（模拟 fill_campaign_context 的行为）
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.campaign_runtime = Some(runtime.clone());
    }

    // 快照应能读到
    let snap = state.snapshot_tool_ctx();
    assert!(
        snap.campaign_runtime.is_some(),
        "写入后 campaign_runtime 不应为 None"
    );
    let rt = snap.campaign_runtime.as_ref().unwrap();
    assert_eq!(rt.turn, 1);
    assert!(rt.instances.is_empty());
    assert_eq!(rt.campaign.name, "test-run");
}

/// CampaignRuntimeContext 写入 instances/definitions/knowledge 后，
/// 通过快照可完整读回

#[test]
fn test_campaign_runtime_full_snapshot_readable() {
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
    use storyforge_domain::variables::default_character_variables;

    let state = AppState::new_for_test();

    let campaign = Campaign::new(Id::from_str("card-1"), "full-test");

    let def = CharacterDefinition {
        id: Id::from_str("def-lin"),
        card_id: Id::from_str("card-1"),
        name: "Lin".into(),
        persona_prompt: "calm surgeon".into(),
        behavior_rules: "save first".into(),
        base_backstory: vec!["is a surgeon".into()],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: default_character_variables(),
    };

    let instance = storyforge_domain::campaign::CharacterInstance {
        id: Id::from_str("inst-lin"),
        campaign_id: campaign.id.clone(),
        definition_id: Some(def.id.clone()),
        name: "Lin".into(),
        persona_override: None,
        behavior_override: None,
        variables: vec![],
        is_temporary: false,
    };

    let knowledge = CharacterKnowledgeEntry::backstory(
        campaign.id.clone(),
        Id::from_str("inst-lin"),
        "我是外科医生",
    );

    let mut definitions_by_id = std::collections::HashMap::new();
    definitions_by_id.insert(def.id.clone(), def);

    let runtime = Arc::new(CampaignRuntimeContext {
        campaign,
        instances: vec![instance],
        definitions_by_id,
        knowledge: vec![knowledge],
        tasks: vec![],
        turn: 3,
    });

    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.campaign_runtime = Some(runtime);
    }

    let snap = state.snapshot_tool_ctx();
    let rt = snap.campaign_runtime.as_ref().unwrap();
    assert_eq!(rt.turn, 3);
    assert_eq!(rt.instances.len(), 1);
    assert_eq!(rt.instances[0].name, "Lin");
    assert_eq!(rt.definitions_by_id.len(), 1);
    assert!(rt.definitions_by_id.contains_key(&Id::from_str("def-lin")));
    assert_eq!(rt.knowledge.len(), 1);
    assert_eq!(rt.knowledge[0].knowledge_text, "我是外科医生");

    // 验证 helper 可用
    let inst = rt.find_instance_by_id_or_name("Lin").unwrap();
    assert_eq!(rt.resolved_persona_for(inst), Some("calm surgeon"));
    assert_eq!(rt.resolved_behavior_for(inst), Some("save first"));
}

/// 验证 fill_campaign_context 在无 active campaign 时会清空旧 runtime
/// （stale runtime cleanup：防止上一轮的脏快照残留）

#[test]
fn test_fill_campaign_context_clears_stale_runtime() {
    use storyforge_domain::campaign::Campaign;
    use storyforge_domain::campaign_runtime::CampaignRuntimeContext;

    let state = AppState::new_for_test();

    // 模拟上一轮残留：手动写入一个 runtime
    let campaign = Campaign::new(Id::from_str("stale-card"), "stale-run");
    let stale_runtime = Arc::new(CampaignRuntimeContext {
        campaign,
        instances: vec![],
        definitions_by_id: std::collections::HashMap::new(),
        knowledge: vec![],
        tasks: vec![],
        turn: 99,
    });
    {
        let mut ctx = state.tool_ctx.write().unwrap_or_else(|p| p.into_inner());
        ctx.campaign_runtime = Some(stale_runtime);
    }

    // 确认写入成功
    let snap_before = state.snapshot_tool_ctx();
    assert!(
        snap_before.campaign_runtime.is_some(),
        "预置 stale runtime 应成功"
    );

    // 调用 fill_campaign_context（无 active campaign → early return，但 runtime 应被清空）
    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    fill_campaign_context(&mut ctx, &state);

    // 验证：WritingContext 的 runtime 应为 None
    assert!(
        ctx.campaign_runtime.is_none(),
        "无 active campaign 时 ctx.campaign_runtime 应被清空"
    );

    // 验证：tool_ctx 的 runtime 也应被清空
    let snap_after = state.snapshot_tool_ctx();
    assert!(
        snap_after.campaign_runtime.is_none(),
        "无 active campaign 时 tool_ctx.campaign_runtime 应被清空"
    );
}

// ─── Phase 6：临时 instance 落盘测试 ─────────────────────────────────────

/// persist_temporary_instances：新实例能落盘，下一轮 list_instances 可读回

#[test]
fn test_campaign_context_snapshot_applies_runtime_to_contexts() {
    use storyforge_domain::agent::RoundSummary;
    use storyforge_domain::campaign::{Campaign, CharacterInstance};
    use storyforge_domain::character_knowledge::CharacterKnowledgeEntry;
    use storyforge_domain::story_task::{StoryTask, TaskTrigger};

    let dir = std::env::temp_dir().join(format!(
        "storyforge_test_campaign_snapshot_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = campaign_store::CampaignStore::new(&dir);
    let mut campaign = Campaign::new(Id::from_str("card-1"), "run");
    campaign.story_clock = "Day 3".into();
    store.save_campaign(campaign.clone()).unwrap();

    let instance = CharacterInstance::temporary(campaign.id.clone(), "Ghost");
    store.add_instance(instance.clone()).unwrap();
    store
        .add_knowledge(vec![CharacterKnowledgeEntry::witnessed(
            campaign.id.clone(),
            instance.id.clone(),
            "Ghost saw the gate",
            1,
        )])
        .unwrap();
    store
        .add_task(StoryTask::user_planned(
            campaign.id.clone(),
            "Open the gate",
            "The gate must open later",
            vec![TaskTrigger::TurnReminder { at_turn: 2 }],
            1,
        ))
        .unwrap();
    store
        .add_summary(RoundSummary::new(
            campaign.id.clone(),
            Id::new(),
            1,
            "A previous turn happened".into(),
        ))
        .unwrap();

    let mut ctx = WritingContext::legacy(vec![], None, Id::new());
    let tool_ctx = Arc::new(RwLock::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    }));

    let snapshot = load_campaign_context_snapshot(&store, &campaign.id).unwrap();
    apply_campaign_context_snapshot(&mut ctx, &tool_ctx, snapshot);

    assert_eq!(ctx.campaign_id, Some(campaign.id.clone()));
    assert_eq!(ctx.story_clock, "Day 3");
    assert_eq!(ctx.turn, 2);
    assert_eq!(ctx.pending_tasks.len(), 1);
    // ContextCompiler 最小版：RoundSummary 进入 WritingContext + ToolContext
    assert_eq!(ctx.recent_summaries.len(), 1);
    assert_eq!(ctx.recent_summaries[0].content, "A previous turn happened");
    let runtime = ctx.campaign_runtime.as_ref().unwrap();
    assert_eq!(runtime.instances.len(), 1);
    assert_eq!(runtime.knowledge.len(), 1);
    assert_eq!(runtime.tasks.len(), 1);
    assert_eq!(runtime.turn, 2);

    let tool_guard = tool_ctx.read().unwrap_or_else(|p| p.into_inner());
    let tool_runtime = tool_guard.campaign_runtime.clone().unwrap();
    assert_eq!(tool_runtime.campaign.id, campaign.id);
    assert_eq!(tool_runtime.instances[0].id, instance.id);
    assert_eq!(
        tool_guard.archived_summaries,
        vec!["A previous turn happened"]
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_take_recent_summaries_for_context_keeps_last_k() {
    let campaign_id = Id::from_str("camp-load-k");
    let summaries: Vec<_> = (1..=15)
        .map(|turn| {
            storyforge_domain::agent::RoundSummary::new(
                campaign_id.clone(),
                Id::new(),
                turn,
                format!("summary-{turn}"),
            )
        })
        .collect();
    let kept = take_recent_summaries_for_context(summaries, 12);
    assert_eq!(kept.len(), 12);
    assert_eq!(kept.first().unwrap().turn, 4);
    assert_eq!(kept.last().unwrap().turn, 15);
    assert_eq!(kept.last().unwrap().content, "summary-15");
}

#[test]
fn test_take_recent_summaries_for_context_short_list_unchanged() {
    let campaign_id = Id::from_str("camp-load-short");
    let summaries = vec![storyforge_domain::agent::RoundSummary::new(
        campaign_id,
        Id::new(),
        1,
        "only".into(),
    )];
    let kept = take_recent_summaries_for_context(summaries, 12);
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].content, "only");
}

#[test]
fn test_next_writing_turn_ignores_stage_summaries() {
    let camp = Id::from_str("camp-turn");
    let mut items = Vec::new();
    for turn in 1..=3 {
        items.push(
            storyforge_domain::agent::RoundSummary::new(
                camp.clone(),
                Id::new(),
                turn,
                format!("a{turn}"),
            )
            .with_code(format!("A{turn:04}")),
        );
    }
    let mut b = storyforge_domain::agent::RoundSummary::new(camp, Id::new(), 1, "stage".into())
        .with_code("B0001");
    b.level = 1;
    b.turn_end = 3;
    items.push(b);
    assert_eq!(committed_turn_count(&items), 3);
    assert_eq!(next_writing_turn(&items), 4);
    // 若错误用 len：会得到 5
    assert_ne!(items.len() as u32 + 1, next_writing_turn(&items));
}

#[test]
fn test_build_chronicle_prompt_catalog_keeps_far_codes() {
    let camp = Id::from_str("camp-pc");
    let mut items = Vec::new();
    for turn in 1..=20 {
        items.push(
            storyforge_domain::agent::RoundSummary::new(
                camp.clone(),
                Id::new(),
                turn,
                format!("a{turn}"),
            )
            .with_code(format!("A{turn:04}")),
        );
    }
    use storyforge_domain::chronicle::{ChronicleCode, ContextEpochSnapshot};
    let mut snap = ContextEpochSnapshot::new_empty("e1", 0);
    snap.overview_codes = vec![ChronicleCode::parse("A0001").unwrap()];
    snap.band_codes = vec![ChronicleCode::parse("A0010").unwrap()];
    let cat = build_chronicle_prompt_catalog(&items, Some(&snap));
    assert!(cat.iter().any(|s| s.code.as_deref() == Some("A0001")));
    assert!(cat.iter().any(|s| s.code.as_deref() == Some("A0010")));
    // all leaves included
    assert!(cat.iter().filter(|s| s.is_leaf_a()).count() >= 20);
}

#[test]
fn test_build_chronicle_tool_catalog_prefers_stages_and_recent_leaves() {
    let campaign_id = Id::from_str("camp-catalog");
    let mut items = Vec::new();
    for turn in 1..=20 {
        items.push(
            storyforge_domain::agent::RoundSummary::new(
                campaign_id.clone(),
                Id::new(),
                turn,
                format!("leaf-{turn}"),
            )
            .with_code(format!("A{turn:04}")),
        );
    }
    let mut stage =
        storyforge_domain::agent::RoundSummary::new(campaign_id, Id::new(), 1, "stage-b".into())
            .with_code("B0001")
            .with_headline("stage");
    stage.level = 1;
    stage.turn_end = 8;
    items.push(stage);

    let kept = build_chronicle_tool_catalog(items, 10);
    assert_eq!(kept.len(), 10);
    assert!(kept.iter().any(|s| s.code.as_deref() == Some("B0001")));
    // remaining 9 slots are latest leaves
    let leaf_turns: Vec<u32> = kept
        .iter()
        .filter(|s| s.level == 0)
        .map(|s| s.turn)
        .collect();
    assert_eq!(leaf_turns, (12..=20).collect::<Vec<_>>());
}
#[tokio::test]
async fn start_writing_command_prompt_hook_messages_reach_mock_llm() {
    let marker = "START_WRITING_COMMAND_HOOK_MARKER";
    let llm = Arc::new(RecordingMockLlm::new(vec![
        mock_chat_response(plan_response_json()),
        mock_chat_response("Seraphina performs a short beat."),
        mock_chat_response("Final draft from start_writing command."),
    ]));
    let app_state = state_with_recording_llm(llm.clone());
    let channel = command_prompt_hook_channel(app_state.clone(), marker);

    start_writing(
        "Write a tiny command hook test scene.".into(),
        None,
        None,
        None,
        None,
        tauri_state_for_test(&app_state),
        channel,
    )
    .await
    .expect("start_writing should complete with recording mock LLM");

    assert!(
        any_recorded_request_contains_marker(&llm, marker),
        "mock LLM should receive marker appended by command prompt hook"
    );
}

#[tokio::test]
async fn regenerate_command_prompt_hook_messages_reach_mock_llm() {
    let setup_marker = "REGENERATE_SETUP_HOOK_MARKER";
    let regenerate_marker = "REGENERATE_COMMAND_HOOK_MARKER";
    let llm = Arc::new(RecordingMockLlm::new(vec![
        mock_chat_response(plan_response_json()),
        mock_chat_response("Seraphina performs a short setup beat."),
        mock_chat_response("Initial draft for regenerate command."),
        mock_chat_response("Regenerated draft from command hook test."),
    ]));
    let app_state = state_with_recording_llm(llm.clone());

    let setup = start_writing(
        "Write setup text for regenerate.".into(),
        None,
        None,
        None,
        Some(storyforge_domain::generation::GenerationMode::BigScene),
        tauri_state_for_test(&app_state),
        command_prompt_hook_channel(app_state.clone(), setup_marker),
    )
    .await
    .expect("start_writing setup should complete");
    let conversation_id = setup["conversation_id"]
        .as_str()
        .expect("start_writing result should include conversation_id")
        .to_string();
    let node_id = setup["node_id"]
        .as_str()
        .expect("start_writing result should include node_id")
        .to_string();
    llm.clear_requests();

    regenerate(
        RegenerateRequestDto {
            conversation_id,
            node_id,
            targets: vec![RegenerateTargetDto {
                kind: "editor".into(),
            }],
            generation_mode: Some(storyforge_domain::generation::GenerationMode::BigScene),
            hint: Some("Keep it brief.".into()),
            seed: None,
        },
        tauri_state_for_test(&app_state),
        command_prompt_hook_channel(app_state.clone(), regenerate_marker),
    )
    .await
    .expect("regenerate should complete with recording mock LLM");

    assert!(
        any_recorded_request_contains_marker(&llm, regenerate_marker),
        "mock LLM should receive marker appended by regenerate command prompt hook"
    );
}
