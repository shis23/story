//! C1-C8 命令层「点遍各按钮」测试（混合：确定性 + 真实 LLM）。
//!
//! 绕开前端，调遍前端会调的 Tauri 命令对应的底层逻辑。
//! 确定性项直接跑，真实 LLM 项 `#[ignore]`。

use std::sync::Arc;

use storyforge_domain::Id;
use storyforge_domain::character::{CharacterCard, CharacterDefinition};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
use storyforge_domain::story_task::{StoryTask, TaskSource, TaskStatus};
use storyforge_infra_llm::LlmClient;
use storyforge_infra_llm::mock_client::MockLlmClient;

use harness_real_llm::HarnessEnv;

// ─── C1：导入 / 识别 ──────────────────────────────────────────────────────

/// C1：import_character → inject → 验证 tool_ctx 中有角色
#[test]
fn c1_import_and_list_characters() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    use storyforge_domain::Source;
    use storyforge_domain::character::Character;
    let ch = Character {
        id: Id::from_str("c1-src"),
        name: "C1-TestChar".into(),
        description: "测试角色".into(),
        personality: "冷静".into(),
        scenario: String::new(),
        first_mes: "你好".into(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        alternate_greetings: vec![],
        embedded_world_info: None,
        extensions: serde_json::Value::Null,
        renderable_assets: None,
        source: Source::Native,
        spec_version: "3.0".into(),
        raw_card_json: serde_json::Value::Null,
    };
    env.inject_character(ch);

    let ctx = env.tool_ctx.read().unwrap();
    assert_eq!(ctx.characters.len(), 1);
    assert_eq!(ctx.characters[0].name, "C1-TestChar");
}

/// C1：extract_characters（真实 LLM）+ list_cards + get_card
#[tokio::test]
#[ignore = "需要真实 LLM 凭证"]
async fn c1_extract_characters_real_llm() {
    let llm: Arc<dyn LlmClient> = harness_real_llm::require_real_llm();
    let env = HarnessEnv::new(llm);

    let card_path = find_fixture("test-card-seraphina.png");
    let bytes = std::fs::read(&card_path)
        .unwrap_or_else(|e| panic!("读不到 fixture {}: {e}", card_path.display()));
    let character = storyforge_infra_import::import_character(&bytes).expect("导入失败");
    let source_id = character.id.clone();
    env.inject_character(character);

    let card = env.extract_characters(source_id.as_str()).await;
    assert!(
        !card.character_definitions.is_empty(),
        "应识别出至少 1 个角色"
    );

    // list_cards
    let cards = env.campaign_store.list_cards();
    assert!(
        cards.iter().any(|c| c.card.id == card.id),
        "list_cards 应含此卡"
    );

    // get_card
    let got = env.campaign_store.get_card(&card.id);
    assert!(got.is_some(), "get_card 应返回此卡");

    env.cleanup();
}

// ─── C2：Campaign 生命周期 ─────────────────────────────────────────────────

/// C2：create → list → get → list_instances → get_instance
#[test]
fn c2_campaign_lifecycle() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    let card = make_minimal_card("c2-card");
    env.campaign_store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "c2-campaign");

    assert_eq!(env.active_campaign_id(), Some(campaign_id.clone()));

    // list campaigns
    let campaigns = env.campaign_store.list_campaigns();
    assert!(campaigns.iter().any(|c| c.id == campaign_id));

    // get campaign
    let camp = env.campaign_store.get_campaign(&campaign_id);
    assert!(camp.is_some());
    assert_eq!(camp.unwrap().name, "c2-campaign");

    // list_instances
    let instances = env.campaign_store.list_instances(&campaign_id);
    assert!(!instances.is_empty(), "应有至少 1 个 instance");

    // get_instance（takes campaign_id + instance_id）
    let inst_id = instances[0].id.clone();
    let inst = env.campaign_store.get_instance(&campaign_id, &inst_id);
    assert!(inst.is_some());

    env.cleanup();
}

/// C2：set_variable on CharacterInstance
#[test]
fn c2_character_variables() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    let card = make_minimal_card("c2-var-card");
    env.campaign_store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "c2-var-campaign");
    let instances = env.campaign_store.list_instances(&campaign_id);
    let inst_id = instances[0].id.clone();

    // set_variable (key, serde_json::Value, turn)
    {
        let mut inst = env
            .campaign_store
            .get_instance(&campaign_id, &inst_id)
            .unwrap();
        inst.set_variable("mood", serde_json::json!("calm"), 1);
        env.campaign_store.update_instance(inst).unwrap();
    }

    let inst = env
        .campaign_store
        .get_instance(&campaign_id, &inst_id)
        .unwrap();
    let mood = inst.get_variable("mood");
    assert!(mood.is_some(), "mood 变量应已设置");
    assert_eq!(mood.unwrap(), &serde_json::json!("calm"));

    env.cleanup();
}

/// C2：set_variable on Campaign
#[test]
fn c2_campaign_variables() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    let card = make_minimal_card("c2-camp-var-card");
    env.campaign_store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "c2-camp-var");

    {
        let mut camp = env.campaign_store.get_campaign(&campaign_id).unwrap();
        camp.set_variable("weather", serde_json::json!("rainy"), 1);
        env.campaign_store.update_campaign(camp).unwrap();
    }

    let camp = env.campaign_store.get_campaign(&campaign_id).unwrap();
    let weather = camp.get_variable("weather");
    assert!(weather.is_some(), "weather 变量应已设置");
    assert_eq!(weather.unwrap(), &serde_json::json!("rainy"));

    env.cleanup();
}

/// C2：add_task / update_task / list_tasks
#[test]
fn c2_task_lifecycle() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    let card = make_minimal_card("c2-task-card");
    env.campaign_store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "c2-task");

    let task = StoryTask {
        id: Id::new(),
        campaign_id: campaign_id.clone(),
        title: "找到线索".into(),
        description: "在废弃仓库中找到关键证据".into(),
        triggers: vec![],
        status: TaskStatus::Pending,
        created_turn: 1,
        related_characters: vec![],
        source: TaskSource::UserPlanned,
        injected_turns: vec![],
    };
    env.campaign_store.add_task(task.clone()).unwrap();

    let tasks = env.campaign_store.list_tasks(&campaign_id);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "找到线索");
    assert!(matches!(tasks[0].status, TaskStatus::Pending));

    // update task status
    let mut updated = tasks[0].clone();
    updated.status = TaskStatus::Completed;
    env.campaign_store.update_task(updated).unwrap();

    let tasks = env.campaign_store.list_tasks(&campaign_id);
    assert!(matches!(tasks[0].status, TaskStatus::Completed));

    env.cleanup();
}

/// C2：list_summaries（空 campaign 应返回空）
#[test]
fn c2_list_summaries_empty() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    let card = make_minimal_card("c2-sum-card");
    env.campaign_store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "c2-sum");

    let summaries = env.campaign_store.list_summaries(&campaign_id);
    assert!(summaries.is_empty(), "新 campaign 应无 round summaries");

    env.cleanup();
}

/// C2：knowledge CRUD
#[test]
fn c2_knowledge_crud() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    let card = make_minimal_card("c2-know-card");
    env.campaign_store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "c2-know");
    let instances = env.campaign_store.list_instances(&campaign_id);
    let inst_id = instances[0].id.clone();

    let entry = CharacterKnowledgeEntry {
        id: Id::new(),
        campaign_id: campaign_id.clone(),
        character_id: inst_id.clone(),
        knowledge_text: "角色有一个秘密".into(),
        source: KnowledgeSource::Backstory,
        source_character_id: None,
        source_knowledge_id: None,
        turn_number: 0,
        event_id: None,
        pinned: false,
        propagation: storyforge_domain::character_knowledge::PropagationPolicy::Open,
    };
    env.campaign_store.add_knowledge(vec![entry]).unwrap();

    let knowledge = env.campaign_store.list_knowledge(&campaign_id);
    assert_eq!(knowledge.len(), 1);
    assert_eq!(knowledge[0].knowledge_text, "角色有一个秘密");

    // list_knowledge_of
    let char_knowledge = env.campaign_store.list_knowledge_of(&campaign_id, &inst_id);
    assert_eq!(char_knowledge.len(), 1);

    env.cleanup();
}

// ─── C5：对话变体 ─────────────────────────────────────────────────────────

/// C5：ConversationStore — create / append / add_variant / switch_variant / edit_variant
#[test]
fn c5_conversation_variants() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    // create conversation
    let conv = env.conv_store.create(None, None);
    let conv_id = conv.id.clone();
    assert!(env.conv_store.get(&conv_id).is_some());

    // append a user message + ai draft to create nodes
    let _user_node_id = env
        .conv_store
        .append_user_message(&conv_id, "你好".into())
        .unwrap();
    let ai_node_id = env
        .conv_store
        .append_ai_draft(&conv_id, "第一版回复".into(), None)
        .unwrap();

    // verify nodes exist
    let conv = env.conv_store.get(&conv_id).unwrap();
    assert!(conv.find_node(&ai_node_id).is_some());
    let node = conv.find_node(&ai_node_id).unwrap();
    assert_eq!(node.variants.len(), 1);
    assert_eq!(node.variants[0].content, "第一版回复");

    // add_variant（regenerate 产生新分支）
    let new_idx = env
        .conv_store
        .add_variant(&conv_id, &ai_node_id, "第二版回复".into(), None)
        .unwrap();
    assert_eq!(new_idx, 1);

    let conv = env.conv_store.get(&conv_id).unwrap();
    let node = conv.find_node(&ai_node_id).unwrap();
    assert_eq!(node.variants.len(), 2);

    // switch_variant
    env.conv_store
        .switch_variant(&conv_id, &ai_node_id, 0)
        .unwrap();
    let conv = env.conv_store.get(&conv_id).unwrap();
    let node = conv.find_node(&ai_node_id).unwrap();
    assert_eq!(node.active_variant, 0);

    // edit_variant
    env.conv_store
        .edit_variant(&conv_id, &ai_node_id, "编辑后的内容".into())
        .unwrap();
    let conv = env.conv_store.get(&conv_id).unwrap();
    let node = conv.find_node(&ai_node_id).unwrap();
    assert_eq!(node.variants[0].content, "编辑后的内容");

    // recent_messages
    let msgs = env.conv_store.recent_messages(&conv_id, 10);
    assert!(!msgs.is_empty(), "recent_messages 应非空");

    env.cleanup();
}

// ─── C6：Meta 流（确定性部分）─────────────────────────────────────────────

/// C6：check_campaign_health（零 LLM）
#[test]
fn c6_health_check() {
    use storyforge_app_meta::{CampaignHealthSnapshot, check_campaign_health};

    let snapshot = CampaignHealthSnapshot {
        instances: &[],
        definitions: &[],
        knowledge: &[],
        tasks: &[],
    };
    let issues = check_campaign_health(&snapshot);
    // 空 campaign 不应有 error 级别 issue（可能有 warning）
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| matches!(i.severity, storyforge_app_meta::IssueSeverity::Error))
        .collect();
    assert!(errors.is_empty(), "空 campaign 不应有 error: {:?}", errors);
}

/// C6：Meta 对话真实 LLM（#[ignore]）
#[tokio::test]
#[ignore = "需要真实 LLM 凭证"]
async fn c6_meta_chat_real_llm() {
    use storyforge_app_agent::runtime::AgentRuntime;
    use storyforge_app_agent::tools::ToolContext;
    use storyforge_app_meta::{MetaConversation, MetaMessage, MetaSession, meta_chat};

    let llm = harness_real_llm::require_real_llm();
    let tool_ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let runtime = AgentRuntime::new(llm, tool_ctx);

    let session = Arc::new(MetaSession::new());
    let mut conv = MetaConversation::new();
    let (_tx, cancel) = tokio::sync::watch::channel(false);

    let turn = meta_chat(
        &runtime,
        &mut conv,
        session,
        "你好，请简要介绍一下你能做什么",
        cancel,
        tokio::sync::mpsc::unbounded_channel::<String>().0,
    )
    .await
    .expect("meta_chat 应成功");

    // Agent 消息非空
    match &turn.agent_message {
        MetaMessage::Agent { content, .. } => {
            assert!(!content.is_empty(), "Agent 回复不应为空");
        }
        _ => panic!("应为 Agent 消息"),
    }

    // 对话历史有 2 条（user + agent）
    assert_eq!(conv.messages.len(), 2);
}

/// C6：PatchStore 生命周期（确定性）
#[test]
fn c6_patch_store_lifecycle() {
    use storyforge_app_meta::PatchAction;
    use storyforge_app_meta::PatchStore;

    let store = PatchStore::new();

    // propose → pending 含该项
    let patch = store.propose(
        "测试补丁".into(),
        vec![PatchAction::Update {
            target: "world_info[0]".into(),
            field: "keys".into(),
            value: serde_json::json!(["测试"]),
        }],
    );
    let patch_id = patch.id.clone();
    assert!(!patch.applied);
    assert_eq!(store.pending().len(), 1);

    // accept → pending 空
    store.accept(&patch_id).unwrap();
    assert!(store.pending().is_empty());

    // 再 propose 一个 → dismiss → pending 空
    let patch2 = store.propose(
        "第二个补丁".into(),
        vec![PatchAction::Delete {
            target: "world_info[1]".into(),
        }],
    );
    let patch2_id = patch2.id.clone();
    assert_eq!(store.pending().len(), 1);

    store.dismiss(&patch2_id).unwrap();
    assert!(store.pending().is_empty());

    // accept 不存在的 id → Err
    assert!(store.accept("nonexistent").is_err());
}

/// C6：typed_patch preview + apply（确定性）
#[test]
fn c6_typed_patch_preview_apply() {
    use storyforge_app_meta::{
        HealthIssue, IssueSeverity, PreviewInput, PreviewInputMut, TypedPatchStatus,
        apply_to_snapshot, build_patch_for_issue, is_patch_stale,
    };
    use storyforge_domain::Id;
    use storyforge_domain::campaign::CharacterInstance;
    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::default_character_variables;

    // 构造一个 orphan_instance 场景：instance 指向不存在的 definition
    let campaign_id = Id::from_str("camp-1");
    let def_id = Id::from_str("def-exist");
    let orphan_def_id = Id::from_str("def-missing");

    let def = CharacterDefinition {
        id: def_id.clone(),
        card_id: Id::from_str("card-1"),
        name: "存在角色".into(),
        persona_prompt: "你是存在的".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: default_character_variables(),
    };

    let mut inst = CharacterInstance::from_definition(campaign_id.clone(), &def);
    // 故意把 definition_id 指向不存在的 def
    inst.definition_id = Some(orphan_def_id.clone());

    let issue = HealthIssue {
        severity: IssueSeverity::Error,
        category: "orphan_instance".into(),
        message: "实例指向不存在的定义".into(),
        affected_id: Some(inst.id.to_string()),
    };

    let defs = vec![def];
    let instances = vec![inst.clone()];
    let knowledge = vec![];
    let tasks = vec![];

    let input = PreviewInput {
        instances: &instances,
        definitions: &defs,
        knowledge: &knowledge,
        tasks: &tasks,
        campaign: None,
    };

    // build_patch_for_issue → 应产生 TypedPatch
    let patch = build_patch_for_issue(&issue, &input).expect("应产生 patch");
    assert_eq!(patch.source_issue_category, "orphan_instance");
    assert!(matches!(patch.status, TypedPatchStatus::Pending));

    // is_patch_stale → false（目标还在）
    assert!(!is_patch_stale(&patch, &input));

    // apply_to_snapshot
    let mut insts = instances.clone();
    let mut defs_mut = defs.clone();
    let mut know_mut = knowledge.clone();
    let mut task_mut = tasks.clone();
    let mut snapshot = PreviewInputMut {
        instances: &mut insts,
        definitions: &mut defs_mut,
        knowledge: &mut know_mut,
        tasks: &mut task_mut,
        campaign: None,
        turn: 1,
    };
    apply_to_snapshot(&patch, &mut snapshot).expect("apply 应成功");

    // orphan_instance patch 清除断裂的 definition_id（设为 None）
    assert_eq!(
        snapshot.instances[0].definition_id, None,
        "orphan instance 的 definition_id 应被清为 None"
    );
}

/// C6：explain_generation（确定性）
#[test]
fn c6_explain_generation() {
    use storyforge_app_meta::explain_generation;
    use storyforge_domain::Id;
    use storyforge_domain::agent::{ContextPackage, Plan, SubagentTask};
    use storyforge_domain::conversation::{Provenance, SubagentSnapshot};

    let provenance = Provenance {
        session_id: Id::from_str("session-1"),
        plan: Some(Plan {
            scene_brief: "废弃仓库中，月光透过破碎的窗户".into(),
            subagent_tasks: vec![SubagentTask {
                character_id: "lin".into(),
                brief: "Lin 检查伤员".into(),
                context_package: ContextPackage {
                    character_brief: "你是外科医生".into(),
                    scene_brief: "废弃仓库".into(),
                    relevant_lore: vec![],
                    constant_lore: vec![],
                    recent_window: vec![],
                    task: "检查伤员".into(),
                },
                current_desire: None,
                ongoing_action: None,
                emotion_stage: None,
            }],
            scene_plan: None,
        }),
        subagent_results: vec![SubagentSnapshot {
            character_id: "lin".into(),
            full_text: "Lin 蹲下检查伤员的脉搏，眉头微皱。".into(),
            character_instance_id: Some("inst-lin".into()),
            display_name: Some("林医生".into()),
            fallback_reason: None,
            reasoning_content: Some("lin reasoning".into()),
        }],
        profile_id: None,
        seed: 42,
        last_hint: None,
        director_reasoning: Some("director reasoning".into()),
        editor_reasoning: Some("editor reasoning".into()),
    };

    let explanation = explain_generation(&provenance);

    assert_eq!(explanation.seed, 42);
    assert!(explanation.scene_brief.is_some());
    assert_eq!(explanation.subagents.len(), 1);
    assert_eq!(explanation.subagents[0].display_name, "林医生");
    assert!(!explanation.subagents[0].output_preview.is_empty());
}

// ─── C7：MVU 流 ─────────────────────────────────────────────────────────────

/// C7：analyze_mvu_card 真实 LLM（#[ignore]）
#[tokio::test]
#[ignore = "需要真实 LLM 凭证"]
async fn c7_mvu_analyze_real_llm() {
    use storyforge_app_agent::runtime::AgentRuntime;
    use storyforge_app_agent::tools::ToolContext;
    use storyforge_app_meta::analyze_mvu_card;

    use storyforge_domain::mvu_translation::MvuRouting;

    let llm = harness_real_llm::require_real_llm();
    let tool_ctx = Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    });
    let runtime = AgentRuntime::new(llm, tool_ctx);

    // 用 seraphina 卡（有 JS extensions，应走 Hybrid 或至少不 panic）
    let card_path = find_fixture("test-card-seraphina.png");
    let bytes = std::fs::read(&card_path)
        .unwrap_or_else(|e| panic!("读不到 fixture {}: {e}", card_path.display()));
    let character = storyforge_infra_import::import_character(&bytes).expect("导入失败");

    let (_tx, cancel) = tokio::sync::watch::channel(false);
    let translation = analyze_mvu_card(&runtime, &character, cancel)
        .await
        .expect("analyze_mvu_card 应成功");

    // 至少不返回空壳（pure_data_fallback 的 confidence=0.0）
    // seraphina 卡有 JS，不应纯降级
    eprintln!(
        "[C7] routing={:?}, confidence={}, fields={}, bindings={}",
        translation.routing,
        translation.analysis_confidence,
        translation.variable_schema.len(),
        translation.ui_bindings.len()
    );

    // 基本断言：不 panic、routing 有值
    assert!(
        translation.routing == MvuRouting::Native
            || matches!(translation.routing, MvuRouting::Hybrid { .. }),
        "routing 应为 Native 或 Hybrid"
    );
}

/// C7：MVU Translation CRUD（确定性）
#[test]
fn c7_mvu_translation_crud() {
    use storyforge_domain::mvu_translation::MvuTranslation;
    use storyforge_domain::variables::{VariableField, VariableType};

    let store = &HarnessEnv::new(Arc::new(MockLlmClient::with_defaults())).campaign_store;

    // 空 store 无 MVU
    assert!(store.list_all_mvu().is_empty());

    // 构造一个 MvuTranslation
    let schema = vec![VariableField {
        key: "hp".into(),
        label: "HP".into(),
        value_type: VariableType::Int,
        default: serde_json::json!(100),
        description: None,
        group: None,
    }];
    let translation = MvuTranslation::pure_data_fallback(schema.clone());
    let src_id = Id::from_str("mvu-src-1");

    // save
    store
        .save_mvu(storyforge_tauri_app::campaign_store::StoredMvuTranslation {
            source_character_id: src_id.clone(),
            character_name: "Seraphina".into(),
            translation: translation.clone(),
            analyzed_at: "2026-06-18T00:00:00Z".into(),
        })
        .unwrap();

    // get
    let got = store.get_mvu(&src_id).expect("应能取回");
    assert_eq!(got.character_name, "Seraphina");
    assert_eq!(got.translation.variable_schema.len(), 1);
    assert_eq!(got.translation.variable_schema[0].key, "hp");

    // list
    assert_eq!(store.list_all_mvu().len(), 1);

    // delete
    assert!(store.delete_mvu(&src_id).unwrap());
    assert!(store.get_mvu(&src_id).is_none());
    assert!(!store.delete_mvu(&src_id).unwrap()); // 再删返回 false
}

/// C7：MVU apply + backfill（确定性，复刻 lib.rs:3822-3845 backfill loop）
#[test]
fn c7_mvu_apply_backfill() {
    use storyforge_app_meta::{apply_schema_to_definition, compute_apply_preview};

    use storyforge_domain::character::{CharacterDefinition, RoleType};
    use storyforge_domain::variables::{VariableField, VariableType, VariableValue};

    let env = HarnessEnv::new(Arc::new(MockLlmClient::with_defaults()));
    let store = &env.campaign_store;

    // 建 definition（schema 含 hp）
    let def_id = Id::from_str("def-backfill");
    let mut def = CharacterDefinition {
        id: def_id.clone(),
        card_id: Id::from_str("card-bf"),
        name: "Seraphina".into(),
        persona_prompt: "你是守护者".into(),
        behavior_rules: String::new(),
        base_backstory: vec![],
        group: None,
        role_type: RoleType::Protagonist,
        variable_schema: vec![VariableField {
            key: "hp".into(),
            label: "HP".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(100),
            description: None,
            group: None,
        }],
    };

    // 建 card + campaign + instance
    let card = {
        let mut c = make_minimal_card("bf-card");
        c.character_definitions = vec![def.clone()];
        c
    };
    store.save_card(card.clone()).unwrap();
    let campaign_id = env.create_campaign(&card, "bf-campaign");

    // 确认 instance 已创建，definition_id 指向 def
    let instances = store.list_instances(&campaign_id);
    assert_eq!(instances.len(), 1);
    assert_eq!(instances[0].definition_id, Some(def_id.clone()));

    // 构造 MVU schema：hp 覆盖 + 新增 mp
    let mvu_schema = vec![
        VariableField {
            key: "hp".into(),
            label: "HP".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(200), // 覆盖
            description: None,
            group: None,
        },
        VariableField {
            key: "mp".into(),
            label: "MP".into(),
            value_type: VariableType::Int,
            default: serde_json::json!(50), // 新增
            description: None,
            group: None,
        },
    ];

    // compute_apply_preview
    let preview = compute_apply_preview(
        &def.variable_schema,
        &mvu_schema,
        def.id.as_str(),
        &def.name,
        "src-seraphina",
    );
    assert!(preview.has_changes, "应有变化");
    assert!(
        preview.added_fields.iter().any(|f| f.key == "mp"),
        "added_fields 应含 mp"
    );

    // apply_schema_to_definition
    apply_schema_to_definition(&mut def, preview.merged_schema.clone());
    assert!(
        def.variable_schema.iter().any(|f| f.key == "mp"),
        "definition schema 应含 mp"
    );
    store
        .update_card({
            let mut c = card.clone();
            c.character_definitions = vec![def.clone()];
            c
        })
        .unwrap();

    // ─── 复刻 backfill loop（lib.rs meta_apply_mvu_schema）─────────────
    // 生产用 list_all_instances() 全量遍历 + definition_id 过滤（一张卡的
    // definition 可被多个 campaign 引用，全部都该 backfill；详见 lib.rs 注释）。
    // 测试同步复刻，确保验证的路径 = 生产实际走的路径。
    for instance in store.list_all_instances() {
        if instance.definition_id.as_ref() != Some(&def_id) {
            continue;
        }
        let mut updated = instance.clone();
        let new_fields: Vec<_> = preview
            .added_fields
            .iter()
            .filter(|f| !updated.variables.iter().any(|v| v.key == f.key))
            .collect();
        if new_fields.is_empty() {
            continue;
        }
        for field in new_fields {
            updated.variables.push(VariableValue::new(
                field.key.clone(),
                field.default.clone(),
                0,
            ));
        }
        store.update_instance(updated).unwrap();
    }

    // ─── 断言 ──────────────────────────────────────────────────────────
    let inst = store
        .list_instances(&campaign_id)
        .into_iter()
        .next()
        .unwrap();
    let mp = inst.get_variable("mp");
    assert!(mp.is_some(), "instance 应含新增的 mp 变量");
    assert_eq!(mp.unwrap(), &serde_json::json!(50), "mp default 应为 50");

    // 不含重复 key
    let keys: Vec<_> = inst.variables.iter().map(|v| &v.key).collect();
    let unique_len = keys.iter().collect::<std::collections::HashSet<_>>().len();
    assert_eq!(keys.len(), unique_len, "不应有重复 key");
}

// ─── 辅助函数 ──────────────────────────────────────────────────────────────

fn make_minimal_card(name: &str) -> CharacterCard {
    use storyforge_domain::Source;
    use storyforge_domain::character::Character;
    let ch = Character {
        id: Id::from_str(format!("{name}-src")),
        name: name.into(),
        description: "测试角色".into(),
        personality: "冷静".into(),
        scenario: String::new(),
        first_mes: "你好".into(),
        mes_example: String::new(),
        system_prompt: String::new(),
        post_history_instructions: String::new(),
        tags: vec![],
        creator: String::new(),
        character_version: String::new(),
        alternate_greetings: vec![],
        embedded_world_info: None,
        extensions: serde_json::Value::Null,
        renderable_assets: None,
        source: Source::Native,
        spec_version: "3.0".into(),
        raw_card_json: serde_json::Value::Null,
    };
    let mut card = CharacterCard::from_character(&ch);
    let def = CharacterDefinition::fallback_from_character(&ch, &[]);
    card.character_definitions = vec![def];
    card
}

fn find_fixture(name: &str) -> std::path::PathBuf {
    let env_key = format!(
        "STORYFORGE_FIXTURE_{}",
        name.trim_end_matches(".png")
            .to_uppercase()
            .replace('-', "_")
    );
    if let Ok(p) = std::env::var(&env_key) {
        let p = std::path::PathBuf::from(p);
        if p.exists() {
            return p;
        }
    }
    let mut dir = std::env::current_dir().expect("无法获取 cwd");
    loop {
        let candidate = dir.join(name);
        if candidate.exists() {
            return candidate;
        }
        dir = match dir.parent() {
            Some(p) => p.to_path_buf(),
            None => return std::path::PathBuf::from(name),
        };
    }
}
