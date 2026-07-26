//! 真实 LLM 知识传播/封口评测。
//!
//! 默认 `#[ignore]`，普通 `cargo test` 不联网、不花钱。发布前有真实凭证时运行：
//! `cargo test -p harness-real-llm knowledge_propagation -- --ignored --nocapture`
//!
//! 目标：验证 PostProcessor 在真实模型下能抽出定向告知、身份组广播和 private
//! 封口；再把抽取结果走线上 normalize 写回函数，确认传话链和封口门禁落盘行为。

use std::collections::HashSet;
use std::sync::Arc;

use storyforge_app_agent::AgentRuntime;
use storyforge_app_agent::ToolContext;
use storyforge_app_agent::postprocess::run_postprocess;
use storyforge_domain::Id;
use storyforge_domain::agent::AgentRole;
use storyforge_domain::agent_profile_config::{AgentRunConfig, default_agent_profile_config};
use storyforge_domain::campaign::{Campaign, CharacterInstance};
use storyforge_domain::character::{
    CharacterCard, CharacterDefinition, CharacterExtractionStatus, RoleType,
};
use storyforge_domain::character_knowledge::{
    BroadcastTarget, CharacterKnowledgeEntry, CharacterKnowledgeUpdate, KnowledgeSource,
    PropagationPolicy,
};
use storyforge_domain::prompt_module::ProfileSource;
use storyforge_tauri_app::campaign_store::CampaignStore;
use storyforge_tauri_app::normalize_knowledge_update_for_postprocess;
use tokio::sync::watch;

use harness_real_llm::resolve_llm_connection;

fn empty_tool_context() -> Arc<ToolContext> {
    Arc::new(ToolContext {
        characters: vec![],
        world_info: None,
        vector_store: None,
        archived_summaries: vec![],
        chronicle_summaries: vec![],
        chronicle_tool_budget: std::sync::Arc::new(storyforge_app_agent::ChronicleToolBudget::new()),
        campaign_runtime: None,
        current_character_instance_id: None,
        regex_scripts: vec![],
    })
}

fn postprocess_profile_for_model(
    model: &str,
) -> storyforge_domain::agent_profile_config::AgentProfileConfig {
    let mut cfg = default_agent_profile_config();
    cfg.name = "knowledge-propagation-real-llm".into();
    cfg.source = ProfileSource::UserCreated;
    cfg.agent_configs.insert(
        AgentRole::PostProcessor,
        AgentRunConfig {
            model_override: Some(model.to_string()),
            max_tool_rounds: Some(6),
            tool_whitelist: None,
        },
    );
    cfg
}

fn make_campaign_store() -> (std::path::PathBuf, CampaignStore, Id) {
    let dir = std::env::temp_dir().join(format!(
        "storyforge_knowledge_propagation_{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let store = CampaignStore::new(&dir);

    let card_id = Id::from_str("card-knowledge-propagation");
    let source_character_id = Id::from_str("source-card");
    let definitions = vec![
        CharacterDefinition {
            id: Id::from_str("def-lin"),
            card_id: card_id.clone(),
            name: "林医生".into(),
            persona_prompt: "外科医生".into(),
            behavior_rules: "谨慎保密".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Protagonist,
            variable_schema: vec![],
        },
        CharacterDefinition {
            id: Id::from_str("def-chen"),
            card_id: card_id.clone(),
            name: "陈警官".into(),
            persona_prompt: "刑警".into(),
            behavior_rules: "追查真相".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        },
        CharacterDefinition {
            id: Id::from_str("def-lord"),
            card_id: card_id.clone(),
            name: "城主".into(),
            persona_prompt: "城主".into(),
            behavior_rules: "发布命令".into(),
            base_backstory: vec![],
            group: None,
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        },
        CharacterDefinition {
            id: Id::from_str("def-guard"),
            card_id: card_id.clone(),
            name: "守卫甲".into(),
            persona_prompt: "守卫".into(),
            behavior_rules: "听从命令".into(),
            base_backstory: vec![],
            group: Some("守卫".into()),
            role_type: RoleType::Supporting,
            variable_schema: vec![],
        },
    ];
    let card = CharacterCard {
        id: card_id.clone(),
        name: "knowledge-propagation-card".into(),
        source_character_id,
        character_definitions: definitions,
        raw_card_json: serde_json::Value::Null,
        extraction_status: CharacterExtractionStatus::Extracted,
        extraction_message: None,
    };
    store.save_card(card).unwrap();

    let campaign = Campaign::new(card_id, "knowledge-propagation");
    let campaign_id = campaign.id.clone();
    store.save_campaign(campaign).unwrap();

    let instances = [
        ("inst-lin", "林医生", "def-lin"),
        ("inst-chen", "陈警官", "def-chen"),
        ("inst-lord", "城主", "def-lord"),
        ("inst-guard", "守卫甲", "def-guard"),
    ];
    for (id, name, def_id) in instances {
        let mut inst = CharacterInstance::temporary(campaign_id.clone(), name);
        inst.id = Id::from_str(id);
        inst.definition_id = Some(Id::from_str(def_id));
        store.add_instance(inst).unwrap();
    }

    (dir, store, campaign_id)
}

fn find_update<'a>(
    updates: &'a [CharacterKnowledgeUpdate],
    predicate: impl Fn(&'a CharacterKnowledgeUpdate) -> bool,
) -> &'a CharacterKnowledgeUpdate {
    updates
        .iter()
        .find(|update| predicate(update))
        .unwrap_or_else(|| {
            panic!(
                "未找到期望知识更新；实际为：{}",
                serde_json::to_string_pretty(updates).unwrap()
            )
        })
}

#[tokio::test]
#[ignore = "需要真实 LLM 凭证（LLM_BASE_URL/API_KEY/MODEL 或 data/connections.json）"]
async fn knowledge_propagation_postprocess_and_writeback_real_llm() {
    let conn = resolve_llm_connection().expect("需要真实 LLM 凭证");
    let client: Arc<dyn storyforge_infra_llm::LlmClient> =
        Arc::from(storyforge_infra_llm::create_client(&conn).expect("LLM client 构造失败"));
    let runtime = AgentRuntime::new(client, empty_tool_context());
    let profile = postprocess_profile_for_model(&conn.model);

    let final_text = r#"
第 7 轮。
林医生在医院走廊低声告诉陈警官：“地下室有尸体。”
城主随后发布命令：“所有守卫都必须知道，北门封锁。”
林医生独自看到保险柜密码是 0427，并明确说：这件事是秘密，禁止外传。
"#;

    let (_cancel_tx, cancel_rx) = watch::channel(false);
    let result = run_postprocess(
        &runtime,
        final_text,
        &["林医生".into(), "陈警官".into(), "城主".into()],
        &["story_clock".into()],
        7,
        "第7轮",
        cancel_rx,
        Some(&profile),
        None,
        &[],
    )
    .await
    .expect("postprocess 真实 LLM 调用失败");

    assert!(
        result.parse_succeeded,
        "postprocess 应能被 parser 解析；实际知识更新：{}",
        serde_json::to_string_pretty(&result.knowledge_updates).unwrap()
    );
    eprintln!(
        "knowledge_updates = {}",
        serde_json::to_string_pretty(&result.knowledge_updates).unwrap()
    );

    let told = find_update(&result.knowledge_updates, |update| {
        update.source == KnowledgeSource::ToldByOther
            && update.character_id.as_str() == "陈警官"
            && update
                .source_character_id
                .as_ref()
                .is_some_and(|id| id.as_str() == "林医生")
            && update.knowledge_text.contains("地下室")
    });
    // 原文「所有守卫都必须知道,北门封锁」可被 LLM 合理映射为 Group("守卫")
    // (定向身份组)或 All(全体广播)——两者语义都成立,代码路径都已完整测过
    // (见 character_knowledge.rs serde 测试 + lib.rs BroadcastTarget::All 分发)。
    // 这里只验证「北门封锁知识被广播出去」,具体形态不锁死,避免 LLM 非确定性误伤。
    let broadcast = find_update(&result.knowledge_updates, |update| {
        (match update.broadcast.as_ref() {
            Some(BroadcastTarget::All) => true,
            Some(BroadcastTarget::Group(group)) => group == "守卫",
            None => false,
        }) && update.knowledge_text.contains("北门")
    });
    let private = find_update(&result.knowledge_updates, |update| {
        update.propagation == PropagationPolicy::Private && update.knowledge_text.contains("0427")
    });
    assert!(
        private.broadcast.is_none(),
        "private 知识不得同时带 broadcast"
    );

    let (dir, store, campaign_id) = make_campaign_store();
    let present_ids = HashSet::from([
        String::from("林医生"),
        String::from("陈警官"),
        String::from("城主"),
    ]);
    let name_collisions = HashSet::new();

    let source_entry = CharacterKnowledgeEntry::witnessed(
        campaign_id.clone(),
        Id::from_str("inst-lin"),
        told.knowledge_text.clone(),
        6,
    );
    let source_entry_id = source_entry.id.clone();
    store.add_knowledge(vec![source_entry]).unwrap();

    let told_entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_id,
        told,
        7,
        &present_ids,
        &name_collisions,
    );
    assert_eq!(told_entries.len(), 1);
    assert_eq!(told_entries[0].character_id, Id::from_str("inst-chen"));
    assert_eq!(
        told_entries[0].source_knowledge_id,
        Some(source_entry_id),
        "真实 LLM 抽取的定向告知应能链接到来源角色已有知识"
    );

    let broadcast_entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_id,
        broadcast,
        7,
        &present_ids,
        &name_collisions,
    );
    // Group("守卫") 只分发给守卫甲;All 会分发给除发起者(城主)外所有 instance,
    // 即林医生+陈警官+守卫甲。两种合法形态都至少包含守卫甲,断言守住这个下界即可。
    assert!(
        broadcast_entries
            .iter()
            .any(|entry| entry.character_id == Id::from_str("inst-guard")),
        "广播至少应分发给守卫组实例(接受 Group 或 All)"
    );

    let private_entries = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_id,
        private,
        7,
        &present_ids,
        &name_collisions,
    );
    assert_eq!(private_entries.len(), 1);
    assert_eq!(private_entries[0].propagation, PropagationPolicy::Private);
    store.add_knowledge(private_entries.clone()).unwrap();

    let forbidden_relay = CharacterKnowledgeUpdate {
        character_id: Id::from_str("陈警官"),
        knowledge_text: private_entries[0].knowledge_text.clone(),
        source: KnowledgeSource::ToldByOther,
        source_character_id: Some(Id::from_str("林医生")),
        pinned: false,
        broadcast: None,
        propagation: PropagationPolicy::Open,
    };
    let blocked = normalize_knowledge_update_for_postprocess(
        &store,
        &campaign_id,
        &forbidden_relay,
        8,
        &present_ids,
        &name_collisions,
    );
    assert!(
        blocked.is_empty(),
        "private 来源知识不得经真实 LLM 后续告知路径继续传播"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
