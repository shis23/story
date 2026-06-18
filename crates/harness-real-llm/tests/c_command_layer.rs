//! C1-C8 命令层「点遍各按钮」测试（混合：确定性 + 真实 LLM）。
//!
//! 绕开前端，调遍前端会调的 Tauri 命令对应的底层逻辑。
//! 确定性项直接跑，真实 LLM 项 `#[ignore]`。

use std::sync::Arc;

use storyforge_domain::character::{CharacterCard, CharacterDefinition};
use storyforge_domain::character_knowledge::{CharacterKnowledgeEntry, KnowledgeSource};
use storyforge_domain::story_task::{StoryTask, TaskSource, TaskStatus};
use storyforge_domain::Id;
use storyforge_infra_llm::mock_client::MockLlmClient;
use storyforge_infra_llm::LlmClient;

use harness_real_llm::HarnessEnv;

// ─── C1：导入 / 识别 ──────────────────────────────────────────────────────

/// C1：import_character → inject → 验证 tool_ctx 中有角色
#[test]
fn c1_import_and_list_characters() {
    let llm: Arc<dyn LlmClient> = Arc::new(MockLlmClient::with_defaults());
    let env = HarnessEnv::new(llm);

    use storyforge_domain::character::Character;
    use storyforge_domain::Source;
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
    let bytes = std::fs::read(&card_path).unwrap_or_else(|e| {
        panic!("读不到 fixture {}: {e}", card_path.display())
    });
    let character = storyforge_infra_import::import_character(&bytes).expect("导入失败");
    let source_id = character.id.clone();
    env.inject_character(character);

    let card = env.extract_characters(source_id.as_str()).await;
    assert!(!card.character_definitions.is_empty(), "应识别出至少 1 个角色");

    // list_cards
    let cards = env.campaign_store.list_cards();
    assert!(cards.iter().any(|c| c.card.id == card.id), "list_cards 应含此卡");

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
    env.campaign_store.save_card(card.clone());
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
    env.campaign_store.save_card(card.clone());
    let campaign_id = env.create_campaign(&card, "c2-var-campaign");
    let instances = env.campaign_store.list_instances(&campaign_id);
    let inst_id = instances[0].id.clone();

    // set_variable (key, serde_json::Value, turn)
    {
        let mut inst = env.campaign_store.get_instance(&campaign_id, &inst_id).unwrap();
        inst.set_variable("mood", serde_json::json!("calm"), 1);
        env.campaign_store.update_instance(inst);
    }

    let inst = env.campaign_store.get_instance(&campaign_id, &inst_id).unwrap();
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
    env.campaign_store.save_card(card.clone());
    let campaign_id = env.create_campaign(&card, "c2-camp-var");

    {
        let mut camp = env.campaign_store.get_campaign(&campaign_id).unwrap();
        camp.set_variable("weather", serde_json::json!("rainy"), 1);
        env.campaign_store.update_campaign(camp);
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
    env.campaign_store.save_card(card.clone());
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
    env.campaign_store.add_task(task.clone());

    let tasks = env.campaign_store.list_tasks(&campaign_id);
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "找到线索");
    assert!(matches!(tasks[0].status, TaskStatus::Pending));

    // update task status
    let mut updated = tasks[0].clone();
    updated.status = TaskStatus::Completed;
    env.campaign_store.update_task(updated);

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
    env.campaign_store.save_card(card.clone());
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
    env.campaign_store.save_card(card.clone());
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
        turn_number: 0,
        event_id: None,
        pinned: false,
    };
    env.campaign_store.add_knowledge(vec![entry]);

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
    let conv = env.conv_store.create(None);
    let conv_id = conv.id.clone();
    assert!(env.conv_store.get(&conv_id).is_some());

    // append a user message + ai draft to create nodes
    let _user_node_id = env.conv_store.append_user_message(&conv_id, "你好".into()).unwrap();
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
    env.conv_store.switch_variant(&conv_id, &ai_node_id, 0).unwrap();
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

// ─── 辅助函数 ──────────────────────────────────────────────────────────────

fn make_minimal_card(name: &str) -> CharacterCard {
    use storyforge_domain::character::Character;
    use storyforge_domain::Source;
    let ch = Character {
        id: Id::from_str(&format!("{name}-src")),
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
        name.trim_end_matches(".png").to_uppercase().replace('-', "_")
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
